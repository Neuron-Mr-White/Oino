//! [`AcpEnv`]: an [`ExecutionEnv`] that lets the ACP client host files and terminals
//! (modes §6.4).
//!
//! - `read_file` goes through `fs/read_text_file` when the client advertises it and the file is
//!   text (binary files such as images are read locally); failures fall back to the local file
//!   system so tools still see Node-style errors (`ENOENT: …`).
//! - `write_file` goes through `fs/write_text_file` for UTF-8 content.
//! - `spawn` runs the command in a client terminal (`terminal/create`, polled
//!   `terminal/output`, `terminal/wait_for_exit`, `terminal/kill`, `terminal/release`) when
//!   the client advertises `terminal` and no stdin is piped.
//! - Everything else (`access`, `stat`, `read_dir`, `realpath`, `mkdir_p`) is local: the
//!   client and agent share the machine.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_tools::env::{
    AccessMode, ExecutionEnv, ExitStatus, FileStat, FsError, LocalEnv, ProcessControl, SpawnError,
    SpawnRequest, SpawnedProcess,
};
use serde_json::json;
use tokio::sync::{mpsc, watch};

use crate::client::ClientLink;

/// How often a running client terminal is polled for output.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// `outputByteLimit` of client terminals (the tools truncate on their side).
const TERMINAL_OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// Bytes sniffed to decide whether a file is text.
const SNIFF_BYTES: usize = 8 * 1024;

/// Receives terminals the environment starts, so the session can attach them to tool calls.
pub trait TerminalObserver: Send + Sync {
    /// `terminal_id` now runs `request`.
    fn terminal_started(&self, request: &SpawnRequest, terminal_id: &str);
}

/// The ACP-client execution environment of one session.
pub struct AcpEnv {
    local: LocalEnv,
    link: ClientLink,
    observer: Arc<dyn TerminalObserver>,
}

impl std::fmt::Debug for AcpEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpEnv")
            .field("caps", &self.link.caps())
            .finish_non_exhaustive()
    }
}

/// Whether `bytes` look like text (valid UTF-8 without NUL).
fn is_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        // A prefix may cut a multi-byte character at its end.
        Err(error) => error.error_len().is_none() && bytes.len() - error.valid_up_to() < 4,
    }
}

impl AcpEnv {
    /// An environment for the session behind `link`.
    #[must_use]
    pub fn new(link: ClientLink, observer: Arc<dyn TerminalObserver>) -> Self {
        Self {
            local: LocalEnv::new(),
            link,
            observer,
        }
    }

    async fn read_via_client(&self, path: &str) -> Result<Vec<u8>, FsError> {
        let prefix = self.local.read_prefix(path, SNIFF_BYTES).await?;
        if !is_text(&prefix) {
            return self.local.read_file(path).await;
        }
        match self.link.read_text_file(path).await {
            Ok(text) => Ok(text.into_bytes()),
            Err(error) => {
                tracing::debug!("fs/read_text_file failed, reading locally: {error}");
                self.local.read_file(path).await
            }
        }
    }

    async fn write_via_client(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        let Ok(text) = std::str::from_utf8(data) else {
            return self.local.write_file(path, data).await;
        };
        match self.link.write_text_file(path, text).await {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::debug!("fs/write_text_file failed, writing locally: {error}");
                self.local.write_file(path, data).await
            }
        }
    }

    async fn spawn_terminal(&self, request: SpawnRequest) -> Result<SpawnedProcess, SpawnError> {
        let inherited: HashMap<String, String> = std::env::vars().collect();
        let env: Vec<_> = request
            .env
            .iter()
            .filter(|(name, value)| inherited.get(name) != Some(value))
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect();
        let spec = json!({
            "command": request.program,
            "args": request.args,
            "env": env,
            "cwd": request.cwd,
            "outputByteLimit": TERMINAL_OUTPUT_LIMIT,
        });
        let terminal_id = self
            .link
            .create_terminal(spec)
            .await
            .map_err(|error| SpawnError {
                program: request.program.clone(),
                code: format!("(ACP terminal/create: {error})"),
            })?;
        self.observer.terminal_started(&request, &terminal_id);
        Ok(ClientTerminal::start(self.link.clone(), terminal_id))
    }
}

impl ExecutionEnv for AcpEnv {
    fn access<'a>(&'a self, path: &'a str, mode: AccessMode) -> BoxFuture<'a, Result<(), FsError>> {
        self.local.access(path, mode)
    }

    fn read_file<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, FsError>> {
        if self.link.caps().read_text_file {
            self.read_via_client(path).boxed()
        } else {
            self.local.read_file(path)
        }
    }

    fn read_prefix<'a>(
        &'a self,
        path: &'a str,
        max: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>, FsError>> {
        self.local.read_prefix(path, max)
    }

    fn write_file<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<(), FsError>> {
        if self.link.caps().write_text_file {
            self.write_via_client(path, data).boxed()
        } else {
            self.local.write_file(path, data)
        }
    }

    fn mkdir_p<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<(), FsError>> {
        self.local.mkdir_p(path)
    }

    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileStat, FsError>> {
        self.local.stat(path)
    }

    fn read_dir<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<String>, FsError>> {
        self.local.read_dir(path)
    }

    fn realpath<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String, FsError>> {
        self.local.realpath(path)
    }

    fn spawn(&self, request: SpawnRequest) -> BoxFuture<'_, Result<SpawnedProcess, SpawnError>> {
        if self.link.caps().terminal && request.stdin.is_none() {
            self.spawn_terminal(request).boxed()
        } else {
            self.local.spawn(request)
        }
    }

    fn is_local(&self) -> bool {
        true
    }
}

/// POSIX signal number of a client-reported signal name.
fn signal_number(name: &str) -> Option<i32> {
    let name = name.trim_start_matches("SIG");
    let number = match name {
        "HUP" => 1,
        "INT" => 2,
        "QUIT" => 3,
        "KILL" => 9,
        "PIPE" => 13,
        "ALRM" => 14,
        "TERM" => 15,
        other => return other.parse().ok(),
    };
    Some(number)
}

/// A command running in a client terminal.
struct ClientTerminal {
    link: ClientLink,
    terminal_id: String,
    exit: watch::Receiver<Option<ExitStatus>>,
    output_closed: Arc<AtomicBool>,
}

impl ClientTerminal {
    /// Starts the output pump and returns the process handle.
    fn start(link: ClientLink, terminal_id: String) -> SpawnedProcess {
        let (output_tx, output_rx) = mpsc::unbounded_channel();
        let (exit_tx, exit_rx) = watch::channel(None);
        let output_closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(pump(
            link.clone(),
            terminal_id.clone(),
            output_tx,
            exit_tx,
            Arc::clone(&output_closed),
        ));
        SpawnedProcess {
            output: output_rx,
            control: Box::new(Self {
                link,
                terminal_id,
                exit: exit_rx,
                output_closed,
            }),
        }
    }
}

/// Polls output until the command exits, then releases the terminal.
async fn pump(
    link: ClientLink,
    terminal_id: String,
    output: mpsc::UnboundedSender<Vec<u8>>,
    exit: watch::Sender<Option<ExitStatus>>,
    output_closed: Arc<AtomicBool>,
) {
    let mut sent = 0_usize;
    let forward = |text: &str, sent: &mut usize| {
        if text.len() > *sent && text.is_char_boundary(*sent) {
            if !output_closed.load(Ordering::Acquire) {
                let _ = output.send(text.as_bytes()[*sent..].to_vec());
            }
            *sent = text.len();
        }
    };
    let waiting = link.wait_for_terminal_exit(&terminal_id);
    tokio::pin!(waiting);
    let status = loop {
        tokio::select! {
            result = &mut waiting => break result.ok().map(|r| r.exit_status),
            () = tokio::time::sleep(POLL_INTERVAL) => {
                if let Ok(response) = link.terminal_output(&terminal_id).await {
                    forward(&response.output, &mut sent);
                }
            }
        }
    };
    let final_output = link.terminal_output(&terminal_id).await.ok();
    if let Some(response) = &final_output {
        forward(&response.output, &mut sent);
    }
    drop(output);
    let status = status.or_else(|| final_output.and_then(|r| r.exit_status));
    let status = ExitStatus {
        code: status
            .as_ref()
            .and_then(|s| s.exit_code)
            .and_then(|c| i32::try_from(c).ok()),
        signal: status
            .as_ref()
            .and_then(|s| s.signal.as_deref())
            .and_then(signal_number),
    };
    let _ = exit.send(Some(status));
    if let Err(error) = link.release_terminal(&terminal_id).await {
        tracing::debug!("terminal/release failed: {error}");
    }
}

impl ProcessControl for ClientTerminal {
    fn pid(&self) -> Option<u32> {
        None
    }

    fn kill_tree(&self) {
        let link = self.link.clone();
        let terminal_id = self.terminal_id.clone();
        tokio::spawn(async move {
            if let Err(error) = link.kill_terminal(&terminal_id).await {
                tracing::debug!("terminal/kill failed: {error}");
            }
        });
    }

    fn wait(&mut self) -> BoxFuture<'_, std::io::Result<ExitStatus>> {
        async move {
            loop {
                if let Some(status) = *self.exit.borrow_and_update() {
                    return Ok(status);
                }
                if self.exit.changed().await.is_err() {
                    return Err(std::io::Error::other("client terminal vanished"));
                }
            }
        }
        .boxed()
    }

    fn close_output(&mut self) {
        self.output_closed.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_detection() {
        assert!(is_text(b"hello\nworld"));
        assert!(!is_text(b"\x89PNG\r\n\x1a\n\0\0"));
        // A cut multi-byte character at the end of a prefix is still text.
        assert!(is_text(&"héllo".as_bytes()[..2]));
        assert!(!is_text(&[0xff, 0xfe, b'a']));
    }

    #[test]
    fn signals() {
        assert_eq!(signal_number("SIGKILL"), Some(9));
        assert_eq!(signal_number("TERM"), Some(15));
        assert_eq!(signal_number("SIGWHAT"), None);
    }
}
