//! `tracing` setup: `<agent>/logs/mobile.log`, plus logcat on Android (tag `oino`).
//!
//! `RUST_LOG` is rarely settable on a phone, so the filter defaults to `info` and can be
//! overridden by a `<agent>/log-filter` file (e.g. `adb shell run-as … sh -c 'echo debug > …'`).

use std::path::Path;
use std::sync::Arc;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt as _;

/// Writes formatted `tracing` lines to the `log` facade (logcat on Android).
#[derive(Clone, Copy, Debug, Default)]
struct LogFacade;

impl std::io::Write for LogFacade {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let line = text.trim_end();
        if !line.is_empty() {
            log::info!(target: "oino", "{line}");
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Installs the global subscriber (idempotent: later calls are ignored).
pub(crate) fn init(agent_dir: &Path) {
    let filter = std::fs::read_to_string(agent_dir.join("log-filter"))
        .ok()
        .and_then(|f| EnvFilter::try_new(f.trim()).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new("info"));
    let dir = agent_dir.join("logs");
    let file = std::fs::create_dir_all(&dir)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("mobile.log"))
        })
        .ok()
        .map(Arc::new);
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false);
    let facade = || LogFacade;
    let result = match file {
        Some(file) => builder.with_writer(file.and(facade)).try_init(),
        None => builder.with_writer(facade).try_init(),
    };
    if let Err(error) = result {
        log::warn!("tracing already initialised: {error}");
    }
}
