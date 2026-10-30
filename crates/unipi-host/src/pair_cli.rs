//! `unipi-host pair` (UNI-266): one pairing attempt, shown as a terminal QR
//! (human mode) or as JSON lines for the UniPi desktop app (`--json`):
//!
//! ```text
//! {"state":"waiting","link":"unipi://pair?…","code":"ABCD…","expires_at":1760000000,"host":"workstation"}
//! {"state":"paired","device":"Pixel 9"}            ← or {"state":"expired","reason":"timeout|replaced|locked|cancelled"}
//! ```
//! An error is one line `{"state":"error","message":…,"hint":…}` (exit 1).
//! When stdin is a pipe (the desktop app), closing it cancels: the code is
//! removed and `{"state":"expired","reason":"cancelled"}` is the last line.
//!
//! Pairing needs a running `unipi-host serve` for the same data dir: the
//! code is checked by serve's handler. We look for its control socket /
//! named pipe (serve always opens it), not only for `endpoint.json` (that
//! file stays behind after serve stops).

use std::time::Duration;

use serde::Serialize;

use crate::data_dir::HostPaths;
use crate::devices::DeviceStore;
use crate::endpoint::EndpointCache;
use crate::pairing::{now_unix, PairingState, PairingStore};

/// One JSON line of `pair --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairEvent {
    Waiting {
        link: String,
        code: String,
        expires_at: u64,
        host: String,
    },
    Paired {
        device: String,
    },
    Expired {
        reason: ExpiredReason,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiredReason {
    /// The code ran out (`--ttl`).
    Timeout,
    /// Another `unipi-host pair` started a new code.
    Replaced,
    /// Too many wrong codes.
    Locked,
    /// The desktop app closed the dialog (stdin closed in `--json` mode).
    Cancelled,
}

/// A started attempt.
#[derive(Debug, Clone)]
pub struct Started {
    pub state: PairingState,
    pub link: String,
    pub host: String,
}

impl Started {
    pub fn event(&self) -> PairEvent {
        PairEvent::Waiting {
            link: self.link.clone(),
            code: self.state.code.clone(),
            expires_at: self.state.expires_at,
            host: self.host.clone(),
        }
    }
}

/// `10m`, `90s`, `90` → seconds (1 s … 1 h).
pub fn parse_ttl(ttl: &str) -> anyhow::Result<u64> {
    let t = ttl.trim();
    let seconds = if let Some(m) = t.strip_suffix('m') {
        m.parse::<u64>()? * 60
    } else {
        t.trim_end_matches('s').parse::<u64>()?
    };
    anyhow::ensure!(
        (1..=3600).contains(&seconds),
        "--ttl must be between 1s and 60m"
    );
    Ok(seconds)
}

pub const NOT_RUNNING: &str =
    "`unipi-host serve` isn't running on this PC — start unipi-host serve first";

/// The hint for "serve isn't running": the command for this OS.
pub fn start_hint() -> &'static str {
    if cfg!(windows) {
        "Start it: schtasks /Run /TN \"UniPi Host\" (or run `unipi-host serve` in a terminal)"
    } else if cfg!(target_os = "macos") {
        "Start it: launchctl kickstart gui/$(id -u)/dev.unipi.host (or run `unipi-host serve` in a terminal)"
    } else {
        "Start it: systemctl --user start unipi-host (or run `unipi-host serve` in a terminal)"
    }
}

/// Is a `serve` answering on this data dir's control channel?
pub async fn serve_running(paths: &HostPaths) -> bool {
    let Ok((mut lines, _write)) =
        crate::push::connect_control(paths, &crate::push::ControlRequest::List).await
    else {
        return false;
    };
    matches!(
        tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await,
        Ok(Ok(Some(_)))
    )
}

/// Starts a new attempt (replacing any previous one) and builds its link.
pub async fn start(paths: &HostPaths, ttl_secs: u64, host: &str) -> anyhow::Result<Started> {
    let cache: Option<EndpointCache> = crate::data_dir::read_json(&paths.endpoint())?;
    let Some(cache) = cache else {
        anyhow::bail!(NOT_RUNNING)
    };
    anyhow::ensure!(serve_running(paths).await, NOT_RUNNING);
    let store = PairingStore::new(paths.pairing());
    let mut state = store.start(now_unix())?;
    state.expires_at = state.created_at + ttl_secs;
    crate::data_dir::write_json(&paths.pairing(), &state)?;
    let ticket = crate::pair_link::pairing_ticket(&cache.addr);
    let link = crate::pair_link::pairing_link(&ticket, &state.code, host)?;
    Ok(Started {
        state,
        link,
        host: host.to_string(),
    })
}

/// UNI-232: start an attempt from inside `serve` (the `pair_begin` op): the
/// running endpoint's address is passed in, so no control-socket check.
pub fn start_in_serve(
    paths: &HostPaths,
    addr: &iroh::EndpointAddr,
    ttl_secs: u64,
    host: &str,
) -> anyhow::Result<Started> {
    let store = PairingStore::new(paths.pairing());
    let mut state = store.start(now_unix())?;
    state.expires_at = state.created_at + ttl_secs;
    crate::data_dir::write_json(&paths.pairing(), &state)?;
    let ticket = crate::pair_link::pairing_ticket(addr);
    let link = crate::pair_link::pairing_link(&ticket, &state.code, host)?;
    Ok(Started {
        state,
        link,
        host: host.to_string(),
    })
}

/// UNI-232: `pair_status` for a code (the app only knows the code): the
/// attempt as it was started, rebuilt from `pairing.json` when it is current.
pub fn status_for_code(paths: &HostPaths, code: &str, now: u64) -> anyhow::Result<PairEvent> {
    let current = PairingStore::new(paths.pairing()).current()?;
    let Some(current) = current.filter(|c| c.code == code) else {
        return Ok(PairEvent::Expired {
            reason: ExpiredReason::Replaced,
        });
    };
    Ok(check(paths, &current, now)?.unwrap_or(PairEvent::Waiting {
        link: String::new(),
        code: current.code.clone(),
        expires_at: current.expires_at,
        host: String::new(),
    }))
}

/// One look at `pairing.json`: `None` = still waiting.
pub fn check(
    paths: &HostPaths,
    ours: &PairingState,
    now: u64,
) -> anyhow::Result<Option<PairEvent>> {
    let current = PairingStore::new(paths.pairing()).current()?;
    let Some(current) = current else {
        return Ok(Some(PairEvent::Expired {
            reason: ExpiredReason::Replaced,
        }));
    };
    if current.code != ours.code {
        return Ok(Some(PairEvent::Expired {
            reason: ExpiredReason::Replaced,
        }));
    }
    if current.used {
        // The device this code added: paired at or after the attempt began.
        let device = DeviceStore::new(paths.devices())
            .list()?
            .into_iter()
            .filter(|d| d.paired_at >= ours.created_at)
            .max_by_key(|d| d.paired_at);
        return Ok(Some(PairEvent::Paired {
            device: device.map(|d| d.name).unwrap_or_else(|| "Phone".into()),
        }));
    }
    if current.is_locked_out() {
        return Ok(Some(PairEvent::Expired {
            reason: ExpiredReason::Locked,
        }));
    }
    if current.is_expired(now) {
        return Ok(Some(PairEvent::Expired {
            reason: ExpiredReason::Timeout,
        }));
    }
    Ok(None)
}

/// Ends an unused attempt early (the dialog was closed), so the QR on the
/// screen stops working at once. Leaves a newer attempt alone.
pub fn cancel(paths: &HostPaths, ours: &PairingState) -> anyhow::Result<()> {
    let store = PairingStore::new(paths.pairing());
    if store
        .current()?
        .is_some_and(|c| c.code == ours.code && !c.used)
    {
        store.clear()?;
    }
    Ok(())
}

/// Polls until paired / expired; `cancelled` resolving ends it as Cancelled.
pub async fn wait(
    paths: &HostPaths,
    ours: &PairingState,
    cancelled: impl std::future::Future<Output = ()>,
) -> anyhow::Result<PairEvent> {
    tokio::pin!(cancelled);
    loop {
        if let Some(event) = check(paths, ours, now_unix())? {
            return Ok(event);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(300)) => {}
            _ = &mut cancelled => {
                // A pairing that landed in the same instant still counts.
                if let Some(event @ PairEvent::Paired { .. }) = check(paths, ours, now_unix())? {
                    return Ok(event);
                }
                cancel(paths, ours)?;
                return Ok(PairEvent::Expired { reason: ExpiredReason::Cancelled });
            }
        }
    }
}

/// Is stdin a pipe (the desktop app's), not a terminal or /dev/null?
pub fn stdin_is_pipe() -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        use std::os::unix::fs::FileTypeExt;
        let Ok(fd) = std::io::stdin().as_fd().try_clone_to_owned() else {
            return false;
        };
        std::fs::File::from(fd)
            .metadata()
            .is_ok_and(|m| m.file_type().is_fifo() || m.file_type().is_socket())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_PIPE};
        // SAFETY: a valid handle for the process's lifetime (or null → not a pipe).
        unsafe { GetFileType(std::io::stdin().as_raw_handle() as _) == FILE_TYPE_PIPE }
    }
    #[cfg(not(any(unix, windows)))]
    false
}

/// Resolves when stdin, a pipe, reaches EOF (the parent closed it or died).
/// Never resolves for a terminal or /dev/null (`pair --json < /dev/null`
/// in a script waits for the phone like the human mode).
pub async fn stdin_closed() {
    if !stdin_is_pipe() {
        return std::future::pending().await;
    }
    let _ = tokio::task::spawn_blocking(|| {
        use std::io::Read;
        let mut buf = [0u8; 256];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_lines_shape() {
        let w = PairEvent::Waiting {
            link: "unipi://pair?t=x&c=AB&n=pc".into(),
            code: "AB".into(),
            expires_at: 5,
            host: "pc".into(),
        };
        assert_eq!(
            serde_json::to_string(&w).unwrap(),
            r#"{"state":"waiting","link":"unipi://pair?t=x&c=AB&n=pc","code":"AB","expires_at":5,"host":"pc"}"#
        );
        assert_eq!(
            serde_json::to_string(&PairEvent::Paired {
                device: "Pixel 9".into()
            })
            .unwrap(),
            r#"{"state":"paired","device":"Pixel 9"}"#
        );
        assert_eq!(
            serde_json::to_string(&PairEvent::Expired {
                reason: ExpiredReason::Timeout
            })
            .unwrap(),
            r#"{"state":"expired","reason":"timeout"}"#
        );
        assert_eq!(
            serde_json::to_string(&PairEvent::Error {
                message: "m".into(),
                hint: None
            })
            .unwrap(),
            r#"{"state":"error","message":"m"}"#
        );
    }

    #[test]
    fn ttl() {
        assert_eq!(parse_ttl("10m").unwrap(), 600);
        assert_eq!(parse_ttl("90s").unwrap(), 90);
        assert_eq!(parse_ttl("45").unwrap(), 45);
        assert!(parse_ttl("0s").is_err());
        assert!(parse_ttl("2h").is_err());
        assert!(parse_ttl("61m").is_err());
    }

    #[test]
    fn check_states() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        let store = PairingStore::new(paths.pairing());
        let ours = store.start(1000).unwrap();
        assert_eq!(check(&paths, &ours, 1001).unwrap(), None);
        assert_eq!(
            check(&paths, &ours, ours.expires_at).unwrap(),
            Some(PairEvent::Expired {
                reason: ExpiredReason::Timeout
            })
        );
        // Wrong codes until locked out.
        for _ in 0..crate::pairing::MAX_ATTEMPTS {
            store.verify("WRONGWRONG", 1001).unwrap();
        }
        assert_eq!(
            check(&paths, &ours, 1001).unwrap(),
            Some(PairEvent::Expired {
                reason: ExpiredReason::Locked
            })
        );
        // A newer attempt replaces ours.
        let ours = store.start(1000).unwrap();
        let _newer = store.start(1000).unwrap();
        assert_eq!(
            check(&paths, &ours, 1001).unwrap(),
            Some(PairEvent::Expired {
                reason: ExpiredReason::Replaced
            })
        );
        // Cancel leaves the newer one alone; cancelling the current one removes it.
        cancel(&paths, &ours).unwrap();
        assert!(store.current().unwrap().is_some());
        let ours = store.current().unwrap().unwrap();
        cancel(&paths, &ours).unwrap();
        assert!(store.current().unwrap().is_none());
    }
}
