//! Terminal wire types for `unipi/oino/1` (host.md §9.3, WP-H6).
//!
//! The PTY lives in the host, keyed by [`TermId`] and scoped to a
//! workspace (the session/project cwd it was opened in). The ops are
//! `term_open`, `term_attach`, `term_list` and `term_kill`; a terminal
//! stream carries output in kind-1 frames, input in kind-2 frames and
//! the shared `Control::Resize` / `Control::Exit` / `Control::Title`
//! controls (no new `Control` variants).
//!
//! `term_list` answers `ok{data:[TermInfo, …]}` (a bare array, as
//! host.md §9.3 writes it) and `term_open` / `term_attach` answer
//! `ok{data:TermAttachData}` before the stream starts.

use serde::{Deserialize, Serialize};

/// Longest [`TermId`] accepted (hex ids are 16 chars today).
pub const TERM_ID_MAX: usize = 64;
/// Longest shell path/command accepted (a `$SHELL -l` style program).
pub const TERM_SHELL_MAX: usize = 256;

/// A host-side terminal id: 1..=64 characters of `[0-9a-zA-Z_-]`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TermId(String);

impl TermId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TermId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > TERM_ID_MAX {
            return Err(format!(
                "term_id must be 1..={TERM_ID_MAX} characters, got {}",
                value.len()
            ));
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("term_id must be ASCII letters, digits, '-' or '_'".to_string());
        }
        Ok(TermId(value))
    }
}

impl<'de> Deserialize<'de> for TermId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        TermId::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// The shell a terminal runs (`$SHELL` by default): 1..=256 characters,
/// no NUL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TermShell(String);

impl TermShell {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TermShell {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > TERM_SHELL_MAX {
            return Err(format!(
                "shell must be 1..={TERM_SHELL_MAX} characters, got {}",
                value.len()
            ));
        }
        if value.bytes().any(|b| b == 0) {
            return Err("shell must not contain NUL bytes".to_string());
        }
        Ok(TermShell(value))
    }
}

impl<'de> Deserialize<'de> for TermShell {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        TermShell::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// One row of `term_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermInfo {
    pub term_id: TermId,
    /// The workspace the terminal belongs to (the cwd it was opened in).
    pub cwd: String,
    pub shell: TermShell,
    /// `false` once the child exited; the terminal stays listed (and
    /// attachable) until it is killed or evicted.
    pub alive: bool,
    /// Unix seconds of the last input or output byte.
    pub last_activity: u64,
}

/// The `term_open` / `term_attach` reply (`ok{data:…}`) sent before the
/// stream carries output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermAttachData {
    pub term_id: TermId,
    pub shell: TermShell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_id_validates_and_round_trips() {
        let id: TermId = "a1b2c3".to_string().try_into().expect("valid");
        assert_eq!(id.as_str(), "a1b2c3");
        assert_eq!(
            serde_json::to_value(&id).expect("serialize"),
            serde_json::json!("a1b2c3")
        );
        let parsed: TermId = serde_json::from_str("\"a1b2c3\"").expect("parse");
        assert_eq!(parsed, id);

        assert!(TermId::try_from(String::new()).is_err());
        assert!(TermId::try_from("a".repeat(TERM_ID_MAX + 1)).is_err());
        assert!(TermId::try_from("a b".to_string()).is_err());
        assert!(serde_json::from_str::<TermId>("\"a/b\"").is_err());
    }

    #[test]
    fn term_shell_validates() {
        let shell: TermShell = "/bin/bash".to_string().try_into().expect("valid");
        assert_eq!(shell.as_str(), "/bin/bash");
        assert!(TermShell::try_from(String::new()).is_err());
        assert!(TermShell::try_from("a".repeat(TERM_SHELL_MAX + 1)).is_err());
        assert!(TermShell::try_from("a\0b".to_string()).is_err());
    }

    #[test]
    fn term_info_shape() {
        let info = TermInfo {
            term_id: "abc".to_string().try_into().expect("id"),
            cwd: "/tmp/project".into(),
            shell: "/bin/sh".to_string().try_into().expect("shell"),
            alive: true,
            last_activity: 1_700_000_000,
        };
        assert_eq!(
            serde_json::to_value(&info).expect("serialize"),
            serde_json::json!({
                "term_id": "abc",
                "cwd": "/tmp/project",
                "shell": "/bin/sh",
                "alive": true,
                "last_activity": 1_700_000_000u64,
            })
        );
        let parsed: TermInfo =
            serde_json::from_value(serde_json::to_value(&info).expect("serialize")).expect("parse");
        assert_eq!(parsed, info);
    }

    #[test]
    fn attach_data_shape() {
        let data = TermAttachData {
            term_id: "abc".to_string().try_into().expect("id"),
            shell: "/bin/sh".to_string().try_into().expect("shell"),
        };
        assert_eq!(
            serde_json::to_value(&data).expect("serialize"),
            serde_json::json!({"term_id": "abc", "shell": "/bin/sh"})
        );
    }
}
