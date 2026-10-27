//! Local shell execution shared by the shell tools and core's `!cmd` executor (pi
//! `createLocalShellOperations().exec` + `waitForChildProcess`).

use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::detach::{CallMessage, DetachRequest};
use super::shell::{CommandTransport, resolve_powershell_config, resolve_shell_config};
use crate::env::{ExecutionEnv, SpawnRequest, SpawnedProcess, path_exists};
use crate::js::number_to_string;

/// Grace period after exit while descendants still write (pi `EXIT_STDIO_GRACE_MS`).
const EXIT_STDIO_GRACE: Duration = Duration::from_millis(100);
/// pi `MAX_TIMEOUT_MS`.
const MAX_TIMEOUT_MS: f64 = 2_147_483_647.0;

/// pi `resolveTimeoutMs`: validates a timeout in seconds.
///
/// # Errors
/// `Invalid timeout: must be a finite number of seconds` / `Invalid timeout: maximum is
/// 2147483.647 seconds`.
pub fn resolve_timeout(timeout: Option<f64>) -> Result<Option<Duration>, String> {
    let Some(timeout) = timeout else {
        return Ok(None);
    };
    if !timeout.is_finite() || timeout <= 0.0 {
        return Err("Invalid timeout: must be a finite number of seconds".to_owned());
    }
    let millis = timeout * 1000.0;
    if millis > MAX_TIMEOUT_MS {
        return Err(format!(
            "Invalid timeout: maximum is {} seconds",
            number_to_string(MAX_TIMEOUT_MS / 1000.0)
        ));
    }
    Ok(Some(Duration::from_secs_f64(timeout)))
}

/// Receives output while a command runs and may ask for timed flushes (throttled updates).
pub(crate) trait OutputSink {
    /// A chunk of stdout or stderr.
    fn data(&mut self, chunk: &[u8]);
    /// When [`OutputSink::flush`] wants to run next.
    fn next_flush(&self) -> Option<Instant>;
    /// Called at [`OutputSink::next_flush`].
    fn flush(&mut self);
}

/// How a command ended without error.
pub(crate) enum ExecEnd {
    /// The shell exited (pi exit-code rule applied).
    Exited(i32),
    /// Core detached the process; it keeps running.
    Detached(DetachRequest, SpawnedProcess),
}

/// Why a command failed (pi's `exec` rejections).
#[derive(Debug)]
pub(crate) enum ExecError {
    /// The cancellation token fired (pi `"aborted"`).
    Aborted,
    /// The timeout elapsed; carries the timeout as JavaScript prints it.
    Timeout(String),
    /// Any other failure, with its message.
    Failed(String),
}

/// Which shell runs the command.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ShellSpec<'a> {
    /// `bash` (explicit `shellPath` or the platform default).
    Bash(Option<&'a str>),
    /// PowerShell (Windows).
    PowerShell,
}

impl ShellSpec<'_> {
    fn name(self) -> &'static str {
        match self {
            Self::Bash(_) => "bash",
            Self::PowerShell => "PowerShell",
        }
    }
}

/// One command to run.
pub(crate) struct ExecRequest<'a> {
    pub(crate) env: &'a dyn ExecutionEnv,
    pub(crate) shell: ShellSpec<'a>,
    pub(crate) command: String,
    pub(crate) cwd: String,
    pub(crate) vars: Vec<(String, String)>,
    pub(crate) timeout: Option<f64>,
}

/// Runs a command to completion (or detach), feeding output to `sink`.
pub(crate) async fn exec(
    request: ExecRequest<'_>,
    sink: &mut (dyn OutputSink + Send),
    cancel: &CancellationToken,
    detach: Option<&mut mpsc::UnboundedReceiver<CallMessage>>,
) -> Result<ExecEnd, ExecError> {
    let timeout = resolve_timeout(request.timeout).map_err(ExecError::Failed)?;
    if cancel.is_cancelled() {
        return Err(ExecError::Aborted);
    }
    let shell = match request.shell {
        ShellSpec::Bash(path) => resolve_shell_config(request.env, path).await,
        ShellSpec::PowerShell => resolve_powershell_config(request.env).await,
    }
    .map_err(ExecError::Failed)?;
    if !path_exists(request.env, &request.cwd).await {
        return Err(ExecError::Failed(format!(
            "Working directory does not exist: {}\nCannot execute {} commands.",
            request.cwd,
            request.shell.name()
        )));
    }
    let from_stdin = shell.transport == CommandTransport::Stdin;
    let mut args = shell.args.clone();
    if !from_stdin {
        args.push(request.command.clone());
    }
    let process = request
        .env
        .spawn(SpawnRequest {
            program: shell.shell.clone(),
            args,
            cwd: request.cwd.clone(),
            env: request.vars,
            stdin: from_stdin.then(|| request.command.clone().into_bytes()),
        })
        .await
        .map_err(|error| ExecError::Failed(error.to_string()))?;
    wait(process, sink, cancel, timeout, request.timeout, detach).await
}

async fn wait(
    mut process: SpawnedProcess,
    sink: &mut (dyn OutputSink + Send),
    cancel: &CancellationToken,
    timeout: Option<Duration>,
    timeout_seconds: Option<f64>,
    mut detach: Option<&mut mpsc::UnboundedReceiver<CallMessage>>,
) -> Result<ExecEnd, ExecError> {
    let timeout_at = timeout.map(|timeout| Instant::now() + timeout);
    let (mut aborted, mut timed_out) = (false, false);
    let mut exit: Option<i32> = None;
    let mut pipes_closed = false;
    let mut idle_deadline: Option<Instant> = None;
    let far = || Instant::now() + Duration::from_hours(24 * 365);
    loop {
        if exit.is_some() && pipes_closed {
            break;
        }
        let flush_at = sink.next_flush();
        tokio::select! {
            chunk = process.output.recv(), if !pipes_closed => match chunk {
                Some(chunk) => {
                    sink.data(&chunk);
                    if exit.is_some() {
                        idle_deadline = Some(Instant::now() + EXIT_STDIO_GRACE);
                    }
                }
                None => pipes_closed = true,
            },
            status = process.control.wait(), if exit.is_none() => match status {
                Ok(status) => {
                    exit = Some(status.shell_exit_code());
                    idle_deadline = Some(Instant::now() + EXIT_STDIO_GRACE);
                }
                Err(error) => return Err(ExecError::Failed(error.to_string())),
            },
            () = tokio::time::sleep_until(idle_deadline.unwrap_or_else(far)), if idle_deadline.is_some() => break,
            () = tokio::time::sleep_until(flush_at.unwrap_or_else(far)), if flush_at.is_some() => sink.flush(),
            () = cancel.cancelled(), if !aborted => {
                aborted = true;
                process.control.kill_tree();
            },
            () = tokio::time::sleep_until(timeout_at.unwrap_or_else(far)), if timeout_at.is_some() && !timed_out => {
                timed_out = true;
                process.control.kill_tree();
            },
            request = recv_detach(&mut detach), if detach.is_some() && exit.is_none() && !aborted && !timed_out => {
                match request {
                    Some(CallMessage::Detach(request)) => return Ok(ExecEnd::Detached(request, process)),
                    Some(CallMessage::Pid(reply)) => {
                        let _ = reply.send(process.control.pid());
                    }
                    None => detach = None,
                }
            },
        }
    }
    process.control.close_output();
    while let Ok(chunk) = process.output.try_recv() {
        sink.data(&chunk);
    }
    if aborted {
        return Err(ExecError::Aborted);
    }
    if timed_out {
        return Err(ExecError::Timeout(
            timeout_seconds.map_or_else(String::new, number_to_string),
        ));
    }
    Ok(ExecEnd::Exited(exit.unwrap_or(1)))
}

async fn recv_detach(
    detach: &mut Option<&mut mpsc::UnboundedReceiver<CallMessage>>,
) -> Option<CallMessage> {
    match detach {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_validation_matches_pi() {
        assert_eq!(resolve_timeout(None), Ok(None));
        assert_eq!(
            resolve_timeout(Some(0.0)),
            Err("Invalid timeout: must be a finite number of seconds".to_owned())
        );
        assert_eq!(
            resolve_timeout(Some(1e10)),
            Err("Invalid timeout: maximum is 2147483.647 seconds".to_owned())
        );
        assert!(resolve_timeout(Some(1.5)).unwrap().is_some());
    }
}
