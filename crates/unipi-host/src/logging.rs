//! Where the host's own logs go.
//!
//! Linux/macOS: stderr, as before (systemd/journald, a terminal, launchd).
//!
//! Windows: the installer's logon task runs
//! `conhost.exe --headless unipi-host.exe serve --mode service`, so stderr
//! goes nowhere — a fresh install had no logs at all, and every phone-side
//! failure was undebuggable. `serve` now also writes
//! `<data dir>\logs\host.log` (no ANSI colours, `info,unipi_host=info` unless
//! `RUST_LOG` says otherwise), rotated to `host.log.1` past 5 MB.

use std::io;
use std::path::PathBuf;

#[cfg(windows)]
use std::io::Write;
#[cfg(windows)]
use std::sync::{Arc, Mutex};

use crate::data_dir::HostPaths;

/// Sub-directory of the data dir that holds `host.log`.
pub const LOG_DIR: &str = "logs";
/// The live log file's name.
pub const LOG_FILE: &str = "host.log";
/// Rotate `host.log` to `host.log.1` once it grows past this at startup.
#[cfg(windows)]
const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Level used when `RUST_LOG` is unset (the host's own lines at info, the
/// rest of the dependency stack quiet).
#[cfg(windows)]
const DEFAULT_FILTER: &str = "info,unipi_host=info";

/// `<data dir>\logs\host.log`.
pub fn log_path(paths: &HostPaths) -> PathBuf {
    paths.root.join(LOG_DIR).join(LOG_FILE)
}

/// Installs the global tracing subscriber once, as early as the data dir is
/// known.
pub fn init(paths: &HostPaths, serve: bool) {
    if serve_to_file(paths, serve) {
        return;
    }
    tracing_subscriber::fmt().with_writer(io::stderr).init();
}

#[cfg(not(windows))]
fn serve_to_file(_paths: &HostPaths, _serve: bool) -> bool {
    false
}

#[cfg(windows)]
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

/// Rotates past [`MAX_BYTES`] and opens the log for appending (`None` when
/// the directory can't be created — logging is never fatal).
#[cfg(windows)]
fn open_log(paths: &HostPaths) -> Option<TeeWriter> {
    let path = log_path(paths);
    std::fs::create_dir_all(path.parent()?).ok()?;
    let rotate = std::fs::metadata(&path)
        .map(|meta| meta.len() > MAX_BYTES)
        .unwrap_or(false);
    if rotate {
        let old = path.with_extension("log.1");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, &old);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some(TeeWriter {
        inner: Arc::new(Mutex::new(file)),
    })
}

/// Writes every line to the log file *and* stderr (a manual run in a console
/// still shows its output). Never fails: a broken log must not break the
/// host.
#[cfg(windows)]
#[derive(Clone)]
struct TeeWriter {
    inner: Arc<Mutex<std::fs::File>>,
}

#[cfg(windows)]
struct TeeGuard {
    inner: Arc<Mutex<std::fs::File>>,
}

#[cfg(windows)]
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TeeWriter {
    type Writer = TeeGuard;

    fn make_writer(&'a self) -> Self::Writer {
        TeeGuard {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(windows)]
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
            std::path::Path::new("/tmp/unipi-test-host/logs/host.log")
        );
    }
}
