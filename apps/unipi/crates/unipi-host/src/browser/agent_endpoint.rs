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
//! Every connection (HTTP `/json*` discovery or a `DevTools` websocket):
//! 1. loads config.json — an explicit `enabled: false` refuses;
//! 2. starts or reuses the **active profile's** browser (`default`, or the
//!    one picked with `unipi-host browser use <profile>`) — so the first
//!    `agent-browser` command starts the browser on the hidden display;
//! 3. is spliced byte-for-byte onto that browser's gate port, which does
//!    everything else (take-over check, humanized input, scoped
//!    `Browser.close`). The gate rewrites ports in `/json*` bodies to
//!    itself, so we rewrite the gate's port to ours on the way back.
//!
//! UNI-303 FIX 4: the address also understands `?profile=<name>` (in the
//! query string) or a `;profile=<name>` path segment. When it names a
//! different profile, the connection lands on THAT profile's browser
//! (started if needed) — each chat's `AGENT_BROWSER_CDP` carries its own
//! profile, so two chats drive two browsers. No `?profile=` means the
//! file's default profile, as before.

use std::fmt::Write as _;
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

#[must_use]
pub fn path(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// The recorded endpoint, if any.
#[must_use]
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
/// # Errors
/// Returns an error if endpoint metadata or the local listener cannot be created or accessed.
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
/// # Errors
/// Returns an error if endpoint metadata or the local listener cannot be created or accessed.
pub async fn serve(paths: HostPaths) -> anyhow::Result<u16> {
    let listener = bind(&paths.root).await?;
    let port = listener.local_addr()?.port();
    let profile = read(&paths.root).map_or_else(|| profiles::DEFAULT.to_string(), |e| e.profile);
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
    // UNI-303 FIX 4: the requested profile rides on the address
    // (`…?profile=<name>` query or `…;profile=<name>` segment); without one
    // this is the stable default address (`browser-agent.json`).
    let mut head = vec![0u8; 16 * 1024];
    let n = client.peek(&mut head).await?;
    let requested = requested_profile(&String::from_utf8_lossy(&head[..n]));
    let config = BrowserConfig::load(&paths.config())?;
    if config.explicit_off {
        refuse(
            &mut client,
            "The browser is off on this PC. Ask the user to turn it on in the Unipi app.",
        )
        .await;
        return Ok(());
    }
    let profile = match requested.as_deref() {
        Some(name) => {
            if let Err(error) = profiles::check_name(name) {
                refuse(
                    &mut client,
                    &format!("Bad browser profile {name:?}: {error}"),
                )
                .await;
                return Ok(());
            }
            name.to_string()
        }
        None => read(&paths.root).map_or_else(|| profiles::DEFAULT.to_string(), |e| e.profile),
    };
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
    // UNI-303 FIX 4: when the address asked for a profile the endpoint file
    // doesn't point at, drop the key and — for a websocket upgrade on a path
    // the browser wouldn't serve (`/`, `/?profile=…`) — point it at THAT
    // profile's root browser websocket, so `agent-browser --cdp
    // ws://127.0.0.1:<stable>/?profile=<name>` (its documented query form)
    // lands as a normal CDP browser socket. Plain paths (`/devtools/…`) pass
    // through unchanged; they already belong to the resolved browser.
    let is_ws_upgrade = String::from_utf8_lossy(&head[..n])
        .to_ascii_lowercase()
        .contains("upgrade: websocket");
    let head = strip_profile_key(head, n);
    let head = if requested.is_some() && is_ws_upgrade && root_ws_path_is_better(&head) {
        swap_ws_path(head, running.ws_url.as_str())
    } else {
        head
    };
    let upstream = TcpStream::connect(("127.0.0.1", gate)).await?;
    splice(client, upstream, gate, ours, Some(head), n).await
}

/// True when the (already key-stripped) head upgrades on a path that isn't
/// the browser's own root websocket (e.g. `/`) — i.e. the client only knows
/// the stable address, not the per-browser id path.
fn root_ws_path_is_better(head: &[u8]) -> bool {
    let target = String::from_utf8_lossy(head)
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    let path = target.split(['?', ';']).next().unwrap_or(&target);
    !(path.starts_with("/devtools/") || path.contains("/devtools/"))
}

/// Replaces the upgrade target's path with `ws_url`'s (the running browser's
/// root browser websocket), keeping the client's other query params.
fn swap_ws_path(mut head: Vec<u8>, ws_url: &str) -> Vec<u8> {
    let text = String::from_utf8_lossy(&head).into_owned();
    let Some((line, rest)) = text.split_once('\n') else {
        return head;
    };
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let Some(target) = parts.next() else {
        return head;
    };
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();
    // The browser's root websocket path: everything after host:port.
    let Some((_, browser_path)) = ws_url.split_once("//") else {
        return head;
    };
    let Some((_, browser_path)) = browser_path.split_once('/') else {
        return head;
    };
    let browser_path = format!("/{browser_path}");
    // Keep any client query that isn't our routing key.
    let kept_query: String = target
        .split_once('?')
        .map(|(_, q)| {
            q.split('&')
                .filter(|p| !p.is_empty() && !p.starts_with("profile="))
                .collect::<Vec<_>>()
                .join("&")
        })
        .unwrap_or_default();
    let new_target = if kept_query.is_empty() {
        browser_path
    } else {
        format!("{browser_path}?{kept_query}")
    };
    let replaced = format!("{method} {new_target} {version}\r\n{rest}");
    let bytes = replaced.as_bytes();
    head.clear();
    head.extend_from_slice(bytes);
    head
}

/// Drops `?profile=…` from the request target (path or query) so the routed
/// connection looks like a plain one to the browser and the gate.
fn strip_profile_key(mut head: Vec<u8>, n: usize) -> Vec<u8> {
    let text = String::from_utf8_lossy(&head[..n]).into_owned();
    let Some((line, rest)) = text.split_once('\n') else {
        return head;
    };
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let Some(target) = parts.next() else {
        return head;
    };
    let version = parts.next().unwrap_or("HTTP/1.1");
    let cleaned = if let Some((base, query)) = target.split_once('?') {
        let kept: Vec<&str> = query
            .split('&')
            .filter(|p| !p.starts_with("profile="))
            .collect();
        if kept.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{}", kept.join("&"))
        }
    } else {
        // Drop `;profile=<name>` segments, keep everything else in order.
        let mut out = String::new();
        let mut rest = target;
        while let Some((before, after)) = rest.split_once(';') {
            out.push_str(before);
            let segment = after.split(['/', '?']).next().unwrap_or(after);
            if !segment.starts_with("profile=") {
                out.push(';');
                out.push_str(segment);
            }
            rest = &after[segment.len().min(after.len())..];
        }
        out.push_str(rest);
        while out.contains("//") {
            out = out.replace("//", "/");
        }
        out
    };
    let new_line = format!("{method} {cleaned} {version}\r\n");
    let replaced = format!("{new_line}{rest}");
    let bytes = replaced.as_bytes();
    head[..bytes.len()].copy_from_slice(bytes);
    head.truncate(bytes.len());
    head
}

/// The profile requested on the address (`?profile=<name>` in the query
/// string, or a `;profile=<name>` segment in the path), `None` when the
/// address doesn't ask for one (then the stable default applies).
fn requested_profile(head: &str) -> Option<String> {
    let target = head.lines().next()?.split_whitespace().nth(1)?;
    if let Some((_, query)) = target.split_once('?') {
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            if key == "profile" && !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    let path = target.split('?').next().unwrap_or(target);
    let mut rest = path;
    while let Some((_, after)) = rest.split_once(';') {
        let segment = after.split('/').next().unwrap_or(after);
        if let Some(value) = segment.strip_prefix("profile=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        rest = after;
    }
    None
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
    head: Option<Vec<u8>>,
    peeked: usize,
) -> anyhow::Result<()> {
    // Already peeked in `connection` (the request head survives the peek).
    let is_ws = if let Some(head) = &head {
        String::from_utf8_lossy(head)
            .to_ascii_lowercase()
            .contains("upgrade: websocket")
    } else {
        let mut head = vec![0u8; 16 * 1024];
        let n = client.peek(&mut head).await?;
        String::from_utf8_lossy(&head[..n])
            .to_ascii_lowercase()
            .contains("upgrade: websocket")
    };
    let (mut client_rx, mut client_tx) = client.into_split();
    let (mut up_rx, mut up_tx) = upstream.into_split();
    let to_gate = tokio::spawn(async move {
        // The head was possibly rewritten in place (profile key stripped, ws
        // path swapped), so it can be SHORTER than the original: consume
        // exactly `peeked` bytes (the original head) from the socket, then
        // send the rewritten one — otherwise the gate sees the unmodified
        // request, or stale trailing bytes corrupt the stream.
        if let Some(head) = head {
            let mut skip = vec![0u8; peeked];
            if client_rx.read_exact(&mut skip).await.is_ok() {
                let _ = up_tx.write_all(&head).await;
            }
        }
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
#[must_use]
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
            let _ = write!(out, "Content-Length: {}\r\n", body.len());
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

    #[test]
    fn the_requested_profile_comes_from_the_query_or_path_segment() {
        let get = |target: &str| format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(
            requested_profile(&get("/json/version?profile=shop")),
            Some("shop".into())
        );
        assert_eq!(
            requested_profile(&get("/devtools/browser/x?profile=shop")),
            Some("shop".into())
        );
        assert_eq!(
            requested_profile(&get("/;profile=shop/json/version")),
            Some("shop".into())
        );
        assert_eq!(
            requested_profile(&get("/devtools/page/x;profile=shop")),
            Some("shop".into())
        );
        // No profile on the address: the stable default applies.
        assert_eq!(requested_profile(&get("/json/version")), None);
        assert_eq!(requested_profile(&get("/devtools/browser/x")), None);
        // Other query keys and empty values don't count.
        assert_eq!(requested_profile(&get("/json/list?wide=1")), None);
        assert_eq!(requested_profile(&get("/json/version?profile=")), None);
    }

    #[test]
    fn the_routing_key_is_stripped_from_the_forwarded_head() {
        let strip = |target: &str| {
            let head = format!("GET {target} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n");
            let len = head.len();
            String::from_utf8(strip_profile_key(head.into_bytes(), len)).unwrap()
        };
        assert_eq!(
            strip("/devtools/browser/x?profile=shop"),
            "GET /devtools/browser/x HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n"
        );
        assert_eq!(
            strip("/;profile=shop/json/version"),
            "GET /json/version HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n"
        );
        // Other query keys survive; only the routing key goes.
        assert_eq!(
            strip("/json/list?a=1&profile=shop&b=2"),
            "GET /json/list?a=1&b=2 HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n"
        );
        // A plain head is untouched.
        assert_eq!(
            strip("/json/version"),
            "GET /json/version HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\n\r\n"
        );
    }

    #[test]
    fn a_keyed_root_upgrade_swaps_to_the_browsers_own_ws_path() {
        // agent-browser's form: ws://127.0.0.1:<stable>/?profile=<name> — the
        // endpoint must upgrade on THAT profile's root browser websocket.
        assert!(root_ws_path_is_better(
            b"GET /?profile=shop HTTP/1.1\r\n\r\n"
        ));
        assert!(root_ws_path_is_better(b"GET / HTTP/1.1\r\n\r\n"));
        assert!(!root_ws_path_is_better(
            b"GET /devtools/browser/x HTTP/1.1\r\n\r\n"
        ));
        let head = b"GET /?profile=shop HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nSec-WebSocket-Key: k\r\n\r\n";
        let swapped = swap_ws_path(
            head.to_vec(),
            "ws://127.0.0.1:40987/devtools/browser/abc-123",
        );
        let out = String::from_utf8(swapped).unwrap();
        assert!(
            out.starts_with("GET /devtools/browser/abc-123 HTTP/1.1\r\n"),
            "{out}"
        );
        assert!(out.contains("Host: x"), "other headers kept: {out}");
        assert!(!out.contains("profile="), "key gone: {out}");
        // Client's own query params (not our key) survive the swap.
        let with_q = b"GET /?pacing=ack&profile=shop HTTP/1.1\r\n\r\n";
        let swapped = swap_ws_path(with_q.to_vec(), "ws://127.0.0.1:1/devtools/browser/id");
        assert!(String::from_utf8(swapped)
            .unwrap()
            .contains("/devtools/browser/id?pacing=ack"));
    }
}
