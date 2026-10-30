//! "Ask the PC to turn this on" (UNI-248). Mirrors
//! `packages/protocol/src/sharing.ts`.
//!
//! Port forwarding, the remote browser and push are off
//! by default and only the PC's owner may switch them on (CLI or the desktop
//! app on that PC). A paired device that finds one off can *ask*:
//!
//! - `feature_request{feature, scope?, note?}` → `ok{data: FeatureRequestItem}`.
//!   The host stores a pending request (one per device and feature; asking
//!   again refreshes it) and tells the owner (desktop app, ntfy, CLI
//!   `unipi-host requests`). Nothing is ever switched on by itself.
//! - `feature_requests` → `ok{data: FeatureRequestsData}`: this device's
//!   latest request per feature, so the phone can show pending / approved /
//!   denied / expired (it polls while a request is pending).
//!
//! And the mirror image, UNI-253 — a phone that has a feature may drop it
//! again (it can never switch one on):
//!
//! - `feature_leave{feature, scope?}` → `ok{data: FeatureLeaveData}`.
//!   `scope: "self"` (the default) takes this device out of the feature's
//!   allow-list; `scope: "all"` turns the feature off on the PC (only for a
//!   device that has it, otherwise `E_UNAUTHORIZED`). The host's `enabled`
//!   flag is never switched on by either.
//!
//! Hosts older than proto 1.11 answer the `feature_request*` ops with
//! `E_SCHEMA` (unknown op); hosts older than proto 1.15 answer
//! `feature_leave` the same way. The app then shows the command to run on
//! the PC instead.

use serde::{Deserialize, Serialize};

/// Longest `note` a device may attach to a request.
pub const FEATURE_NOTE_MAX: usize = 200;
/// Most ports one request may name.
pub const FEATURE_PORTS_MAX: usize = 16;

/// The features a device may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    /// Port forwarding (Preview → Web apps).
    Forward,
    /// The remote browser (Preview → Browser).
    Browser,
    /// Files pushed from the PC (`unipi-host push`).
    Push,
    /// UNI-317/UNI-318: scheduled cron jobs (`unipi-host cron`).
    Cron,
}

impl FeatureKind {
    pub const ALL: [FeatureKind; 4] = [
        FeatureKind::Forward,
        FeatureKind::Browser,
        FeatureKind::Push,
        FeatureKind::Cron,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FeatureKind::Forward => "forward",
            FeatureKind::Browser => "browser",
            FeatureKind::Push => "push",
            FeatureKind::Cron => "cron",
        }
    }

    /// Plain words for notifications and the CLI.
    pub fn label(self) -> &'static str {
        match self {
            FeatureKind::Forward => "web app forwarding",
            FeatureKind::Browser => "the remote browser",
            FeatureKind::Push => "file push",
            FeatureKind::Cron => "scheduled cron jobs",
        }
    }

    pub fn parse(value: &str) -> Option<FeatureKind> {
        FeatureKind::ALL.into_iter().find(|f| f.as_str() == value)
    }
}

/// What exactly the device asks for. Every field is optional; the owner
/// sees it before approving.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawScope")]
pub struct FeatureScope {
    /// forward: any loopback port.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub any_port: Option<bool>,
    /// forward: just these ports (1..=65535, at most [`FEATURE_PORTS_MAX`]).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub ports: Vec<u16>,
    /// browser: show its window on the PC.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub headed: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScope {
    #[serde(default)]
    any_port: Option<bool>,
    #[serde(default)]
    ports: Vec<u16>,
    #[serde(default)]
    headed: Option<bool>,
}

impl TryFrom<RawScope> for FeatureScope {
    type Error = String;
    fn try_from(raw: RawScope) -> Result<Self, Self::Error> {
        if raw.ports.len() > FEATURE_PORTS_MAX {
            return Err(format!("at most {FEATURE_PORTS_MAX} ports"));
        }
        if raw.ports.contains(&0) {
            return Err("port must be 1..=65535".into());
        }
        Ok(FeatureScope {
            any_port: raw.any_port,
            ports: raw.ports,
            headed: raw.headed,
        })
    }
}

impl FeatureScope {
    pub fn is_empty(&self) -> bool {
        self == &FeatureScope::default()
    }

    /// Plain words, e.g. "any port", "ports 5173, 8080", "watch only".
    pub fn describe(&self, feature: FeatureKind) -> Option<String> {
        match feature {
            FeatureKind::Forward if self.any_port == Some(true) => Some("any port".into()),
            FeatureKind::Forward if !self.ports.is_empty() => Some(format!(
                "port{} {}",
                if self.ports.len() == 1 { "" } else { "s" },
                self.ports
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            FeatureKind::Browser if self.headed == Some(true) => {
                Some("window shown on the PC".into())
            }
            _ => None,
        }
    }
}

/// A request note: ≤ [`FEATURE_NOTE_MAX`] characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct FeatureNote(String);

impl FeatureNote {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FeatureNote {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.chars().count() > FEATURE_NOTE_MAX {
            return Err(format!(
                "note must be at most {FEATURE_NOTE_MAX} characters"
            ));
        }
        Ok(FeatureNote(value))
    }
}

impl<'de> Deserialize<'de> for FeatureNote {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        FeatureNote::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// Where a request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRequestStatus {
    /// Waiting for the PC's owner.
    Pending,
    /// The owner switched it on for this device (`until` = time limit).
    Approved,
    Denied,
    /// Nobody answered in time, or a time-limited approval ran out.
    Expired,
}

impl FeatureRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            FeatureRequestStatus::Pending => "pending",
            FeatureRequestStatus::Approved => "approved",
            FeatureRequestStatus::Denied => "denied",
            FeatureRequestStatus::Expired => "expired",
        }
    }
}

/// One request as both sides see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureRequestItem {
    /// `r` + hex, 1..=64 of `[A-Za-z0-9_-]`.
    pub id: String,
    pub feature: FeatureKind,
    #[serde(default, skip_serializing_if = "FeatureScope::is_empty")]
    pub scope: FeatureScope,
    /// The asking device's name (as paired).
    pub device_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
    pub status: FeatureRequestStatus,
    /// Unix milliseconds.
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub decided_at: Option<u64>,
    /// Approved with a time limit: switched off again at this time (unix ms).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub until: Option<u64>,
}

/// `ok.data` for `feature_requests`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureRequestsData {
    #[serde(default)]
    pub requests: Vec<FeatureRequestItem>,
}

/// How much of a feature a device turns off (UNI-253). Nothing here can
/// switch a feature on: it only takes access away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureLeaveScope {
    /// Just this device: it leaves the feature's allow-list.
    #[default]
    #[serde(rename = "self")]
    OnlySelf,
    /// Everyone: the feature goes off on the PC.
    All,
}

impl FeatureLeaveScope {
    pub fn as_str(self) -> &'static str {
        match self {
            FeatureLeaveScope::OnlySelf => "self",
            FeatureLeaveScope::All => "all",
        }
    }

    /// `self` is the default, so requests leave it out.
    pub fn is_self(&self) -> bool {
        *self == FeatureLeaveScope::OnlySelf
    }
}

/// `ok.data` for `feature_leave`: what is left of the feature afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureLeaveData {
    pub feature: FeatureKind,
    /// This op actually removed something (false when the device wasn't in
    /// the allow-list and the scope was `self` — still a success).
    pub left: bool,
    /// Is the feature on for anyone after this?
    pub now_enabled: bool,
    /// The remaining allow-list entries; `[]` means "every paired device".
    #[serde(default)]
    pub devices: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_bounds() {
        assert!(serde_json::from_str::<FeatureScope>(r#"{"ports":[0]}"#).is_err());
        assert!(serde_json::from_str::<FeatureScope>(r#"{"bogus":1}"#).is_err());
        let many: Vec<u16> = (1..=17).collect();
        assert!(
            serde_json::from_value::<FeatureScope>(serde_json::json!({ "ports": many })).is_err()
        );
        let ok: FeatureScope = serde_json::from_str(r#"{"ports":[5173]}"#).unwrap();
        assert_eq!(
            ok.describe(FeatureKind::Forward).as_deref(),
            Some("port 5173")
        );
        assert_eq!(
            serde_json::to_string(&FeatureScope::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn kinds_and_notes() {
        assert_eq!(
            serde_json::to_string(&FeatureKind::Browser).unwrap(),
            "\"browser\""
        );
        // v2: desktop streaming is gone from the protocol.
        assert!(serde_json::from_str::<FeatureKind>("\"desktop\"").is_err());
        assert!(serde_json::from_str::<FeatureKind>("\"shell\"").is_err());
        assert_eq!(FeatureKind::parse("push"), Some(FeatureKind::Push));
        assert_eq!(
            serde_json::to_string(&FeatureKind::ALL)
                .unwrap()
                .matches("desktop")
                .count(),
            0
        );
        assert!(serde_json::from_value::<FeatureNote>(serde_json::json!("x".repeat(201))).is_err());
    }

    #[test]
    fn leave_scope_and_data_round_trip() {
        assert_eq!(
            serde_json::to_string(&FeatureLeaveScope::OnlySelf).unwrap(),
            "\"self\""
        );
        assert_eq!(
            serde_json::to_string(&FeatureLeaveScope::All).unwrap(),
            "\"all\""
        );
        assert_eq!(FeatureLeaveScope::default(), FeatureLeaveScope::OnlySelf);
        assert!(serde_json::from_str::<FeatureLeaveScope>("\"self\"")
            .unwrap()
            .is_self());
        assert!(serde_json::from_str::<FeatureLeaveScope>("\"everyone\"").is_err());
        let left = FeatureLeaveData {
            feature: FeatureKind::Forward,
            left: true,
            now_enabled: false,
            devices: vec!["Pixel 8".into()],
        };
        let json = serde_json::to_value(&left).unwrap();
        assert_eq!(json["feature"], "forward");
        assert_eq!(json["left"], true);
        assert_eq!(json["now_enabled"], false);
        assert_eq!(json["devices"][0], "Pixel 8");
        assert_eq!(
            serde_json::from_value::<FeatureLeaveData>(json).unwrap(),
            left
        );
        // `devices` may be omitted (no allow-list left).
        let bare: FeatureLeaveData =
            serde_json::from_str(r#"{"feature":"push","left":false,"now_enabled":true}"#).unwrap();
        assert!(bare.devices.is_empty());
        // Not a leave result: no answer, no feature, or the wrong type.
        assert!(serde_json::from_str::<FeatureLeaveData>(r#"{"feature":"push"}"#).is_err());
        assert!(serde_json::from_str::<FeatureLeaveData>(
            r#"{"feature":"shell","left":true,"now_enabled":false}"#
        )
        .is_err());
        assert!(serde_json::from_str::<FeatureLeaveData>(
            r#"{"feature":"push","left":true,"now_enabled":"yes"}"#
        )
        .is_err());
    }
}
