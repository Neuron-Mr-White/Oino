//! Board wire types, v2 (app.md §3.14 "Board" delta, WP-A22; ext-abi §5.5).
//!
//! v1's `board_http`/`board_sse` daemon relay is gone: the Board talks to the
//! `unipi-kanboard` **CLI** through the host. `board_cli{args, project}` runs
//! one process per call (`--json`, actor pinned to `user` by the host, a
//! subcommand allow-list), `board_watch{on}` turns the file watcher on and
//! `HubEvent::BoardChanged{project}` (session.rs) is its refresh signal.
//!
//! The reply carries the CLI's §5.5 outcome verbatim: one JSON value on
//! stdout on success, the stderr line `{ok:false, error, kind}` on failure.

use serde::{Deserialize, Serialize};

use crate::error::ProtoError;

/// One argv element of a `board_cli` call: non-empty, printable, bounded (it
/// becomes one process argument — never a shell string).
pub const BOARD_ARG_MAX: usize = 1024;
/// Most arguments one call may carry (every subcommand needs far fewer).
pub const BOARD_ARGS_MAX: usize = 64;
/// A project slug: 1..=128 characters, no control characters.
pub const BOARD_SLUG_MAX: usize = 128;

/// The CLI's failure vocabulary (ext-abi §5.5): `usage` (exit 2), the rest
/// exit 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardErrorKind {
    /// clap refused the invocation (exit 2).
    Usage,
    /// Unknown project or task.
    NotFound,
    /// A transition/ownership rule refused the write.
    Rule,
    /// The files could not be read or written.
    Io,
}

/// `ok.data` of `board_cli`: the §5.5 contract decoded by the host —
/// `{ok:true, payload}` on exit 0, `{ok:false, error, kind}` on exit 1/2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardCliData {
    /// Exit 0: the call succeeded.
    pub ok: bool,
    /// The CLI's stdout payload, parsed (success only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// The user-facing stderr message (failure only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Which class of failure (failure only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BoardErrorKind>,
}

impl BoardCliData {
    /// Exit 0: exactly one JSON value on stdout.
    #[must_use]
    pub fn success(payload: serde_json::Value) -> Self {
        Self {
            ok: true,
            payload: Some(payload),
            error: None,
            kind: None,
        }
    }

    /// Exit 1/2: one `{ok:false, error, kind}` line on stderr. A domain
    /// failure, not a transport failure — the app surfaces `error`.
    #[must_use]
    pub fn failure(error: impl Into<String>, kind: BoardErrorKind) -> Self {
        Self {
            ok: false,
            payload: None,
            error: Some(error.into()),
            kind: Some(kind),
        }
    }
}

/// `ok.data` of `board_watch`: whether the watcher is on after the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardWatchData {
    /// The board file watcher is running (someone still wants events).
    pub watching: bool,
}

/// Validates one `board_cli` argument.
pub fn board_arg_error(value: &str) -> Option<String> {
    if value.is_empty() {
        return Some("board_cli arguments must not be empty".to_string());
    }
    if value.len() > BOARD_ARG_MAX {
        return Some(format!(
            "board_cli arguments must be at most {BOARD_ARG_MAX} bytes, got {}",
            value.len()
        ));
    }
    if value.chars().any(|ch| ch.is_control()) {
        return Some("board_cli arguments must be printable".to_string());
    }
    None
}

/// Validates a `board_cli` argv: subcommand first, bounded length.
pub fn board_args_error(args: &[String]) -> Option<String> {
    if args.is_empty() {
        return Some("board_cli needs a subcommand".to_string());
    }
    if args.len() > BOARD_ARGS_MAX {
        return Some(format!(
            "board_cli takes at most {BOARD_ARGS_MAX} arguments, got {}",
            args.len()
        ));
    }
    args.iter().find_map(|arg| board_arg_error(arg))
}

/// A project slug pinned per view (`--project <slug>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BoardSlug(String);

impl TryFrom<String> for BoardSlug {
    type Error = ProtoError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > BOARD_SLUG_MAX {
            return Err(ProtoError::Schema {
                message: format!(
                    "board project must be 1..={BOARD_SLUG_MAX} characters, got {}",
                    value.len()
                ),
                path: Vec::new(),
                detail: None,
            });
        }
        if value.chars().any(|ch| ch.is_control()) {
            return Err(ProtoError::Schema {
                message: "board project must be printable".to_string(),
                path: Vec::new(),
                detail: None,
            });
        }
        Ok(BoardSlug(value))
    }
}

impl BoardSlug {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BoardSlug {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        BoardSlug::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// Serde helper for `Request::BoardCli::args`: [`Vec<String>`] with the argv
/// rules applied (so a bad request fails the hello with `E_SCHEMA`, like every
/// other newtype in this crate) while staying plain strings on the wire.
pub mod serde_args {
    use super::board_args_error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(args: &[String], serializer: S) -> Result<S::Ok, S::Error> {
        args.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<String>, D::Error> {
        let raw = Vec::<String>::deserialize(deserializer)?;
        match board_args_error(&raw) {
            Some(reason) => Err(serde::de::Error::custom(reason)),
            None => Ok(raw),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cli_data_round_trips_both_outcomes() {
        let ok = serde_json::to_value(BoardCliData::success(json!({"tasks": []}))).unwrap();
        assert_eq!(ok, json!({"ok": true, "payload": {"tasks": []}}));
        let err = serde_json::to_value(BoardCliData::failure(
            "P-1 → done requires --comment",
            BoardErrorKind::Rule,
        ))
        .unwrap();
        assert_eq!(
            err,
            json!({"ok": false, "error": "P-1 → done requires --comment", "kind": "rule"})
        );
        let parsed: BoardCliData = serde_json::from_value(err).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.kind, Some(BoardErrorKind::Rule));
        let watch: BoardWatchData = serde_json::from_value(json!({"watching": true})).unwrap();
        assert!(watch.watching);
    }

    #[test]
    fn argv_and_slug_validation() {
        assert!(board_args_error(&["list".to_string()]).is_none());
        assert!(board_args_error(&[]).is_some());
        assert!(board_args_error(&["add".to_string(), String::new()]).is_some());
        assert!(board_args_error(&["add".to_string(), "\u{1}x".to_string()]).is_some());
        let long = vec!["list".to_string(); BOARD_ARGS_MAX + 1];
        assert!(board_args_error(&long).is_some());
        assert_eq!(
            BoardSlug::try_from("demo-a1b2".to_string())
                .unwrap()
                .as_str(),
            "demo-a1b2"
        );
        assert!(BoardSlug::try_from(String::new()).is_err());
        assert!(BoardSlug::try_from("p\n".to_string()).is_err());
        assert!(BoardSlug::try_from("x".repeat(BOARD_SLUG_MAX + 1)).is_err());
    }
}
