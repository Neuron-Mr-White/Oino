//! WASI context per instance: env limited to `permissions.env` (+ `OINO_EXTENSION_ID`),
//! stdout/stderr appended to `logs/ext/<id>.log`, named preopens (§5.7).

use wasmtime_wasi::cli::OutputFile;
use wasmtime_wasi::{FsPerms, WasiCtx};

use crate::state::ExtShared;

fn log_file(ext: &ExtShared) -> std::io::Result<std::fs::File> {
    if let Some(parent) = ext.paths.log_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&ext.paths.log_file)
}

/// Builds the WASI context of one store.
pub(crate) fn build(ext: &ExtShared) -> wasmtime::Result<WasiCtx> {
    let mut builder = WasiCtx::builder();
    builder.env("OINO_EXTENSION_ID", ext.id.as_str());
    for name in &ext.perms().env {
        if let Ok(value) = std::env::var(name) {
            builder.env(name, value);
        }
    }
    builder.stdout(OutputFile::new(log_file(ext)?));
    builder.stderr(OutputFile::new(log_file(ext)?));
    for pre in &ext.preopen_list {
        std::fs::create_dir_all(&pre.host).ok();
        let perms = if pre.writable {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        builder.preopened_dir(&pre.host, pre.guest_path(), perms)?;
    }
    Ok(builder.build())
}
