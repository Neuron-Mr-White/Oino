//! D-LOCK lease integration tests (spec core.md §2.1 D-LOCK): in-process single-writer
//! enforcement, advisory metadata, and kernel release on process death.
//!
//! Temp dirs live under `CARGO_TARGET_TMPDIR` (repo convention) so the re-executed child
//! process below sees the same paths.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use oino_session::{
    LeaseInfo, SessionLease, SessionLockError, lock_metadata_path, read_takeover_request,
    write_takeover_request,
};

mod common;

const CHILD_FILE_ENV: &str = "OINO_LEASE_CHILD_FILE";
const CHILD_STATE_ENV: &str = "OINO_LEASE_CHILD_STATE";

fn info(client: &str) -> LeaseInfo {
    LeaseInfo::for_process(client, common::tempdir().path())
}

/// A session file path in a fresh temp dir (the session file itself never needs to exist).
fn session_file(tag: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = common::tempdir();
    let file = dir.path().join(format!("{tag}.jsonl"));
    (dir, file)
}

/// Acquire, hold, release: metadata appears and disappears with the lease.
#[test]
fn acquire_release_roundtrip() {
    let (_dir, file) = session_file("roundtrip");
    let holder = info("oino test");
    let lease = SessionLease::acquire(&file, &holder).unwrap();
    assert_eq!(
        lease.session_file().file_name(),
        file.file_name(),
        "the lease names the resolved session file"
    );
    assert_eq!(lease.info(), &holder);
    let metadata = std::fs::read_to_string(lock_metadata_path(&file)).unwrap();
    assert!(
        metadata.contains(&format!("\"pid\":{}", std::process::id())),
        "{metadata}"
    );
    assert!(metadata.contains("\"bootId\""), "{metadata}");
    assert!(metadata.contains("\"client\":\"oino test\""), "{metadata}");
    lease.release();
    assert!(
        !lock_metadata_path(&file).exists(),
        "metadata removed on release"
    );
    // Releasing makes the file acquirable again.
    SessionLease::acquire(&file, &holder).unwrap().release();
}

/// A second acquire in the same process fails fast, naming the caller's own process
/// (the in-process registry: `flock` alone would let one process lock twice).
#[test]
fn second_acquire_in_process_fails_fast() {
    let (_dir, file) = session_file("double");
    let holder = info("oino test");
    let first = SessionLease::acquire(&file, &holder).unwrap();
    match SessionLease::acquire(&file, &info("oino other")) {
        Err(SessionLockError::HeldBy(info)) => {
            assert_eq!(
                info.pid,
                std::process::id(),
                "names our own process: {info:?}"
            );
            assert_eq!(info.client, "oino test");
        }
        other => panic!("expected HeldBy, got {other:?}"),
    }
    drop(first);
    SessionLease::acquire(&file, &info("oino other")).unwrap();
}

/// Different sessions never contend.
#[test]
fn independent_sessions_lock_independently() {
    let (_dir, file_a) = session_file("a");
    let (_dir_b, file_b) = session_file("b");
    let holder = info("oino test");
    let a = SessionLease::acquire(&file_a, &holder).unwrap();
    let b = SessionLease::acquire(&file_b, &holder).unwrap();
    drop(a);
    drop(b);
}

/// Waits until `marker` exists under `dir` (child-process handshake, C1 pattern).
fn wait_for_marker(dir: &Path, marker: &str) {
    let marker = dir.join(marker);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(
            Instant::now() < deadline,
            "child never wrote {}",
            marker.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A killed holder releases the kernel lock: the successor acquires without any expiry
/// (D-LOCK: the kernel is the arbiter; there is no lease timeout to expire).
///
/// The child process is this test binary re-executed (`--exact`) so only
/// [`lease_holder_child`] runs; it acquires on the path in [`CHILD_FILE_ENV`], writes the
/// `locked` marker and parks.
#[cfg(unix)]
#[test]
fn kernel_lock_released_on_process_death() {
    let (dir, file) = session_file("killed");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lease_holder_child"])
        .env(CHILD_FILE_ENV, &file)
        .env(CHILD_STATE_ENV, dir.path().join("state"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_marker(dir.path(), "child.locked");
    // Held: our acquire fails and names the child.
    let holder = info("oino test");
    match SessionLease::acquire(&file, &holder) {
        Err(SessionLockError::HeldBy(info)) => assert_eq!(info.pid, child.id()),
        other => panic!("expected HeldBy, got {other:?}"),
    }
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap()),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    child.wait().unwrap();
    // SIGKILL: no release code ran; the kernel dropped the lock.
    let deadline = Instant::now() + Duration::from_secs(5);
    while SessionLease::acquire(&file, &holder).is_err() {
        assert!(
            Instant::now() < deadline,
            "kernel lock not released after SIGKILL"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Child half of [`kernel_lock_released_on_process_death`]: acquire, mark, park.
#[test]
fn lease_holder_child() {
    let Some(file) = std::env::var_os(CHILD_FILE_ENV) else {
        return; // normal test run: nothing to do
    };
    let state = std::env::var_os(CHILD_STATE_ENV).map_or_else(std::env::temp_dir, PathBuf::from);
    let holder = LeaseInfo::for_process("lease-child", &state);
    let file = PathBuf::from(file);
    let _lease = SessionLease::acquire(&file, &holder).expect("child acquires the lease");
    let _ = std::fs::write(file.with_file_name("child.locked"), "locked");
    std::thread::sleep(Duration::from_secs(60));
}

/// A takeover request left behind by a failed takeover attempt is readable metadata only —
/// acquiring is never blocked by it (the kernel lock arbitrates).
#[test]
fn takeover_request_does_not_block_acquire() {
    let (_dir, file) = session_file("request");
    let holder = info("oino test");
    write_takeover_request(&file, &holder).unwrap();
    assert_eq!(read_takeover_request(&file).unwrap().requester, holder);
    let lease = SessionLease::acquire(&file, &holder).unwrap();
    lease.release();
    // Release cleans the request (best-effort).
    assert!(read_takeover_request(&file).is_none());
}
