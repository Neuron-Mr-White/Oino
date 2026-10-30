//! UNI-303: one **stable** agent address per host.
//!
//! Before this, an agent had to run `unipi-host browser start --json`, read
//! the per-browser gate port (`agent_http`, a new random port each launch)
//! and `agent-browser connect <port>`. Agents skipped or fumbled that and
//! launched their own Chrome nobody could see.
//!
//! Now `unipi-host serve` listens on one 127.0.0.1 port for its whole life
//! and writes it to `<host dir>/browser-agent.json` (`{port, profile}`). Pi
//! puts that address in `AGENT_BROWSER_CDP` while the browser is on, so a
//! plain `agent-browser open …` drives the browser the user sees in the app.
//!
//! Every connection (HTTP `/json*` discovery or a DevTools websocket):
//! 1. loads config.json — an explicit `enabled: false` refuses;
//! 2. starts or reuses the **active profile's** browser (`default`, or the
//!    one picked with `unipi-host browser use <profile>`) — so the first
//!    `agent-browser` command starts the browser on the hidden display;
//! 3. is spliced byte-for-byte onto that browser's gate port, which does
//!    everything else (take-over check, humanized input, scoped
//!    `Browser.close`). The gate rewrites ports in `/json*` bodies to
//!    itself, so we rewrite the gate's port to ours on the way back.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{profiles, BrowserConfig, Manager};
use crate::data_dir::HostPaths;

/// `<host dir>/browser-agent.json`.
pub const FILE: &str = "browser-agent.json";

/// What `browser-agent.json` holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEndpoint {
    /// The stable port on 127.0.0.1 (`AGENT_BROWSER_CDP`).
    pub port: u16,
    /// The profile a new connection drives.
    pub profile: String,
    /// The serve's pid (stale file detection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// The recorded endpoint, if any.
pub fn read(root: &Path) -> Option<AgentEndpoint> {
    serde_json::from_str(&std::fs::read_to_string(path(root)).ok()?).ok()
}

fn write(root: &Path, endpoint: &AgentEndpoint) -> std::io::Result<()> {
    let tmp = root.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(endpoint)?)?;
    std::fs::rename(tmp, path(root))
}

/// Picks the profile new agent connections drive (`browser use`). Keeps
/// the port; works whether or not the serve runs (it re-reads the file).
pub fn set_profile(root: &Path, profile: &str) -> anyhow::Result<AgentEndpoint> {
    profiles::check_name(profile)?;
    let mut endpoint = read(root)
        .ok_or_else(|| anyhow::anyhow!("the host isn't running (no {FILE}); start it first"))?;
    endpoint.profile = profile.to_string();
    write(root, &endpoint)?;
    Ok(endpoint)
}

/// The port to listen on: the last one recorded (so `AGENT_BROWSER_CDP`
/// stays valid across host restarts) when it is free, else any.
async fn bind(root: &Path) -> std::io::Result<TcpListener> {
    if let Some(previous) = read(root) {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", previous.port)).await {
            return Ok(listener);
        }
    }
    TcpListener::bind("127.0.0.1:0").await
}

/// Starts the stable endpoint for this host (call once from `serve`).
pub async fn serve(paths: HostPaths) -> anyhow::Result<u16> {
    let listener = bind(&paths.root).await?;
    let port = listener.local_addr()?.port();
    let profile = read(&paths.root)
        .map(|e| e.profile)
        .unwrap_or_else(|| profiles::DEFAULT.to_string());
    write(
        &paths.root,
        &AgentEndpoint {
            port,
            profile,
            pid: Some(std::process::id()),
        },
    )?;
    tracing::info!(op = "browser_agent", port, "stable agent endpoint ready");
    let paths = Arc::new(paths);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let paths = paths.clone();
            tokio::spawn(async move {
                if let Err(error) = connection(&paths, stream, port).await {
                    tracing::debug!(op = "browser_agent", %error, "connection ended");
                }
            });
        }
    });
    Ok(port)
}

/// Removes the file at host exit (a stale port would point agents nowhere).
pub fn forget(root: &Path) {
    if read(root).is_some_and(|e| e.pid == Some(std::process::id())) {
        let _ = std::fs::remove_file(path(root));
    }
}

async fn connection(paths: &HostPaths, mut client: TcpStream, ours: u16) -> anyhow::Result<()> {
    let config = BrowserConfig::load(&paths.config())?;
    if config.explicit_off {
        refuse(
            &mut client,
            "The browser is off on this PC. Ask the user to turn it on in the UniPi app.",
        )
        .await;
        return Ok(());
    }
    let profile = read(&paths.root)
        .map(|e| e.profile)
        .unwrap_or_else(|| profiles::DEFAULT.to_string());
    let manager = Manager::for_paths(paths);
    let running = match manager.start(&profile, &config, None).await {
        Ok((_, running)) => running,
        Err(error) => {
            refuse(
                &mut client,
                &format!("The browser could not start: {}", error.message.as_str()),
            )
            .await;
            return Ok(());
        }
    };
    let Some(gate) = running.gate_port() else {
        refuse(
            &mut client,
            "The browser's agent endpoint isn't ready yet; try again in a moment.",
        )
        .await;
        return Ok(());
    };
    let upstream = TcpStream::connect(("127.0.0.1", gate)).await?;
    splice(client, upstream, gate, ours).await
}

/// Pipes client ↔ gate. Responses have the gate's port rewritten to ours
/// (it only appears in `/json*` bodies and their `Content-Length` changes
/// with it — so rewrite whole HTTP replies, and pass websocket frames as
/// they are).
async fn splice(
    client: TcpStream,
    upstream: TcpStream,
    gate: u16,
    ours: u16,
) -> anyhow::Result<()> {
    let mut head = vec![0u8; 16 * 1024];
    let n = client.peek(&mut head).await?;
    let is_ws = String::from_utf8_lossy(&head[..n])
        .to_ascii_lowercase()
        .contains("upgrade: websocket");
    let (mut client_rx, mut client_tx) = client.into_split();
    let (mut up_rx, mut up_tx) = upstream.into_split();
    let to_gate = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut client_rx, &mut up_tx).await;
        let _ = up_tx.shutdown().await;
    });
    if is_ws {
        let _ = tokio::io::copy(&mut up_rx, &mut client_tx).await;
    } else {
        let mut reply = Vec::new();
        up_rx.read_to_end(&mut reply).await?;
        client_tx
            .write_all(&rewrite_reply(&reply, gate, ours))
            .await?;
    }
    let _ = client_tx.shutdown().await;
    to_gate.abort();
    Ok(())
}

/// An HTTP reply with `127.0.0.1:<gate>` swapped for ours and the
/// `Content-Length` fixed to match.
pub fn rewrite_reply(reply: &[u8], gate: u16, ours: u16) -> Vec<u8> {
    let Some(split) = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
    else {
        return reply.to_vec();
    };
    let (head, body) = reply.split_at(split);
    let body = super::gate::rewrite_ports(&String::from_utf8_lossy(body), gate, ours);
    let head = String::from_utf8_lossy(head);
    let mut out = String::new();
    for line in head.split("\r\n") {
        if line.is_empty() {
            continue;
        }
        if line.to_ascii_lowercase().starts_with("content-length:") {
            out.push_str(&format!("Content-Length: {}\r\n", body.len()));
        } else {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body.as_bytes());
    bytes
}

/// A plain HTTP 503 with a reason an agent (and its user) can read.
async fn refuse(client: &mut TcpStream, message: &str) {
    let body = serde_json::json!({ "error": message }).to_string();
    let reply = format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = client.write_all(reply.as_bytes()).await;
    let _ = client.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_point_at_the_stable_port_with_a_fixed_length() {
        let body = r#"{"webSocketDebuggerUrl":"ws://127.0.0.1:41234/devtools/browser/x"}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let out = String::from_utf8(rewrite_reply(reply.as_bytes(), 41234, 9)).unwrap();
        assert!(out.contains("ws://127.0.0.1:9/devtools/browser/x"), "{out}");
        let (head, new_body) = out.split_once("\r\n\r\n").unwrap();
        assert!(
            head.contains(&format!("Content-Length: {}", new_body.len())),
            "{head}"
        );
    }

    #[test]
    fn the_file_round_trips_and_profile_switches_keep_the_port() {
        let dir = tempfile::tempdir().unwrap();
        assert!(set_profile(dir.path(), "shop").is_err(), "no host yet");
        write(
            dir.path(),
            &AgentEndpoint {
                port: 4242,
                profile: "default".into(),
                pid: Some(1),
            },
        )
        .unwrap();
        let switched = set_profile(dir.path(), "shop").unwrap();
        assert_eq!((switched.port, switched.profile.as_str()), (4242, "shop"));
        assert!(set_profile(dir.path(), "Bad Name!").is_err());
        assert_eq!(read(dir.path()).unwrap().profile, "shop");
    }

    #[tokio::test]
    async fn the_port_is_reused_across_restarts_when_free() {
        let dir = tempfile::tempdir().unwrap();
        let first = bind(dir.path()).await.unwrap();
        let port = first.local_addr().unwrap().port();
        drop(first);
        write(
            dir.path(),
            &AgentEndpoint {
                port,
                profile: "default".into(),
                pid: None,
            },
        )
        .unwrap();
        let again = bind(dir.path()).await.unwrap();
        assert_eq!(again.local_addr().unwrap().port(), port);
    }
}
