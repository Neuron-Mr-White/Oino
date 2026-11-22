//! Extension settings (runtime §10.3): read the namespaces this extension registered (or any
//! namespace, without secrets it is not allowed to see) and write its own keys.
//!
//! Namespaces are declared with [`crate::Init::settings_namespace`]. Writes land in
//! `~/.oino/settings.json` ([`Scope::Global`]) or the trusted `.oino/settings.json`
//! ([`Scope::Project`]); secret fields go to the secret store. Every write is followed by an
//! `oino_settings_changed {namespace, keys, scope}` event to the owner.

use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::registry as wr;
use crate::error::parse;

/// Settings file a write targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// `~/.oino/settings.json`.
    Global,
    /// `<project>/.oino/settings.json` (fails while the project is untrusted).
    Project,
}

/// Values a read returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Layer {
    /// `defaults ⊕ global ⊕ project`.
    #[default]
    Effective,
    /// Global file only.
    Global,
    /// Project file only.
    Project,
}

/// Values of `namespace` (a JSON object).
pub fn get(namespace: &str, layer: Layer) -> Result<Value> {
    let layer = match layer {
        Layer::Effective => wr::SettingsLayer::Effective,
        Layer::Global => wr::SettingsLayer::Global,
        Layer::Project => wr::SettingsLayer::Project,
    };
    parse(&wr::extension_settings(namespace, layer)?)
}

/// Writes dot-path `key` of an owned namespace; `None` deletes it ("use default").
pub fn set(namespace: &str, key: &str, value: Option<&Value>, scope: Scope) -> Result<()> {
    let scope = match scope {
        Scope::Global => wr::SettingsScope::Global,
        Scope::Project => wr::SettingsScope::Project,
    };
    let value = value.map(Value::to_string);
    Ok(wr::set_extension_setting(
        namespace,
        key,
        value.as_deref(),
        scope,
    )?)
}
