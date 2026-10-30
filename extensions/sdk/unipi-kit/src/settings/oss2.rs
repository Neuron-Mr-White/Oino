//! OSS-2 settings builder (ui-protocol §7): typed, additive over [`oino_ui::settings`].
//!
//! The v1 builder ([`super::schema`]) keeps emitting namespaces that OSS-2 accepts unchanged;
//! this module is the path for the v2 additions — `list` / `dict` / `json` fields,
//! `visibleIf` / `disabledIf` conditions, `restart`, and namespace `icon` / `order` /
//! `description`. It builds the same wire type core, the TUI hub and the app form engine read.
//!
//! ```ignore
//! use unipi_kit::settings::oss2::*;
//!
//! let ns = Namespace::new("unipi.mcp", "MCP", "unipi.mcp")
//!     .icon(Icon::Bot)
//!     .description("MCP servers this extension may start")
//!     .section(
//!         Section::new("servers", "Servers").field(
//!             list("servers", "Servers", Field::string("command", "Command"))
//!                 .add_label("Add server"),
//!         ),
//!     );
//! cx.settings_namespace(&ns.to_json());
//! ```
//!
//! # Errors
//!
//! Building cannot fail; `to_json` only serializes (and cannot fail for these types).

pub use oino_ui::extras::icon::Icon;
pub use oino_ui::settings::{
    Choice, Cond, Dynamic, EnumDisplay, Field, FieldProps, FieldScope, FieldType, Restart,
    SettingsNamespace, SettingsSection, StringFormat,
};
use serde_json::Value as Json;

/// A stored-value choice (`{value, label, description?}`).
#[must_use]
pub fn choice(value: &str, label: &str) -> Choice {
    Choice {
        value: Json::String(value.to_owned()),
        label: label.to_owned(),
        description: None,
    }
}

/// A choice with a description.
#[must_use]
pub fn choice_with(value: &str, label: &str, description: &str) -> Choice {
    let mut choice = choice(value, label);
    choice.description = Some(description.to_owned());
    choice
}

/// `visibleIf` / `disabledIf` condition helpers (§7 `Cond`).
pub mod cond {
    use serde_json::Value as Json;

    use oino_ui::settings::Cond;

    /// Field `key` equals `value`.
    #[must_use]
    pub fn eq(key: &str, value: impl Into<Json>) -> Cond {
        Cond {
            key: key.to_owned(),
            eq: Some(value.into()),
            ..Cond::default()
        }
    }

    /// Field `key` differs from `value`.
    #[must_use]
    pub fn ne(key: &str, value: impl Into<Json>) -> Cond {
        Cond {
            key: key.to_owned(),
            ne: Some(value.into()),
            ..Cond::default()
        }
    }

    /// Field `key` is one of `values`.
    #[must_use]
    pub fn one_of(key: &str, values: impl IntoIterator<Item = Json>) -> Cond {
        Cond {
            key: key.to_owned(),
            in_: Some(values.into_iter().collect()),
            ..Cond::default()
        }
    }

    /// Field `key` is truthy (non-empty string/array/object, `true`, non-zero).
    #[must_use]
    pub fn truthy(key: &str) -> Cond {
        Cond {
            key: key.to_owned(),
            truthy: Some(true),
            ..Cond::default()
        }
    }
}

/// OSS-2 setters shared by every field type (`Field` is a plain wire struct).
pub trait Oss2Field: Sized {
    /// Render only when `cond` holds on the effective values.
    #[must_use]
    fn visible_if(self, cond: Cond) -> Self;
    /// Editable only when `cond` holds.
    #[must_use]
    fn disabled_if(self, cond: Cond) -> Self;
    /// Shows "applies after restart".
    #[must_use]
    fn restart(self, restart: Restart) -> Self;
    /// Collapsed behind "Advanced".
    #[must_use]
    fn advanced(self) -> Self;
    /// Value layer the field writes to (default `both`).
    #[must_use]
    fn scope(self, scope: FieldScope) -> Self;
    /// Explanation under the label.
    #[must_use]
    fn description(self, text: impl Into<String>) -> Self;
    /// Dim hint under the editor.
    #[must_use]
    fn hint(self, text: impl Into<String>) -> Self;
    /// Default value (`default`, distinct from the namespace `defaults`).
    #[must_use]
    fn default_value(self, value: impl Into<Json>) -> Self;
    /// Loads choices from a host service (`dynamic: {service, op?}`).
    #[must_use]
    fn dynamic(self, service: &str) -> Self;
    /// List add-row label (`list { addLabel }`).
    #[must_use]
    fn add_label(self, label: &str) -> Self;
}

impl Oss2Field for Field {
    fn visible_if(mut self, cond: Cond) -> Self {
        self.visible_if = Some(cond);
        self
    }

    fn disabled_if(mut self, cond: Cond) -> Self {
        self.disabled_if = Some(cond);
        self
    }

    fn restart(mut self, restart: Restart) -> Self {
        self.restart = Some(restart);
        self
    }

    fn advanced(mut self) -> Self {
        self.advanced = Some(true);
        self
    }

    fn scope(mut self, scope: FieldScope) -> Self {
        self.scope = Some(scope);
        self
    }

    fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    fn hint(mut self, text: impl Into<String>) -> Self {
        self.hint = Some(text.into());
        self
    }

    fn default_value(mut self, value: impl Into<Json>) -> Self {
        self.default = Some(value.into());
        self
    }

    fn dynamic(mut self, service: &str) -> Self {
        self.dynamic = Some(Dynamic {
            service: service.to_owned(),
            op: None,
        });
        self
    }

    fn add_label(mut self, label: &str) -> Self {
        self.props.add_label = Some(label.to_owned());
        self
    }
}

/// Repeatable rows of `item` (`list { item }`, new in OSS-2).
#[must_use]
pub fn list(key: &str, label: &str, item: Field) -> Field {
    let mut field = Field::new(key, FieldType::List, label);
    field.props.item = Some(Box::new(item));
    field
}

/// Key/value rows with a `value` template (`dict { value, keyPattern?, keyLabel? }`, new).
#[must_use]
pub fn dict(key: &str, label: &str, value: Field) -> Field {
    let mut field = Field::new(key, FieldType::Dict, label);
    field.props.value = Some(Box::new(value));
    field
}

/// JSON escape hatch with an optional schema (`json { schema }`, new).
#[must_use]
pub fn json(key: &str, label: &str) -> Field {
    Field::new(key, FieldType::Json, label)
}

/// Button running a command through `command-invoke`.
#[must_use]
pub fn action(key: &str, label: &str, command: &str) -> Field {
    let mut field = Field::new(key, FieldType::Action, label);
    field.props.command = Some(command.to_owned());
    field
}

/// Drill-in page (fields keep full dot-path keys).
#[must_use]
pub fn page(key: &str, label: &str, sections: Vec<SettingsSection>) -> Field {
    let mut field = Field::new(key, FieldType::Page, label);
    field.props.sections = sections;
    field
}

/// Drag-list of `items`.
#[must_use]
pub fn order(key: &str, label: &str, items: impl IntoIterator<Item = String>) -> Field {
    let mut field = Field::new(key, FieldType::Order, label);
    field.props.items = items.into_iter().collect();
    field
}

/// `provider/model` picker.
#[must_use]
pub fn model(key: &str, label: &str) -> Field {
    Field::new(key, FieldType::Model, label)
}

/// Masked value kept in the secret store (never echoed, always global).
#[must_use]
pub fn secret(key: &str, label: &str) -> Field {
    Field::new(key, FieldType::Secret, label)
}

/// Set of options (`string[]`).
#[must_use]
pub fn multiselect(key: &str, label: &str, options: Vec<Choice>) -> Field {
    let mut field = Field::new(key, FieldType::Multiselect, label);
    field.props.options = options;
    field
}

/// One-of options with an optional display style.
#[must_use]
pub fn enumeration(
    key: &str,
    label: &str,
    options: Vec<Choice>,
    display: Option<EnumDisplay>,
) -> Field {
    let mut field = Field::new(key, FieldType::Enum, label);
    field.props.options = options;
    field.props.display = display;
    field
}

/// Text with an optional format (`path` / `url` / `glob` / `keybinding`).
#[must_use]
pub fn string(key: &str, label: &str, format: Option<StringFormat>) -> Field {
    let mut field = Field::new(key, FieldType::String, label);
    field.props.format = format;
    field
}

/// One page of fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section(SettingsSection);

impl Section {
    /// A section named `title` (`id` is stable, used for deep links and search).
    #[must_use]
    pub fn new(id: &str, title: &str) -> Self {
        Self(SettingsSection {
            id: id.to_owned(),
            title: title.to_owned(),
            ..SettingsSection::default()
        })
    }

    /// Explanation under the title.
    #[must_use]
    pub fn description(mut self, text: &str) -> Self {
        self.0.description = Some(text.to_owned());
        self
    }

    /// Collapsed behind "Advanced".
    #[must_use]
    pub fn advanced(mut self) -> Self {
        self.0.advanced = Some(true);
        self
    }

    /// Adds a field (in render order).
    #[must_use]
    pub fn field(mut self, field: Field) -> Self {
        self.0.fields.push(field);
        self
    }

    /// The wire section.
    #[must_use]
    pub fn into_section(self) -> SettingsSection {
        self.0
    }
}

/// One settings namespace (`SettingsNamespace`, §7): rendered as one section of the single
/// Settings screen, written through the settings commands.
#[derive(Clone, Debug, PartialEq)]
pub struct Namespace(SettingsNamespace);

impl Namespace {
    /// Namespace `namespace` (unique per host, `[a-z0-9][a-z0-9.-]*`), owned by
    /// `extension_id` (`"oino"` for core, `"host"`/`"app"`, else the extension id).
    #[must_use]
    pub fn new(namespace: &str, title: &str, extension_id: &str) -> Self {
        Self(SettingsNamespace {
            namespace: namespace.to_owned(),
            title: title.to_owned(),
            extension_id: extension_id.to_owned(),
            ..SettingsNamespace::default()
        })
    }

    /// Section icon.
    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.0.icon = Some(icon);
        self
    }

    /// Sort weight in the Extensions group (§6.4).
    #[must_use]
    pub fn order(mut self, order: i32) -> Self {
        self.0.order = Some(order);
        self
    }

    /// One-line description shown under the title.
    #[must_use]
    pub fn description(mut self, text: &str) -> Self {
        self.0.description = Some(text.to_owned());
        self
    }

    /// Values for keys without a field default.
    #[must_use]
    pub fn defaults(mut self, defaults: Json) -> Self {
        self.0.defaults = Some(defaults);
        self
    }

    /// Adds a page of fields.
    #[must_use]
    pub fn section(mut self, section: Section) -> Self {
        self.0.sections.push(section.into_section());
        self
    }

    /// The wire namespace (register with `Init::settings_namespace` or
    /// `oino_extension::ui::register_settings`).
    #[must_use]
    pub fn into_namespace(self) -> SettingsNamespace {
        self.0
    }

    /// The wire JSON (`SettingsNamespace` per §7).
    #[must_use]
    pub fn to_json(&self) -> Json {
        serde_json::to_value(&self.0).unwrap_or(Json::Null)
    }

    /// Registers the namespace during `Init` (adds it to the init registration).
    pub fn register<'a, E: 'static>(
        &self,
        cx: &'a mut oino_extension::Init<E>,
    ) -> &'a mut oino_extension::Init<E> {
        cx.settings_namespace(&self.to_json())
    }

    /// Registers the namespace at runtime (`register-settings`).
    ///
    /// # Errors
    ///
    /// The SDK error of the host import (duplicate namespace, invalid schema).
    pub fn register_runtime(&self) -> oino_extension::Result<()> {
        oino_extension::ui::register_settings(&self.0)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use serde_json::json;

    use super::*;

    #[test]
    fn builds_an_oss2_namespace() {
        let ns = Namespace::new("unipi.mcp", "MCP", "unipi.mcp")
            .icon(Icon::Bot)
            .order(10)
            .description("servers")
            .defaults(json!({"enabled": true}))
            .section(
                Section::new("servers", "Servers").field(
                    list("servers", "Servers", Field::string("command", "Command"))
                        .advanced()
                        .restart(Restart::Session),
                ),
            )
            .section(
                Section::new("env", "Environment")
                    .field(dict("env", "Env", Field::string("value", "Value")))
                    .field(json("raw", "Raw"))
                    .field(
                        Field::boolean("enabled", "Enabled").visible_if(cond::eq("mode", "custom")),
                    ),
            );
        let json = ns.to_json();
        assert_eq!(json["namespace"], "unipi.mcp");
        assert_eq!(json["extensionId"], "unipi.mcp");
        assert_eq!(json["icon"], "bot");
        assert_eq!(json["order"], 10);
        assert_eq!(json["sections"][0]["fields"][0]["type"], "list");
        assert_eq!(json["sections"][0]["fields"][0]["item"]["type"], "string");
        assert_eq!(json["sections"][0]["fields"][0]["restart"], "session");
        assert_eq!(json["sections"][1]["fields"][1]["type"], "json");
        assert_eq!(
            json["sections"][1]["fields"][2]["visibleIf"],
            json!({"key": "mode", "eq": "custom"})
        );
        let typed: SettingsNamespace = serde_json::from_value(json).unwrap_or_default();
        assert_eq!(typed, ns.into_namespace());
    }

    #[test]
    fn field_helpers_cover_the_new_types() {
        assert_eq!(
            action("go", "Go", "run").props.command.as_deref(),
            Some("run")
        );
        assert_eq!(
            order("cols", "Columns", ["a".to_owned()]).props.items,
            ["a".to_owned()]
        );
        assert_eq!(secret("token", "Token").type_, FieldType::Secret);
        assert_eq!(model("m", "Model").type_, FieldType::Model);
        assert_eq!(
            multiselect("tags", "Tags", vec![choice("a", "A")])
                .props
                .options[0]
                .label,
            "A"
        );
        assert_eq!(
            enumeration(
                "mode",
                "Mode",
                vec![choice("a", "A")],
                Some(EnumDisplay::Segmented)
            )
            .props
            .display,
            Some(EnumDisplay::Segmented)
        );
        assert_eq!(
            string("path", "Path", Some(StringFormat::Path))
                .props
                .format,
            Some(StringFormat::Path)
        );
        assert_eq!(
            page(
                "advanced",
                "Advanced",
                vec![Section::new("s", "S").into_section()]
            )
            .props
            .sections
            .len(),
            1
        );
        let cond = cond::truthy("x");
        assert_eq!(cond.truthy, Some(true));
        assert_eq!(cond::ne("a", 1).ne, Some(json!(1)));
        assert_eq!(cond::one_of("a", [json!(1)]).in_, Some(vec![json!(1)]));
        assert_eq!(choice_with("v", "V", "d").description.as_deref(), Some("d"));
    }

    #[test]
    fn field_setters_chain() {
        let field = Field::boolean("a", "A")
            .disabled_if(cond::eq("b", false))
            .scope(FieldScope::Project)
            .hint("h")
            .description("d")
            .default_value(true)
            .dynamic("svc");
        assert_eq!(field.scope, Some(FieldScope::Project));
        assert_eq!(field.hint.as_deref(), Some("h"));
        assert_eq!(field.description.as_deref(), Some("d"));
        assert_eq!(field.default, Some(json!(true)));
        assert_eq!(
            field.dynamic,
            Some(Dynamic {
                service: "svc".into(),
                op: None
            })
        );
        assert_eq!(field.disabled_if.and_then(|c| c.eq), Some(json!(false)));
    }
}
