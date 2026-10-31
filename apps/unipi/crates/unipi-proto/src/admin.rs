//! UNI-232 (plan H3, docs/m8 §3.3): local-only admin ops — what the UniPi
//! desktop app on THIS PC uses to manage its own host. Mirrors
//! `packages/protocol/src/admin.ts`. Only a `Peer::Local` stream (the host's
//! `app.sock`, same user) may call them; over iroh they answer
//! `E_UNAUTHORIZED` ("Only this computer can …"). Older hosts answer
//! `E_SCHEMA` (unknown op) → the app says "update the host".
//!
//! - `host_info` → `HostInfoData`
//! - `pair_begin{ttl_secs?}` → `PairBeginData` (link for a QR, code, expiry)
//! - `pair_status{code}` → `PairStatusData` (waiting | paired | expired)
//! - `devices_list` → `DevicesListData`
//! - `device_revoke{id}` → `ok{}` (E_NOT_FOUND when no such device)

use serde::{Deserialize, Serialize};

/// Longest pairing window `pair_begin` grants (1 h), default 10 min.
pub const PAIR_TTL_MAX: u32 = 3600;
pub const PAIR_TTL_DEFAULT: u32 = 600;

/// `host_info`: what runs here and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfoData {
    pub version: String,
    pub proto: String,
    /// service | app | manual (see unipi-host `instance::Mode`).
    pub mode: String,
    pub pid: u32,
    /// Unix seconds.
    pub started_at: u64,
    pub host_id: String,
    pub host_name: String,
    pub data_dir: String,
    /// Tools found on the PATH the host sees.
    pub tools: HostTools,
    /// Paired phones.
    pub devices: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostTools {
    /// The `oino` binary (the agent core the host drives) is on the PATH.
    pub oino: bool,
    /// The `unipi-kanboard` CLI is on the PATH (the Board screen's backend).
    pub unipi_kanboard: bool,
    /// Chromium-based browsers installed here: `chrome`, `chromium`, `edge`,
    /// `brave` (host.md §7.2 "browser engines").
    pub browser_engines: Vec<String>,
    /// A ready CloakBrowser build (nothing to download).
    pub cloak: bool,
}

/// `pair_begin` reply: show `link` as a QR (and as text to copy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBeginData {
    pub link: String,
    pub code: String,
    /// Unix seconds.
    pub expires_at: u64,
    pub host_name: String,
}

/// `pair_status` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairStatusData {
    Waiting { expires_at: u64 },
    Paired { device: String },
    Expired { reason: PairExpiredReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairExpiredReason {
    Timeout,
    Replaced,
    Locked,
    Cancelled,
}

/// One paired phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceItem {
    pub id: String,
    pub name: String,
    pub paired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicesListData {
    pub devices: Vec<DeviceItem>,
}

/// A pairing code as sent back by `pair_status` (8–32 URL-safe chars).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PairCodeRef(String);

impl PairCodeRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PairCodeRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if !(4..=64).contains(&s.len())
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(serde::de::Error::custom("code must be 4–64 letters/digits"));
        }
        Ok(PairCodeRef(s))
    }
}

/// A device id for `device_revoke` (an iroh endpoint id, hex; or a ≥ 8-char prefix).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DeviceIdRef(String);

impl DeviceIdRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for DeviceIdRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if !(8..=64).contains(&s.len()) || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(serde::de::Error::custom("id must be 8–64 hex chars"));
        }
        Ok(DeviceIdRef(s))
    }
}

/// `pair_begin.ttl_secs`: 30 s … 1 h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PairTtl(u32);

impl PairTtl {
    pub fn value(self) -> u32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for PairTtl {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = u32::deserialize(d)?;
        if !(30..=PAIR_TTL_MAX).contains(&v) {
            return Err(serde::de::Error::custom("ttl_secs must be 30–3600"));
        }
        Ok(PairTtl(v))
    }
}
