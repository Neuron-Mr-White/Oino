//! Format detection, parsing and migration to v3.
//!
//! - [`detect`] classifies a file by its first line.
//! - [`parse_session_text`] parses pi files (v1/v2 migrated in memory, exactly like pi
//!   `migrateToCurrentVersion`).
//! - [`legacy::migrate_oino_legacy_text`] maps pre-v1 Oino `record` files to v3 entries
//!   (session-v3 §5.2); the file-moving wrapper lives in the manager.

pub mod legacy;
mod pi;

use oino_ai::JsonValue;

pub use legacy::{LegacyMigration, MigrationWarning};
pub use pi::migrate_pi_values;

use crate::entry::{CURRENT_SESSION_VERSION, FileLine, SessionEntry, SessionHeader};
use crate::error::SessionError;

/// File format of a session file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileFormat {
    /// pi v1 (linear, no ids).
    PiV1,
    /// pi v2 (tree, `hookMessage`).
    PiV2,
    /// pi v3 (current).
    PiV3,
    /// pi version newer than supported.
    PiFuture(u32),
    /// Pre-v1 Oino `record` file.
    OinoLegacy {
        /// Legacy header version (1 or 2).
        version: u32,
    },
    /// Blank first line / empty file.
    Empty,
    /// Not a session file.
    Unknown,
}

/// Classifies a file from its first line: key `"record"` ⇒ Oino legacy; `"type":"session"`
/// ⇒ pi (missing `version` ⇒ 1).
#[must_use]
pub fn detect(first_line: &str) -> FileFormat {
    if first_line.trim().is_empty() {
        return FileFormat::Empty;
    }
    let Ok(value) = serde_json::from_str::<JsonValue>(first_line) else {
        return FileFormat::Unknown;
    };
    if value.get("record").is_some() {
        let version = value
            .pointer("/header/version")
            .and_then(JsonValue::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(1);
        return FileFormat::OinoLegacy { version };
    }
    if value.get("type").and_then(JsonValue::as_str) != Some("session") {
        return FileFormat::Unknown;
    }
    let version = value
        .get("version")
        .and_then(JsonValue::as_u64)
        .map_or(1, |v| u32::try_from(v).unwrap_or(u32::MAX));
    match version {
        0 | 1 => FileFormat::PiV1,
        2 => FileFormat::PiV2,
        CURRENT_SESSION_VERSION => FileFormat::PiV3,
        other => FileFormat::PiFuture(other),
    }
}

/// A parsed pi session file.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedSession {
    /// Header (version 3 after migration).
    pub header: SessionHeader,
    /// Entries in file order.
    pub entries: Vec<SessionEntry>,
    /// Whether migration changed the data (the file should be rewritten).
    pub migrated: bool,
    /// Lines that were not valid JSON or not valid v3 entries (1-based numbers).
    pub skipped_lines: Vec<usize>,
}

/// Parses pi session text. Returns `Ok(None)` when the first valid JSON line is not a
/// `session` header with a string id (pi `loadEntriesFromFile` returns `[]`).
///
/// # Errors
///
/// [`SessionError::UnsupportedVersion`] for versions above 3.
pub fn parse_session_text(text: &str) -> Result<Option<ParsedSession>, SessionError> {
    let mut values = Vec::new();
    let mut line_numbers = Vec::new();
    let mut skipped_lines = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JsonValue>(line) {
            Ok(value) => {
                values.push(value);
                line_numbers.push(index + 1);
            }
            Err(_) => skipped_lines.push(index + 1),
        }
    }
    let Some(first) = values.first() else {
        return Ok(None);
    };
    if first.get("type").and_then(JsonValue::as_str) != Some("session")
        || !first.get("id").is_some_and(JsonValue::is_string)
    {
        return Ok(None);
    }
    let version = first
        .get("version")
        .and_then(JsonValue::as_u64)
        .map_or(1, |v| u32::try_from(v).unwrap_or(u32::MAX));
    if version > CURRENT_SESSION_VERSION {
        return Err(SessionError::UnsupportedVersion(version));
    }
    let migrated = migrate_pi_values(&mut values);
    let mut header = None;
    let mut entries = Vec::with_capacity(values.len());
    for (value, line) in values.into_iter().zip(line_numbers) {
        match FileLine::from_value(value) {
            Ok(FileLine::Header(parsed)) if header.is_none() => header = Some(parsed),
            Ok(FileLine::Entry(entry)) => entries.push(entry),
            Ok(FileLine::Header(_)) | Err(_) => skipped_lines.push(line),
        }
    }
    skipped_lines.sort_unstable();
    Ok(header.map(|header| ParsedSession {
        header,
        entries,
        migrated,
        skipped_lines,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(detect(r#"{"type":"session","id":"x"}"#), FileFormat::PiV1);
        assert_eq!(
            detect(r#"{"type":"session","version":2,"id":"x"}"#),
            FileFormat::PiV2
        );
        assert_eq!(
            detect(r#"{"type":"session","version":3,"id":"x"}"#),
            FileFormat::PiV3
        );
        assert_eq!(
            detect(r#"{"type":"session","version":4,"id":"x"}"#),
            FileFormat::PiFuture(4)
        );
        assert_eq!(
            detect(r#"{"record":"header","header":{"version":2}}"#),
            FileFormat::OinoLegacy { version: 2 }
        );
        assert_eq!(detect(""), FileFormat::Empty);
        assert_eq!(detect("{"), FileFormat::Unknown);
    }

    #[test]
    fn future_versions_are_rejected() {
        assert!(matches!(
            parse_session_text(r#"{"type":"session","version":9,"id":"x"}"#),
            Err(SessionError::UnsupportedVersion(9))
        ));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let text = "{\"type\":\"session\",\"version\":3,\"id\":\"s\",\"timestamp\":\"t\",\"cwd\":\"/\"}\n{oops\n{\"type\":\"session_info\",\"id\":\"a\",\"parentId\":null,\"timestamp\":\"t\",\"name\":\"n\"}\n";
        let parsed = parse_session_text(text).unwrap().unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.skipped_lines, [2]);
        assert!(!parsed.migrated);
    }
}
