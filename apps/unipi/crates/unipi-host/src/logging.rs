//! Where the host's own logs go.
//!
//! Standalone `serve` writes stderr and a rotating `<data dir>/host.log`.
//! App-supervised hosts write stderr only: the supervisor pump owns that log
//! and rotation, avoiding two writers renaming each other's open files.
//! Rotation runs before open and during writes at 50 MiB, keeping three backups.

use std::io;
use std::path::PathBuf;

use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::data_dir::HostPaths;

/// The live log file's name (directly in the data dir, spec §5.8).
pub const LOG_FILE: &str = "host.log";
/// Level used when `RUST_LOG` is unset (the host's own lines at info, the
/// rest of the dependency stack quiet).
const DEFAULT_FILTER: &str = "info,unipi_host=info";

/// Rotation threshold of `host.log` (v3-perf T14: ≤ 50 MB × 3).
pub const ROTATE_BYTES: u64 = 50 * 1024 * 1024;
/// Rotated files kept: `host.log.1` … `host.log.3`.
pub const ROTATE_KEEP: u32 = 3;

/// Shifts `log` → `log.1` → … → `log.<keep>` (the oldest is dropped). Best-effort.
pub fn rotate(log: &std::path::Path, keep: u32) {
    let numbered = |n: u32| {
        let mut name = log.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    };
    // Inherited logs may predate rotation entirely. Dropping oversized files
    // avoids retaining unbounded archives (including existing backups).
    for path in std::iter::once(log.to_path_buf()).chain((1..=keep).map(numbered)) {
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > ROTATE_BYTES) {
            let _ = std::fs::remove_file(path);
        }
    }
    let _ = std::fs::remove_file(numbered(keep));
    for n in (1..keep).rev() {
        let _ = std::fs::rename(numbered(n), numbered(n + 1));
    }
    let _ = std::fs::rename(log, numbered(1));
}

/// Discard oversized or excess inherited archives independently of the active log.
fn sanitize_archives(log: &std::path::Path, max: u64, keep: u32) {
    let Some(parent) = log.parent() else { return };
    let Some(name) = log.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let prefix = format!("{name}.");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(n) = name
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if n > keep || entry.metadata().is_ok_and(|meta| meta.len() > max) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Rotates `log` when it is larger than `max` bytes; `true` when it did.
#[must_use]
pub fn rotate_if_over(log: &std::path::Path, max: u64, keep: u32) -> bool {
    sanitize_archives(log, ROTATE_BYTES, keep);
    let over = std::fs::metadata(log).is_ok_and(|meta| meta.len() > max);
    if over {
        rotate(log, keep);
    }
    over
}

/// `<data dir>/host.log`.
#[must_use]
pub fn log_path(paths: &HostPaths) -> PathBuf {
    paths.root.join(LOG_FILE)
}

/// Installs the global tracing subscriber once, as early as the data dir is
/// known.
pub fn init(paths: &HostPaths, serve: bool) {
    if serve && std::env::var_os("OINO_HOST_LOG_PUMP").is_none() {
        let _ = rotate_if_over(&log_path(paths), ROTATE_BYTES, ROTATE_KEEP);
    }
    if serve_to_file(paths, serve) {
        return;
    }
    tracing_subscriber::fmt().with_writer(io::stderr).init();
}

fn serve_to_file(paths: &HostPaths, serve: bool) -> bool {
    if !serve {
        return false;
    }
    let Some(writer) = open_log(paths) else {
        return false;
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .init();
    true
}

/// Rotates past [`ROTATE_BYTES`] and opens the log for appending (`None` when
/// the directory can't be created — logging is never fatal).
fn open_log(paths: &HostPaths) -> Option<TeeWriter> {
    let path = log_path(paths);
    std::fs::create_dir_all(path.parent()?).ok()?;
    // A supervised host writes stderr only: the pump owns the file and rotation.
    if std::env::var_os("OINO_HOST_LOG_PUMP").is_some() {
        return None;
    }
    let file = RotatingFile::open(path, ROTATE_BYTES, ROTATE_KEEP).ok()?;
    Some(TeeWriter {
        inner: Arc::new(Mutex::new(file)),
    })
}

/// Synchronous file sink for standalone hosts on every platform.
struct RotatingFile {
    path: PathBuf,
    file: Option<std::fs::File>,
    written: u64,
    max: u64,
    keep: u32,
}

impl RotatingFile {
    fn open(path: PathBuf, max: u64, keep: u32) -> io::Result<Self> {
        let _ = rotate_if_over(&path, max, keep);
        let mut sink = Self {
            path,
            file: None,
            written: 0,
            max,
            keep,
        };
        sink.reopen()?;
        Ok(sink)
    }

    fn reopen(&mut self) -> io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written >= self.max {
            self.file = None;
            rotate(&self.path, self.keep);
            self.reopen()?;
        }
        let count = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("log unavailable"))?
            .write(
                &buf[..buf
                    .len()
                    .min(usize::try_from(self.max - self.written).unwrap_or(usize::MAX))],
            )?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

/// Writes every line to the log file *and* stderr (a manual run in a console
/// still shows its output). Never fails: a broken log must not break the
/// host.
#[derive(Clone)]
struct TeeWriter {
    inner: Arc<Mutex<RotatingFile>>,
}

struct TeeGuard {
    inner: Arc<Mutex<RotatingFile>>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TeeWriter {
    type Writer = TeeGuard;

    fn make_writer(&'a self) -> Self::Writer {
        TeeGuard {
            inner: self.inner.clone(),
        }
    }
}

impl Write for TeeGuard {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        if let Ok(mut file) = self.inner.lock() {
            let _ = file.write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Ok(mut file) = self.inner.lock() {
            let _ = file.flush();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_sits_under_the_data_dir() {
        let paths = HostPaths::from_dir("/tmp/unipi-test-host");
        assert_eq!(
            log_path(&paths),
            std::path::Path::new("/tmp/unipi-test-host/host.log")
        );
    }

    #[test]
    fn oversized_startup_and_runtime_writes_rotate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        // Sparse production-sized input, not a reduced-threshold simulation.
        std::fs::File::create(&path)
            .unwrap()
            .set_len(ROTATE_BYTES + 1)
            .unwrap();
        for n in 1..=ROTATE_KEEP {
            std::fs::File::create(dir.path().join(format!("host.log.{n}")))
                .unwrap()
                .set_len(ROTATE_BYTES * 4)
                .unwrap();
        }
        let mut sink = RotatingFile::open(path.clone(), ROTATE_BYTES, ROTATE_KEEP).unwrap();
        for n in 1..=ROTATE_KEEP {
            assert!(!dir.path().join(format!("host.log.{n}")).exists());
        }
        assert!(!dir.path().join("host.log.1").exists());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        sink.max = 4;
        for _ in 0..5 {
            sink.write_all(b"line").unwrap();
        }
        sink.flush().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"line");
        for n in 1..=3 {
            assert_eq!(
                std::fs::read(dir.path().join(format!("host.log.{n}"))).unwrap(),
                b"line"
            );
        }
        assert!(!dir.path().join("host.log.4").exists());
    }

    #[test]
    fn inherited_archives_are_bounded_with_small_missing_or_exact_active() {
        for active_size in [None, Some(4), Some(ROTATE_BYTES)] {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join(LOG_FILE);
            if let Some(size) = active_size {
                std::fs::File::create(&log).unwrap().set_len(size).unwrap();
            }
            for n in 1..=5 {
                std::fs::File::create(dir.path().join(format!("host.log.{n}")))
                    .unwrap()
                    .set_len(if n == 2 { 8 } else { ROTATE_BYTES * 4 })
                    .unwrap();
            }
            assert!(!rotate_if_over(&log, ROTATE_BYTES, ROTATE_KEEP));
            let archives: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .filter(|entry| entry.file_name() != LOG_FILE)
                .collect();
            assert!(archives.len() <= ROTATE_KEEP as usize);
            assert!(archives
                .iter()
                .all(|entry| entry.metadata().unwrap().len() <= ROTATE_BYTES));
            assert_eq!(
                std::fs::read(dir.path().join("host.log.2")).unwrap().len(),
                8
            );
        }
    }

    #[test]
    fn rotation_keeps_three_and_drops_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("host.log");
        for round in 0..5 {
            std::fs::write(&log, format!("round {round} {}", "x".repeat(64))).unwrap();
            assert!(!rotate_if_over(&log, 1024, 3), "small logs stay");
            assert!(rotate_if_over(&log, 16, 3));
            assert!(!log.exists());
        }
        let read =
            |n: u32| std::fs::read_to_string(dir.path().join(format!("host.log.{n}"))).unwrap();
        assert!(read(1).starts_with("round 4"));
        assert!(read(2).starts_with("round 3"));
        assert!(read(3).starts_with("round 2"));
        assert!(!dir.path().join("host.log.4").exists());
    }
}
