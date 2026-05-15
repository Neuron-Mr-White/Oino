#![doc = r#"Execution environment boundary for Oino tools.

Tools should depend on `ExecutionEnv` instead of directly calling process or filesystem APIs.
The local adapter is intentionally small and typed so future sandbox, remote, and container
adapters can provide the same surface.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::{fs, io::AsyncWriteExt, process::Command, time};

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("command timed out")]
    Timeout,
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub type EnvResult<T> = Result<T, EnvError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOptions {
    pub cwd: Option<PathBuf>,
    pub timeout_ms: Option<u64>,
}

impl Default for CommandOptions {
    fn default() -> Self {
        Self {
            cwd: None,
            timeout_ms: Some(30_000),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileStat {
    pub path: PathBuf,
    pub is_file: bool,
    pub is_dir: bool,
    pub len: u64,
}

#[async_trait]
pub trait ExecutionEnv: Send + Sync {
    async fn shell(&self, command: &str, options: CommandOptions) -> EnvResult<CommandOutput>;
    async fn read_text(&self, path: &Path) -> EnvResult<String>;
    async fn write_text(&self, path: &Path, content: &str) -> EnvResult<()>;
    async fn append_text(&self, path: &Path, content: &str) -> EnvResult<()>;
    async fn read_binary(&self, path: &Path) -> EnvResult<Vec<u8>>;
    async fn write_binary(&self, path: &Path, content: &[u8]) -> EnvResult<()>;
    async fn list_dir(&self, path: &Path) -> EnvResult<Vec<PathBuf>>;
    async fn stat(&self, path: &Path) -> EnvResult<FileStat>;
    async fn realpath(&self, path: &Path) -> EnvResult<PathBuf>;
    async fn create_dir_all(&self, path: &Path) -> EnvResult<()>;
    async fn remove_file(&self, path: &Path) -> EnvResult<()>;
    async fn remove_dir_all(&self, path: &Path) -> EnvResult<()>;
    async fn temp_dir(&self) -> EnvResult<PathBuf>;
    async fn cleanup(&self, path: &Path) -> EnvResult<()>;
}

#[derive(Debug, Clone, Default)]
pub struct LocalExecutionEnv;

#[async_trait]
impl ExecutionEnv for LocalExecutionEnv {
    async fn shell(&self, command: &str, options: CommandOptions) -> EnvResult<CommandOutput> {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc").arg(command);
        if let Some(cwd) = options.cwd {
            cmd.current_dir(cwd);
        }
        let fut = cmd.output();
        let output = if let Some(ms) = options.timeout_ms {
            match time::timeout(Duration::from_millis(ms), fut).await {
                Ok(output) => output?,
                Err(_) => return Err(EnvError::Timeout),
            }
        } else {
            fut.await?
        };
        Ok(CommandOutput {
            status: output.status.code(),
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        })
    }

    async fn read_text(&self, path: &Path) -> EnvResult<String> {
        Ok(fs::read_to_string(path).await?)
    }
    async fn write_text(&self, path: &Path, content: &str) -> EnvResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(fs::write(path, content).await?)
    }
    async fn append_text(&self, path: &Path, content: &str) -> EnvResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        Ok(())
    }
    async fn read_binary(&self, path: &Path) -> EnvResult<Vec<u8>> {
        Ok(fs::read(path).await?)
    }
    async fn write_binary(&self, path: &Path, content: &[u8]) -> EnvResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(fs::write(path, content).await?)
    }
    async fn list_dir(&self, path: &Path) -> EnvResult<Vec<PathBuf>> {
        let mut out = Vec::new();
        let mut dir = fs::read_dir(path).await?;
        while let Some(entry) = dir.next_entry().await? {
            out.push(entry.path());
        }
        out.sort();
        Ok(out)
    }
    async fn stat(&self, path: &Path) -> EnvResult<FileStat> {
        let metadata = fs::metadata(path).await?;
        Ok(FileStat {
            path: path.to_path_buf(),
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            len: metadata.len(),
        })
    }
    async fn realpath(&self, path: &Path) -> EnvResult<PathBuf> {
        Ok(fs::canonicalize(path).await?)
    }
    async fn create_dir_all(&self, path: &Path) -> EnvResult<()> {
        Ok(fs::create_dir_all(path).await?)
    }
    async fn remove_file(&self, path: &Path) -> EnvResult<()> {
        Ok(fs::remove_file(path).await?)
    }
    async fn remove_dir_all(&self, path: &Path) -> EnvResult<()> {
        Ok(fs::remove_dir_all(path).await?)
    }
    async fn temp_dir(&self) -> EnvResult<PathBuf> {
        let base = std::env::temp_dir().join(format!("oino-{}", uuid_like()));
        fs::create_dir_all(&base).await?;
        Ok(base)
    }
    async fn cleanup(&self, path: &Path) -> EnvResult<()> {
        match fs::metadata(path).await {
            Ok(meta) if meta.is_dir() => self.remove_dir_all(path).await,
            Ok(_) => self.remove_file(path).await,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(EnvError::Io(err)),
        }
    }
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_file_operations() {
        let env = LocalExecutionEnv;
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(err) => panic!("tempdir failed: {err}"),
        };
        let path = dir.path().join("a/b.txt");
        assert!(env.write_text(&path, "hello").await.is_ok());
        assert!(env.append_text(&path, " world").await.is_ok());
        let content = env.read_text(&path).await;
        let content = match content {
            Ok(content) => content,
            Err(err) => panic!("read failed: {err}"),
        };
        assert_eq!(content, "hello world");
        let stat = env.stat(&path).await;
        let stat = match stat {
            Ok(stat) => stat,
            Err(err) => panic!("stat failed: {err}"),
        };
        assert!(stat.is_file);
    }

    #[tokio::test]
    async fn shell_timeout_is_typed() {
        let env = LocalExecutionEnv;
        let output = env
            .shell(
                "sleep 1",
                CommandOptions {
                    cwd: None,
                    timeout_ms: Some(1),
                },
            )
            .await;
        assert!(matches!(output, Err(EnvError::Timeout)));
    }

    #[tokio::test]
    async fn shell_output_captures_status() {
        let env = LocalExecutionEnv;
        let output = env.shell("printf hi", CommandOptions::default()).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("shell failed: {err}"),
        };
        assert_eq!(output.stdout, "hi");
        assert_eq!(output.status, Some(0));
    }
}
