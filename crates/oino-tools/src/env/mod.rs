//! Execution environment: the file system and process boundary every tool goes through.
//!
//! pi lets tools swap their `operations` (for SSH, containers…); Oino generalises that into
//! one [`ExecutionEnv`] so ACP clients can host the file system and terminals (modes §6.4).
//! [`LocalEnv`] is the default. Errors carry Node's `code`/`syscall`/`path` so the texts the
//! model sees match pi (`ENOENT: no such file or directory, open '/x'`).

mod error;
mod local;

use futures::future::BoxFuture;
use tokio::sync::mpsc;

pub use error::{FsError, SpawnError};
pub use local::LocalEnv;

/// Accessibility check of [`ExecutionEnv::access`] (Node `fs.access` modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    /// `F_OK`: the path exists.
    Exists,
    /// `R_OK`: the path is readable.
    Read,
    /// `R_OK | W_OK`: the path is readable and writable.
    ReadWrite,
}

/// What [`ExecutionEnv::stat`] reports (symlinks followed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStat {
    /// The path is a directory.
    pub is_dir: bool,
    /// The path is a regular file.
    pub is_file: bool,
    /// Size in bytes.
    pub size: u64,
}

/// A process to start with [`ExecutionEnv::spawn`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnRequest {
    /// Executable.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: String,
    /// The complete environment (the parent environment is not inherited).
    pub env: Vec<(String, String)>,
    /// Bytes written to stdin before it is closed; `None` connects stdin to the null device.
    pub stdin: Option<Vec<u8>>,
}

/// How a process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    /// Exit code, when the process exited normally.
    pub code: Option<i32>,
    /// Terminating signal number, when the process was killed by a signal.
    pub signal: Option<i32>,
}

impl ExitStatus {
    /// pi's exit-code rule: the code, else `128 + signal`, else `1`.
    #[must_use]
    pub fn shell_exit_code(self) -> i32 {
        match (self.code, self.signal) {
            (Some(code), _) => code,
            (None, Some(signal)) => 128 + signal,
            (None, None) => 1,
        }
    }
}

/// Control half of a running process.
pub trait ProcessControl: Send {
    /// OS process id, when known.
    fn pid(&self) -> Option<u32>;

    /// Kills the process and all its descendants (pi `killProcessTree`: `killpg(SIGKILL)` on
    /// Unix, `taskkill /F /T` on Windows). Never fails; a dead process is ignored.
    fn kill_tree(&self);

    /// Waits for the process to exit. Cancel-safe.
    fn wait(&mut self) -> BoxFuture<'_, std::io::Result<ExitStatus>>;

    /// Stops reading output (pipes inherited by detached descendants may never close).
    fn close_output(&mut self);
}

/// A started process: stdout and stderr interleaved in arrival order, plus its control.
pub struct SpawnedProcess {
    /// Output chunks; closed when both pipes reached end of file or output was closed.
    pub output: mpsc::UnboundedReceiver<Vec<u8>>,
    /// Kill / wait handle.
    pub control: Box<dyn ProcessControl>,
}

impl std::fmt::Debug for SpawnedProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnedProcess")
            .field("pid", &self.control.pid())
            .finish_non_exhaustive()
    }
}

/// The file system and process host used by the tools.
pub trait ExecutionEnv: Send + Sync {
    /// Node `fs.access(path, mode)`.
    fn access<'a>(&'a self, path: &'a str, mode: AccessMode) -> BoxFuture<'a, Result<(), FsError>>;

    /// Node `fs.readFile(path)`.
    fn read_file<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, FsError>>;

    /// Reads at most `max` bytes from the start of the file (image type sniffing).
    fn read_prefix<'a>(
        &'a self,
        path: &'a str,
        max: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>, FsError>>;

    /// Node `fs.writeFile(path, data)`: creates or truncates.
    fn write_file<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<(), FsError>>;

    /// Node `fs.mkdir(path, {recursive: true})`.
    fn mkdir_p<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<(), FsError>>;

    /// Node `fs.stat(path)`.
    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileStat, FsError>>;

    /// Node `fs.readdir(path)`: entry names in directory order.
    fn read_dir<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<String>, FsError>>;

    /// Node `fs.realpath(path)`.
    fn realpath<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String, FsError>>;

    /// Starts a process in its own process group with piped stdout/stderr.
    fn spawn(&self, request: SpawnRequest) -> BoxFuture<'_, Result<SpawnedProcess, SpawnError>>;

    /// Whether paths refer to this machine's file system. `grep` and `find` walk the local file
    /// system directly (pi runs local `rg`/`fd` too) and refuse to run otherwise.
    fn is_local(&self) -> bool;
}

/// pi `pathExists`: `access(F_OK)` succeeds.
pub async fn path_exists(env: &dyn ExecutionEnv, path: &str) -> bool {
    env.access(path, AccessMode::Exists).await.is_ok()
}
