//! UNI-231 (plan A2, docs/m8 §3.3 "This computer"): on the desktop app, this
//! PC's own unipi-host is reached over its local socket `app.sock` — no
//! iroh, no relay, works offline. When the socket is there, `hosts()` adds a
//! synthetic, unpersisted host "This computer" (id = the host's endpoint id
//! from its `endpoint.json`); a paired iroh entry with the same id (this PC
//! paired the old way) merges into it instead of showing twice. `open_op`
//! prefers the socket for that id and falls back to iroh if it is gone.
//! Phone builds never see any of this.

use std::path::{Path, PathBuf};

use unipi_proto::AppErrorDto;

use crate::host_store::{self, HostInfo};

/// Name of the synthetic host.
pub const THIS_COMPUTER: &str = "This computer";

/// The local host's data dir: `$UNIPI_HOST_DIR`, else `~/.unipi/app-host`
/// (same rule as `unipi-host`'s `HostPaths::resolve`).
pub fn host_dir() -> Option<PathBuf> {
    if !cfg!(desktop) {
        return None;
    }
    // UNI-298: the same rule as unipi-host (incl. the Windows legacy dir).
    Some(unipi_proto::paths::default_host_dir())
}

/// What the local host advertises (its `endpoint.json`): `{addr:{id,…}, ticket}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalHost {
    pub id: String,
    pub ticket: String,
    pub socket: PathBuf,
}

/// The local host, when its socket file exists and it has published an endpoint.
pub fn detect_in(dir: &Path) -> Option<LocalHost> {
    let socket = dir.join("app.sock");
    if !socket.exists() {
        return None;
    }
    let raw = std::fs::read(dir.join("endpoint.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let id = v.get("addr")?.get("id")?.as_str()?.to_string();
    let ticket = v.get("ticket")?.as_str()?.to_string();
    Some(LocalHost { id, ticket, socket })
}

pub fn detect() -> Option<LocalHost> {
    detect_in(&host_dir()?)
}

/// `hosts.json` + "This computer" (merged by id; the paired entry keeps its
/// name only if the user renamed it away from the default).
pub fn merge(mut hosts: Vec<HostInfo>, local: Option<&LocalHost>) -> Vec<HostInfo> {
    let Some(local) = local else { return hosts };
    match hosts.iter_mut().find(|h| h.id == local.id) {
        Some(h) => {
            h.transport = Some("local".into());
            h.last_path = Some(crate::path_kind::PathKind::Local);
        }
        None => hosts.insert(
            0,
            HostInfo {
                id: local.id.clone(),
                name: THIS_COMPUTER.into(),
                ticket: local.ticket.clone(),
                paired_at: 0,
                last_path: Some(crate::path_kind::PathKind::Local),
                transport: Some("local".into()),
            },
        ),
    }
    hosts
}

/// Every host the app can reach: paired ones plus this PC's own (desktop).
pub fn hosts(dir: &Path) -> Result<Vec<HostInfo>, AppErrorDto> {
    Ok(merge(host_store::read_hosts(dir)?, detect().as_ref()))
}

/// The local socket for `remote_id`, when it is this PC's host.
pub fn socket_for(remote_id: &str) -> Option<PathBuf> {
    detect().filter(|l| l.id == remote_id).map(|l| l.socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_host_dir(dir: &Path, id: &str) {
        std::fs::write(dir.join("app.sock"), b"").unwrap();
        std::fs::write(dir.join("endpoint.json"), serde_json::json!({"addr":{"id":id,"addrs":[]},"ticket":"endpointlocal"}).to_string()).unwrap();
    }

    fn paired(id: &str, name: &str) -> HostInfo {
        HostInfo { id: id.into(), name: name.into(), ticket: "endpointpaired".into(), paired_at: 1, last_path: None, transport: None }
    }

    #[test]
    fn detects_only_with_a_socket_and_an_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_in(dir.path()), None);
        std::fs::write(dir.path().join("endpoint.json"), r#"{"addr":{"id":"abc"},"ticket":"t"}"#).unwrap();
        assert_eq!(detect_in(dir.path()), None, "no socket = host not running");
        write_host_dir(dir.path(), "abc");
        let l = detect_in(dir.path()).unwrap();
        assert_eq!((l.id.as_str(), l.ticket.as_str()), ("abc", "endpointlocal"));
    }

    #[test]
    fn this_computer_is_added_first_or_merged_by_id() {
        let local = LocalHost { id: "me".into(), ticket: "endpointlocal".into(), socket: "/x/app.sock".into() };
        let out = merge(vec![paired("other", "Laptop")], Some(&local));
        assert_eq!(out.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["This computer", "Laptop"]);
        assert_eq!(out[0].transport.as_deref(), Some("local"));
        let merged = merge(vec![paired("me", "oi-desktop"), paired("other", "Laptop")], Some(&local));
        assert_eq!(merged.len(), 2, "same id → one card");
        assert_eq!(merged[0].name, "oi-desktop");
        assert_eq!(merged[0].transport.as_deref(), Some("local"));
        assert_eq!(merged[0].ticket, "endpointpaired", "keeps the iroh ticket as the fallback");
        assert_eq!(merge(vec![paired("a", "A")], None).len(), 1);
    }
}

/// End to end, offline: a real `unipi-host serve` on a temp data dir with a
/// dead relay and no IP transports (iroh can't reach anything), and the
/// app's `NetState::open_op` reaching it through `app.sock` anyway.
/// Needs the host binary: `cargo build -p unipi-host` first (skips without it).
#[cfg(all(test, unix))]
mod offline_e2e {
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use unipi_proto::{frame::{encode, FrameKind}, Control, PROTO_VERSION};

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test(flavor = "multi_thread")]
    async fn this_computer_works_with_no_network() {
        let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/unipi-host");
        if !bin.exists() {
            eprintln!("SKIP: build unipi-host first ({})", bin.display());
            return;
        }
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let host_dir = tempfile::tempdir().unwrap();
        let app_dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(&bin)
            .args(["--data-dir", host_dir.path().to_str().unwrap(), "serve", "--relay", "https://127.0.0.1:9", "--relay-only", "--mode", "manual"])
            .env("HOME", host_dir.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while super::detect_in(host_dir.path()).is_none() {
            assert!(Instant::now() < deadline, "host never published app.sock + endpoint.json");
            std::thread::sleep(Duration::from_millis(100));
        }
        std::env::set_var("UNIPI_HOST_DIR", host_dir.path());
        let hosts = super::hosts(app_dir.path()).unwrap();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "This computer");
        assert_eq!(hosts[0].transport.as_deref(), Some("local"));

        let state = crate::net::NetState::default();
        assert_eq!(state.path_for(&hosts[0].ticket), crate::path_kind::PathKind::Local);
        let (mut send, mut recv) = tokio::time::timeout(Duration::from_secs(5), state.open_op(app_dir.path(), None, &hosts[0].ticket))
            .await
            .expect("open_op must not try iroh")
            .unwrap();
        assert!(matches!(send, crate::op_stream::OpSend::Local(_)));
        let hello = serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":{"op":"fs_roots"}});
        send.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&hello).unwrap())).await.unwrap();
        send.finish().unwrap();
        let kind = recv.read_u8().await.unwrap();
        let len = recv.read_u32().await.unwrap() as usize;
        let mut bytes = vec![0; len];
        recv.read_exact(&mut bytes).await.unwrap();
        assert_eq!(kind, 0);
        let reply: Control = serde_json::from_slice(&bytes).unwrap();
        assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

        std::env::remove_var("UNIPI_HOST_DIR");
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// UNI-233: one local-only admin op (host_info, pair_begin, pair_status,
/// devices_list, device_revoke) over this PC's `app.sock`. `E_NOT_FOUND`
/// when no local host is running (or its version has no socket).
#[cfg(unix)]
pub async fn admin(req: serde_json::Value) -> Result<serde_json::Value, AppErrorDto> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use unipi_proto::{frame::{encode, FrameKind}, AppErrorCode, Control, PROTO_VERSION};
    let Some(local) = detect() else {
        return Err(AppErrorDto::new(AppErrorCode::E_NOT_FOUND, "unipi-host isn't running on this PC (or is too old to manage from here)", false));
    };
    let fut = async {
        let mut s = tokio::net::UnixStream::connect(&local.socket).await.map_err(|e| AppErrorDto::new(AppErrorCode::E_TRANSPORT, format!("can't reach this PC's host: {e}"), true))?;
        let hello = serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req});
        s.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&hello).unwrap_or_default())).await.map_err(|e| AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true))?;
        let kind = s.read_u8().await.map_err(|e| AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true))?;
        let len = s.read_u32().await.map_err(|e| AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true))? as usize;
        if kind != 0 || len > unipi_proto::MAX_FRAME {
            return Err(AppErrorDto::new(AppErrorCode::E_INTERNAL, "bad reply from this PC's host", false));
        }
        let mut bytes = vec![0; len];
        s.read_exact(&mut bytes).await.map_err(|e| AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true))?;
        match serde_json::from_slice::<Control>(&bytes).map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))? {
            Control::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
            Control::Err { error } => Err(error),
            _ => Err(AppErrorDto::new(AppErrorCode::E_INTERNAL, "unexpected reply", false)),
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), fut)
        .await
        .map_err(|_| AppErrorDto::new(unipi_proto::AppErrorCode::E_TIMEOUT, "this PC's host didn't answer", true))?
}

#[cfg(not(unix))]
pub async fn admin(_req: serde_json::Value) -> Result<serde_json::Value, AppErrorDto> {
    Err(AppErrorDto::new(unipi_proto::AppErrorCode::E_UNSUPPORTED, "Managing this PC's host from the app needs Linux or macOS for now", false))
}

/// Ops the UI may send through `local_admin` (nothing else reaches the host this way).
pub const ADMIN_OPS: [&str; 5] = ["host_info", "pair_begin", "pair_status", "devices_list", "device_revoke"];

#[tauri::command]
pub async fn local_admin(req: serde_json::Value) -> Result<serde_json::Value, AppErrorDto> {
    let op = req.get("op").and_then(|v| v.as_str()).unwrap_or("");
    if !ADMIN_OPS.contains(&op) {
        return Err(AppErrorDto::new(unipi_proto::AppErrorCode::E_SCHEMA, format!("not an admin op: {op}"), false));
    }
    // Validate against the shared schema before it leaves the app.
    let hello = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    let parsed: unipi_proto::Control = serde_json::from_value(hello).map_err(|e| AppErrorDto::new(unipi_proto::AppErrorCode::E_SCHEMA, e.to_string(), false))?;
    parsed.validate().map_err(|e| e.to_app_error())?;
    admin(req).await
}

/// How many phones are paired with this PC's host (its `devices.json`
/// allowlist) — the tray line and the Quit confirm (UNI-237).
pub fn paired_devices() -> usize {
    let Some(dir) = host_dir() else { return 0 };
    let Ok(raw) = std::fs::read_to_string(dir.join("devices.json")) else { return 0 };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else { return 0 };
    match value {
        serde_json::Value::Array(list) => list.len(),
        serde_json::Value::Object(map) => map.get("devices").and_then(|d| d.as_array()).map(|a| a.len()).unwrap_or(0),
        _ => 0,
    }
}

#[cfg(test)]
mod paired_tests {
    use super::*;

    #[test]
    fn counts_devices_from_the_hosts_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("UNIPI_HOST_DIR", dir.path());
        assert_eq!(paired_devices(), 0, "no file = no phones");
        std::fs::write(dir.path().join("devices.json"), r#"[{"id":"a"},{"id":"b"}]"#).unwrap();
        assert_eq!(paired_devices(), 2);
        std::fs::write(dir.path().join("devices.json"), r#"{"devices":[{"id":"a"}]}"#).unwrap();
        assert_eq!(paired_devices(), 1);
        std::env::remove_var("UNIPI_HOST_DIR");
    }
}
