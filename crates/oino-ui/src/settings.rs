//! Settings schema dialect OSS-2 (§7) — wire types.
//!
//! This module holds the namespace / section / field types needed by
//! [`UiView::Settings`](crate::slot::UiView::Settings) and the registry schema.
//! The `resolve` / `simplify` semantics (Schemastery layering) land in WP-U8.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::extras::icon::Icon;

/// One settings namespace (§7 `SettingsNamespace`): rendered as one Settings
/// section per namespace in the single Settings screen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsNamespace {
    /// Unique per host: `[a-z0-9][a-z0-9.-]*`; collisions reject the second
    /// registration.
    pub namespace: String,
    /// Display title.
    pub title: String,
    /// Owner (`"oino"` for core, `"host"`/`"app"`, or the extension id).
    pub extension_id: String,
    /// Section icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    /// Sort weight (§6.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
    /// Section description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Values for keys without a field default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Json>,
    /// Pages of fields.
    #[serde(default)]
    pub sections: Vec<SettingsSection>,
}

/// One page of fields inside a namespace (§7 `Section`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    /// Section id.
    pub id: String,
    /// Section title.
    pub title: String,
    /// Section description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Collapsed behind "Advanced".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced: Option<bool>,
    /// The section's fields.
    #[serde(default)]
    pub fields: Vec<Field>,
}

/// Value scope of a field (§7 `scope`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldScope {
    /// Global and project layers (project wins).
    #[default]
    Both,
    /// Global only.
    Global,
    /// Project only.
    Project,
}

/// Where a dynamic field fetches its options (§7 `dynamic`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dynamic {
    /// Service name (host.md §9 / services).
    pub service: String,
    /// Optional operation within the service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
}

/// Visibility / enabled condition (§7 `Cond`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct Cond {
    /// Key the condition reads.
    pub key: String,
    /// Equals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<Json>,
    /// Not equals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ne: Option<Json>,
    /// One of.
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub in_: Option<Vec<Json>>,
    /// Non-empty / true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truthy: Option<bool>,
}

/// When a restart notice is shown (§7 `restart`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Restart {
    /// The session restarts.
    Session,
    /// The host restarts.
    Host,
}

/// String formats with dedicated editors (§7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StringFormat {
    /// File / directory path (picker).
    Path,
    /// URL.
    Url,
    /// Glob pattern.
    Glob,
    /// Keybinding (recorder).
    Keybinding,
}

/// Option display of an enum field (§7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnumDisplay {
    /// Dropdown.
    #[default]
    Select,
    /// Segmented control (≤ 4 options).
    Segmented,
    /// Radio list.
    Radio,
}

/// One enum / multiselect choice (§7).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    /// Stored value.
    pub value: Json,
    /// Display label.
    pub label: String,
    /// Description line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Field type discriminator (§7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    /// Boolean switch.
    #[default]
    Boolean,
    /// Single choice.
    Enum,
    /// Multiple choice.
    Multiselect,
    /// Text.
    String,
    /// Number.
    Number,
    /// Secret (stored via secrets, never echoed).
    Secret,
    /// Ordered subset drag list.
    Order,
    /// `provider/model` picker.
    Model,
    /// Runs a command.
    Action,
    /// Nested page.
    Page,
    /// Repeatable field rows (e.g. MCP servers).
    List,
    /// Map of keys to values (e.g. env maps).
    Dict,
    /// JSON escape hatch with optional schema.
    Json,
}

/// Type-specific props of a field (§7). Which props are meaningful depends on
/// [`Field::type_`]; unknown ones are ignored (D-U14). Numbers keep their JSON
/// form so integer settings round-trip exactly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldProps {
    /// Enum / multiselect options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
    /// Enum accepts a custom value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_custom: Option<bool>,
    /// Enum display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<EnumDisplay>,
    /// Multiselect minimum / number minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<serde_json::Number>,
    /// Multiselect maximum / number maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<serde_json::Number>,
    /// Number step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<serde_json::Number>,
    /// Number unit suffix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Number must be an integer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integer: Option<bool>,
    /// String multi-line editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiline: Option<bool>,
    /// String validation pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// String placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// String minimum length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    /// String maximum length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// String format with a dedicated editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<StringFormat>,
    /// Order field universe.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// Model picker filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Json>,
    /// Action command (`command-invoke`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Action confirmation question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    /// Page nested sections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SettingsSection>,
    /// List item template (`key` is unused inside).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Box<Field>>,
    /// Dict value template (`key` is unused inside).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Box<Field>>,
    /// Dict key validation pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_pattern: Option<String>,
    /// Dict key label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_label: Option<String>,
    /// List "add" button label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_label: Option<String>,
    /// JSON schema for the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Json>,
}

/// One settings field (§7 `Field (common)`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// Dot path into the namespace object.
    pub key: String,
    /// Field type.
    #[serde(rename = "type")]
    pub type_: FieldType,
    /// Label.
    pub label: String,
    /// Explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Dim hint under the editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Badge text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    /// Field default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Json>,
    /// Value scope (default `both`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<FieldScope>,
    /// Collapsed behind "Advanced".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced: Option<bool>,
    /// Options fetched from a service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<Dynamic>,
    /// Render only when the condition holds.
    #[serde(rename = "visibleIf", default, skip_serializing_if = "Option::is_none")]
    pub visible_if: Option<Cond>,
    /// Editable only when the condition holds.
    #[serde(
        rename = "disabledIf",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub disabled_if: Option<Cond>,
    /// Shows "applies after restart".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<Restart>,
    /// Type-specific props.
    #[serde(flatten)]
    pub props: FieldProps,
}

impl Field {
    /// A simple field.
    #[must_use]
    pub fn new(key: impl Into<String>, type_: FieldType, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            type_,
            label: label.into(),
            description: None,
            hint: None,
            badge: None,
            default: None,
            scope: None,
            advanced: None,
            dynamic: None,
            visible_if: None,
            disabled_if: None,
            restart: None,
            props: FieldProps::default(),
        }
    }

    /// A boolean field.
    #[must_use]
    pub fn boolean(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(key, FieldType::Boolean, label)
    }

    /// An enum field with options.
    #[must_use]
    pub fn enumeration(
        key: impl Into<String>,
        label: impl Into<String>,
        options: Vec<Choice>,
    ) -> Self {
        let mut field = Self::new(key, FieldType::Enum, label);
        field.props.options = options;
        field
    }

    /// A string field.
    #[must_use]
    pub fn string(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(key, FieldType::String, label)
    }

    /// A number field.
    #[must_use]
    pub fn number(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(key, FieldType::Number, label)
    }
}
