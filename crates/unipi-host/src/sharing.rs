//! Sharing with phones (UNI-248): the one code path that switches port
//! forwarding, the remote browser, scheduled cron jobs and push on or off,
//! plus the store of "ask the PC to turn this on" requests.
//!
//! Every writer goes through here: the `unipi-host forward|browser|push|cron
//! enable|disable` CLI, `unipi-host requests approve|deny`, and the desktop app
//! (which runs that same CLI). The serve process reads `config.json` on every
//! request, so a change applies without a restart.
//!
//! Requests (`<host dir>/requests.json`, 0600):
//! - a paired device sends `feature_request{feature, scope?, note?}`; the host
//!   keeps one request per device and feature (asking again refreshes it) and
//!   tells the owner (desktop app poll, ntfy, `unipi-host requests`).
//! - **nothing is ever switched on by itself**: only `approve` (the owner, on
//!   the PC) does, for that one device, optionally with a time limit; the
//!   serve ticker / every later read switches it back off when it runs out.
//! - pending requests expire after [`PENDING_TTL_MS`].
//!
//! UNI-253 (`leave`): the mirror image — a device that has a feature may drop
//! it again, for itself (`scope: "self"`) or for everyone (`scope: "all"`,
//! which is the owner's switch and needs the device to have it). Only access
//! is taken away here; no `enabled` flag is ever set.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use unipi_proto::{
    AppErrorCode, AppErrorDto, FeatureKind, FeatureLeaveData, FeatureLeaveScope,
    FeatureRequestItem, FeatureRequestStatus, FeatureScope,
};

use crate::browser::BrowserConfig;
use crate::data_dir::HostPaths;
use crate::forward::{HostConfig, PortRule};
use crate::push::{matches_device, now_ms};

/// A pending request nobody answered expires after a day.
pub const PENDING_TTL_MS: u64 = 24 * 60 * 60 * 1000;
/// Answered requests are kept (for the phone to see the outcome) this long.
pub const KEEP_DONE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// Re-asking within this window doesn't notify the owner again.
pub const RENOTIFY_MS: u64 = 10 * 60 * 1000;
/// At most this many requests are stored (oldest answered ones go first).
pub const MAX_STORED: usize = 100;

pub fn requests_path(paths: &HostPaths) -> PathBuf {
    paths.root.join("requests.json")
}

// ------------------------------------------------------------ switches

/// What `forward enable` changes.
#[derive(Debug, Clone, Default)]
pub struct ForwardEnable {
    pub any: bool,
    pub ports: Vec<u16>,
    /// Replace the port list instead of adding to it.
    pub replace: bool,
    /// `Some` replaces the device list.
    pub devices: Option<Vec<String>>,
}

pub fn forward_enable(paths: &HostPaths, change: ForwardEnable) -> anyhow::Result<HostConfig> {
    anyhow::ensure!(
        change.any || !change.ports.is_empty(),
        "pick --any or at least one --port <n>"
    );
    anyhow::ensure!(
        change.ports.iter().all(|p| *p != 0),
        "port must be 1..65535"
    );
    let path = paths.config();
    let mut config = HostConfig::load(&path)?;
    config.forwarding.enabled = true;
    config.forwarding.ports = if change.any {
        PortRule::Any
    } else {
        let mut list = match (&config.forwarding.ports, change.replace) {
            (PortRule::List(list), false) => list.clone(),
            _ => Vec::new(),
        };
        list.extend(change.ports);
        list.sort_unstable();
        list.dedup();
        PortRule::List(list)
    };
    if let Some(devices) = change.devices {
        config.forwarding.devices = devices;
    }
    config.save(&path)?;
    Ok(config)
}

/// What `browser enable` changes (`None` = keep).
#[derive(Debug, Clone, Default)]
pub struct BrowserEnable {
    pub headless: Option<bool>,
    pub idle_minutes: Option<u32>,
    /// `Some("")` clears it (auto-detect).
    pub path: Option<String>,
    pub devices: Option<Vec<String>>,
    /// UNI-292.
    pub engine: Option<crate::browser::Engine>,
    pub max_running: Option<u32>,
}

pub fn browser_enable(paths: &HostPaths, change: BrowserEnable) -> anyhow::Result<BrowserConfig> {
    let path = paths.config();
    let mut config = BrowserConfig::load(&path)?;
    config.enabled = true;
    if let Some(headless) = change.headless {
        config.headless = headless;
    }
    if let Some(minutes) = change.idle_minutes {
        config.idle_minutes = minutes;
    }
    if let Some(binary) = change.path {
        config.path = (!binary.trim().is_empty()).then(|| binary.trim().to_string());
    }
    if let Some(devices) = change.devices {
        config.devices = devices;
    }
    if let Some(engine) = change.engine {
        config.engine = engine;
    }
    if let Some(max) = change.max_running {
        config.max_running = max.max(1);
    }
    config.save(&path)?;
    Ok(config)
}

pub fn push_enable(
    paths: &HostPaths,
    devices: Option<Vec<String>>,
) -> anyhow::Result<crate::push::PushConfig> {
    let path = paths.config();
    let mut config = HostConfig::load(&path)?;
    config.push.enabled = true;
    if let Some(devices) = devices {
        config.push.devices = devices;
    }
    config.save(&path)?;
    Ok(config.push)
}

/// The `cron` sharing block of `config.json` (WP-H8): who may use the `cron_*` ops. The
/// scheduler's own settings (concurrency, pause, retention, …) moved to the library's
/// `<agent dir>/cron/settings.json`; this block only grants the feature.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronShareConfig {
    /// The feature is on for phones (the PC's own app always has it).
    #[serde(default)]
    pub enabled: bool,
    /// Ids/prefixes/names allowed; empty = every paired device.
    #[serde(default)]
    pub devices: Vec<String>,
}

/// Reads the `cron` sharing block (fresh from `config.json`; a missing block is off).
#[must_use]
pub fn cron_share(paths: &HostPaths) -> CronShareConfig {
    let Ok(mut config) = HostConfig::load(&paths.config()) else {
        return CronShareConfig::default();
    };
    let Some(block) = config.rest.remove("cron") else {
        return CronShareConfig::default();
    };
    CronShareConfig {
        enabled: block
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        devices: block
            .get("devices")
            .and_then(serde_json::Value::as_array)
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|d| d.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Writes the `cron` sharing block, keeping every other key of the block (and of the
/// config) untouched.
fn save_cron_share(paths: &HostPaths, share: &CronShareConfig) -> anyhow::Result<()> {
    let path = paths.config();
    let mut config = HostConfig::load(&path)?;
    let mut block = config
        .rest
        .remove("cron")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    block.insert("enabled".to_owned(), serde_json::Value::Bool(share.enabled));
    block.insert("devices".to_owned(), serde_json::to_value(&share.devices)?);
    config
        .rest
        .insert("cron".to_owned(), serde_json::Value::Object(block));
    config.save(&path)
}

/// UNI-317: switches `cron` on for phones (the PC's own app always has it).
/// Like `push_enable`, this only grants the sharing feature — it has
/// nothing to do with `cron pause`/`cron resume` (which stop/start runs).
pub fn cron_enable(
    paths: &HostPaths,
    devices: Option<Vec<String>>,
) -> anyhow::Result<CronShareConfig> {
    let mut share = cron_share(paths);
    share.enabled = true;
    if let Some(devices) = devices {
        share.devices = devices;
    }
    save_cron_share(paths, &share)?;
    Ok(share)
}

/// Switches a feature off (its settings are kept for next time). Approved
/// requests for it end ("until turned off").
pub fn disable(paths: &HostPaths, feature: FeatureKind) -> anyhow::Result<()> {
    let path = paths.config();
    match feature {
        FeatureKind::Forward | FeatureKind::Push => {
            let mut config = HostConfig::load(&path)?;
            match feature {
                FeatureKind::Forward => config.forwarding.enabled = false,
                _ => config.push.enabled = false,
            }
            config.save(&path)?;
        }
        FeatureKind::Browser => {
            let mut config = BrowserConfig::load(&path)?;
            config.enabled = false;
            config.save(&path)?;
        }
        FeatureKind::Cron => {
            let mut share = cron_share(paths);
            share.enabled = false;
            save_cron_share(paths, &share)?;
        }
    }
    end_grants(paths, feature, now_ms())
}

/// `(enabled, devices)` of a feature's config block.
fn feature_state(paths: &HostPaths, feature: FeatureKind) -> anyhow::Result<(bool, Vec<String>)> {
    let path = paths.config();
    Ok(match feature {
        FeatureKind::Forward => {
            let f = HostConfig::load(&path)?.forwarding;
            // "on" with no ports allowed is still off for a phone.
            (f.enabled && !matches!(f.ports, PortRule::None), f.devices)
        }
        FeatureKind::Push => {
            let p = HostConfig::load(&path)?.push;
            (p.enabled, p.devices)
        }
        FeatureKind::Browser => {
            let b = BrowserConfig::load(&path)?;
            (b.enabled, b.devices)
        }
        FeatureKind::Cron => {
            let share = cron_share(paths);
            (share.enabled, share.devices)
        }
    })
}

/// Is `feature` switched on for this device right now?
pub fn allowed_for(
    paths: &HostPaths,
    feature: FeatureKind,
    device_id: &str,
    device_name: Option<&str>,
) -> bool {
    // UNI-301: the browser works out of the box for the PC's own app
    // (`Peer::Local`) unless the owner explicitly disabled it — the one
    // feature with that default. Everything else still needs `enabled: true`.
    if feature == FeatureKind::Browser && device_id == crate::protocol::LOCAL_DEVICE_ID {
        return match BrowserConfig::load(&paths.config()) {
            Ok(config) => {
                config.is_enabled_for(device_id) && config.device_allowed(device_id, device_name)
            }
            Err(_) => false,
        };
    }
    match feature_state(paths, feature) {
        Ok((enabled, devices)) => {
            enabled
                && (devices.is_empty()
                    || device_id == crate::protocol::LOCAL_DEVICE_ID
                    || devices
                        .iter()
                        .any(|e| matches_device(e, device_id, device_name)))
        }
        Err(_) => false,
    }
}

fn set_devices(
    paths: &HostPaths,
    feature: FeatureKind,
    enabled: bool,
    devices: Vec<String>,
) -> anyhow::Result<()> {
    let path = paths.config();
    match feature {
        FeatureKind::Forward | FeatureKind::Push => {
            let mut config = HostConfig::load(&path)?;
            if feature == FeatureKind::Forward {
                config.forwarding.enabled = enabled;
                config.forwarding.devices = devices;
            } else {
                config.push.enabled = enabled;
                config.push.devices = devices;
            }
            config.save(&path)
        }
        FeatureKind::Browser => {
            let mut config = BrowserConfig::load(&path)?;
            config.enabled = enabled;
            config.devices = devices;
            config.save(&path)
        }
        FeatureKind::Cron => save_cron_share(paths, &CronShareConfig { enabled, devices }),
    }
}

// ------------------------------------------------------------ requests

/// What an approval changed, so a time limit can undo exactly that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The feature was off before: on expiry it goes back off (with this device list).
    pub was_off: bool,
    #[serde(default)]
    pub prev_devices: Vec<String>,
    /// The device was added to an existing allow-list: on expiry it is removed again.
    #[serde(default)]
    pub added_device: bool,
}

/// One request on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRequest {
    #[serde(flatten)]
    pub item: FeatureRequestItem,
    pub device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<Grant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified_at: Option<u64>,
}

fn load(paths: &HostPaths) -> anyhow::Result<Vec<StoredRequest>> {
    Ok(crate::data_dir::read_json(&requests_path(paths))?.unwrap_or_default())
}

fn save(paths: &HostPaths, list: &[StoredRequest]) -> anyhow::Result<()> {
    crate::data_dir::write_json(&requests_path(paths), &list)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn not_found(id: &str) -> anyhow::Error {
    anyhow::anyhow!("no request {id} (see `unipi-host requests`)")
}

/// Outcome of a device asking.
#[derive(Debug, Clone)]
pub struct Asked {
    pub item: FeatureRequestItem,
    /// Tell the owner now (a new request, or one not announced recently).
    pub notify: bool,
}

/// A device asks for `feature`. Already on for it → `approved` (nothing stored).
#[allow(clippy::too_many_arguments)]
pub fn ask(
    paths: &HostPaths,
    device_id: &str,
    device_name: &str,
    feature: FeatureKind,
    scope: FeatureScope,
    note: Option<String>,
    now: u64,
) -> anyhow::Result<Asked> {
    let _guard = lock();
    let mut list = load(paths)?;
    sweep_list(paths, &mut list, now)?;
    if allowed_for(paths, feature, device_id, Some(device_name)) {
        // Already on: say so (a stale phone screen), and close any pending ask.
        let existing = list.iter_mut().rev().find(|r| {
            r.device_id == device_id
                && r.item.feature == feature
                && r.item.status == FeatureRequestStatus::Pending
        });
        let item = match existing {
            Some(r) => {
                r.item.status = FeatureRequestStatus::Approved;
                r.item.decided_at = Some(now);
                r.item.clone()
            }
            None => FeatureRequestItem {
                id: new_id(now),
                feature,
                scope,
                device_name: device_name.to_string(),
                note,
                status: FeatureRequestStatus::Approved,
                created_at: now,
                decided_at: Some(now),
                until: None,
            },
        };
        save(paths, &list)?;
        return Ok(Asked {
            item,
            notify: false,
        });
    }
    let pending = list.iter_mut().find(|r| {
        r.device_id == device_id
            && r.item.feature == feature
            && r.item.status == FeatureRequestStatus::Pending
    });
    let (item, notify) = match pending {
        Some(r) => {
            r.item.scope = scope;
            r.item.note = note;
            r.item.device_name = device_name.to_string();
            r.item.created_at = now;
            let notify = r
                .notified_at
                .is_none_or(|t| now.saturating_sub(t) >= RENOTIFY_MS);
            if notify {
                r.notified_at = Some(now);
            }
            (r.item.clone(), notify)
        }
        None => {
            let item = FeatureRequestItem {
                id: new_id(now),
                feature,
                scope,
                device_name: device_name.to_string(),
                note,
                status: FeatureRequestStatus::Pending,
                created_at: now,
                decided_at: None,
                until: None,
            };
            list.push(StoredRequest {
                item: item.clone(),
                device_id: device_id.to_string(),
                grant: None,
                notified_at: Some(now),
            });
            (item, true)
        }
    };
    trim(&mut list);
    save(paths, &list)?;
    Ok(Asked { item, notify })
}

fn new_id(now: u64) -> String {
    format!("r{:x}{:04x}", now, rand::random::<u16>())
}

fn trim(list: &mut Vec<StoredRequest>) {
    while list.len() > MAX_STORED {
        let at = list
            .iter()
            .position(|r| r.item.status != FeatureRequestStatus::Pending)
            .unwrap_or(0);
        list.remove(at);
    }
}

/// This device's latest request per feature (newest first).
pub fn for_device(
    paths: &HostPaths,
    device_id: &str,
    now: u64,
) -> anyhow::Result<Vec<FeatureRequestItem>> {
    let list = all(paths, now)?;
    let mut out: Vec<FeatureRequestItem> = Vec::new();
    for r in list.iter().rev().filter(|r| r.device_id == device_id) {
        if !out.iter().any(|o| o.feature == r.item.feature) {
            out.push(r.item.clone());
        }
    }
    Ok(out)
}

/// Every stored request after expiring old ones (oldest first).
pub fn all(paths: &HostPaths, now: u64) -> anyhow::Result<Vec<StoredRequest>> {
    let _guard = lock();
    let mut list = load(paths)?;
    if sweep_list(paths, &mut list, now)? {
        save(paths, &list)?;
    }
    Ok(list)
}

/// Expires stale pending requests and ends time-limited approvals (switching
/// the feature back off for that device). Returns the requests that changed.
pub fn sweep(paths: &HostPaths, now: u64) -> anyhow::Result<Vec<FeatureRequestItem>> {
    let _guard = lock();
    let mut list = load(paths)?;
    let before = list.clone();
    if sweep_list(paths, &mut list, now)? {
        save(paths, &list)?;
    }
    Ok(list
        .iter()
        .filter(|r| !before.contains(r))
        .map(|r| r.item.clone())
        .collect())
}

fn sweep_list(paths: &HostPaths, list: &mut Vec<StoredRequest>, now: u64) -> anyhow::Result<bool> {
    let mut changed = false;
    for r in list.iter_mut() {
        match r.item.status {
            FeatureRequestStatus::Pending
                if now.saturating_sub(r.item.created_at) >= PENDING_TTL_MS =>
            {
                r.item.status = FeatureRequestStatus::Expired;
                r.item.decided_at = Some(now);
                changed = true;
            }
            FeatureRequestStatus::Approved if r.item.until.is_some_and(|u| u <= now) => {
                if let Some(grant) = r.grant.take() {
                    revoke(paths, r.item.feature, &r.device_id, &grant)?;
                }
                r.item.status = FeatureRequestStatus::Expired;
                changed = true;
            }
            _ => {}
        }
    }
    let len = list.len();
    list.retain(|r| {
        r.item.status == FeatureRequestStatus::Pending
            || now.saturating_sub(r.item.decided_at.unwrap_or(r.item.created_at)) < KEEP_DONE_MS
    });
    Ok(changed || list.len() != len)
}

/// Undo one time-limited grant.
fn revoke(
    paths: &HostPaths,
    feature: FeatureKind,
    device_id: &str,
    grant: &Grant,
) -> anyhow::Result<()> {
    let (enabled, devices) = feature_state_raw(paths, feature)?;
    if grant.was_off {
        // Back to how the owner left it: off, with the old list. If the owner
        // added other devices meanwhile, keep those and only drop this one.
        let rest: Vec<String> = devices.into_iter().filter(|d| d != device_id).collect();
        if rest.is_empty() {
            set_devices(paths, feature, false, grant.prev_devices.clone())
        } else {
            set_devices(paths, feature, enabled, rest)
        }
    } else if grant.added_device {
        let rest: Vec<String> = devices.into_iter().filter(|d| d != device_id).collect();
        // An empty list means "every device": never widen — switch off instead.
        let on = enabled && !rest.is_empty();
        set_devices(paths, feature, on, rest)
    } else {
        Ok(())
    }
}

fn feature_state_raw(
    paths: &HostPaths,
    feature: FeatureKind,
) -> anyhow::Result<(bool, Vec<String>)> {
    let path = paths.config();
    Ok(match feature {
        FeatureKind::Forward => {
            let f = HostConfig::load(&path)?.forwarding;
            (f.enabled, f.devices)
        }
        _ => feature_state(paths, feature)?,
    })
}

/// The owner says yes: switch `feature` on for that device (with the asked
/// scope), until `until` (unix ms) or until it is turned off.
pub fn approve(
    paths: &HostPaths,
    id: &str,
    until: Option<u64>,
    now: u64,
) -> anyhow::Result<FeatureRequestItem> {
    let _guard = lock();
    let mut list = load(paths)?;
    sweep_list(paths, &mut list, now)?;
    let r = list
        .iter_mut()
        .find(|r| r.item.id == id)
        .ok_or_else(|| not_found(id))?;
    anyhow::ensure!(
        r.item.status == FeatureRequestStatus::Pending,
        "request {id} is already {} — ask the device to send it again",
        r.item.status.as_str()
    );
    if let Some(until) = until {
        anyhow::ensure!(until > now, "the time limit must be in the future");
    }
    let feature = r.item.feature;
    let device = r.device_id.clone();
    let (enabled, devices) = feature_state_raw(paths, feature)?;
    let scope = r.item.scope.clone();
    // Open it for this one device. Already on for everyone → nothing to add.
    let grant = if !enabled {
        Grant {
            was_off: true,
            prev_devices: devices.clone(),
            added_device: false,
        }
    } else if devices.is_empty() {
        Grant {
            was_off: false,
            prev_devices: Vec::new(),
            added_device: false,
        }
    } else {
        Grant {
            was_off: false,
            prev_devices: devices.clone(),
            added_device: !devices.iter().any(|d| d == &device),
        }
    };
    let new_devices = if grant.was_off {
        Some(vec![device.clone()])
    } else if grant.added_device {
        let mut list = devices.clone();
        list.push(device.clone());
        Some(list)
    } else {
        None
    };
    match feature {
        FeatureKind::Forward => {
            let current = HostConfig::load(&paths.config())?.forwarding;
            let any = scope.any_port == Some(true)
                || (scope.ports.is_empty() && !grant.was_off && current.ports == PortRule::Any);
            let ports = if scope.ports.is_empty() && !any {
                vec![]
            } else {
                scope.ports.clone()
            };
            if any || !ports.is_empty() {
                forward_enable(
                    paths,
                    ForwardEnable {
                        any,
                        ports,
                        replace: false,
                        devices: new_devices,
                    },
                )?;
            } else if matches!(current.ports, PortRule::None) {
                // Nothing asked and nothing allowed yet: any port (what the switch defaults to).
                forward_enable(
                    paths,
                    ForwardEnable {
                        any: true,
                        ports: vec![],
                        replace: false,
                        devices: new_devices,
                    },
                )?;
            } else {
                let mut config = HostConfig::load(&paths.config())?;
                config.forwarding.enabled = true;
                if let Some(d) = new_devices {
                    config.forwarding.devices = d;
                }
                config.save(&paths.config())?;
            }
        }
        FeatureKind::Browser => {
            let headless = grant.was_off.then(|| scope.headed != Some(true));
            browser_enable(
                paths,
                BrowserEnable {
                    headless,
                    devices: new_devices,
                    ..Default::default()
                },
            )?;
        }
        FeatureKind::Push => {
            push_enable(paths, new_devices)?;
        }
        FeatureKind::Cron => {
            cron_enable(paths, new_devices)?;
        }
    }
    let r = list
        .iter_mut()
        .find(|r| r.item.id == id)
        .expect("found above");
    r.item.status = FeatureRequestStatus::Approved;
    r.item.decided_at = Some(now);
    r.item.until = until;
    r.grant = Some(grant);
    let item = r.item.clone();
    save(paths, &list)?;
    Ok(item)
}

pub fn deny(paths: &HostPaths, id: &str, now: u64) -> anyhow::Result<FeatureRequestItem> {
    let _guard = lock();
    let mut list = load(paths)?;
    sweep_list(paths, &mut list, now)?;
    let r = list
        .iter_mut()
        .find(|r| r.item.id == id)
        .ok_or_else(|| not_found(id))?;
    anyhow::ensure!(
        r.item.status == FeatureRequestStatus::Pending,
        "request {id} is already {}",
        r.item.status.as_str()
    );
    r.item.status = FeatureRequestStatus::Denied;
    r.item.decided_at = Some(now);
    let item = r.item.clone();
    save(paths, &list)?;
    Ok(item)
}

/// The owner turned `feature` off: approvals for it are over, and the
/// devices they added leave the allow-list again (so switching it back on
/// later means what the owner had, not "only that phone").
fn end_grants(paths: &HostPaths, feature: FeatureKind, now: u64) -> anyhow::Result<()> {
    let _guard = lock();
    let mut list = load(paths)?;
    let mut changed = false;
    for r in list
        .iter_mut()
        .filter(|r| r.item.feature == feature && r.item.status == FeatureRequestStatus::Approved)
    {
        if let Some(grant) = r.grant.take() {
            if grant.was_off || grant.added_device {
                let (_, devices) = feature_state_raw(paths, feature)?;
                let rest: Vec<String> = devices.into_iter().filter(|d| d != &r.device_id).collect();
                let restored = if grant.was_off && rest.is_empty() {
                    grant.prev_devices.clone()
                } else {
                    rest
                };
                set_devices(paths, feature, false, restored)?;
            }
        }
        r.item.status = FeatureRequestStatus::Expired;
        r.item.decided_at = Some(now);
        changed = true;
    }
    if changed {
        save(paths, &list)?;
    }
    Ok(())
}

/// Parses `1h`, `30m`, `90s`, `2d` into milliseconds.
pub fn parse_duration_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let (num, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = num.parse().ok().filter(|n| *n > 0)?;
    let unit_ms = match unit {
        "s" => 1_000,
        "m" | "min" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    n.checked_mul(unit_ms)
}

/// Plain words for a request: "Pixel asks for the remote browser (window shown on the PC)".
pub fn describe(item: &FeatureRequestItem) -> String {
    let scope = item
        .scope
        .describe(item.feature)
        .map(|s| format!(" ({s})"))
        .unwrap_or_default();
    format!(
        "{} asks for {}{scope}",
        item.device_name,
        item.feature.label()
    )
}

/// Tell the owner (ntfy topic + registered UnifiedPush endpoints, encrypted
/// like every UniPi notification). Best effort.
pub async fn notify_owner(paths: &HostPaths, host_name: &str, item: &FeatureRequestItem) -> bool {
    let connection = crate::notify::read_ntfy_config(&crate::notify::notify_config_json_path());
    let endpoints = crate::notify::read_app_endpoints(&crate::notify::app_endpoints_path(paths));
    if connection.topic.is_none() && endpoints.is_empty() {
        return false;
    }
    let Ok(key) = crate::notify::notify_key_or_create(paths) else {
        return false;
    };
    let payload = serde_json::json!({
        "title": format!("{host_name}: {}", describe(item)),
        "body": format!("Allow it in UniPi on the PC, or run `unipi-host requests approve {}`", item.id),
        "kind": "feature_request",
        "host": host_name,
    });
    match crate::notify::send_payload(&connection, &endpoints, &key, &payload).await {
        Ok(sent) => sent > 0,
        Err(error) => {
            tracing::warn!(%error, "feature request notification failed");
            false
        }
    }
}

/// The wire error when a device names a request op badly.
pub fn bad_request(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
}

/// A `feature_leave` the device isn't allowed to make (UNI-253): turning a
/// feature off for everyone is the owner's switch. Carries the wire error so
/// the serve side doesn't have to guess a code from a message.
#[derive(Debug)]
pub struct Refused {
    pub error: AppErrorDto,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.error.message.as_str())
    }
}

impl std::error::Error for Refused {}

/// The gate error for `feature_leave{scope: "all"}` from a device that
/// doesn't have the feature (same shape as `push::not_allowed`).
pub fn not_allowed(feature: FeatureKind) -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_UNAUTHORIZED,
        format!("{} is not on for this device", feature.label()),
        false,
    )
    .with_hint("Take only this device off it (scope \"self\"), or ask on the PC.")
}

/// A device turns `feature` off (UNI-253). Never switches anything on:
/// - `scope: "self"` takes this device out of the allow-list. A list that is
///   empty means "every paired device", so it is spelled out (paired devices
///   minus this one) instead of staying empty — that would widen the feature
///   to a phone that pairs later. When nobody is left the feature is switched
///   off, exactly like the owner turning it off.
/// - `scope: "all"` switches the feature off for everyone, reusing
///   [`disable`] so the grants are reverted the same way.
///
/// In both cases the device's own approvals of the feature end, so
/// `feature_requests` stops showing "approved" for access it hasn't got.
///
/// The device's live sessions (forward tunnels, the browser screencast) are
/// one per connection and nothing registers them per device: the allow-list
/// above is what the host re-reads on the next request, so the device's next
/// one is refused (`E_FORWARD_DISABLED` / `E_BROWSER_DISABLED`) — the same as
/// after the owner's `unipi-host forward|browser disable`, which also leaves
/// a live stream running until the phone hangs up.
pub fn leave(
    paths: &HostPaths,
    feature: FeatureKind,
    scope: FeatureLeaveScope,
    device_id: &str,
    device_name: Option<&str>,
    now: u64,
) -> anyhow::Result<FeatureLeaveData> {
    let (enabled, devices) = feature_state_raw(paths, feature)?;
    if scope == FeatureLeaveScope::All {
        anyhow::ensure!(
            allowed_for(paths, feature, device_id, device_name),
            Refused {
                error: not_allowed(feature)
            }
        );
        disable(paths, feature)?;
        end_own_grants(paths, feature, device_id, now)?;
        let (_, devices) = feature_state_raw(paths, feature)?;
        return Ok(FeatureLeaveData {
            feature,
            left: true,
            now_enabled: false,
            devices,
        });
    }
    // Off already: nothing to undo, and never a reason to switch it on.
    if !enabled {
        return Ok(FeatureLeaveData {
            feature,
            left: false,
            now_enabled: false,
            devices,
        });
    }
    // This PC's own app is the owner: it turns features off, it doesn't leave.
    if device_id == crate::protocol::LOCAL_DEVICE_ID {
        let (now_enabled, devices) = feature_state(paths, feature)?;
        return Ok(FeatureLeaveData {
            feature,
            left: false,
            now_enabled,
            devices,
        });
    }
    // Not on for this device (the allow-list doesn't name it): still a success.
    if !devices.is_empty()
        && !devices
            .iter()
            .any(|e| matches_device(e, device_id, device_name))
    {
        let (now_enabled, devices) = feature_state(paths, feature)?;
        return Ok(FeatureLeaveData {
            feature,
            left: false,
            now_enabled,
            devices,
        });
    }
    let kept: Vec<String> = if devices.is_empty() {
        // "Every paired device": spell out who is left instead of leaving an
        // empty list behind (that would mean everyone again).
        crate::devices::DeviceStore::new(paths.devices())
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.id)
            .filter(|id| id != device_id)
            .collect()
    } else {
        devices
            .iter()
            .filter(|e| !matches_device(e, device_id, device_name))
            .cloned()
            .collect()
    };
    if kept.is_empty() {
        // Nobody is left on the allow-list: off for everyone, grants reverted.
        disable(paths, feature)?;
        let (_, devices) = feature_state_raw(paths, feature)?;
        return Ok(FeatureLeaveData {
            feature,
            left: true,
            now_enabled: false,
            devices,
        });
    }
    set_devices(paths, feature, enabled, kept)?;
    end_own_grants(paths, feature, device_id, now)?;
    let (now_enabled, devices) = feature_state(paths, feature)?;
    Ok(FeatureLeaveData {
        feature,
        left: true,
        now_enabled,
        devices,
    })
}

/// The device that just left: its approvals of `feature` are over, so
/// `feature_requests` doesn't keep showing "approved" for access it hasn't
/// got any more. (The allow-list is what gates it; this is only the bookkeeping.)
fn end_own_grants(
    paths: &HostPaths,
    feature: FeatureKind,
    device_id: &str,
    now: u64,
) -> anyhow::Result<()> {
    let _guard = lock();
    let mut list = load(paths)?;
    let mut changed = false;
    for r in list.iter_mut().filter(|r| {
        r.item.feature == feature
            && r.device_id == device_id
            && r.item.status == FeatureRequestStatus::Approved
    }) {
        r.grant = None;
        r.item.status = FeatureRequestStatus::Expired;
        r.item.decided_at = Some(now);
        changed = true;
    }
    if changed {
        save(paths, &list)?;
    }
    Ok(())
}

// ------------------------------------------------------------ status (desktop app)

#[derive(Debug, Clone, Serialize)]
pub struct ForwardStatus {
    pub enabled: bool,
    pub any: bool,
    pub ports: Vec<u16>,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserStatus {
    pub enabled: bool,
    pub headless: bool,
    pub idle_minutes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The Chrome-family browser found (the `chrome` engine / the fallback).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<unipi_proto::BrowserInfo>,
    pub devices: Vec<String>,
    /// UNI-292: the configured engine (`cloak` | `chrome`) and whether a
    /// start would be stealthy right now.
    pub engine: crate::browser::Engine,
    pub stealth: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushStatus {
    pub enabled: bool,
    pub devices: Vec<String>,
}

/// UNI-317/UNI-318: whether phones may use `cron_*` ops (the PC's own app
/// always has them regardless of this).
#[derive(Debug, Clone, Serialize)]
pub struct CronShareStatus {
    pub enabled: bool,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestView {
    #[serde(flatten)]
    pub item: FeatureRequestItem,
    pub device_id: String,
}

/// `unipi-host sharing status --json`: everything the desktop app's switches need.
#[derive(Debug, Clone, Serialize)]
pub struct SharingStatus {
    pub version: String,
    pub host_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    pub forward: ForwardStatus,
    pub browser: BrowserStatus,
    pub push: PushStatus,
    pub cron: CronShareStatus,
    pub devices: Vec<PairedDevice>,
    pub requests: Vec<RequestView>,
    /// systemd linger for this user (keeps the service running after logout
    /// and starts it at boot); `None` without loginctl.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linger: Option<bool>,
    /// UNI-263: self-update (owner setting + what `serve` last saw in the feed).
    pub update: crate::update::UpdateStatus,
}

pub fn status(paths: &HostPaths, now: u64) -> anyhow::Result<SharingStatus> {
    let path = paths.config();
    let host = HostConfig::load(&path)?;
    let browser = BrowserConfig::load(&path)?;
    let requests = all(paths, now)?;
    let host_id = crate::data_dir::read_json::<crate::endpoint::EndpointCache>(&paths.endpoint())
        .ok()
        .flatten()
        .map(|c| c.addr.id.to_string());
    Ok(SharingStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        host_dir: paths.root.display().to_string(),
        host_id,
        forward: ForwardStatus {
            enabled: host.forwarding.enabled,
            any: host.forwarding.ports == PortRule::Any,
            ports: match &host.forwarding.ports {
                PortRule::List(list) => list.clone(),
                _ => Vec::new(),
            },
            devices: host.forwarding.devices.clone(),
        },
        browser: BrowserStatus {
            // UNI-301: this is the local app's own view, which works out of
            // the box unless the owner explicitly disabled it.
            enabled: browser.is_enabled_for(crate::protocol::LOCAL_DEVICE_ID),
            headless: browser.headless,
            idle_minutes: browser.idle_minutes,
            found: crate::browser::detect_here(&browser),
            path: browser.path.clone(),
            engine: browser.engine,
            stealth: crate::browser::would_use(&browser).2,
            devices: browser.devices.clone(),
        },
        push: PushStatus {
            enabled: host.push.enabled,
            devices: host.push.devices.clone(),
        },
        cron: {
            let share = cron_share(paths);
            CronShareStatus {
                enabled: share.enabled,
                devices: share.devices,
            }
        },
        devices: crate::devices::DeviceStore::new(paths.devices())
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|d| PairedDevice {
                id: d.id,
                name: d.name,
            })
            .collect(),
        requests: requests
            .into_iter()
            .map(|r| RequestView {
                item: r.item,
                device_id: r.device_id,
            })
            .collect(),
        linger: current_user().and_then(|u| linger_enabled(&u)),
        update: crate::update::status(paths),
    })
}

// ------------------------------------------------------------ keep running (linger)

/// `loginctl show-user <user> -p Linger --value` → yes/no; `None` when
/// loginctl isn't there (no systemd).
pub fn linger_enabled(user: &str) -> Option<bool> {
    let out = std::process::Command::new("loginctl")
        .args(["show-user", user, "-p", "Linger", "--value"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match text.as_str() {
        "yes" => Some(true),
        "no" => Some(false),
        // "Failed to get user: User … is not logged in" → no linger file.
        _ if !out.status.success() => {
            Some(Path::new("/var/lib/systemd/linger").join(user).exists())
        }
        _ => None,
    }
}

/// The polkit-prompted command that keeps the user's services running after
/// logout and at boot (needs root once): `pkexec loginctl enable-linger <user>`.
pub fn linger_enable_argv(user: &str) -> Vec<String> {
    vec![
        "pkexec".into(),
        "loginctl".into(),
        "enable-linger".into(),
        user.into(),
    ]
}

pub fn current_user() -> Option<String> {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| {
            let out = std::process::Command::new("id").arg("-un").output().ok()?;
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|u| !u.is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, HostPaths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        (tmp, paths)
    }

    const PIXEL: &str = "aaaaaaaaaaaaaaaa1111";
    const TAB: &str = "bbbbbbbbbbbbbbbb2222";

    /// UNI-301: the browser works for the local app with no config at all;
    /// a phone still needs `enabled: true` + the allow-list; an explicit
    /// `enabled: false` refuses local too.
    #[test]
    fn browser_allowed_for_local_by_default_unlike_phones() {
        let (_t, p) = paths();
        assert!(allowed_for(
            &p,
            FeatureKind::Browser,
            crate::protocol::LOCAL_DEVICE_ID,
            None
        ));
        assert!(!allowed_for(&p, FeatureKind::Browser, PIXEL, Some("Pixel")));
        browser_enable(
            &p,
            BrowserEnable {
                devices: Some(vec![PIXEL.into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(allowed_for(&p, FeatureKind::Browser, PIXEL, Some("Pixel")));
        assert!(allowed_for(
            &p,
            FeatureKind::Browser,
            crate::protocol::LOCAL_DEVICE_ID,
            None
        ));
        disable(&p, FeatureKind::Browser).unwrap();
        assert!(!allowed_for(
            &p,
            FeatureKind::Browser,
            crate::protocol::LOCAL_DEVICE_ID,
            None
        ));
        assert!(!allowed_for(&p, FeatureKind::Browser, PIXEL, Some("Pixel")));
    }

    #[test]
    fn ask_stores_one_pending_per_device_and_feature() {
        let (_t, p) = paths();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            1_000,
        )
        .unwrap();
        assert!(a.notify);
        assert_eq!(a.item.status, FeatureRequestStatus::Pending);
        // Asking again refreshes the same request and doesn't re-notify yet.
        let b = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            Some("please".into()),
            2_000,
        )
        .unwrap();
        assert_eq!(a.item.id, b.item.id);
        assert!(!b.notify);
        assert_eq!(b.item.note.as_deref(), Some("please"));
        // …but does after RENOTIFY_MS.
        assert!(
            ask(
                &p,
                PIXEL,
                "Pixel",
                FeatureKind::Push,
                FeatureScope::default(),
                None,
                1_000 + RENOTIFY_MS
            )
            .unwrap()
            .notify
        );
        // Another feature / device is its own request.
        ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Browser,
            FeatureScope::default(),
            None,
            3_000,
        )
        .unwrap();
        ask(
            &p,
            TAB,
            "Tab",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            3_000,
        )
        .unwrap();
        assert_eq!(all(&p, 4_000).unwrap().len(), 3);
        assert_eq!(for_device(&p, PIXEL, 4_000).unwrap().len(), 2);
        // Never switched on by asking.
        assert!(!allowed_for(&p, FeatureKind::Push, PIXEL, Some("Pixel")));
        assert!(!HostConfig::load(&p.config()).unwrap().push.enabled);
    }

    #[test]
    fn approve_switches_on_for_that_device_only() {
        let (_t, p) = paths();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            1_000,
        )
        .unwrap();
        let item = approve(&p, &a.item.id, None, 2_000).unwrap();
        assert_eq!(item.status, FeatureRequestStatus::Approved);
        assert!(item.until.is_none());
        let config = HostConfig::load(&p.config()).unwrap().push;
        assert!(config.enabled);
        assert_eq!(config.devices, vec![PIXEL.to_string()]);
        assert!(allowed_for(&p, FeatureKind::Push, PIXEL, None));
        assert!(!allowed_for(&p, FeatureKind::Push, TAB, None));
        // Answered twice → refused.
        assert!(approve(&p, &a.item.id, None, 3_000).is_err());
        assert!(deny(&p, &a.item.id, 3_000).is_err());
        // Asking once it's on → approved straight away, nothing pending.
        let again = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            4_000,
        )
        .unwrap();
        assert_eq!(again.item.status, FeatureRequestStatus::Approved);
        assert!(!again.notify);
    }

    #[test]
    fn approve_forward_ports_and_any() {
        let (_t, p) = paths();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Forward,
            FeatureScope {
                ports: vec![5173],
                ..Default::default()
            },
            None,
            1,
        )
        .unwrap();
        approve(&p, &a.item.id, None, 2).unwrap();
        let f = HostConfig::load(&p.config()).unwrap().forwarding;
        assert!(f.enabled);
        assert_eq!(f.ports, PortRule::List(vec![5173]));
        let b = ask(
            &p,
            TAB,
            "Tab",
            FeatureKind::Forward,
            FeatureScope::default(),
            None,
            3,
        )
        .unwrap();
        approve(&p, &b.item.id, None, 4).unwrap();
        let f = HostConfig::load(&p.config()).unwrap().forwarding;
        // Tab added to the list; ports kept.
        assert_eq!(f.devices, vec![PIXEL.to_string(), TAB.to_string()]);
        assert_eq!(f.ports, PortRule::List(vec![5173]));
        // A fresh PC, no scope → any port.
        let (_t2, q) = paths();
        let c = ask(
            &q,
            PIXEL,
            "Pixel",
            FeatureKind::Forward,
            FeatureScope::default(),
            None,
            1,
        )
        .unwrap();
        approve(&q, &c.item.id, None, 2).unwrap();
        assert_eq!(
            HostConfig::load(&q.config()).unwrap().forwarding.ports,
            PortRule::Any
        );
    }

    #[test]
    fn deny_and_pending_expiry() {
        let (_t, p) = paths();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Browser,
            FeatureScope::default(),
            None,
            1_000,
        )
        .unwrap();
        assert_eq!(
            deny(&p, &a.item.id, 2_000).unwrap().status,
            FeatureRequestStatus::Denied
        );
        assert!(!BrowserConfig::load(&p.config()).unwrap().enabled);
        assert_eq!(
            for_device(&p, PIXEL, 3_000).unwrap()[0].status,
            FeatureRequestStatus::Denied
        );
        // A new ask after a denial is a new pending request.
        let b = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Browser,
            FeatureScope::default(),
            None,
            4_000,
        )
        .unwrap();
        assert_ne!(a.item.id, b.item.id);
        assert_eq!(b.item.status, FeatureRequestStatus::Pending);
        // Nobody answers for a day → expired.
        let changed = sweep(&p, 4_000 + PENDING_TTL_MS).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].status, FeatureRequestStatus::Expired);
        assert!(approve(&p, &b.item.id, None, 5_000 + PENDING_TTL_MS).is_err());
        assert!(approve(&p, "rnope", None, 1).is_err());
    }

    #[test]
    fn time_limited_approval_switches_back_off() {
        let (_t, p) = paths();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            1_000,
        )
        .unwrap();
        let item = approve(&p, &a.item.id, Some(1_000 + 3_600_000), 1_000).unwrap();
        assert_eq!(item.until, Some(3_601_000));
        assert!(allowed_for(&p, FeatureKind::Push, PIXEL, None));
        assert!(sweep(&p, 3_600_999).unwrap().is_empty());
        let changed = sweep(&p, 3_601_000).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].status, FeatureRequestStatus::Expired);
        let push = HostConfig::load(&p.config()).unwrap().push;
        assert!(!push.enabled);
        assert!(push.devices.is_empty());
        // In the past → refused.
        let b = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            5_000_000,
        )
        .unwrap();
        assert!(approve(&p, &b.item.id, Some(4_000_000), 5_000_000).is_err());
    }

    #[test]
    fn time_limited_grant_on_an_allow_list_removes_only_that_device() {
        let (_t, p) = paths();
        browser_enable(
            &p,
            BrowserEnable {
                devices: Some(vec![TAB.into()]),
                ..Default::default()
            },
        )
        .unwrap();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Browser,
            FeatureScope::default(),
            None,
            1,
        )
        .unwrap();
        approve(&p, &a.item.id, Some(100), 2).unwrap();
        assert_eq!(
            BrowserConfig::load(&p.config()).unwrap().devices,
            vec![TAB.to_string(), PIXEL.to_string()]
        );
        sweep(&p, 100).unwrap();
        let config = BrowserConfig::load(&p.config()).unwrap();
        assert!(config.enabled);
        assert_eq!(config.devices, vec![TAB.to_string()]);
    }

    #[test]
    fn turning_off_ends_approvals() {
        let (_t, p) = paths();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Browser,
            FeatureScope {
                headed: Some(true),
                ..Default::default()
            },
            None,
            1,
        )
        .unwrap();
        approve(&p, &a.item.id, None, 2).unwrap();
        let b = BrowserConfig::load(&p.config()).unwrap();
        assert!(b.enabled && !b.headless);
        assert_eq!(b.devices, vec![PIXEL.to_string()]);
        disable(&p, FeatureKind::Browser).unwrap();
        let b = BrowserConfig::load(&p.config()).unwrap();
        assert!(!b.enabled);
        // The approval's device left the list: turning it on again means "every paired device", as before.
        assert!(b.devices.is_empty());
        assert_eq!(
            for_device(&p, PIXEL, 3).unwrap()[0].status,
            FeatureRequestStatus::Expired
        );
    }

    #[test]
    fn switches_keep_other_keys() {
        let (_t, p) = paths();
        std::fs::write(
            p.config(),
            r#"{"fs":{"allow":["/srv"]},"push":{"enabled":true}}"#,
        )
        .unwrap();
        forward_enable(
            &p,
            ForwardEnable {
                ports: vec![8080, 5173],
                ..Default::default()
            },
        )
        .unwrap();
        forward_enable(
            &p,
            ForwardEnable {
                ports: vec![3000],
                replace: true,
                ..Default::default()
            },
        )
        .unwrap();
        browser_enable(
            &p,
            BrowserEnable {
                headless: Some(false),
                path: Some("/opt/b".into()),
                ..Default::default()
            },
        )
        .unwrap();
        disable(&p, FeatureKind::Push).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(p.config()).unwrap()).unwrap();
        assert_eq!(raw["fs"]["allow"][0], "/srv");
        assert_eq!(raw["forwarding"]["ports"], serde_json::json!([3000]));
        assert_eq!(raw["browser"]["headless"], false);
        assert_eq!(raw["browser"]["path"], "/opt/b");
        assert_eq!(raw["push"]["enabled"], false);
        assert!(forward_enable(&p, ForwardEnable::default()).is_err());
        let s = status(&p, 1).unwrap();
        assert!(s.forward.enabled && !s.forward.any && s.forward.ports == vec![3000]);
        assert!(!s.push.enabled);
    }

    // ------------------------------------------------------------ leave (UNI-253)

    /// Pairs `devices` (so "every paired device" can be spelled out).
    fn pair(p: &HostPaths, devices: &[(&str, &str)]) {
        let list: Vec<serde_json::Value> = devices
            .iter()
            .map(|(id, name)| serde_json::json!({"id":id,"name":name,"paired_at":1,"last_seen":null}))
            .collect();
        crate::data_dir::write_json(&p.devices(), &list).unwrap();
    }

    fn leave_self(
        p: &HostPaths,
        feature: FeatureKind,
        id: &str,
        name: Option<&str>,
    ) -> FeatureLeaveData {
        leave(p, feature, FeatureLeaveScope::OnlySelf, id, name, 1_000).unwrap()
    }

    #[test]
    fn leave_self_removes_only_this_device() {
        let (_t, p) = paths();
        browser_enable(
            &p,
            BrowserEnable {
                devices: Some(vec![PIXEL.into(), TAB.into()]),
                ..Default::default()
            },
        )
        .unwrap();
        let left = leave_self(&p, FeatureKind::Browser, PIXEL, Some("Pixel"));
        assert!(left.left);
        assert!(left.now_enabled);
        assert_eq!(left.devices, vec![TAB.to_string()]);
        let config = BrowserConfig::load(&p.config()).unwrap();
        assert!(config.enabled);
        assert_eq!(config.devices, vec![TAB.to_string()]);
        assert!(!allowed_for(&p, FeatureKind::Browser, PIXEL, Some("Pixel")));
        assert!(allowed_for(&p, FeatureKind::Browser, TAB, Some("Tab")));
        // Leaving again changes nothing (and says so).
        let again = leave_self(&p, FeatureKind::Browser, PIXEL, Some("Pixel"));
        assert!(!again.left);
        assert!(again.now_enabled);
        assert_eq!(again.devices, vec![TAB.to_string()]);
    }

    #[test]
    fn leaving_ends_the_devices_own_approval() {
        let (_t, p) = paths();
        push_enable(&p, Some(vec![TAB.into()])).unwrap();
        let a = ask(
            &p,
            PIXEL,
            "Pixel",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            1,
        )
        .unwrap();
        approve(&p, &a.item.id, None, 2).unwrap();
        assert_eq!(
            for_device(&p, PIXEL, 3).unwrap()[0].status,
            FeatureRequestStatus::Approved
        );
        // Leaving it behind → its approval is over, and Tab keeps push.
        leave_self(&p, FeatureKind::Push, PIXEL, Some("Pixel"));
        assert_eq!(
            for_device(&p, PIXEL, 4).unwrap()[0].status,
            FeatureRequestStatus::Expired
        );
        assert!(allowed_for(&p, FeatureKind::Push, TAB, Some("Tab")));
    }

    #[test]
    fn leaving_the_last_allowed_device_turns_it_off() {
        let (_t, p) = paths();
        push_enable(&p, Some(vec![PIXEL.into(), TAB.into()])).unwrap();
        // Pixel leaves itself out; Tab is still allowed and push stays on.
        let left = leave_self(&p, FeatureKind::Push, PIXEL, None);
        assert!(left.left && left.now_enabled);
        assert_eq!(left.devices, vec![TAB.to_string()]);
        // Tab was granted push (the owner said yes): the only device with it.
        disable(&p, FeatureKind::Push).unwrap();
        let a = ask(
            &p,
            TAB,
            "Tab",
            FeatureKind::Push,
            FeatureScope::default(),
            None,
            1,
        )
        .unwrap();
        approve(&p, &a.item.id, None, 2).unwrap();
        assert_eq!(
            HostConfig::load(&p.config()).unwrap().push.devices,
            vec![TAB.to_string()]
        );
        // The last device allowed leaves → off for everyone, the grant with it.
        let left = leave_self(&p, FeatureKind::Push, TAB, Some("Tab"));
        assert!(left.left);
        assert!(!left.now_enabled);
        // The owner's allow-list is kept (as `disable` leaves it): off is off.
        assert_eq!(left.devices, vec![TAB.to_string()]);
        let push = HostConfig::load(&p.config()).unwrap().push;
        assert!(!push.enabled);
        assert_eq!(push.devices, vec![TAB.to_string()]);
        assert_eq!(
            for_device(&p, TAB, 3).unwrap()[0].status,
            FeatureRequestStatus::Expired
        );
        assert!(!allowed_for(&p, FeatureKind::Push, TAB, Some("Tab")));
    }

    #[test]
    fn leaving_spells_out_the_paired_devices() {
        let (_t, p) = paths();
        // On for every paired device (empty list), two of them paired.
        pair(&p, &[(PIXEL, "Pixel"), (TAB, "Tab")]);
        browser_enable(
            &p,
            BrowserEnable {
                devices: Some(Vec::new()),
                ..Default::default()
            },
        )
        .unwrap();
        let left = leave_self(&p, FeatureKind::Browser, PIXEL, Some("Pixel"));
        assert!(left.left);
        assert!(left.now_enabled);
        assert_eq!(left.devices, vec![TAB.to_string()]);
        let config = BrowserConfig::load(&p.config()).unwrap();
        assert!(config.enabled);
        // Not "every paired device" any more: the phone that pairs later stays out.
        assert_eq!(config.devices, vec![TAB.to_string()]);
        assert!(allowed_for(&p, FeatureKind::Browser, TAB, Some("Tab")));
        assert!(!allowed_for(
            &p,
            FeatureKind::Browser,
            "cccccccccccccccc3333",
            Some("Later")
        ));
    }

    #[test]
    fn leave_all_needs_the_feature_and_switches_it_off() {
        let (_t, p) = paths();
        // Off: a phone that doesn't have it can't turn it off for everyone.
        let refused = leave(
            &p,
            FeatureKind::Push,
            FeatureLeaveScope::All,
            PIXEL,
            Some("Pixel"),
            1,
        )
        .unwrap_err();
        let refused = refused.downcast_ref::<Refused>().expect("a Refused");
        assert_eq!(refused.error.code, AppErrorCode::E_UNAUTHORIZED);
        assert!(!HostConfig::load(&p.config()).unwrap().push.enabled);
        // On, but only for Tab.
        push_enable(&p, Some(vec![TAB.into()])).unwrap();
        assert!(leave(
            &p,
            FeatureKind::Push,
            FeatureLeaveScope::All,
            PIXEL,
            Some("Pixel"),
            1
        )
        .is_err());
        assert_eq!(
            HostConfig::load(&p.config()).unwrap().push.devices,
            vec![TAB.to_string()]
        );
        // Pixel has it: it may, and Tab loses it too (the feature is off).
        push_enable(&p, Some(vec![TAB.into(), PIXEL.into()])).unwrap();
        let left = leave(
            &p,
            FeatureKind::Push,
            FeatureLeaveScope::All,
            PIXEL,
            Some("Pixel"),
            5,
        )
        .unwrap();
        assert!(left.left && !left.now_enabled);
        let config = HostConfig::load(&p.config()).unwrap().push;
        assert!(!config.enabled);
        // The owner's allow-list is kept, as `unipi-host push disable` does.
        assert_eq!(config.devices, vec![TAB.to_string(), PIXEL.to_string()]);
        assert!(!allowed_for(&p, FeatureKind::Push, TAB, Some("Tab")));
    }

    #[test]
    fn leaving_never_switches_anything_on() {
        let (_t, p) = paths();
        // Off: nothing to undo, and it stays off.
        let left = leave_self(&p, FeatureKind::Forward, PIXEL, Some("Pixel"));
        assert!(!left.left && !left.now_enabled && left.devices.is_empty());
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(p.config()).unwrap_or_default())
                .unwrap_or_default();
        assert!(raw.get("forwarding").is_none(), "no config written: {raw}");
        // The owner's own client never "leaves" either (it turns features off).
        forward_enable(
            &p,
            ForwardEnable {
                any: true,
                ..Default::default()
            },
        )
        .unwrap();
        let left = leave(
            &p,
            FeatureKind::Forward,
            FeatureLeaveScope::OnlySelf,
            crate::protocol::LOCAL_DEVICE_ID,
            None,
            1,
        )
        .unwrap();
        assert!(!left.left && left.now_enabled);
        assert_eq!(
            HostConfig::load(&p.config()).unwrap().forwarding.ports,
            PortRule::Any
        );
        assert!(HostConfig::load(&p.config())
            .unwrap()
            .forwarding
            .devices
            .is_empty());
    }

    #[test]
    fn durations_and_words() {
        assert_eq!(parse_duration_ms("1h"), Some(3_600_000));
        assert_eq!(parse_duration_ms("30m"), Some(1_800_000));
        assert_eq!(parse_duration_ms("2d"), Some(172_800_000));
        assert_eq!(parse_duration_ms("0h"), None);
        assert_eq!(parse_duration_ms("h"), None);
        assert_eq!(parse_duration_ms("5y"), None);
        let item = FeatureRequestItem {
            id: "r1".into(),
            feature: FeatureKind::Forward,
            scope: FeatureScope {
                ports: vec![5173, 8080],
                ..Default::default()
            },
            device_name: "Pixel".into(),
            note: None,
            status: FeatureRequestStatus::Pending,
            created_at: 0,
            decided_at: None,
            until: None,
        };
        assert_eq!(
            describe(&item),
            "Pixel asks for web app forwarding (ports 5173, 8080)"
        );
        assert_eq!(
            linger_enable_argv("me"),
            vec!["pkexec", "loginctl", "enable-linger", "me"]
        );
    }
}
