//! UNI-229 (plan H2, docs/m8 §3.3): the local listener `<dir>/app.sock`.
//! The UniPi desktop app on this PC reaches its own host here — no iroh, no
//! relay, works offline. Each CONNECTION is one logical op, exactly like one
//! QUIC bi-stream: the same `u8 kind · u32 len` frames and the same first
//! `Hello`, handled by the same `Handler::handle` with `Peer::Local`.
//!
//! Trust: only processes of the same user (peer uid == ours, checked with
//! SO_PEERCRED / getpeereid) — that user can already read `secret.key`,
//! `devices.json` and the pi sessions, so this gives nothing new away. The
//! socket file is 0600 inside the 0700 data dir as a second fence.

use crate::{data_dir::HostPaths, protocol::Handler};

impl HostPaths {
    pub fn app_socket(&self) -> std::path::PathBuf {
        self.root.join("app.sock")
    }
}

/// Is this peer the same user as us?
#[cfg(unix)]
pub fn same_user(stream: &tokio::net::UnixStream) -> bool {
    match stream.peer_cred() {
        Ok(cred) => cred.uid() == unsafe { libc::geteuid() },
        Err(error) => {
            tracing::warn!(%error, "app.sock: can't read the peer's uid; refusing");
            false
        }
    }
}

/// Bind `app.sock` and serve it until the returned task is aborted (or the
/// listener fails). The caller holds the instance lock, so a leftover socket
/// file from a dead host is ours to replace.
#[cfg(unix)]
pub fn serve(handler: Handler) -> std::io::Result<tokio::task::JoinHandle<()>> {
    use std::os::unix::fs::PermissionsExt;
    let path = handler.paths.app_socket();
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!(path = %path.display(), "local socket ready");
    Ok(tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(error) => {
                    tracing::warn!(%error, "app.sock accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    continue;
                }
            };
            if !same_user(&stream) {
                tracing::warn!("app.sock: refused a connection from another user");
                continue;
            }
            let handler = handler.clone();
            tokio::spawn(async move {
                let (read, write) = stream.into_split();
                if let Err(error) = handler.handle(read, write, crate::protocol::Peer::Local).await {
                    tracing::debug!(%error, "local stream ended");
                }
            });
        }
    }))
}

/// Remove the socket file (shutdown).
pub fn cleanup(paths: &HostPaths) {
    let _ = std::fs::remove_file(paths.app_socket());
}

#[cfg(not(unix))]
pub fn serve(_handler: Handler) -> std::io::Result<tokio::task::JoinHandle<()>> {
    // Windows: a named pipe with an owner-only DACL comes with the Windows host.
    Ok(tokio::spawn(async {}))
}

#[cfg(all(test, unix))]
mod tests {
    #[tokio::test]
    async fn same_user_accepts_our_own_process() {
        let (a, _b) = tokio::net::UnixStream::pair().unwrap();
        assert!(super::same_user(&a));
    }
}
