//! Minimal client for one `oino --mode rpc` child: JSON lines on stdio.
//!
//! Envelope (oino-client/src/protocol.rs): requests are `{id?, type, …fields}`,
//! replies `{id?, type:"response", command, success, data? | error}`, events
//! are any other object. We speak plain pi commands (no `_oino/hello`), so the
//! server auto-cancels dialogs we cannot render.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, oneshot};

#[derive(Debug)]
pub struct RpcError(pub String);

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One spawned oino RPC session.
pub struct RpcChild {
    writer: tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>,
    next_id: AtomicU64,
    events: broadcast::Sender<Value>,
    child: Mutex<Option<Child>>,
    pub pid: u32,
}

impl RpcChild {
    /// Spawns `<bin> --mode rpc [--model provider/model] --agent-dir <dir>` in `cwd`.
    pub fn spawn(bin: &Path, agent_dir: &Path, model: Option<&str>, cwd: &Path) -> std::io::Result<Arc<Self>> {
        let mut cmd = Command::new(bin);
        cmd.args(["--mode", "rpc", "--agent-dir", &agent_dir.display().to_string()])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(model) = model {
            cmd.arg("--model").arg(model);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let (tx, _) = broadcast::channel(1024);
        let rpc = Arc::new(Self {
            writer: tokio::sync::Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events: tx,
            child: Mutex::new(Some(child)),
            pid,
        });
        if let Some(stdout) = stdout {
            let weak = Arc::downgrade(&rpc);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                    let is_response = value["type"].as_str() == Some("response");
                    if is_response && let Some(id) = value["id"].as_str() {
                        let waiter = weak
                            .upgrade()
                            .and_then(|r| r.pending.lock().ok().and_then(|mut p| p.remove(id)));
                        if let Some(waiter) = waiter {
                            let outcome = if value["success"].as_bool().unwrap_or(false) {
                                Ok(value["data"].clone())
                            } else {
                                Err(RpcError(value["error"].as_str().unwrap_or("request failed").to_owned()))
                            };
                            let _ = waiter.send(outcome);
                            continue;
                        }
                    }
                    if let Some(rpc) = weak.upgrade() {
                        let _ = rpc.events.send(value);
                    }
                }
                if let Some(rpc) = weak.upgrade() {
                    if let Ok(mut pending) = rpc.pending.lock() {
                        for (_, waiter) in pending.drain() {
                            let _ = waiter.send(Err(RpcError("oino exited".into())));
                        }
                    }
                }
            });
        }
        Ok(rpc)
    }

    /// Server events (not replies to us), from now on.
    pub fn events(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    /// Sends one command and waits for its reply.
    pub async fn request(&self, command: Value) -> Result<Value, RpcError> {
        let id = format!("b{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let request = {
            let mut request = command;
            if let Some(obj) = request.as_object_mut() {
                obj.insert("id".into(), json!(id));
            }
            request
        };
        {
            let mut guard = self.writer.lock().await;
            let Some(stdin) = guard.as_mut() else {
                self.pending.lock().unwrap().remove(&id);
                return Err(RpcError("oino exited".into()));
            };
            if let Err(e) = stdin.write_all(request.to_string().as_bytes()).await {
                self.pending.lock().unwrap().remove(&id);
                return Err(RpcError(e.to_string()));
            }
            let _ = stdin.write_all(b"\n").await;
            let _ = stdin.flush().await;
        }
        rx.await.map_err(|_| RpcError("oino exited".into()))?
    }

    /// Graceful stop: close stdin (the server exits 0), then SIGTERM.
    pub async fn stop(&self) {
        if let Some(mut stdin) = self.writer.lock().await.take() {
            let _ = stdin.shutdown().await;
        }
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await;
            let _ = child.start_kill();
        }
    }
}
