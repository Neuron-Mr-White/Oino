//! Host settings (host.md §9.9): one Settings screen for the app.
//!
//! The host's own settings live in `<host dir>/config.json` and are read
//! fresh on every request (no restart after a change):
//!
//! ```json
//! { "relay": "n0" | "https://relay.example",
//!   "relay_only": false,
//!   "forwarding": { "enabled": false, "ports": "any" | [5173], "devices": [] },
//!   "push": { "enabled": false, "devices": [] },
//!   "fs": { "allow": [], "deny": [] },
//!   "browser": { … }, "sharing": { … }, "cron": { … },
//!   "term": { "max": 16 },
//!   "update": { "mode": "prompt" | "auto" } }
//! ```
//!
//! - `host_settings_get` → the whole object, secret-looking values masked
//!   ([`SECRET_MASK`]); the file's unknown keys are preserved but only the
//!   known ones are returned.
//! - `host_settings_set{key, value}` → one dotted key ([`HostSettingsKey`]);
//!   the merged `config.json` is validated before it is written, so an
//!   unknown key or a value of the wrong shape is `E_SCHEMA` and changes
//!   nothing. Only this computer may change its host settings.
//!
//! App settings (`closeAction`, autostart, UI prefs) live in the app's own
//! config dir and never travel here; pi agent settings and extension settings
//! are core's (`_oino/get_settings` etc. over a session stream). This module
//! is only the middle layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Mask shown in place of a secret-looking value (core's `SECRET_MASK`).
pub const SECRET_MASK: &str = "••••••";

/// The top-level blocks of `config.json` (host.md §9.9).
pub const HOST_SETTINGS_BLOCKS: [&str; 9] = [
    "forwarding",
    "push",
    "fs",
    "browser",
    "sharing",
    "cron",
    "term",
    "update",
    "lsp",
];

/// Leaves (and block names) `host_settings_set` accepts. Blocks owned by
/// other modules (`browser`, `sharing`, `cron`) accept any leaf under their
/// prefix — their own loaders are the schema.
pub const HOST_SETTINGS_KEYS: [&str; 20] = [
    "relay",
    "relay_only",
    "forwarding",
    "forwarding.enabled",
    "forwarding.ports",
    "forwarding.devices",
    "push",
    "push.enabled",
    "push.devices",
    "fs",
    "fs.allow",
    "fs.deny",
    "term",
    "term.max",
    "update",
    "update.mode",
    "browser",
    "sharing",
    "lsp",
    "lsp.enabled",
];

/// Is `key` a host settings key (an exact leaf/block or a leaf of an
/// open block)? `cron` is open too; it is not in [`HOST_SETTINGS_KEYS`]
/// because its whole block is a settings object of its own.
pub fn is_host_settings_key(key: &str) -> bool {
    if HOST_SETTINGS_KEYS.contains(&key) || key == "cron" {
        return true;
    }
    // app spec §3.12 (WP-A21): `lsp.*` (`lsp.enabled`, `lsp.idle_minutes`,
    // `lsp.servers.<language>.command`) is an open block too — the host's
    // `lsp` loader is its schema, like browser/sharing/cron.
    ["browser", "sharing", "cron", "lsp"].iter().any(|block| {
        key.strip_prefix(block)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('.') && !rest.ends_with('.'))
    })
}

/// A dotted host settings key: `relay`, `term.max`, `forwarding.enabled`, …
/// Unknown keys are a schema error, never a silent write into nowhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HostSettingsKey(String);

impl HostSettingsKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HostSettingsKey {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !is_host_settings_key(&value) {
            return Err(format!(
                "unknown host settings key `{value}` (known: {})",
                HOST_SETTINGS_KEYS.join(", ")
            ));
        }
        Ok(HostSettingsKey(value))
    }
}

impl<'de> Deserialize<'de> for HostSettingsKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        HostSettingsKey::try_from(String::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

/// `relay`: the public N0 relays (`"n0"`) or a self-hosted relay URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RelaySetting(String);

impl RelaySetting {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RelaySetting {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "n0" {
            return Ok(RelaySetting(value));
        }
        // Same loose check as the other URL fields (`NotifyEndpoint`): the
        // host parses the URL for real when it dials the relay.
        let rest = value.strip_prefix("https://").unwrap_or_default();
        if rest.is_empty() || rest.starts_with('/') {
            return Err("relay must be \"n0\" or an https URL".into());
        }
        Ok(RelaySetting(value))
    }
}

impl<'de> Deserialize<'de> for RelaySetting {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        RelaySetting::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// `host_settings_set{key, value}`: patch one dotted key of `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostSettingsSetRequest {
    pub key: HostSettingsKey,
    pub value: Value,
}

/// App spec §3.14 (WP-A24): the hub-level `_oino/*` subset — settings, auth and
/// extension commands against core services, **without a session runtime**
/// (`host_cmd{cmd}`). Tag names are the `_oino/*` command names minus the
/// prefix; replies match the session commands' `{settings}`, `{value}`,
/// `{namespaces}`, `{extensions}` … shapes exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostCmd {
    /// User-only Settings action, deliberately absent from session/model commands.
    BuildSearchIndex { cwd: String, consent: bool },
    /// `_oino/get_settings`: the merged pi settings object, or one dot-path
    /// value → `{settings}` / `{value}`.
    GetSettings {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// `_oino/set_setting`: write (or, with `null`, delete) one dot-path key.
    SetSetting {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        /// `global` (`<agent>/settings.json`) or `project` (`<cwd>/.oino/settings.json`).
        scope: String,
    },
    /// `_oino/get_settings_schema`: every extension settings namespace →
    /// `{namespaces: […]}`.
    GetSettingsSchema,
    /// `_oino/get_extension_settings`: one namespace's resolved values →
    /// `{values, diagnostics}`.
    GetExtensionSettings {
        namespace: String,
        /// `effective` (default) | `global` | `project`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layer: Option<String>,
        /// The project whose services answer (its `.oino/settings.json` is the `project`
        /// layer); omitted = the host's primary cwd. Global writes reach every live
        /// services set either way.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// `_oino/set_extension_setting`: write (or, with `null`, unset) one key.
    SetExtensionSetting {
        namespace: String,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        /// `global` | `project`.
        scope: String,
        /// The project whose services answer (its `.oino/settings.json` is the `project`
        /// layer); omitted = the host's primary cwd. Global writes reach every live
        /// services set either way.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// `_oino/unset_extension_setting`: delete one key (back to the default).
    UnsetExtensionSetting {
        namespace: String,
        key: String,
        scope: String,
        /// The project whose services answer (its `.oino/settings.json` is the `project`
        /// layer); omitted = the host's primary cwd. Global writes reach every live
        /// services set either way.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// `_oino/extensions`: loaded + disabled extensions → `{extensions: […]}`.
    Extensions,
    /// `_oino/set_extension_enabled`: add to / remove from `disabledExtensions`
    /// (applies to sessions created afterwards). → `null`.
    SetExtensionEnabled { id: String, enabled: bool },
    /// Settings › Extensions › "Install extension…": install the package at `path` **on
    /// the host machine** (a folder with `extension.json` + component, or a `.zip` of one —
    /// the phone uploads it through `blob_put` first) into `<agent dir>/extensions/<id>`,
    /// validated like core's discovery (manifest + checksums). A same-id user package is
    /// only replaced with `replace: true`; without it the reply is
    /// `{installed: false, confirm: {id, name, version, installedVersion}}`. Success →
    /// `{installed: true, id, name, version, path, replaced?, warnings, settingsNamespaces,
    /// needsRestart}` (`needsRestart`: the running host loads it after a restart).
    InstallExtension {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace: Option<bool>,
    },
    /// Remove a user-installed package (`<agent dir>/extensions/<id>`); bundled packages
    /// are refused (turn them off instead) → `{id, path, bundledRemains, needsRestart}`.
    RemoveExtension { id: String },
    /// `_oino/reload_extensions`: drop the host's cached extension sets and
    /// discover the packages again — the freshly installed (or removed) package
    /// is loaded without restarting the host. → `{extensions: [...]}`.
    ReloadExtensions,
    /// `_oino/reload`: everything `reload_extensions` does, plus `models.json` and
    /// the provider credentials — the settings page's "Reload" button, so a model or
    /// key edited outside the app (or a package dropped into the folder) is picked up
    /// without restarting it. Live chats follow with their own `reload` command.
    /// → `{extensions: [...], models: [...]}`.
    Reload,
    /// `_oino/auth_providers`: providers with sign-in support and credential state →
    /// `{providers: […]}`. Rows carry the hub's cached credential validation (D-37):
    /// `state` (`ready` | `invalid_key` | `unreachable` | `unknown` | `not_configured`),
    /// `ready` (present iff validated), `reason`, `checkedAt`. With `validate: true` the
    /// host waits for this call's probes (bounded, parallel) instead of answering from
    /// the cache and refreshing in the background.
    AuthProviders {
        /// Wait for fresh validation of configured providers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        validate: Option<bool>,
    },
    /// `default_model` (D-01): the model a new session will start on →
    /// `{model, source, reason, defaultConfigured, providerState}` — the draft chip shows
    /// `model`, or (with `model: null`) that no usable provider exists yet.
    DefaultModel,
    /// `available_models`: the model catalogue a new session would offer (`{models}`, the
    /// `get_available_models` shape, each row with the hub's readiness) — the draft's
    /// picker asks this when no chat is open, instead of a stale remembered list.
    AvailableModels,
    /// `_oino/login`: **not served here** — the OAuth flow needs a session's
    /// frontend for its dialogs; the app opens it from a chat (WP-A25).
    Login { provider: String },
    /// `_oino/logout`: remove a provider's stored credential.
    Logout { provider: String },
    /// `_oino/set_api_key`: store an API key in `auth.json`.
    SetApiKey { provider: String, key: String },
    /// `_oino/get_shortcuts`: every extension's registered shortcuts →
    /// `{shortcuts: […]}`.
    GetShortcuts,
}

impl HostCmd {
    /// The `_oino/*` command name this is the hub-level twin of.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BuildSearchIndex { .. } => "build_search_index",
            Self::GetSettings { .. } => "get_settings",
            Self::SetSetting { .. } => "set_setting",
            Self::GetSettingsSchema => "get_settings_schema",
            Self::GetExtensionSettings { .. } => "get_extension_settings",
            Self::SetExtensionSetting { .. } => "set_extension_setting",
            Self::UnsetExtensionSetting { .. } => "unset_extension_setting",
            Self::Extensions => "extensions",
            Self::SetExtensionEnabled { .. } => "set_extension_enabled",
            Self::InstallExtension { .. } => "install_extension",
            Self::RemoveExtension { .. } => "remove_extension",
            Self::ReloadExtensions => "reload_extensions",
            Self::Reload => "reload",
            Self::AuthProviders { .. } => "auth_providers",
            Self::DefaultModel => "default_model",
            Self::AvailableModels => "available_models",
            Self::Login { .. } => "login",
            Self::Logout { .. } => "logout",
            Self::SetApiKey { .. } => "set_api_key",
            Self::GetShortcuts => "get_shortcuts",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_closed_but_open_blocks_take_leaves() {
        assert!(is_host_settings_key("relay"));
        assert!(is_host_settings_key("relay_only"));
        assert!(is_host_settings_key("forwarding.enabled"));
        assert!(is_host_settings_key("fs.allow"));
        assert!(is_host_settings_key("term.max"));
        assert!(is_host_settings_key("update.mode"));
        assert!(is_host_settings_key("cron"));
        assert!(is_host_settings_key("cron.concurrency"));
        assert!(is_host_settings_key("browser.headless"));
        assert!(is_host_settings_key("sharing.anything"));
        assert!(!is_host_settings_key("nope"));
        assert!(!is_host_settings_key("browser."));
        assert!(!is_host_settings_key("browser..x"));
        assert!(!is_host_settings_key("term."));
        assert!(!is_host_settings_key(""));
    }

    #[test]
    fn unknown_key_is_a_schema_error() {
        assert!(serde_json::from_str::<HostSettingsKey>("\"term.max\"").is_ok());
        assert!(serde_json::from_str::<HostSettingsKey>("\"term.nope\"").is_err());
        let request: HostSettingsSetRequest =
            serde_json::from_str(r#"{"key":"update.mode","value":"auto"}"#).unwrap();
        assert_eq!(request.key.as_str(), "update.mode");
        assert_eq!(request.value, serde_json::json!("auto"));
    }

    #[test]
    fn relay_is_n0_or_https() {
        assert_eq!(
            RelaySetting::try_from("n0".to_string()).unwrap().as_str(),
            "n0"
        );
        assert!(RelaySetting::try_from("https://relay.example".to_string()).is_ok());
        assert!(RelaySetting::try_from("http://relay.example".to_string()).is_err());
        assert!(RelaySetting::try_from("relay.example".to_string()).is_err());
        assert!(RelaySetting::try_from("".to_string()).is_err());
    }

    #[test]
    fn host_cmd_wire_names_match_the_oino_commands() {
        // The tag is the `_oino/*` name minus the prefix, args keep the same keys.
        let cmd: HostCmd = serde_json::from_str(
            r#"{"kind":"set_extension_enabled","id":"unipi.memory","enabled":false}"#,
        )
        .unwrap();
        assert_eq!(
            cmd,
            HostCmd::SetExtensionEnabled {
                id: "unipi.memory".into(),
                enabled: false,
            }
        );
        assert_eq!(cmd.kind(), "set_extension_enabled");
        let round = serde_json::to_value(&cmd).unwrap();
        assert_eq!(round["kind"], "set_extension_enabled");
        assert_eq!(round["id"], "unipi.memory");
        assert_eq!(round["enabled"], false);
        // Absent-optionals stay off the wire.
        let get: HostCmd = serde_json::from_str(r#"{"kind":"get_settings"}"#).unwrap();
        assert_eq!(get, HostCmd::GetSettings { key: None });
        assert_eq!(
            serde_json::to_value(&get).unwrap().to_string(),
            "{\"kind\":\"get_settings\"}"
        );
        for (cmd, name) in [
            (HostCmd::GetSettingsSchema, "get_settings_schema"),
            (HostCmd::Extensions, "extensions"),
            (HostCmd::AuthProviders { validate: None }, "auth_providers"),
            (HostCmd::GetShortcuts, "get_shortcuts"),
            (HostCmd::Reload, "reload"),
            (
                HostCmd::RemoveExtension {
                    id: "acme.x".into(),
                },
                "remove_extension",
            ),
        ] {
            assert_eq!(cmd.kind(), name);
            assert_eq!(serde_json::to_value(&cmd).unwrap()["kind"], name);
        }
        let install: HostCmd =
            serde_json::from_str(r#"{"kind":"install_extension","path":"/tmp/pkg"}"#).unwrap();
        assert_eq!(
            install,
            HostCmd::InstallExtension {
                path: "/tmp/pkg".into(),
                replace: None,
            }
        );
        assert_eq!(install.kind(), "install_extension");
        for (cmd, name) in [(install.clone(), "install_extension")] {
            assert_eq!(cmd.kind(), name);
            assert_eq!(serde_json::to_value(&cmd).unwrap()["kind"], name);
        }
    }
}
