//! Settings schema builder (ui-protocol §6.6; unipi `src/settings/schema.ts`).
//!
//! A package declares one [`Namespace`] (sections → fields, dot-path keys into its settings
//! object) and registers it with `Init::settings_namespace(&ns.to_json())`. Field defaults are
//! filled from the namespace defaults when not set explicitly, so frontends can show
//! "default" without a second source.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::values::get_field;

/// Hub scope a field is shown and written in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// Global and project (default).
    #[default]
    Both,
    /// Global only.
    Global,
    /// Project only.
    Project,
}

impl Scope {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip_serializing_if passes &T"
    )]
    fn is_both(&self) -> bool {
        *self == Self::Both
    }
}

/// An enum / multiselect option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// Stored value.
    pub value: String,
    /// Display label.
    pub label: String,
    /// Explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `Choice` with label = value and no description (unipi bare string option).
#[must_use]
pub fn plain(value: &str) -> Choice {
    Choice {
        value: value.to_owned(),
        label: value.to_owned(),
        description: None,
    }
}

/// `Choice` with a label and a description.
#[must_use]
pub fn choice(value: &str, label: &str, description: &str) -> Choice {
    Choice {
        value: value.to_owned(),
        label: label.to_owned(),
        description: Some(description.to_owned()),
    }
}

/// Physical unit of a number field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    /// Milliseconds.
    #[serde(rename = "ms")]
    Ms,
    /// Seconds.
    #[serde(rename = "s")]
    S,
    /// Minutes.
    #[serde(rename = "min")]
    Min,
    /// Days.
    #[serde(rename = "days")]
    Days,
    /// Tokens.
    #[serde(rename = "tokens")]
    Tokens,
    /// Bytes.
    #[serde(rename = "bytes")]
    Bytes,
    /// Characters.
    #[serde(rename = "chars")]
    Chars,
    /// Percent.
    #[serde(rename = "%")]
    Percent,
}

impl Unit {
    /// Display suffix (`ms`, `%`, …).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ms => "ms",
            Self::S => "s",
            Self::Min => "min",
            Self::Days => "days",
            Self::Tokens => "tokens",
            Self::Bytes => "bytes",
            Self::Chars => "chars",
            Self::Percent => "%",
        }
    }
}

/// Type-specific field properties (`type` tag).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[non_exhaustive]
pub enum Kind {
    /// On/off.
    Boolean,
    /// One of `options`.
    #[serde(rename_all = "camelCase")]
    Enum {
        /// Cycle order.
        options: Vec<Choice>,
        /// Free-text value allowed (`custom…`).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        /// Trailing "use default" option in the project scope.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        clearable: bool,
    },
    /// Set of options (`string[]`).
    #[serde(rename_all = "camelCase")]
    Multiselect {
        /// Options.
        options: Vec<Choice>,
        /// Label when empty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_label: Option<String>,
    },
    /// Text.
    #[serde(rename_all = "camelCase")]
    String {
        /// Multi-line editor.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
        /// Validation regex.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
        /// Label when empty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_label: Option<String>,
    },
    /// Number.
    #[serde(rename_all = "camelCase")]
    Number {
        /// Minimum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        /// Maximum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        /// Step.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<f64>,
        /// Unit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<Unit>,
        /// Label shown for `0`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zero_label: Option<String>,
    },
    /// Masked value stored in the secret store (never in `settings.json`).
    #[serde(rename_all = "camelCase")]
    Secret {
        /// Label when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_label: Option<String>,
    },
    /// Ordered subset of `items`.
    Order {
        /// Item universe in canonical order.
        items: Vec<Choice>,
    },
    /// `provider/model` picker.
    #[serde(rename_all = "camelCase")]
    Model {
        /// Catalog filter (`{input: ["image"]}`, `{capability}` …).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<Value>,
        /// Only these ids (plus custom…).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        presets: Vec<String>,
        /// Preset lists keyed by the sibling `provider_key` field value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        presets_by_provider: Option<Value>,
        /// Sibling key selecting `presets_by_provider`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_key: Option<String>,
        /// First entry label that picks `""`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_option: Option<String>,
        /// Label when `""`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty_label: Option<String>,
    },
    /// Runs a command through `command-invoke` (unipi command runners).
    Action {
        /// Command name (`unipi:hint-reset`).
        command: String,
    },
    /// Nested page (fields use full keys).
    Page {
        /// Sections.
        sections: Vec<Section>,
    },
}

/// One field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// Dot-path key into the namespace object.
    pub key: String,
    /// Type and type-specific props.
    #[serde(flatten)]
    pub kind: Kind,
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
    /// Default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Scope.
    #[serde(default, skip_serializing_if = "Scope::is_both")]
    pub scope: Scope,
    /// Collapsed behind "Advanced".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub advanced: bool,
    /// Options fetched from a service (`{service}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<Value>,
}

macro_rules! setters {
    ($($(#[$doc:meta])* $name:ident: $field:ident;)*) => {
        $(
            $(#[$doc])*
            #[must_use]
            pub fn $name(mut self, v: impl Into<String>) -> Self {
                self.$field = Some(v.into());
                self
            }
        )*
    };
}

impl Field {
    fn new(key: &str, label: &str, kind: Kind) -> Self {
        Self {
            key: key.to_owned(),
            kind,
            label: label.to_owned(),
            description: None,
            hint: None,
            badge: None,
            default: None,
            scope: Scope::Both,
            advanced: false,
            dynamic: None,
        }
    }

    /// Boolean field.
    #[must_use]
    pub fn boolean(key: &str, label: &str) -> Self {
        Self::new(key, label, Kind::Boolean)
    }

    /// Enum field.
    #[must_use]
    pub fn enumeration(key: &str, label: &str, options: Vec<Choice>) -> Self {
        Self::new(
            key,
            label,
            Kind::Enum {
                options,
                allow_custom: false,
                clearable: false,
            },
        )
    }

    /// Multiselect field.
    #[must_use]
    pub fn multiselect(key: &str, label: &str, options: Vec<Choice>) -> Self {
        Self::new(
            key,
            label,
            Kind::Multiselect {
                options,
                empty_label: None,
            },
        )
    }

    /// String field.
    #[must_use]
    pub fn string(key: &str, label: &str) -> Self {
        Self::new(
            key,
            label,
            Kind::String {
                multiline: false,
                pattern: None,
                empty_label: None,
            },
        )
    }

    /// Number field.
    #[must_use]
    pub fn number(key: &str, label: &str) -> Self {
        Self::new(
            key,
            label,
            Kind::Number {
                min: None,
                max: None,
                step: None,
                unit: None,
                zero_label: None,
            },
        )
    }

    /// Secret field.
    #[must_use]
    pub fn secret(key: &str, label: &str) -> Self {
        Self::new(key, label, Kind::Secret { empty_label: None })
    }

    /// Order field.
    #[must_use]
    pub fn order(key: &str, label: &str, items: Vec<Choice>) -> Self {
        Self::new(key, label, Kind::Order { items })
    }

    /// Model field.
    #[must_use]
    pub fn model(key: &str, label: &str) -> Self {
        Self::new(
            key,
            label,
            Kind::Model {
                filter: None,
                presets: Vec::new(),
                presets_by_provider: None,
                provider_key: None,
                empty_option: None,
                empty_label: None,
            },
        )
    }

    /// Action field running `command`.
    #[must_use]
    pub fn action(key: &str, label: &str, command: &str) -> Self {
        Self::new(
            key,
            label,
            Kind::Action {
                command: command.to_owned(),
            },
        )
    }

    /// Page field.
    #[must_use]
    pub fn page(key: &str, label: &str, sections: Vec<Section>) -> Self {
        Self::new(key, label, Kind::Page { sections })
    }

    setters! {
        /// Description.
        description: description;
        /// Hint.
        hint: hint;
        /// Badge.
        badge: badge;
    }

    /// Default value.
    #[must_use]
    pub fn default_value(mut self, v: impl Into<Value>) -> Self {
        self.default = Some(v.into());
        self
    }

    /// Scope.
    #[must_use]
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Collapsed behind "Advanced".
    #[must_use]
    pub fn advanced(mut self) -> Self {
        self.advanced = true;
        self
    }

    /// Options from service `name`.
    #[must_use]
    pub fn dynamic(mut self, service: &str) -> Self {
        self.dynamic = Some(json!({"service": service}));
        self
    }

    /// Mutable type-specific props (for `allow_custom`, `unit`, `min`, …).
    #[must_use]
    pub fn with_kind(mut self, f: impl FnOnce(&mut Kind)) -> Self {
        f(&mut self.kind);
        self
    }

    /// Label shown when empty (string, secret, multiselect, model).
    #[must_use]
    pub fn empty_label(self, label: &str) -> Self {
        self.with_kind(|k| match k {
            Kind::String { empty_label, .. }
            | Kind::Secret { empty_label }
            | Kind::Multiselect { empty_label, .. }
            | Kind::Model { empty_label, .. } => *empty_label = Some(label.to_owned()),
            _ => {}
        })
    }

    fn fill_defaults(&mut self, defaults: &Value) {
        if self.default.is_none()
            && let Some(v) = get_field(defaults, &self.key)
            && !matches!(
                self.kind,
                Kind::Action { .. } | Kind::Page { .. } | Kind::Secret { .. }
            )
        {
            self.default = Some(v.clone());
        }
        if let Kind::Page { sections } = &mut self.kind {
            for s in sections {
                s.fill_defaults(defaults);
            }
        }
    }

    /// Formats a value for list display (unipi `formatFieldValue`).
    #[must_use]
    pub fn format_value(&self, value: Option<&Value>) -> String {
        let value = value.filter(|v| !v.is_null());
        let unset = || "unset".to_owned();
        match &self.kind {
            Kind::Boolean => match value.and_then(Value::as_bool) {
                Some(true) => "on".into(),
                Some(false) => "off".into(),
                None => unset(),
            },
            Kind::Enum {
                options,
                allow_custom,
                ..
            } => {
                let Some(v) = value else { return unset() };
                let raw = scalar_text(v);
                match options.iter().find(|o| o.value == raw) {
                    Some(o) => o.label.clone(),
                    None if *allow_custom => format!("⚙ {raw}"),
                    None => raw,
                }
            }
            Kind::Secret { empty_label } => match value.and_then(Value::as_str) {
                Some(s) if !s.is_empty() => "••••••".into(),
                _ => empty_label.clone().unwrap_or_else(unset),
            },
            Kind::Number {
                unit, zero_label, ..
            } => {
                let Some(v) = value else { return unset() };
                if v.as_f64() == Some(0.0)
                    && let Some(z) = zero_label
                {
                    return z.clone();
                }
                let raw = scalar_text(v);
                match unit {
                    Some(Unit::Percent) => format!("{raw}%"),
                    Some(u) => format!("{raw} {}", u.as_str()),
                    None => raw,
                }
            }
            Kind::Multiselect {
                options,
                empty_label,
            } => {
                let chosen: Vec<String> = value
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(scalar_text).collect())
                    .unwrap_or_default();
                if chosen.is_empty() {
                    return empty_label.clone().unwrap_or_else(|| "none".into());
                }
                chosen
                    .iter()
                    .map(|c| {
                        options
                            .iter()
                            .find(|o| &o.value == c)
                            .map_or(c.as_str(), |o| o.label.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            Kind::Order { items } => {
                let ids: Vec<String> = value
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(scalar_text).collect())
                    .unwrap_or_default();
                if ids.is_empty() {
                    return unset();
                }
                let labels: Vec<&str> = ids
                    .iter()
                    .map(|id| {
                        items
                            .iter()
                            .find(|i| &i.value == id)
                            .map_or(id.as_str(), |i| i.label.as_str())
                    })
                    .collect();
                let head = labels
                    .iter()
                    .take(3)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" › ");
                if labels.len() > 3 {
                    format!("{head} …")
                } else {
                    head
                }
            }
            Kind::String { empty_label, .. } | Kind::Model { empty_label, .. } => match value {
                None => empty_label.clone().unwrap_or_else(unset),
                Some(Value::String(s)) if s.is_empty() => empty_label.clone().unwrap_or_default(),
                Some(v) => scalar_text(v),
            },
            Kind::Action { .. } | Kind::Page { .. } => value.map_or_else(unset, scalar_text),
        }
    }

    /// Parses raw input text (unipi `parseFieldValue`); `None` when invalid or not text-edited.
    #[must_use]
    pub fn parse_value(&self, raw: &str) -> Option<Value> {
        let trimmed = raw.trim();
        match &self.kind {
            Kind::String { .. } | Kind::Secret { .. } | Kind::Model { .. } => {
                Some(Value::String(trimmed.to_owned()))
            }
            Kind::Number { min, max, .. } => {
                if trimmed.is_empty() {
                    return None;
                }
                let n: f64 = trimmed.parse().ok().filter(|n: &f64| n.is_finite())?;
                if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                    return None;
                }
                Some(number_value(n))
            }
            _ => None,
        }
    }
}

fn number_value(n: f64) -> Value {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "guarded by the fract/range check"
    )]
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        return Value::from(n as i64);
    }
    Value::from(n)
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// One section of fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    /// Section id (unique within the namespace).
    pub id: String,
    /// Heading.
    pub title: String,
    /// Explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Collapsed behind "Advanced".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub advanced: bool,
    /// Fields.
    pub fields: Vec<Field>,
}

impl Section {
    /// Empty section.
    #[must_use]
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_owned(),
            title: title.to_owned(),
            description: None,
            advanced: false,
            fields: Vec::new(),
        }
    }

    /// Description.
    #[must_use]
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    /// Collapsed behind "Advanced".
    #[must_use]
    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    /// Adds a field.
    #[must_use]
    pub fn field(mut self, f: Field) -> Self {
        self.fields.push(f);
        self
    }

    /// Adds fields.
    #[must_use]
    pub fn fields(mut self, fs: impl IntoIterator<Item = Field>) -> Self {
        self.fields.extend(fs);
        self
    }

    fn fill_defaults(&mut self, defaults: &Value) {
        for f in &mut self.fields {
            f.fill_defaults(defaults);
        }
    }
}

/// A settings namespace (`oino_protocol::ext::SettingsNamespace` JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Namespace {
    /// Namespace (`hints`).
    pub namespace: String,
    /// Hub title.
    pub title: String,
    /// Owning extension id.
    pub extension_id: String,
    /// Sections.
    pub sections: Vec<Section>,
    /// Defaults object (`defaults ⊕ global ⊕ project`).
    pub defaults: Value,
}

impl Namespace {
    /// Empty namespace with `{}` defaults.
    #[must_use]
    pub fn new(namespace: &str, title: &str, extension_id: &str) -> Self {
        Self {
            namespace: namespace.to_owned(),
            title: title.to_owned(),
            extension_id: extension_id.to_owned(),
            sections: Vec::new(),
            defaults: Value::Object(Map::new()),
        }
    }

    /// Defaults object; fills field defaults that are not set explicitly.
    #[must_use]
    pub fn defaults(mut self, defaults: Value) -> Self {
        self.defaults = defaults;
        let d = self.defaults.clone();
        for s in &mut self.sections {
            s.fill_defaults(&d);
        }
        self
    }

    /// Adds a section (field defaults filled from the current defaults).
    #[must_use]
    pub fn section(mut self, mut s: Section) -> Self {
        s.fill_defaults(&self.defaults);
        self.sections.push(s);
        self
    }

    /// Every field (depth-first through pages).
    #[must_use]
    pub fn all_fields(&self) -> Vec<&Field> {
        fn walk<'a>(sections: &'a [Section], out: &mut Vec<&'a Field>) {
            for s in sections {
                for f in &s.fields {
                    out.push(f);
                    if let Kind::Page { sections } = &f.kind {
                        walk(sections, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.sections, &mut out);
        out
    }

    /// Field by key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.all_fields().into_iter().find(|f| f.key == key)
    }

    /// JSON for `Init::settings_namespace` / `registry.register-settings`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notify_like() -> Namespace {
        Namespace::new("notify", "Notify", "unipi.notify")
            .defaults(json!({"events": {"agentEnd": true}, "platform": "native", "timeoutMs": 0}))
            .section(
                Section::new("events", "Events")
                    .field(Field::boolean("events.agentEnd", "Agent finished"))
                    .field(Field::enumeration(
                        "platform",
                        "Platform",
                        vec![choice("native", "Native", "OS notifications")],
                    ))
                    .field(Field::secret("ntfy.token", "ntfy token").scope(Scope::Global))
                    .field(Field::action("test", "Send test", "unipi:notify-test"))
                    .field(Field::number("timeoutMs", "Timeout").with_kind(|k| {
                        if let Kind::Number {
                            unit,
                            zero_label,
                            min,
                            ..
                        } = k
                        {
                            *unit = Some(Unit::Ms);
                            *zero_label = Some("∞ none".into());
                            *min = Some(0.0);
                        }
                    })),
            )
    }

    #[test]
    fn serialises_to_ui_protocol_shape() {
        let v = notify_like().to_json();
        assert_eq!(v["namespace"], "notify");
        assert_eq!(v["extensionId"], "unipi.notify");
        let fields = &v["sections"][0]["fields"];
        assert_eq!(
            fields[0],
            json!({"key": "events.agentEnd", "type": "boolean", "label": "Agent finished", "default": true})
        );
        assert_eq!(
            fields[1]["options"][0],
            json!({"value": "native", "label": "Native", "description": "OS notifications"})
        );
        assert_eq!(fields[1]["default"], "native");
        assert_eq!(fields[2]["scope"], "global");
        assert!(
            fields[2].get("default").is_none(),
            "secrets never carry defaults"
        );
        assert_eq!(
            fields[3],
            json!({"key": "test", "type": "action", "label": "Send test", "command": "unipi:notify-test"})
        );
        assert_eq!(fields[4]["unit"], "ms");
        let back: Namespace =
            serde_json::from_value(v).unwrap_or_else(|_| Namespace::new("x", "x", "x"));
        assert_eq!(back, notify_like());
    }

    #[test]
    fn formats_values_like_unipi() {
        let ns = notify_like();
        let f = |k: &str| {
            ns.field(k)
                .cloned()
                .unwrap_or_else(|| Field::boolean("?", "?"))
        };
        assert_eq!(f("events.agentEnd").format_value(Some(&json!(true))), "on");
        assert_eq!(f("events.agentEnd").format_value(None), "unset");
        assert_eq!(f("platform").format_value(Some(&json!("native"))), "Native");
        assert_eq!(f("platform").format_value(Some(&json!("x"))), "x");
        assert_eq!(f("ntfy.token").format_value(Some(&json!("abc"))), "••••••");
        assert_eq!(f("ntfy.token").format_value(Some(&json!(""))), "unset");
        assert_eq!(f("timeoutMs").format_value(Some(&json!(0))), "∞ none");
        assert_eq!(f("timeoutMs").format_value(Some(&json!(6000))), "6000 ms");
        let pct = Field::number("p", "P").with_kind(|k| {
            if let Kind::Number { unit, .. } = k {
                *unit = Some(Unit::Percent);
            }
        });
        assert_eq!(pct.format_value(Some(&json!(80))), "80%");
        let ms = Field::multiselect("m", "M", vec![choice("a", "Alpha", ""), plain("b")])
            .empty_label("all");
        assert_eq!(ms.format_value(Some(&json!(["a", "b"]))), "Alpha, b");
        assert_eq!(ms.format_value(Some(&json!([]))), "all");
        let order = Field::order(
            "o",
            "O",
            vec![plain("a"), plain("b"), plain("c"), plain("d")],
        );
        assert_eq!(
            order.format_value(Some(&json!(["d", "c", "b", "a"]))),
            "d › c › b …"
        );
        let custom = Field::enumeration("e", "E", vec![plain("x")]).with_kind(|k| {
            if let Kind::Enum { allow_custom, .. } = k {
                *allow_custom = true;
            }
        });
        assert_eq!(custom.format_value(Some(&json!("y"))), "⚙ y");
        let s = Field::string("s", "S").empty_label("inherit (session model)");
        assert_eq!(s.format_value(Some(&json!(""))), "inherit (session model)");
    }

    #[test]
    fn parses_input() {
        let n = Field::number("n", "N").with_kind(|k| {
            if let Kind::Number { min, max, .. } = k {
                *min = Some(1.0);
                *max = Some(10.0);
            }
        });
        assert_eq!(n.parse_value(" 5 "), Some(json!(5)));
        assert_eq!(n.parse_value("2.5"), Some(json!(2.5)));
        assert_eq!(n.parse_value("11"), None);
        assert_eq!(n.parse_value(""), None);
        assert_eq!(n.parse_value("abc"), None);
        assert_eq!(
            Field::string("s", "S").parse_value("  hi "),
            Some(json!("hi"))
        );
        assert_eq!(Field::boolean("b", "B").parse_value("true"), None);
    }
}
