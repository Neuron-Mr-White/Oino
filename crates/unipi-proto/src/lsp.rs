//! Language-server wire types for `unipi/oino/1` (app spec §3.12, WP-A21).
//!
//! The host owns one language-server process per `(root, language)` — the
//! files live on the host and so do the servers, so a paired phone gets LSP
//! too. `lsp_servers{path}` answers the detected project root plus every
//! known server's availability; `lsp_open{root, language}` answers
//! `ok{data:LspAttachData}` and then becomes a persistent bi-stream of raw
//! LSP stdio bytes (kind 1 server→app, kind 2 app→server, ≤ 256 KiB chunks,
//! the Content-Length framing untouched), ending with `Control::Exit` when
//! the server dies.
//!
//! One client attaches to a server at a time: a second `lsp_open` takes over
//! and the previous stream ends with `Err{code:"E_LSP_TAKEN"}`. The command
//! comes only from the host's `config.json` (`lsp.servers.<language>.command`
//! or the compiled-in defaults) — a request naming a binary is rejected, so
//! a paired device can never run an arbitrary program.

use serde::{Deserialize, Serialize};

/// Longest language id accepted (`lsp.servers.<language>` keys).
pub const LSP_LANGUAGE_MAX: usize = 64;

/// A language id (`rust`, `typescript`, `python`, …): 1..=64 characters of
/// `[0-9a-zA-Z_-]`. It names an entry in the host's server table — never a
/// binary path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct LspLanguage(String);

impl LspLanguage {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LspLanguage {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > LSP_LANGUAGE_MAX {
            return Err(format!(
                "language must be 1..={LSP_LANGUAGE_MAX} characters, got {}",
                value.len()
            ));
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("language must be ASCII letters, digits, '-' or '_'".to_string());
        }
        Ok(LspLanguage(value))
    }
}

impl<'de> Deserialize<'de> for LspLanguage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        LspLanguage::try_from(raw).map_err(serde::de::Error::custom)
    }
}

/// One row of `lsp_servers` — a server the host could run for `language`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspServerInfo {
    /// The language id `lsp_open` takes.
    pub language: String,
    /// Display name ("rust-analyzer", "TypeScript", …).
    pub name: String,
    /// The command the host runs — from `lsp.servers.<language>.command`
    /// when configured, else the compiled-in default. Shown read-only.
    pub command: String,
    /// The binary (first word of `command`) was found on the host's PATH.
    pub available: bool,
    /// How to install it when `available` is false (no auto-download, §3.12).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub install_hint: Option<String>,
}

/// `ok.data` payload of `lsp_servers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspServersData {
    /// The detected project root for `path` (nearest ancestor with a
    /// marker — Cargo.toml, package.json, tsconfig.json, pyproject.toml,
    /// go.mod, compile_commands.json, .git — inside the fs scope), or
    /// `null` when nothing above `path` marks a project.
    pub root: Option<String>,
    pub servers: Vec<LspServerInfo>,
}

/// `ok.data` payload of `lsp_open`, sent before the stream starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspAttachData {
    /// The display name of the server that was attached (or started).
    pub server: String,
    /// `true` when the stream attached to a server that was already
    /// running (re-attach or take-over); `false` for a fresh spawn.
    pub attached: bool,
    /// The cached `initialize` result when the server already
    /// initialized for an earlier client — the host answers the new
    /// client's `initialize` from this and swallows its `initialized`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub initialize_result: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_validates() {
        assert!(LspLanguage::try_from("rust".to_string()).is_ok());
        assert!(LspLanguage::try_from("c++".to_string()).is_err());
        assert!(LspLanguage::try_from("".to_string()).is_err());
        assert!(LspLanguage::try_from("a/b".to_string()).is_err());
        assert!(LspLanguage::try_from("x".repeat(65)).is_err());
    }

    #[test]
    fn servers_data_shape() {
        let data = LspServersData {
            root: Some("/home/u/app".into()),
            servers: vec![LspServerInfo {
                language: "rust".into(),
                name: "rust-analyzer".into(),
                command: "rust-analyzer".into(),
                available: true,
                install_hint: None,
            }],
        };
        assert_eq!(
            serde_json::to_value(&data).expect("serialize"),
            serde_json::json!({
                "root": "/home/u/app",
                "servers": [{
                    "language": "rust",
                    "name": "rust-analyzer",
                    "command": "rust-analyzer",
                    "available": true,
                }]
            })
        );
    }

    #[test]
    fn attach_data_shape() {
        let data = LspAttachData {
            server: "rust-analyzer".into(),
            attached: true,
            initialize_result: Some(serde_json::json!({"capabilities":{}})),
        };
        assert_eq!(
            serde_json::to_value(&data).expect("serialize"),
            serde_json::json!({
                "server": "rust-analyzer",
                "attached": true,
                "initialize_result": {"capabilities":{}},
            })
        );
        let fresh = LspAttachData {
            server: "gopls".into(),
            attached: false,
            initialize_result: None,
        };
        assert_eq!(
            serde_json::to_value(&fresh).expect("serialize"),
            serde_json::json!({"server":"gopls","attached":false})
        );
    }
}
