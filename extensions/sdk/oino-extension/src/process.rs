//! Processes (permission `process.spawn` + `process.allow`).

use futures::Stream;

use crate::Result;
use crate::bindings::oino::extension::process as wp;
pub use crate::bindings::oino::extension::process::{
    ExecResult, ExitStatus, OutputTarget, ProcStats, Signal, SpawnSpec, StdioMode,
};

/// pi `exec(command, args, {timeout, cwd})`.
pub async fn exec(
    command: &str,
    args: &[&str],
    cwd: Option<&str>,
    timeout_ms: Option<u64>,
) -> Result<ExecResult> {
    let options = wp::ExecOptions {
        cwd: cwd.map(ToOwned::to_owned),
        timeout_ms,
        env: Vec::new(),
    };
    Ok(wp::exec(
        command.to_owned(),
        args.iter().map(ToString::to_string).collect(),
        options,
    )
    .await?)
}

/// A spawn spec with piped stdio and defaults.
#[must_use]
pub fn spec(command: &str, args: &[&str]) -> SpawnSpec {
    SpawnSpec {
        command: command.to_owned(),
        args: args.iter().map(ToString::to_string).collect(),
        cwd: None,
        env: Vec::new(),
        clear_env: false,
        stdin: StdioMode::Pipe,
        stdout: OutputTarget::Pipe,
        stderr: OutputTarget::Pipe,
        new_process_group: true,
        detached: false,
        timeout_ms: None,
    }
}

/// A spawned child.
pub struct Child(wp::Child);

impl Child {
    /// Spawns `spec`.
    pub fn spawn(spec: &SpawnSpec) -> Result<Self> {
        Ok(Self(wp::Child::spawn(spec)?))
    }
    /// Process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.0.pid()
    }
    /// Writes stdin.
    pub async fn write_stdin(&self, data: &[u8]) -> Result<()> {
        Ok(self.0.write_stdin(data.to_vec()).await?)
    }
    /// Closes stdin.
    pub fn close_stdin(&self) {
        self.0.close_stdin();
    }
    /// Stdout byte chunks (once).
    pub fn stdout(&self) -> Option<impl Stream<Item = u8> + use<>> {
        self.0
            .stdout()
            .map(wit_bindgen::rt::async_support::StreamReader::into_stream)
    }
    /// Stderr byte chunks (once).
    pub fn stderr(&self) -> Option<impl Stream<Item = u8> + use<>> {
        self.0
            .stderr()
            .map(wit_bindgen::rt::async_support::StreamReader::into_stream)
    }
    /// Collects stdout to a string.
    pub async fn read_stdout(&self) -> String {
        match self.0.stdout() {
            Some(r) => String::from_utf8_lossy(&r.collect().await).into_owned(),
            None => String::new(),
        }
    }
    /// Waits for exit.
    pub async fn wait(&self) -> ExitStatus {
        self.0.wait().await
    }
    /// Non-blocking status.
    #[must_use]
    pub fn try_wait(&self) -> Option<ExitStatus> {
        self.0.try_wait()
    }
    /// Sends a signal (to the group when `group`).
    pub fn signal(&self, sig: Signal, group: bool) -> Result<()> {
        Ok(self.0.signal(sig, group)?)
    }
}

/// Descendants of `pid`.
pub fn tree(pid: u32) -> Result<Vec<u32>> {
    Ok(wp::process_tree(pid)?)
}
/// Stats of `pid`.
pub fn stats(pid: u32) -> Result<ProcStats> {
    Ok(wp::process_stats(pid)?)
}
pub use wp::self_pid;
