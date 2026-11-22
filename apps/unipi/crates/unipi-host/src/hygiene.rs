//! Conservative, best-effort host resource retention. Unknown or busy files stay put.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const DAY: Duration = Duration::from_hours(24);

/// Regular files only: never traverse a symlink, even at the root.
fn files(root: &Path, recursive: bool, out: &mut Vec<(PathBuf, std::fs::Metadata)>) {
    if !std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !m.is_symlink()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_file() {
            out.push((path, meta));
        } else if recursive && meta.is_dir() {
            files(&path, true, out);
        }
    }
}

/// Fail closed if procfs itself cannot be inspected. An open descriptor or mapped executable
/// counts as in use (including another host's). No process is signalled here.
fn in_use(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(meta) = std::fs::metadata(path) else {
            return true;
        };
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return true;
        };
        for proc in procs.flatten() {
            if proc.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            let fd_dir = proc.path().join("fd");
            // Non-dumpable sandbox/setuid processes deny fd inspection even to their
            // owner. They cannot be inspected; check all accessible processes.
            let Ok(fds) = std::fs::read_dir(&fd_dir) else {
                continue;
            };
            for fd in fds.flatten() {
                if std::fs::metadata(fd.path())
                    .is_ok_and(|m| m.dev() == meta.dev() && m.ino() == meta.ino())
                {
                    return true;
                }
            }
            if std::fs::read_link(proc.path().join("exe")).is_ok_and(|p| p == path) {
                return true;
            }
            if std::fs::read_to_string(proc.path().join("maps")).is_ok_and(|m| {
                m.lines()
                    .any(|line| line.ends_with(path.to_string_lossy().as_ref()))
            }) {
                return true;
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        true
    }
}

pub fn prune_tmp(root: &Path, now: SystemTime) {
    let mut entries = Vec::new();
    files(root, true, &mut entries);
    for (path, meta) in entries {
        if meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > DAY)
            && !in_use(&path)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Evict least-recently-modified cache files, never live binaries or symlinks.
/// `protected` is the browser installation about to launch. A live/selected build may
/// exceed the soft cap by itself; deleting it would break the browser.
#[must_use]
pub fn cap_cache(root: &Path, protected: &Path, cap: u64) -> u64 {
    let mut entries = Vec::new();
    files(root, true, &mut entries);
    entries.sort_by_key(|(_, m)| m.modified().unwrap_or(SystemTime::UNIX_EPOCH));
    let mut total: u64 = entries.iter().map(|(_, m)| m.len()).sum();
    for (path, meta) in entries {
        if total <= cap {
            break;
        }
        if !path.starts_with(protected) && !in_use(&path) && std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(meta.len());
        }
    }
    total
}

/// An explicit trash timestamp, separate from session mtime (last conversation activity).
/// Legacy trash without this marker is retained rather than guessing its archive date.
pub fn mark_trashed(path: &Path) {
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = crate::data_dir::write_atomic(
        &path.with_extension("trashed-at"),
        stamp.to_string().as_bytes(),
    );
}

pub fn prune_trash(root: &Path, now: SystemTime) {
    let mut entries = Vec::new();
    files(root, false, &mut entries);
    for (path, _) in entries {
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let marker = path.with_extension("trashed-at");
        if !std::fs::symlink_metadata(&marker).is_ok_and(|m| m.is_file() && !m.is_symlink()) {
            continue;
        }
        let stamp = std::fs::read_to_string(&marker)
            .ok()
            .and_then(|s| s.parse::<u64>().ok());
        let expired = stamp
            .and_then(|s| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(s)))
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > DAY * 90);
        if !expired || in_use(&path) {
            continue;
        }
        let holder =
            oino_session::manager::LeaseInfo::for_process("trash retention", &root.join("state"));
        if let Ok(lease) = oino_session::manager::SessionLease::acquire(&path, &holder) {
            if std::fs::remove_file(&path).is_ok() {
                let _ = std::fs::remove_file(marker);
            }
            lease.release();
        }
    }
}

/// Host-owned children have an independent process group on Unix and die with the
/// spawning thread on Linux. Check ppid after prctl to close the fork/prctl race.
pub fn own_child(command: &mut tokio::process::Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(target_os = "linux")]
    {
        let parent = std::process::id() as libc::pid_t;
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    libc::_exit(127);
                }
                Ok(())
            });
        }
    }
}

/// Record pid + kernel start time + executable identity to reject PID reuse.
pub fn record_display(host: &Path, pid: u32) {
    #[cfg(target_os = "linux")]
    if let Some((_, start, exe)) = process_identity(pid) {
        let dir = host.join("browser-pids");
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(
                dir.join(format!("{pid}.json")),
                serde_json::json!({"pid":pid,"start":start,"exe":exe}).to_string(),
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn process_identity(pid: u32) -> Option<(u32, String, PathBuf)> {
    let root = PathBuf::from(format!("/proc/{pid}"));
    let stat = std::fs::read_to_string(root.join("stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    Some((
        fields.get(1)?.parse().ok()?,
        fields.get(19)?.to_string(),
        std::fs::read_link(root.join("exe")).ok()?,
    ))
}

/// Only recorded, identity-matched Xvfb children now adopted by init. Never scan
/// and kill arbitrary X servers or act on a symlinked pid directory/file.
pub fn reap_displays(host: &Path) {
    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        files(&host.join("browser-pids"), false, &mut entries);
        for (path, _) in entries {
            let Some(record) = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            else {
                continue;
            };
            let Some(pid) = record["pid"].as_u64().and_then(|p| u32::try_from(p).ok()) else {
                continue;
            };
            let Some((parent, start, exe)) = process_identity(pid) else {
                let _ = std::fs::remove_file(path);
                continue;
            };
            if parent == 1
                && record["start"].as_str() == Some(&start)
                && record["exe"].as_str() == exe.to_str()
                && exe.file_name().is_some_and(|n| n == "Xvfb")
            {
                // Recheck immediately before signalling. pidfd avoids reuse between check/kill.
                let fd = unsafe {
                    i32::try_from(libc::syscall(libc::SYS_pidfd_open, pid, 0)).unwrap_or(-1)
                };
                if fd >= 0 {
                    if process_identity(pid)
                        .is_some_and(|(p, s, e)| p == 1 && s == start && e == exe)
                    {
                        unsafe {
                            libc::syscall(
                                libc::SYS_pidfd_send_signal,
                                fd,
                                libc::SIGKILL,
                                std::ptr::null::<libc::siginfo_t>(),
                                0,
                            );
                        }
                    }
                    unsafe {
                        libc::close(fd);
                    }
                }
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_requires_an_expired_trash_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["old", "recent", "legacy", "future"] {
            std::fs::write(dir.path().join(format!("{name}.jsonl")), "session").unwrap();
        }
        std::fs::write(dir.path().join("old.trashed-at"), "1").unwrap();
        std::fs::write(
            dir.path().join("recent.trashed-at"),
            (DAY.as_secs() * 99).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("future.trashed-at"),
            (DAY.as_secs() * 101).to_string(),
        )
        .unwrap();
        prune_trash(dir.path(), SystemTime::UNIX_EPOCH + DAY * 100);
        assert!(!dir.path().join("old.jsonl").exists());
        for name in ["recent", "legacy", "future"] {
            assert!(dir.path().join(format!("{name}.jsonl")).exists());
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn pruning_skips_open_files_and_symlinks_and_evicts_oldest() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        let new = dir.path().join("new");
        std::fs::write(&old, "12345").unwrap();
        std::fs::write(&new, "12345").unwrap();
        let file = std::fs::File::open(&old).unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        symlink(outside.path(), dir.path().join("linked-dir")).unwrap();
        std::fs::write(outside.path().join("keep"), "keep").unwrap();
        prune_tmp(dir.path(), SystemTime::now());
        assert!(old.exists());
        drop(file);
        assert_eq!(cap_cache(dir.path(), &new, 5), 5);
        assert!(!old.exists());
        assert!(new.exists());
        assert!(outside.path().join("keep").exists());
        prune_tmp(dir.path(), SystemTime::now() + DAY * 2);
        assert!(!new.exists());
    }
}
