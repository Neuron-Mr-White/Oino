//! Push a file from the PC to the phone (UNI-214, "forwarding phase 2").
//! Mirrors `packages/protocol/src/push.ts`.
//!
//! `unipi-host push <file>` (CLI) hands the file to the running
//! `unipi-host serve` over its local control socket; the host records a
//! pending push per target device. The phone learns about it with:
//!
//! - `push_list{wait?}` → `ok{data: PushListData}`. Long-poll: with `wait`
//!   (seconds, ≤ [`PUSH_WAIT_MAX`]) and nothing pending, the host holds the
//!   reply until a push arrives or the wait ends. Never an error when push
//!   is switched off on the host: `enabled:false, pushes:[]` (the default).
//! - the bytes ride the existing `blob_get{path}`: a pending push's path is
//!   readable by the device it was pushed to (until it is done/expired).
//!   The phone verifies `size` + `sha256` itself.
//! - `push_done{id, outcome, detail?}` → `ok{}`: the phone's answer
//!   (installed / saved / opened / dismissed / failed); the push stops being
//!   pending for that device and the waiting CLI prints the outcome.

use serde::{Deserialize, Serialize};

use crate::blob::Sha256Hex;

/// Longest `push_list` long-poll, in seconds.
pub const PUSH_WAIT_MAX: u32 = 30;

/// A push id: 1..=64 characters of `[A-Za-z0-9_-]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash)]
#[serde(transparent)]
pub struct PushId(String);

impl PushId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PushId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 64 {
            return Err(format!("id must be 1..=64 characters, got {}", value.len()));
        }
        if !value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return Err("id must be [A-Za-z0-9_-]".into());
        }
        Ok(PushId(value))
    }
}

impl<'de> Deserialize<'de> for PushId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        PushId::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// `push_list` long-poll seconds: 0..=`PUSH_WAIT_MAX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(transparent)]
pub struct PushWait(u32);

impl PushWait {
    pub fn value(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for PushWait {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value <= PUSH_WAIT_MAX {
            Ok(PushWait(value))
        } else {
            Err(format!("wait must be 0..={PUSH_WAIT_MAX} seconds, got {value}"))
        }
    }
}

impl<'de> Deserialize<'de> for PushWait {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u32::deserialize(deserializer)?;
        PushWait::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// What the phone did with a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PushOutcome {
    /// Handed to the system installer (Android asks the user to confirm).
    Installed,
    /// Saved (Downloads on Android, a chosen folder on desktop).
    Saved,
    /// Opened with another app.
    Opened,
    /// The user dismissed it.
    Dismissed,
    /// Download/verify failed and the user gave up.
    Failed,
}

impl PushOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            PushOutcome::Installed => "installed",
            PushOutcome::Saved => "saved",
            PushOutcome::Opened => "opened",
            PushOutcome::Dismissed => "dismissed",
            PushOutcome::Failed => "failed",
        }
    }
}

/// One pending push, as the phone sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushItem {
    pub id: PushId,
    /// File name (no directories).
    pub name: String,
    pub size: u64,
    pub sha256: Sha256Hex,
    pub mime: String,
    /// Absolute path on the PC; fetch it with `blob_get{path}`.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
    /// Unix milliseconds.
    pub created_at: u64,
}

/// `ok.data` for `push_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushListData {
    /// Push is switched on on this PC for this device.
    pub enabled: bool,
    pub host_name: String,
    #[serde(default)]
    pub pushes: Vec<PushItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_wait_bounds() {
        assert!(serde_json::from_str::<PushId>("\"\"").is_err());
        assert!(serde_json::from_str::<PushId>("\"a/b\"").is_err());
        assert!(serde_json::from_str::<PushId>("\"p_1a2b-3\"").is_ok());
        assert!(serde_json::from_str::<PushWait>("31").is_err());
        assert_eq!(serde_json::from_str::<PushWait>("25").unwrap().value(), 25);
    }

    #[test]
    fn outcome_is_snake_case() {
        assert_eq!(serde_json::to_string(&PushOutcome::Installed).unwrap(), "\"installed\"");
        assert!(serde_json::from_str::<PushOutcome>("\"exploded\"").is_err());
    }
}
