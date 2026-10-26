//! [`LocalEnv`]: the local file system and process table.

use std::process::Stdio;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    AccessMode, ExecutionEnv, ExitStatus, FileStat, FsError, ProcessControl, SpawnError,
    SpawnRequest, SpawnedProcess,
};

/// The local machine (default [`ExecutionEnv`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalEnv;

impl LocalEnv {
    /// The local environment.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn access_sync(path: &str, mode: AccessMode) -> Result<(), FsError> {
    use nix::unistd::AccessFlags;
    let flags = match mode {
        AccessMode::Exists => AccessFlags::F_OK,
        AccessMode::Read => AccessFlags::R_OK,
        AccessMode::ReadWrite => AccessFlags::R_OK | AccessFlags::W_OK,
    };
    nix::unistd::access(path, flags)
        .map_err(|errno| FsError::from_io(&std::io::Error::from(errno), "access", Some(path)))
}

#[cfg(not(unix))]
fn access_sync(path: &str, mode: AccessMode) -> Result<(), FsError> {
    let metadata =
        std::fs::metadata(path).map_err(|error| FsError::from_io(&error, "access", Some(path)))?;
    if mode == AccessMode::ReadWrite && metadata.permissions().readonly() {
        return Err(FsError::new("EPERM", "access", Some(path)));
    }
    Ok(())
}

async fn read_limited(path: &str, max: Option<usize>) -> Result<Vec<u8>, FsError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| FsError::from_io(&error, "open", Some(path)))?;
    let mut buffer = Vec::new();
    let result = match max {
        Some(max) => {
            let mut limited = (&mut file).take(u64::try_from(max).unwrap_or(u64::MAX));
            limited.read_to_end(&mut buffer).await
        }
        None => file.read_to_end(&mut buffer).await,
    };
    result.map_err(|error| FsError::from_io(&error, "read", None))?;
    Ok(buffer)
}

impl ExecutionEnv for LocalEnv {
    fn access<'a>(&'a self, path: &'a str, mode: AccessMode) -> BoxFuture<'a, Result<(), FsError>> {
        async move {
            let owned = path.to_owned();
            tokio::task::spawn_blocking(move || access_sync(&owned, mode))
                .await
                .unwrap_or_else(|_| Err(FsError::new("UNKNOWN", "access", Some(path))))
        }
        .boxed()
    }

    fn read_file<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<u8>, FsError>> {
        read_limited(path, None).boxed()
    }

    fn read_prefix<'a>(
        &'a self,
        path: &'a str,
        max: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>, FsError>> {
        read_limited(path, Some(max)).boxed()
    }

    fn write_file<'a>(
        &'a self,
        path: &'a str,
        data: &'a [u8],
    ) -> BoxFuture<'a, Result<(), FsError>> {
        async move {
            let mut file = tokio::fs::File::create(path)
                .await
                .map_err(|error| FsError::from_io(&error, "open", Some(path)))?;
            file.write_all(data)
                .await
                .map_err(|error| FsError::from_io(&error, "write", None))?;
            file.flush()
                .await
                .map_err(|error| FsError::from_io(&error, "write", None))
        }
        .boxed()
    }

    fn mkdir_p<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<(), FsError>> {
        async move {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|error| FsError::from_io(&error, "mkdir", Some(path)))
        }
        .boxed()
    }

    fn stat<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<FileStat, FsError>> {
        async move {
            let metadata = tokio::fs::metadata(path)
                .await
                .map_err(|error| FsError::from_io(&error, "stat", Some(path)))?;
            Ok(FileStat {
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
                size: metadata.len(),
            })
        }
        .boxed()
    }

    fn read_dir<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<Vec<String>, FsError>> {
        async move {
            let to_error = |error: std::io::Error| FsError::from_io(&error, "scandir", Some(path));
            let mut entries = tokio::fs::read_dir(path).await.map_err(to_error)?;
            let mut names = Vec::new();
            while let Some(entry) = entries.next_entry().await.map_err(to_error)? {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            Ok(names)
        }
        .boxed()
    }

    fn realpath<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String, FsError>> {
        async move {
            tokio::fs::canonicalize(path)
                .await
                .map(|resolved| path_string(&resolved))
                .map_err(|error| FsError::from_io(&error, "realpath", Some(path)))
        }
        .boxed()
    }

    fn spawn(&self, request: SpawnRequest) -> BoxFuture<'_, Result<SpawnedProcess, SpawnError>> {
        async move { spawn_local(request) }.boxed()
    }

    fn is_local(&self) -> bool {
        true
    }
}

fn spawn_local(request: SpawnRequest) -> Result<SpawnedProcess, SpawnError> {
    let mut command = Command::new(&request.program);
    command
        .args(&request.args)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(request.env.iter().map(|(key, value)| (key, value)))
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| SpawnError::from_io(&error, &request.program))?;
    if let (Some(input), Some(mut stdin)) = (request.stdin, child.stdin.take()) {
        tokio::spawn(async move {
            // Errors writing stdin are ignored like pi (`child.stdin.on("error", () => {})`).
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        });
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(pump(stdout, tx.clone())));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(pump(stderr, tx)));
    }
    Ok(SpawnedProcess {
        output: rx,
        control: Box::new(LocalProcess {
            pid: child.id(),
            child,
            readers,
        }),
    })
}

async fn pump(mut pipe: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if tx.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
        }
    }
}

struct LocalProcess {
    pid: Option<u32>,
    child: Child,
    readers: Vec<JoinHandle<()>>,
}

impl ProcessControl for LocalProcess {
    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn kill_tree(&self) {
        if let Some(pid) = self.pid {
            kill_process_tree(pid);
        }
    }

    fn wait(&mut self) -> BoxFuture<'_, std::io::Result<ExitStatus>> {
        async move {
            let status = self.child.wait().await?;
            Ok(convert_status(status))
        }
        .boxed()
    }

    fn close_output(&mut self) {
        for reader in self.readers.drain(..) {
            reader.abort();
        }
    }
}

impl Drop for LocalProcess {
    fn drop(&mut self) {
        self.close_output();
    }
}

#[cfg(unix)]
fn convert_status(status: std::process::ExitStatus) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus {
        code: status.code(),
        signal: status.signal(),
    }
}

#[cfg(not(unix))]
fn convert_status(status: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: status.code(),
        signal: None,
    }
}

/// pi `killProcessTree`: SIGKILL to the process group, falling back to the process.
#[cfg(unix)]
pub(crate) fn kill_process_tree(pid: u32) {
    use nix::sys::signal::{Signal, kill, killpg};
    use nix::unistd::Pid;
    let Ok(raw) = i32::try_from(pid) else {
        return;
    };
    let pid = Pid::from_raw(raw);
    if killpg(pid, Signal::SIGKILL).is_err() {
        // Already dead when this fails too.
        let _ = kill(pid, Signal::SIGKILL);
    }
}

/// pi `killProcessTree`: `%SystemRoot%\System32\taskkill.exe /F /T /PID <pid>`.
#[cfg(windows)]
pub(crate) fn kill_process_tree(pid: u32) {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
    let taskkill = format!("{root}\\System32\\taskkill.exe");
    // Failures are ignored like pi.
    let _ = std::process::Command::new(taskkill)
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
