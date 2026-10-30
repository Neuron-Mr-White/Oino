//! The `shell` op: a plain login shell in `$HOME` (the picker's
//! Skip button), with the same env stripping as the multiplexer
//! clients.

use tokio::process::Command;
use unipi_proto::{AppErrorCode, AppErrorDto};

fn unavailable(reason: String) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_MUX_UNAVAILABLE, reason, true)
}

/// Builds the PTY command for `$SHELL -l` (fallback `/bin/sh`) in
/// `$HOME`, `TERM=xterm-256color`, with `HERDR_*` and `TMUX*` vars
/// stripped.
pub async fn command(_cols: u16, _rows: u16) -> Result<portable_pty::CommandBuilder, AppErrorDto> {
    let shell = match std::env::var("SHELL") {
        Ok(shell) if !shell.is_empty() => shell,
        _ => "/bin/sh".into(),
    };
    let home = crate::fs::home_dir_opt();
    let mut command = portable_pty::CommandBuilder::new(&shell);
    command.arg("-l");
    if let Some(home) = &home {
        command.cwd(home);
    }
    command.env("TERM", "xterm-256color");
    for (key, _) in std::env::vars() {
        if key.starts_with("HERDR_") || key.starts_with("TMUX") {
            command.env_remove(key);
        }
    }
    let probe = Command::new(&shell).arg("-c").arg("true").output().await;
    if let Err(error) = probe {
        return Err(unavailable(format!(
            "shell {shell} is not runnable: {error}"
        )));
    }
    Ok(command)
}
