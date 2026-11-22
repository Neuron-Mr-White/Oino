//! `log` import: `tracing` plus one line per message in `logs/ext/<id>.log`.

use std::io::Write as _;

use oino_extension_api::LogLevel;

use crate::state::ExtShared;

/// Records `message` at `level` for `ext`.
pub(crate) fn write(ext: &ExtShared, level: LogLevel, message: &str) {
    let id = ext.id.as_str();
    match level {
        LogLevel::Trace => tracing::trace!(extension = id, "{message}"),
        LogLevel::Debug => tracing::debug!(extension = id, "{message}"),
        LogLevel::Info => tracing::info!(extension = id, "{message}"),
        LogLevel::Warn => tracing::warn!(extension = id, "{message}"),
        LogLevel::Error => tracing::error!(extension = id, "{message}"),
    }
    let level = format!("{level:?}").to_ascii_uppercase();
    let line = format!("{} {level} {message}\n", crate::timers::now_ms());
    if let Some(parent) = ext.paths.log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ext.paths.log_file)
    {
        let _ = f.write_all(line.as_bytes());
    }
}
