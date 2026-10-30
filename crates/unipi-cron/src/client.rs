//! The daemon's control client: one JSON line over a loopback TCP socket.
//!
//! The daemon records `{pid, port}` in `<cron>/serve.json`; [`ControlClient`] connects
//! there and sends [`crate::control::CronControl`] requests.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::control::CronControl;
use crate::error::{CronError, Result};
use crate::paths::{CronPaths, read_json};

/// The running daemon's identity (`serve.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeInfo {
    /// The daemon's pid.
    pub pid: u32,
    /// The control socket's loopback port.
    pub port: u16,
}

/// Reads `serve.json`, if the daemon has written one.
///
/// # Errors
/// [`CronError::Io`]/[`CronError::Json`] only when the file exists but is broken.
pub fn serve_info(paths: &CronPaths) -> Result<Option<ServeInfo>> {
    read_json(&paths.serve_info())
}

/// Is a process with `pid` alive?
#[must_use]
pub fn pid_alive(pid: u32) -> bool {
    crate::store::pid_alive(pid)
}

/// Whether a daemon from `serve.json` is reachable (pid alive and port open is checked
/// lazily by the first request; this only checks the pid).
#[must_use]
pub fn daemon_running(paths: &CronPaths) -> bool {
    match serve_info(paths) {
        Ok(Some(info)) => pid_alive(info.pid),
        _ => false,
    }
}

/// One reply line off the control socket.
#[derive(Debug, Serialize, Deserialize)]
pub enum Reply {
    /// The op's JSON payload.
    Ok(serde_json::Value),
    /// A user-facing error message.
    Error(String),
}

/// A client of the running daemon.
#[derive(Debug, Clone)]
pub struct ControlClient {
    /// Loopback port of the daemon.
    pub port: u16,
    /// Request timeout.
    pub timeout: Duration,
}

impl ControlClient {
    /// A client for the daemon recorded in `serve.json`.
    ///
    /// # Errors
    /// [`CronError::NotRunning`] when no daemon is recorded (or its pid is gone).
    pub fn connect(paths: &CronPaths) -> Result<Self> {
        let Some(info) = serve_info(paths)? else {
            return Err(CronError::NotRunning);
        };
        if !pid_alive(info.pid) {
            return Err(CronError::NotRunning);
        }
        Ok(Self {
            port: info.port,
            timeout: Duration::from_secs(10),
        })
    }

    /// Sends one request and returns the daemon's reply payload.
    ///
    /// # Errors
    /// [`CronError::Control`] carrying the daemon's error, or an I/O error.
    pub async fn send(&self, request: CronControl) -> Result<serde_json::Value> {
        let connect = async { TcpStream::connect(("127.0.0.1", self.port)).await };
        let mut stream = tokio::time::timeout(self.timeout, connect)
            .await
            .map_err(|_| CronError::control("the cron scheduler didn't answer in time"))?
            .map_err(|e| CronError::control(format!("couldn't reach the cron scheduler: {e}")))?;
        let line = serde_json::to_string(&request)
            .map_err(|e| CronError::control(format!("couldn't encode the request: {e}")))?;
        stream
            .write_all(line.as_bytes())
            .await
            .map_err(|e| CronError::control(format!("couldn't reach the cron scheduler: {e}")))?;
        stream
            .write_all(b"\n")
            .await
            .map_err(|e| CronError::control(format!("couldn't reach the cron scheduler: {e}")))?;
        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        let read = reader.read_line(&mut reply).await.map_err(|e| {
            CronError::control(format!("the cron scheduler closed the connection: {e}"))
        })?;
        if read == 0 {
            return Err(CronError::control(
                "the cron scheduler closed the connection",
            ));
        }
        match serde_json::from_str::<Reply>(reply.trim()) {
            Ok(Reply::Ok(value)) => Ok(value),
            Ok(Reply::Error(message)) => Err(CronError::control(message)),
            Err(e) => Err(CronError::control(format!(
                "bad reply from the cron scheduler: {e}"
            ))),
        }
    }
}
