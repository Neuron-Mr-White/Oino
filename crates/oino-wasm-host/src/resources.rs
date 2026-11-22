//! Host-side representations of WIT resources.

use std::path::Path;
use std::sync::Arc;

use notify::Watcher as _;
use oino_extension_api::{AgentHandle, CancellationToken, HostError};
use tokio::sync::{Mutex, mpsc};

use crate::http::Listener;
use crate::process::ChildHandle;

/// `types.cancel-token`.
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct CancelToken(pub(crate) CancellationToken);

/// `agents.agent`.
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct AgentRes(pub(crate) Arc<dyn AgentHandle>);

/// `process.child`.
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct ChildRes(pub(crate) Arc<ChildHandle>);

/// `http.listener`.
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct ListenerRes(pub(crate) Arc<Listener>);

/// `fs.lock`: an advisory lock on a file, released on `release` or drop.
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct LockRes(pub(crate) Option<std::fs::File>);

impl LockRes {
    pub(crate) async fn acquire(path: &Path, exclusive: bool) -> Result<Self, HostError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&path)?;
            if exclusive {
                file.lock()?;
            } else {
                file.lock_shared()?;
            }
            Ok::<_, std::io::Error>(file)
        })
        .await
        .map_err(|e| HostError::Failed(e.to_string()))?
        .map(|file| Self(Some(file)))
        .map_err(|e| HostError::io(&e))
    }

    pub(crate) fn release(&mut self) {
        if let Some(file) = self.0.take() {
            let _ = file.unlock();
        }
    }
}

/// One change seen by a watcher (WIT `fs-event`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct FsChange {
    pub(crate) kind: &'static str,
    pub(crate) path: String,
}

/// `fs.watcher`.
#[allow(unreachable_pub, reason = "re-exported by the generated bindings")]
pub struct WatcherRes {
    pub(crate) rx: Arc<Mutex<mpsc::UnboundedReceiver<Vec<FsChange>>>>,
    _watcher: notify::RecommendedWatcher,
}

impl WatcherRes {
    /// Watches `host` and reports paths relative to it, prefixed with `guest_prefix`.
    pub(crate) fn watch(
        host: &Path,
        guest_prefix: String,
        recursive: bool,
    ) -> Result<Self, HostError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let root = host.to_path_buf();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            let kind = match event.kind {
                notify::EventKind::Create(_) => "create",
                notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => "rename",
                notify::EventKind::Modify(_) => "modify",
                notify::EventKind::Remove(_) => "remove",
                _ => return,
            };
            let changes: Vec<FsChange> = event
                .paths
                .iter()
                .map(|p| {
                    let rel = p.strip_prefix(&root).unwrap_or(p);
                    FsChange {
                        kind,
                        path: format!("{guest_prefix}/{}", rel.to_string_lossy())
                            .replace("//", "/"),
                    }
                })
                .collect();
            if !changes.is_empty() {
                let _ = tx.send(changes);
            }
        })
        .map_err(|e| HostError::Failed(e.to_string()))?;
        let mode = if recursive {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        watcher
            .watch(host, mode)
            .map_err(|e| HostError::Failed(e.to_string()))?;
        Ok(Self {
            rx: Arc::new(Mutex::new(rx)),
            _watcher: watcher,
        })
    }
}
