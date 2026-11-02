//! Sharing with phones (UNI-248), app side.
//!
//! Remote (any paired PC, phone or desktop app):
//! - `feature_request { hostId, feature, scope?, note? }` → `FeatureRequestItem`
//! - `feature_requests { hostId }` → `FeatureRequestsData`
//!
//! Local (desktop app only, the PC this app runs on):
//! - `local_host` → `{ hostDir, hostId?, bin? }`: the host's PUBLIC endpoint id
//!   from `<host dir>/endpoint.json` (never `secret.key`), so the UI can tell
//!   "this computer" apart from other PCs.
//! - `sharing_status` → `unipi-host sharing status --json`.
//! - `sharing_change { change }` → runs one whitelisted `unipi-host …` command
//!   (the same CLI the owner would type: one code path for config.json), then
//!   returns the fresh status. The serve process re-reads config.json on every
//!   request, so nothing needs a restart.
//!
//! The host dir is `$OINO_HOST_DIR` or `~/.oino/host` (`local_host::host_dir`,
//! the same rule as unipi-host); the binary is `$UNIPI_HOST_BIN`, `~/.local/bin/unipi-host`, or
//! `unipi-host` on `PATH`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use unipi_proto::{
    AppErrorCode, AppErrorDto, FeatureKind, FeatureNote, FeatureScope, ProtoError, Request,
};

use crate::net::NetState;

const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

fn schema(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
}

fn directory(app: &tauri::AppHandle) -> Result<PathBuf, AppErrorDto> {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

async fn remote(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    req: Request,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|h| h.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let id = crate::net::parse_ticket(&host.ticket)?
        .endpoint_addr()
        .id
        .to_string();
    let data = crate::retry::retry_once(|fresh| {
        let (dir, ticket, id, req) = (&dir, &host.ticket, &id, req.clone());
        async move {
            if fresh {
                state.drop_connection(id);
            }
            let relay = state.current_relay().await;
            let (mut send, mut recv) = state.open_op(dir, relay.as_deref(), ticket).await?;
            let data = crate::browser::request(&mut send, &mut recv, req, REPLY_TIMEOUT).await;
            let _ = send.finish();
            data
        }
    })
    .await?;
    Ok(data.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub async fn feature_request(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    feature: FeatureKind,
    scope: Option<FeatureScope>,
    note: Option<String>,
) -> Result<serde_json::Value, AppErrorDto> {
    let note = note
        .filter(|n| !n.trim().is_empty())
        .map(FeatureNote::try_from)
        .transpose()
        .map_err(schema)?;
    remote(
        &app,
        &state,
        &host_id,
        Request::FeatureRequest {
            feature,
            scope: scope.unwrap_or_default(),
            note,
        },
    )
    .await
}

#[tauri::command]
pub async fn feature_requests(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
) -> Result<serde_json::Value, AppErrorDto> {
    remote(&app, &state, &host_id, Request::FeatureRequests).await
}

// ------------------------------------------------------------ local host

pub fn host_dir() -> PathBuf {
    // UNI-298: the same rule as unipi-host (crates/proto paths.rs).
    unipi_proto::paths::default_host_dir()
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The binary's file name on this OS.
pub const BIN_NAME: &str = if cfg!(windows) {
    "unipi-oino-host.exe"
} else {
    "unipi-oino-host"
};

/// Where to look for the host outside the app bundle, in order (UNI-266):
/// `$UNIPI_HOST_BIN`; `~/.local/bin` (older standalone installs on
/// Linux/macOS), `%LOCALAPPDATA%\UniPi`; then `PATH` (also
/// `/opt/homebrew/bin` and `/usr/local/bin`, which a macOS app started from
/// Finder doesn't have on its PATH).
pub fn find_bin(
    env_bin: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    path_var: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    find_bin_in(
        env_bin,
        home,
        std::env::var_os("LOCALAPPDATA").as_deref().map(Path::new),
        path_var,
    )
}

pub fn find_bin_in(
    env_bin: Option<&std::ffi::OsStr>,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    path_var: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(bin) = env_bin.filter(|b| !b.is_empty()) {
        let bin = PathBuf::from(bin);
        return is_executable(&bin).then_some(bin);
    }
    let mut candidates = Vec::new();
    if let Some(home) = home {
        candidates.push(home.join(".local").join("bin").join(BIN_NAME));
    }
    if let Some(lad) = local_app_data {
        candidates.push(lad.join("UniPi").join(BIN_NAME));
    }
    if let Some(p) = path_var {
        candidates.extend(std::env::split_paths(p).map(|d| d.join(BIN_NAME)));
    }
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/opt/homebrew/bin").join(BIN_NAME));
        candidates.push(PathBuf::from("/usr/local/bin").join(BIN_NAME));
    }
    candidates.into_iter().find(|p| is_executable(p))
}

/// `$HOME`, or `%USERPROFILE%` on Windows (no HOME outside ssh/Git Bash).
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The binary or the "not installed" error.
pub fn resolve_bin() -> Result<PathBuf, AppErrorDto> {
    bin().ok_or_else(not_installed)
}

/// The directories the app bundle ships its host sidecar in (beside the exe,
/// the resource dir — `host_supervisor::sidecar_dirs`), recorded once at
/// startup so the app-handle-less commands (sharing, pairing) find the same
/// host the supervisor runs.
static SIDECAR_DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();

/// Records [`SIDECAR_DIRS`] (the first call wins).
pub fn set_sidecar_dirs(dirs: Vec<PathBuf>) {
    let _ = SIDECAR_DIRS.set(dirs);
}

/// The host binary: `$UNIPI_HOST_BIN` (an explicit override), else the
/// bundled sidecar — the installer always ships it — else an older
/// standalone install (`~/.local/bin`, `PATH`).
pub fn bin() -> Option<PathBuf> {
    bin_in(
        std::env::var_os("UNIPI_HOST_BIN").as_deref(),
        SIDECAR_DIRS.get().map_or(&[][..], Vec::as_slice),
        || find_bin(None, home().as_deref(), std::env::var_os("PATH").as_deref()),
    )
}

/// [`bin`]'s order over explicit inputs (tests).
pub fn bin_in(
    env_bin: Option<&std::ffi::OsStr>,
    sidecar_dirs: &[PathBuf],
    otherwise: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(bin) = env_bin.filter(|b| !b.is_empty()) {
        let bin = PathBuf::from(bin);
        return is_executable(&bin).then_some(bin);
    }
    crate::host_supervisor::sidecar_in(sidecar_dirs).or_else(otherwise)
}

/// Only the public id from endpoint.json.
pub fn read_host_id(dir: &Path) -> Option<String> {
    #[derive(Deserialize)]
    struct Addr {
        id: String,
    }
    #[derive(Deserialize)]
    struct Cache {
        addr: Addr,
    }
    let bytes = std::fs::read(dir.join("endpoint.json")).ok()?;
    let cache: Cache = serde_json::from_slice(&bytes).ok()?;
    (cache.addr.id.len() == 64 && cache.addr.id.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(cache.addr.id)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalHost {
    pub host_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin: Option<String>,
}

fn unsupported() -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_UNSUPPORTED,
        "Only the desktop app can change this PC's sharing",
        false,
    )
}

#[tauri::command]
pub fn local_host() -> Result<LocalHost, AppErrorDto> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(unsupported());
    }
    let dir = host_dir();
    Ok(LocalHost {
        host_id: read_host_id(&dir),
        host_dir: dir.display().to_string(),
        bin: bin().map(|b| b.display().to_string()),
    })
}

/// One change the desktop app may make. Mapped to a fixed argv below — the UI
/// never passes free-form arguments.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SharingChange {
    ForwardEnable {
        #[serde(default)]
        any: bool,
        #[serde(default)]
        ports: Vec<u16>,
    },
    BrowserEnable {
        #[serde(default)]
        headed: bool,
        /// `Some("")` = auto-detect again.
        #[serde(default)]
        path: Option<String>,
    },
    DesktopEnable {
        #[serde(default)]
        view_only: bool,
    },
    PushEnable,
    /// UNI-317/UNI-318: let phones use `cron_*` ops.
    CronEnable,
    Disable {
        feature: FeatureKind,
    },
    Approve {
        id: String,
        /// `1h`, `30m`, `8h`, `1d`, or none = until turned off.
        #[serde(default, rename = "for")]
        for_: Option<String>,
    },
    Deny {
        id: String,
    },
    LingerEnable,
    /// UNI-263: `unipi-host update` (download, verify, replace, restart the service).
    HostUpdate,
    /// UNI-263: the owner's update setting (user decision 6).
    HostUpdateMode {
        auto: bool,
    },
}

fn valid_id(id: &str) -> bool {
    id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The `unipi-host` arguments for a change.
pub fn argv(change: &SharingChange) -> Result<Vec<String>, AppErrorDto> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Ok(match change {
        SharingChange::ForwardEnable { any: true, .. } => s(&["forward", "enable", "--any"]),
        SharingChange::ForwardEnable { ports, .. } => {
            if ports.is_empty() || ports.len() > 64 || ports.contains(&0) {
                return Err(schema("pick at least one port (1–65535)"));
            }
            let mut a = s(&["forward", "enable", "--only"]);
            for p in ports {
                a.push(format!("--port={p}"));
            }
            a
        }
        SharingChange::BrowserEnable { headed, path } => {
            let mut a = s(&[
                "browser",
                "enable",
                if *headed { "--headed" } else { "--headless" },
            ]);
            if let Some(path) = path {
                if path.len() > 4096 || path.contains('\n') {
                    return Err(schema("browser path is too long"));
                }
                a.push(format!("--path={}", path.trim()));
            }
            a
        }
        SharingChange::DesktopEnable { view_only } => s(&[
            "desktop",
            "enable",
            if *view_only {
                "--view-only"
            } else {
                "--control"
            },
        ]),
        SharingChange::PushEnable => s(&["push", "enable"]),
        SharingChange::CronEnable => s(&["cron", "share"]),
        SharingChange::Disable { feature } => vec![
            feature.as_str().to_string(),
            if matches!(feature, FeatureKind::Cron) {
                "unshare".into()
            } else {
                "disable".into()
            },
        ],
        SharingChange::Approve { id, for_ } => {
            if !valid_id(id) {
                return Err(schema("bad request id"));
            }
            let mut a = s(&["requests", "approve", id]);
            if let Some(f) = for_ {
                if !["15m", "1h", "8h", "1d"].contains(&f.as_str()) {
                    return Err(schema("time limit must be 15m, 1h, 8h or 1d"));
                }
                a.push(format!("--for={f}"));
            }
            a
        }
        SharingChange::Deny { id } => {
            if !valid_id(id) {
                return Err(schema("bad request id"));
            }
            s(&["requests", "deny", id])
        }
        SharingChange::LingerEnable => s(&["linger", "enable"]),
        SharingChange::HostUpdate => s(&["update"]),
        SharingChange::HostUpdateMode { auto } => vec![
            "update".into(),
            format!("--mode={}", if *auto { "auto" } else { "prompt" }),
        ],
    })
}

/// The fix for a broken install: the app ships its host, so reinstalling the
/// app puts it back (there is no separate host installer).
pub const REINSTALL_HINT: &str = "Reinstall Unipi: the app ships its agent host";

fn not_installed() -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_NOT_FOUND,
        "This Unipi install is missing its agent host",
        false,
    )
    .with_hint(REINSTALL_HINT)
}

/// `unipi-host` without a console window popping up on Windows.
fn command(bin: &Path) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Runs `unipi-host <args>` with the app's environment (so `UNIPI_HOST_DIR`
/// applies). stdout on success; the CLI's own message as the error.
pub async fn run(
    bin: PathBuf,
    args: Vec<String>,
    limit: Option<Duration>,
) -> Result<String, AppErrorDto> {
    let task = tokio::task::spawn_blocking(move || command(&bin).args(&args).output());
    let output = match limit {
        Some(limit) => tokio::time::timeout(limit, task).await.map_err(|_| {
            AppErrorDto::new(AppErrorCode::E_TIMEOUT, "unipi-host didn't answer", true)
        })?,
        None => task.await,
    }
    .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?
    .map_err(|e| {
        AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!("couldn't run unipi-host: {e}"),
            false,
        )
    })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("unrecognized subcommand") || stderr.contains("unexpected argument") {
        return Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "This PC's agent host is too old for one-click sharing",
            false,
        )
        .with_hint("Update Unipi (Settings › About)"));
    }
    let message = stderr
        .lines()
        .map(|l| l.trim().trim_start_matches("Error: ").to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_else(|| "unipi-host failed".into());
    Err(AppErrorDto::new(AppErrorCode::E_INTERNAL, message, false))
}

async fn status_json(bin: PathBuf) -> Result<serde_json::Value, AppErrorDto> {
    let out = run(
        bin,
        vec!["sharing".into(), "status".into(), "--json".into()],
        Some(Duration::from_secs(15)),
    )
    .await?;
    serde_json::from_str(&out)
        .map_err(|e| ProtoError::Internal(format!("unipi-host status: {e}")).to_app_error())
}

/// Pending requests from `<host dir>/requests.json` (read-only; cheap enough
/// for the owner-side poll — no process spawned). Anything that changes them
/// goes through `sharing_change` (the CLI).
pub fn read_pending(dir: &Path, now_ms: u64) -> Vec<serde_json::Value> {
    const PENDING_TTL_MS: u64 = 24 * 60 * 60 * 1000;
    let Ok(bytes) = std::fs::read(dir.join("requests.json")) else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_slice::<Vec<serde_json::Value>>(&bytes) else {
        return Vec::new();
    };
    list.into_iter()
        .filter(|r| {
            r["status"] == "pending"
                && r["created_at"]
                    .as_u64()
                    .is_some_and(|t| now_ms.saturating_sub(t) < PENDING_TTL_MS)
        })
        .map(|mut r| {
            // Only what the dialog shows.
            if let Some(o) = r.as_object_mut() {
                o.retain(|k, _| {
                    [
                        "id",
                        "feature",
                        "scope",
                        "device_name",
                        "device_id",
                        "note",
                        "status",
                        "created_at",
                    ]
                    .contains(&k.as_str())
                });
            }
            r
        })
        .collect()
}

#[tauri::command]
pub fn local_requests() -> Result<Vec<serde_json::Value>, AppErrorDto> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(unsupported());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(read_pending(&host_dir(), now))
}

#[tauri::command]
pub async fn sharing_status() -> Result<serde_json::Value, AppErrorDto> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(unsupported());
    }
    status_json(bin().ok_or_else(not_installed)?).await
}

#[tauri::command]
pub async fn sharing_change(change: SharingChange) -> Result<serde_json::Value, AppErrorDto> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err(unsupported());
    }
    let bin = bin().ok_or_else(not_installed)?;
    let args = argv(&change)?;
    // The polkit prompt waits for the person at the PC: no time limit there.
    let limit = match change {
        SharingChange::LingerEnable => None,
        // Downloads ~20 MB, then restarts the service.
        SharingChange::HostUpdate => Some(Duration::from_secs(300)),
        _ => Some(Duration::from_secs(20)),
    };
    run(bin.clone(), args, limit).await?;
    status_json(bin).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_bundled_sidecar_beats_local_bin_and_path() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let mk = |dir: &str, name: &str| {
            let d = tmp.path().join(dir);
            std::fs::create_dir_all(&d).unwrap();
            let p = d.join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let sidecar = mk("usr/bin", crate::host_supervisor::SIDECAR_NAME);
        let local = mk("home/.local/bin", BIN_NAME);
        let env = mk("env", "my-host");
        let dirs = vec![tmp.path().join("usr/bin")];
        // The sidecar first, even with an older standalone host around.
        assert_eq!(bin_in(None, &dirs, || Some(local.clone())), Some(sidecar));
        // An explicit override still wins.
        assert_eq!(bin_in(Some(env.as_os_str()), &dirs, || None), Some(env));
        // No sidecar (a dev build): the standalone host.
        assert_eq!(bin_in(None, &[], || Some(local.clone())), Some(local));
    }

    #[test]
    fn changes_map_to_fixed_argv() {
        let a = |c: SharingChange| argv(&c).unwrap().join(" ");
        assert_eq!(
            a(SharingChange::ForwardEnable {
                any: true,
                ports: vec![]
            }),
            "forward enable --any"
        );
        assert_eq!(
            a(SharingChange::ForwardEnable {
                any: false,
                ports: vec![5173, 8080]
            }),
            "forward enable --only --port=5173 --port=8080"
        );
        assert!(argv(&SharingChange::ForwardEnable {
            any: false,
            ports: vec![]
        })
        .is_err());
        assert_eq!(
            a(SharingChange::BrowserEnable {
                headed: true,
                path: None
            }),
            "browser enable --headed"
        );
        // A path starting with '-' can't become a flag.
        assert_eq!(
            a(SharingChange::BrowserEnable {
                headed: false,
                path: Some("--evil".into())
            }),
            "browser enable --headless --path=--evil"
        );
        assert_eq!(
            a(SharingChange::DesktopEnable { view_only: true }),
            "desktop enable --view-only"
        );
        assert_eq!(
            a(SharingChange::Disable {
                feature: FeatureKind::Push
            }),
            "push disable"
        );
        assert_eq!(
            a(SharingChange::Approve {
                id: "r1a".into(),
                for_: Some("1h".into())
            }),
            "requests approve r1a --for=1h"
        );
        // An id can't smuggle a flag in.
        assert!(argv(&SharingChange::Approve {
            id: "--for=1d".into(),
            for_: None
        })
        .is_err());
        assert!(argv(&SharingChange::Approve {
            id: "r1;rm".into(),
            for_: None
        })
        .is_err());
        assert!(argv(&SharingChange::Approve {
            id: "r1".into(),
            for_: Some("99y".into())
        })
        .is_err());
        assert_eq!(a(SharingChange::LingerEnable), "linger enable");
        assert_eq!(a(SharingChange::HostUpdate), "update");
        assert_eq!(
            a(SharingChange::HostUpdateMode { auto: true }),
            "update --mode=auto"
        );
        assert_eq!(
            a(SharingChange::HostUpdateMode { auto: false }),
            "update --mode=prompt"
        );
        let c: SharingChange =
            serde_json::from_str(r#"{"kind":"host_update_mode","auto":true}"#).unwrap();
        assert_eq!(c, SharingChange::HostUpdateMode { auto: true });
        // The UI's JSON shape.
        let c: SharingChange =
            serde_json::from_str(r#"{"kind":"approve","id":"r1","for":"8h"}"#).unwrap();
        assert_eq!(
            c,
            SharingChange::Approve {
                id: "r1".into(),
                for_: Some("8h".into())
            }
        );
        assert!(serde_json::from_str::<SharingChange>(r#"{"kind":"shell","cmd":"x"}"#).is_err());
    }

    #[test]
    fn host_id_only_from_endpoint_json() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_host_id(tmp.path()), None);
        std::fs::write(tmp.path().join("endpoint.json"), r#"{"addr":{"id":"zz"}}"#).unwrap();
        assert_eq!(read_host_id(tmp.path()), None);
        let id = "ab".repeat(32);
        std::fs::write(
            tmp.path().join("endpoint.json"),
            format!(r#"{{"addr":{{"id":"{id}","addrs":[]}},"ticket":"t"}}"#),
        )
        .unwrap();
        assert_eq!(read_host_id(tmp.path()), Some(id));
    }

    #[cfg(unix)]
    #[test]
    fn finds_the_binary() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let pathdir = tmp.path().join("bin");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::fs::create_dir_all(&pathdir).unwrap();
        let on_path = pathdir.join(BIN_NAME);
        std::fs::write(&on_path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&on_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_bin(None, Some(&home), Some(pathdir.as_os_str())),
            Some(on_path.clone())
        );
        let local = home.join(".local/bin").join(BIN_NAME);
        std::fs::write(&local, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_bin(None, Some(&home), Some(pathdir.as_os_str())),
            Some(local)
        );
        assert_eq!(
            find_bin(Some(on_path.as_os_str()), Some(&home), None),
            Some(on_path)
        );
        assert_eq!(find_bin_in(None, None, None, None), None);
        // UNI-266: install.ps1's %LOCALAPPDATA%\UniPi comes before PATH.
        let lad = tmp.path().join("lad");
        std::fs::create_dir_all(lad.join("UniPi")).unwrap();
        let win = lad.join("UniPi").join(BIN_NAME);
        std::fs::write(&win, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&win, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            find_bin_in(None, None, Some(&lad), Some(pathdir.as_os_str())),
            Some(win.clone())
        );
        let nohome = tmp.path().join("nohome");
        assert_eq!(
            find_bin_in(None, Some(&nohome), Some(&lad), None),
            Some(win)
        );
    }

    #[test]
    fn pending_from_requests_json() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_pending(tmp.path(), 1).is_empty());
        std::fs::write(
            tmp.path().join("requests.json"),
            r#"[{"id":"r1","feature":"desktop","device_name":"Pixel","device_id":"abc","status":"pending","created_at":1000,"grant":{"x":1}},
               {"id":"r2","feature":"push","device_name":"Pixel","device_id":"abc","status":"denied","created_at":1000},
               {"id":"r3","feature":"forward","device_name":"Tab","device_id":"def","status":"pending","created_at":0}]"#,
        )
        .unwrap();
        let pending = read_pending(tmp.path(), 1000 + 24 * 60 * 60 * 1000 - 1);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["id"], "r1");
        assert!(pending[0].get("grant").is_none());
    }

    /// Against a real unipi-host binary and a throwaway host dir:
    /// `UNIPI_HOST_BIN=…/target/debug/unipi-host UNIPI_HOST_DIR=/tmp/qa-248 cargo test -p unipi-mobile sharing::tests::live -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live() {
        let dir = host_dir();
        assert!(
            dir.starts_with("/tmp"),
            "point UNIPI_HOST_DIR at a /tmp dir, never the real one"
        );
        let bin = bin().expect("UNIPI_HOST_BIN");
        let s = sharing_change(SharingChange::ForwardEnable {
            any: false,
            ports: vec![5173],
        })
        .await
        .unwrap();
        assert_eq!(s["forward"]["ports"], serde_json::json!([5173]));
        let s = sharing_change(SharingChange::DesktopEnable { view_only: true })
            .await
            .unwrap();
        assert_eq!(s["desktop"]["view_only"], true);
        let s = sharing_change(SharingChange::Disable {
            feature: FeatureKind::Browser,
        })
        .await
        .unwrap();
        assert_eq!(s["browser"]["enabled"], false);
        let s = status_json(bin).await.unwrap();
        assert!(s["host_dir"].as_str().unwrap().starts_with("/tmp"));
    }
}
