//! Board relay wire types (DESIGN.md §3.9 "Data path", §5.4).
//!
//! The phone never talks to the kanboard daemon's port. It sends a
//! `board_http` or `board_sse` request to `unipi-host`, which forwards it
//! to the local `unipi-kanboard` daemon on loopback.
//!
//! `board_http` stream:
//! - client → `hello{req:{op:"board_http", method, path, content_type?, body_len?}}`,
//!   then exactly `body_len` bytes in kind-2 frames (each ≤ `MAX_FRAME`).
//! - host → `ok{data:{status, content_type}}`, the daemon's body verbatim in
//!   kind-1 frames, then `exit{code:0}` as the end-of-body marker. Host or
//!   transport failures are an `err` envelope (before or instead of `exit`).
//!
//! `board_sse` stream:
//! - client → `hello{req:{op:"board_sse", project}}`.
//! - host → `ok{}` once the daemon accepted `GET /events?project=<slug>`, then
//!   one `board_event{event?, data}` per SSE message; `exit{code:null}` when the
//!   daemon ended the stream (idle shutdown, restart) or an `err` envelope on
//!   failure. The client closes its send half to unsubscribe.
//!
//! Mirrors `packages/protocol/src/control.ts` (`BoardPathSchema` & co.).

use serde::{Deserialize, Serialize};

/// Largest request body the host accepts for `board_http` (26 MiB: the
/// daemon's 25 MiB attachment limit plus slack; the daemon enforces its own).
pub const BOARD_MAX_BODY: u64 = 26 * 1024 * 1024;
/// Longest daemon path (incl. query) a `board_http` request may carry.
pub const BOARD_MAX_PATH: usize = 4096;

/// HTTP method for `board_http`. Only what the kanboard API uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "PUT")]
    Put,
}

impl BoardMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            BoardMethod::Get => "GET",
            BoardMethod::Post => "POST",
            BoardMethod::Put => "PUT",
        }
    }
}

impl std::str::FromStr for BoardMethod {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "GET" => Ok(BoardMethod::Get),
            "POST" => Ok(BoardMethod::Post),
            "PUT" => Ok(BoardMethod::Put),
            other => Err(format!("method must be GET, POST or PUT, got {other:?}")),
        }
    }
}

/// The board path allow-list. A path must:
/// - start with `/api/` (the daemon's JSON/file API; `/events` is only
///   reachable through `board_sse`, the web UI assets never),
/// - be at most `BOARD_MAX_PATH` characters,
/// - contain no control characters, `#` or `\`,
/// - contain no empty (`//`) or dot (`.`, `..`, `%2e`…) segments before the
///   query, so URL normalisation cannot climb out of `/api/`.
pub fn check_board_path(path: &str) -> Result<(), String> {
    if !path.starts_with("/api/") {
        return Err("path must start with /api/".into());
    }
    let len = path.chars().count();
    if len > BOARD_MAX_PATH {
        return Err(format!(
            "path must be at most {BOARD_MAX_PATH} characters, got {len}"
        ));
    }
    if path
        .chars()
        .any(|c| c.is_control() || c == '#' || c == '\\')
    {
        return Err("path must not contain control characters, '#' or '\\'".into());
    }
    let route = path.split_once('?').map_or(path, |(route, _)| route);
    for segment in route.split('/').skip(1) {
        if segment.is_empty() {
            return Err("path must not contain empty segments".into());
        }
        let decoded = segment.to_ascii_lowercase().replace("%2e", ".");
        if decoded == "." || decoded == ".." {
            return Err("path must not contain dot segments".into());
        }
    }
    Ok(())
}

/// A daemon path (incl. query) that passed [`check_board_path`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BoardPath(String);

impl BoardPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BoardPath {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        check_board_path(&value).map(|()| BoardPath(value))
    }
}

impl<'de> Deserialize<'de> for BoardPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        BoardPath::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A kanboard project slug: 1..=128 chars of `[A-Za-z0-9._-]`, starting
/// with a letter or digit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BoardProject(String);

impl BoardProject {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BoardProject {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let len = value.chars().count();
        if !(1..=128).contains(&len) {
            return Err(format!("project must be 1..=128 characters, got {len}"));
        }
        let mut chars = value.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
        if !first_ok
            || !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err("project must be a slug ([A-Za-z0-9._-], starting alphanumeric)".into());
        }
        Ok(BoardProject(value))
    }
}

impl<'de> Deserialize<'de> for BoardProject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        BoardProject::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// A request `Content-Type`: 1..=256 printable ASCII characters (so it can
/// never smuggle a header line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ContentType(String);

impl ContentType {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentType {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 256 {
            return Err(format!(
                "content_type must be 1..=256 characters, got {}",
                value.len()
            ));
        }
        if !value.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Err("content_type must be printable ASCII".into());
        }
        Ok(ContentType(value))
    }
}

impl<'de> Deserialize<'de> for ContentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        ContentType::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// Request body length in bytes: 0..=`BOARD_MAX_BODY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(transparent)]
pub struct BodyLen(u64);

impl BodyLen {
    pub fn value(self) -> u64 {
        self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl TryFrom<u64> for BodyLen {
    type Error = String;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value <= BOARD_MAX_BODY {
            Ok(BodyLen(value))
        } else {
            Err(format!(
                "body_len must be at most {BOARD_MAX_BODY} bytes, got {value}"
            ))
        }
    }
}

impl<'de> Deserialize<'de> for BodyLen {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u64::deserialize(deserializer)?;
        BodyLen::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// `ok.data` for a `board_http` request: the daemon's status and
/// `Content-Type`. The body follows in kind-1 frames, ended by `exit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardHttpData {
    pub status: HttpStatus,
    pub content_type: String,
}

/// An HTTP status code, 100..=599.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HttpStatus(u16);

impl HttpStatus {
    pub fn value(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for HttpStatus {
    type Error = String;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (100..=599).contains(&value) {
            Ok(HttpStatus(value))
        } else {
            Err(format!("status must be 100..=599, got {value}"))
        }
    }
}

impl<'de> Deserialize<'de> for HttpStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u16::deserialize(deserializer)?;
        HttpStatus::try_from(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_api_paths() {
        for path in [
            "/api/health",
            "/api/projects",
            "/api/tasks/foo-1a2b3c/FOO-1/move",
            "/api/tasks/foo-1a2b3c/FOO-1/attachments?name=shot%201.png",
            "/api/tasks/foo/FOO-1/attachments?name=a/../b",
            "/api/files/foo/FOO-1/image.png",
            "/api/files/foo/FOO-1/my file.png",
            "/api/files/foo/FOO-1/...png",
        ] {
            assert!(check_board_path(path).is_ok(), "{path}");
        }
    }

    #[test]
    fn rejects_paths_outside_api() {
        for path in [
            "",
            "/",
            "/events?project=x",
            "/index.html",
            "api/health",
            "/apix/health",
            "/api",
            "http://127.0.0.1:1/api/health",
            "/api/../events",
            "/api/./health",
            "/api/%2e%2e/index.html",
            "/api/%2E./x",
            "/api//health",
            "/api/health#frag",
            "/api\\..\\x",
            "/api/health\r\nX: y",
            "/api/he\u{7f}alth",
        ] {
            assert!(check_board_path(path).is_err(), "{path:?}");
        }
        let long = format!("/api/{}", "a".repeat(BOARD_MAX_PATH));
        assert!(check_board_path(&long).is_err());
    }

    #[test]
    fn project_and_content_type_validation() {
        assert!(BoardProject::try_from("unipi-app-scratch-27cf44".to_string()).is_ok());
        assert!(BoardProject::try_from("a.b_c".to_string()).is_ok());
        assert!(BoardProject::try_from(String::new()).is_err());
        assert!(BoardProject::try_from("-x".to_string()).is_err());
        assert!(BoardProject::try_from("a b".to_string()).is_err());
        assert!(BoardProject::try_from("a&b=c".to_string()).is_err());
        assert!(ContentType::try_from("application/json".to_string()).is_ok());
        assert!(ContentType::try_from("text/plain; charset=utf-8".to_string()).is_ok());
        assert!(ContentType::try_from("a\r\nb".to_string()).is_err());
        assert!(ContentType::try_from(String::new()).is_err());
        assert!(BodyLen::try_from(BOARD_MAX_BODY).is_ok());
        assert!(BodyLen::try_from(BOARD_MAX_BODY + 1).is_err());
        assert!(HttpStatus::try_from(99).is_err());
        assert!(HttpStatus::try_from(422).is_ok());
    }

    #[test]
    fn method_serde_is_uppercase() {
        assert_eq!(
            serde_json::to_value(BoardMethod::Post).unwrap(),
            serde_json::json!("POST")
        );
        assert!(serde_json::from_str::<BoardMethod>("\"get\"").is_err());
        assert_eq!("PUT".parse::<BoardMethod>().unwrap(), BoardMethod::Put);
    }
}
