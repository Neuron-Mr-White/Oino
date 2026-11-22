//! WASI context per instance: env limited to `permissions.env` (+ `OINO_EXTENSION_ID`),
//! stdout/stderr appended to `logs/ext/<id>.log`, named preopens (§5.7).

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
/// stdout and stderr of one instance over **one** append-mode descriptor:
/// [`OutputFile`] shares its `Arc<File>` on clone, so both streams write the same fd
/// (a per-stream open, or a `try_clone`, would cost one extra descriptor per wasm
/// instance — ~30 per services set on a full install, which exhausted GUI hosts'
/// `RLIMIT_NOFILE`, v2 QA os error 24). `O_APPEND` keeps the interleaved writes atomic.
fn shared_log(
    file: std::fs::File,
) -> (
    wasmtime_wasi::cli::OutputFile,
    wasmtime_wasi::cli::OutputFile,
) {
    let log = wasmtime_wasi::cli::OutputFile::new(file);
    (log.clone(), log)
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
    let (stdout, stderr) = shared_log(log_file(ext)?);
    builder.stdout(stdout);
    builder.stderr(stderr);
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

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use wasmtime_wasi::cli::StdoutStream as _;

    /// stdout and stderr share one descriptor: both streams' writes land in the same
    /// log file (one fd per instance, not two — the v2 QA fd budget).
    #[test]
    fn stdout_and_stderr_share_one_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ext.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        let (stdout, stderr) = shared_log(file);
        let mut out = stdout.p2_stream();
        let mut err = stderr.p2_stream();
        out.write(Bytes::from_static(b"stdout line\n")).unwrap();
        err.write(Bytes::from_static(b"stderr line\n")).unwrap();
        out.flush().unwrap();
        err.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("stdout line"), "{text}");
        assert!(text.contains("stderr line"), "{text}");
    }
}
