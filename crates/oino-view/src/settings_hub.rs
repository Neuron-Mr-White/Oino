//! Extension settings hub model (ui-protocol §7, OSS-2): namespaces from the typed
//! [`SettingsNamespace`] descriptors (or from `get_settings_schema` JSON), flattened fields per
//! namespace, values from `get_extension_settings`, and the edits a frontend sends back
//! (`set_extension_setting` / `unset_extension_setting`).
//!
//! The layering semantics (`resolve` / `simplify`) live in `oino_ui::settings` (WP-U8); the hub
//! only projects a descriptor into rows and decides what a row shows and accepts.
//!
//! Pure data: the TUI and the desktop app render it natively.

use oino_ai::JsonValue;
use oino_ui::settings::{Choice, Field, FieldProps, FieldType, SettingsNamespace, SettingsSection};
pub use oino_ui::settings::{Cond, Dynamic, EnumDisplay, FieldScope, Restart, StringFormat};
use oino_ui::slot::{UiEntry, UiView};

use oino_ui::extras::icon::Icon;

/// Value shown for a set secret (`oino_core::extensions::settings::SECRET_MASK`).
pub const SECRET_MASK: &str = "••••••";

/// A namespace as listed in the hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubNamespace {
    /// Namespace id.
    pub namespace: String,
    /// Title (defaults to the namespace).
    pub title: String,
    /// Owning extension (`"oino"` for core, `"host"`/`"app"`, or the extension id).
    pub extension_id: String,
    /// Section icon (§7 new).
    pub icon: Option<Icon>,
    /// Sort weight (`order` desc, then title A→Z, §6.4).
    pub order: Option<i32>,
    /// Section description (§7 new).
    pub description: Option<String>,
    /// Values for keys without a field default (§7 `defaults`).
    pub defaults: Option<JsonValue>,
}

impl HubNamespace {
    /// Projects a registry descriptor (§7 `SettingsNamespace`).
    #[must_use]
    pub fn from_descriptor(descriptor: &SettingsNamespace) -> Self {
        Self {
            namespace: descriptor.namespace.clone(),
            title: if descriptor.title.is_empty() {
                descriptor.namespace.clone()
            } else {
                descriptor.title.clone()
            },
            extension_id: descriptor.extension_id.clone(),
            icon: descriptor.icon,
            order: descriptor.order,
            description: descriptor.description.clone(),
            defaults: descriptor.defaults.clone(),
        }
    }

    /// The namespace of a `settings` registry entry (`UiView::Settings`, §6.3); `None` for
    /// other entries.
    #[must_use]
    pub fn from_entry(entry: &UiEntry) -> Option<Self> {
        match &entry.view {
            UiView::Settings { namespace } => Some(Self::from_descriptor(namespace)),
            _ => None,
        }
    }
}

/// One option of an `enum` / `multiselect` field (§7).
#[derive(Clone, Debug, PartialEq)]
pub struct HubChoice {
    /// Stored value.
    pub value: JsonValue,
    /// Display label.
    pub label: String,
    /// Description line.
    pub description: Option<String>,
}

/// Field type (ui-protocol §7).
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    /// On/off.
    Boolean,
    /// One of `choices`.
    Enum {
        /// Options.
        choices: Vec<HubChoice>,
        /// Accepts a value outside `choices`.
        allow_custom: bool,
        /// `select` | `segmented` | `radio`.
        display: EnumDisplay,
    },
    /// Several of `choices`.
    MultiSelect {
        /// Options.
        choices: Vec<HubChoice>,
        /// Minimum selections.
        min: Option<usize>,
        /// Maximum selections.
        max: Option<usize>,
    },
    /// Free text.
    Text {
        /// Multi-line editor.
        multiline: bool,
        /// Placeholder shown while empty.
        placeholder: Option<String>,
        /// Dedicated editor (`path` picker, `keybinding` recorder, …).
        format: Option<StringFormat>,
    },
    /// Number with optional bounds, step and unit.
    Number {
        /// Minimum.
        min: Option<f64>,
        /// Maximum.
        max: Option<f64>,
        /// Increment.
        step: Option<f64>,
        /// Unit suffix.
        unit: Option<String>,
        /// Integers only.
        integer: bool,
    },
    /// Secret (masked).
    Secret,
    /// Reorderable list of `items` (§7 `order`).
    Order {
        /// Every value that may appear.
        items: Vec<String>,
    },
    /// `provider/id` model.
    Model {
        /// Picker filter (§7 `filter`).
        filter: Option<JsonValue>,
    },
    /// Runs `command` (`/name`).
    Action {
        /// Command to run.
        command: String,
        /// Confirmation question.
        confirm: Option<String>,
    },
    /// Repeatable rows of one template field (§7 new).
    List {
        /// Row template (`key` unused).
        item: Option<Box<HubField>>,
        /// Minimum rows.
        min: Option<usize>,
        /// Maximum rows.
        max: Option<usize>,
        /// "Add" button label.
        add_label: Option<String>,
    },
    /// Map of keys to a value template (§7 new).
    Dict {
        /// Value template (`key` unused).
        value: Option<Box<HubField>>,
        /// Key validation pattern.
        key_pattern: Option<String>,
        /// Key column label.
        key_label: Option<String>,
    },
    /// JSON escape hatch (§7 new), edited in a code editor / `editExternally`.
    Json {
        /// Schema for completion and validation.
        schema: Option<JsonValue>,
    },
}

/// One editable row.
#[derive(Clone, Debug, PartialEq)]
pub struct HubField {
    /// Dot-path key.
    pub key: String,
    /// Label.
    pub label: String,
    /// Description.
    pub description: Option<String>,
    /// Dim hint under the editor.
    pub hint: Option<String>,
    /// Badge text.
    pub badge: Option<String>,
    /// Section (and page) title path, e.g. `Events` or `Advanced › Retry`.
    pub section: String,
    /// Kind.
    pub kind: FieldKind,
    /// Where it may be written.
    pub scope: FieldScope,
    /// Default value.
    pub default: Option<JsonValue>,
    /// Hidden unless "advanced" is shown.
    pub advanced: bool,
    /// Shows "applies after restart" (§7 `restart`).
    pub restart: Option<Restart>,
    /// Rendered only when the condition holds (§7 `visibleIf`).
    pub visible_if: Option<Cond>,
    /// Editable only when the condition holds (§7 `disabledIf`).
    pub disabled_if: Option<Cond>,
    /// Options fetched from a service (§7 `dynamic`).
    pub dynamic: Option<Dynamic>,
}

/// Normalises a `get_settings_schema` namespace into the typed descriptor: tolerant of the v1
/// encodings (`options` as bare strings, missing titles/labels) so old namespaces keep loading
/// unchanged, and of icon names this frontend does not know (§7, D-U14).
fn descriptor(value: &JsonValue) -> Option<SettingsNamespace> {
    let mut value = value.clone();
    normalise_namespace(&mut value);
    serde_json::from_value(value).ok()
}

fn normalise_namespace(value: &mut JsonValue) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let namespace = map
        .get("namespace")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned();
    for (key, default) in [("title", namespace.clone()), ("extensionId", String::new())] {
        if map.get(key).and_then(JsonValue::as_str).is_none() {
            map.insert(key.to_owned(), JsonValue::String(default));
        }
    }
    if let Some(icon) = map.get("icon").and_then(JsonValue::as_str)
        && serde_json::from_value::<Icon>(JsonValue::String(icon.to_owned())).is_err()
    {
        map.remove("icon");
    }
    if let Some(sections) = map.get_mut("sections").and_then(JsonValue::as_array_mut) {
        for section in sections {
            normalise_section(section);
        }
    }
}

fn normalise_section(section: &mut JsonValue) {
    let Some(map) = section.as_object_mut() else {
        return;
    };
    let id = map
        .get("id")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned();
    if map.get("id").and_then(JsonValue::as_str).is_none() {
        let title = map
            .get("title")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        map.insert("id".to_owned(), JsonValue::String(title));
    }
    if map.get("title").and_then(JsonValue::as_str).is_none() {
        map.insert("title".to_owned(), JsonValue::String(id));
    }
    if let Some(fields) = map.get_mut("fields").and_then(JsonValue::as_array_mut) {
        for field in fields {
            normalise_field(field);
        }
    }
}

fn normalise_field(field: &mut JsonValue) {
    let Some(map) = field.as_object_mut() else {
        return;
    };
    let key = map
        .get("key")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_owned();
    if map.get("key").and_then(JsonValue::as_str).is_none() {
        // `FieldWithoutKey` templates of `list` / `dict` have no key (§7).
        map.insert("key".to_owned(), JsonValue::String(key.clone()));
    }
    if map.get("label").and_then(JsonValue::as_str).is_none() {
        map.insert("label".to_owned(), JsonValue::String(key.clone()));
    }
    if map.get("type").and_then(JsonValue::as_str).is_none() {
        map.insert("type".to_owned(), JsonValue::String("boolean".to_owned()));
    }
    if let Some(options) = map.get_mut("options").and_then(JsonValue::as_array_mut) {
        for option in options {
            match option {
                JsonValue::String(label) => {
                    *option = serde_json::json!({"value": label, "label": label});
                }
                JsonValue::Object(option) => {
                    let label = option
                        .get("label")
                        .cloned()
                        .unwrap_or(JsonValue::String(String::new()));
                    option.entry("value").or_insert(label);
                }
                _ => {}
            }
        }
    }
    if let Some(sections) = map.get_mut("sections").and_then(JsonValue::as_array_mut) {
        for section in sections {
            normalise_section(section);
        }
    }
    for template in ["item", "value"] {
        if let Some(inner) = map.get_mut(template)
            && inner.is_object()
        {
            normalise_field(inner);
        }
    }
}

fn usize_prop(number: Option<&serde_json::Number>) -> Option<usize> {
    number
        .and_then(serde_json::Number::as_u64)
        .and_then(|n| usize::try_from(n).ok())
}

fn choices(options: &[Choice]) -> Vec<HubChoice> {
    options
        .iter()
        .map(|option| HubChoice {
            value: option.value.clone(),
            label: if option.label.is_empty() {
                option
                    .value
                    .as_str()
                    .map_or_else(|| option.value.to_string(), str::to_owned)
            } else {
                option.label.clone()
            },
            description: option.description.clone(),
        })
        .collect()
}

fn field_kind(field: &Field) -> Option<FieldKind> {
    let FieldProps {
        options: props_options,
        allow_custom,
        display,
        min,
        max,
        step,
        unit,
        integer,
        multiline,
        placeholder,
        format,
        items,
        filter,
        command,
        confirm,
        item,
        value,
        key_pattern,
        key_label,
        add_label,
        schema,
        ..
    } = &field.props;
    Some(match field.type_ {
        FieldType::Boolean => FieldKind::Boolean,
        FieldType::Enum => FieldKind::Enum {
            choices: choices(props_options),
            allow_custom: allow_custom.unwrap_or(false),
            display: display.unwrap_or_default(),
        },
        FieldType::Multiselect => FieldKind::MultiSelect {
            choices: choices(props_options),
            min: usize_prop(min.as_ref()),
            max: usize_prop(max.as_ref()),
        },
        FieldType::String => FieldKind::Text {
            multiline: multiline.unwrap_or(false),
            placeholder: placeholder.clone(),
            format: *format,
        },
        FieldType::Number => FieldKind::Number {
            min: min.as_ref().and_then(serde_json::Number::as_f64),
            max: max.as_ref().and_then(serde_json::Number::as_f64),
            step: step.as_ref().and_then(serde_json::Number::as_f64),
            unit: unit.clone(),
            integer: integer.unwrap_or(false),
        },
        FieldType::Secret => FieldKind::Secret,
        FieldType::Order => FieldKind::Order {
            items: items.clone(),
        },
        FieldType::Model => FieldKind::Model {
            filter: filter.clone(),
        },
        FieldType::Action => FieldKind::Action {
            command: command.clone().unwrap_or_default(),
            confirm: confirm.clone(),
        },
        FieldType::List => FieldKind::List {
            item: item
                .as_deref()
                .and_then(|field| hub_field(field, "", false))
                .map(Box::new),
            min: usize_prop(min.as_ref()),
            max: usize_prop(max.as_ref()),
            add_label: add_label.clone(),
        },
        FieldType::Dict => FieldKind::Dict {
            value: value
                .as_deref()
                .and_then(|field| hub_field(field, "", false))
                .map(Box::new),
            key_pattern: key_pattern.clone(),
            key_label: key_label.clone(),
        },
        FieldType::Json => FieldKind::Json {
            schema: schema.clone(),
        },
        // Pages are inlined by `collect_sections` (§7 `page`); unknown types are skipped
        // (D-U14: parsers stay tolerant).
        FieldType::Page | FieldType::Unknown => return None,
    })
}

fn hub_field(field: &Field, section: &str, advanced: bool) -> Option<HubField> {
    Some(HubField {
        key: field.key.clone(),
        label: if field.label.is_empty() {
            field.key.clone()
        } else {
            field.label.clone()
        },
        description: field.description.clone().or_else(|| field.hint.clone()),
        hint: field.hint.clone(),
        badge: field.badge.clone(),
        section: section.to_owned(),
        kind: field_kind(field)?,
        scope: field.scope.unwrap_or_default(),
        default: field.default.clone(),
        advanced: advanced || field.advanced.unwrap_or(false),
        restart: field.restart,
        visible_if: field.visible_if.clone(),
        disabled_if: field.disabled_if.clone(),
        dynamic: field.dynamic.clone(),
    })
}

fn collect_sections(
    sections: &[SettingsSection],
    prefix: &str,
    inherited_advanced: bool,
    out: &mut Vec<HubField>,
) {
    for section in sections {
        let title = if section.title.is_empty() {
            section.id.clone()
        } else {
            section.title.clone()
        };
        let path = match (prefix.is_empty(), title.is_empty()) {
            (true, _) => title.clone(),
            (false, true) => prefix.to_owned(),
            (false, false) => format!("{prefix} › {title}"),
        };
        let advanced = inherited_advanced || section.advanced.unwrap_or(false);
        for field in &section.fields {
            if field.type_ == FieldType::Page {
                let nested = if path.is_empty() {
                    field.label.clone()
                } else {
                    format!("{path} › {}", field.label)
                };
                collect_sections(&field.props.sections, &nested, advanced, out);
                continue;
            }
            if let Some(field) = hub_field(field, &path, advanced) {
                out.push(field);
            }
        }
    }
}

/// Flattened fields of one descriptor (pages inlined with a `Page › Section` path).
#[must_use]
pub fn fields_of(descriptor: &SettingsNamespace) -> Vec<HubField> {
    let mut out = Vec::new();
    collect_sections(&descriptor.sections, "", false, &mut out);
    out
}

/// Namespaces of typed descriptors, in hub order (`order` desc, then title A→Z, §6.4).
#[must_use]
pub fn namespaces_of<'a>(
    descriptors: impl IntoIterator<Item = &'a SettingsNamespace>,
) -> Vec<HubNamespace> {
    let mut out: Vec<HubNamespace> = descriptors
        .into_iter()
        .map(HubNamespace::from_descriptor)
        .collect();
    out.sort_by(|a, b| {
        b.order
            .unwrap_or(0)
            .cmp(&a.order.unwrap_or(0))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

/// Namespaces of a `get_settings_schema` reply, in hub order (`order` desc, then title A→Z).
#[must_use]
pub fn namespaces(schema: &[JsonValue]) -> Vec<HubNamespace> {
    let descriptors: Vec<SettingsNamespace> = schema.iter().filter_map(descriptor).collect();
    namespaces_of(&descriptors)
}

/// Flattened fields of `namespace` (pages inlined with a `Page › Section` path).
#[must_use]
pub fn fields(schema: &[JsonValue], namespace: &str) -> Vec<HubField> {
    schema
        .iter()
        .filter_map(descriptor)
        .find(|descriptor| descriptor.namespace == namespace)
        .map_or_else(Vec::new, |descriptor| fields_of(&descriptor))
}

/// The value at dot path `key` of a namespace's values (nested objects; flat dotted keys
/// tolerated).
#[must_use]
pub fn value_at<'a>(values: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    if let Some(flat) = values.get(key) {
        return Some(flat);
    }
    key.split('.').try_fold(values, |v, part| v.get(part))
}

fn label_of(choices: &[HubChoice], value: &JsonValue) -> String {
    let raw = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned);
    choices
        .iter()
        .find(|choice| {
            choice
                .value
                .as_str()
                .map_or_else(|| choice.value.to_string(), str::to_owned)
                == raw
        })
        .map_or(raw, |choice| choice.label.clone())
}

impl HubField {
    /// Text shown for `value` (the field's default when unset).
    #[must_use]
    pub fn display(&self, value: Option<&JsonValue>) -> String {
        let value = value.or(self.default.as_ref());
        match (&self.kind, value) {
            (FieldKind::Action { command, .. }, _) => {
                format!("run /{}", command.trim_start_matches('/'))
            }
            (FieldKind::Secret, Some(JsonValue::String(s))) if !s.is_empty() => {
                SECRET_MASK.to_owned()
            }
            (FieldKind::Secret, _) => "(not set)".to_owned(),
            (_, None | Some(JsonValue::Null)) => "(default)".to_owned(),
            (FieldKind::Boolean, Some(JsonValue::Bool(b))) => {
                if *b { "on" } else { "off" }.to_owned()
            }
            (FieldKind::Enum { choices, .. }, Some(v)) => label_of(choices, v),
            (FieldKind::MultiSelect { choices, .. }, Some(JsonValue::Array(items))) => items
                .iter()
                .map(|item| label_of(choices, item))
                .collect::<Vec<_>>()
                .join(", "),
            (FieldKind::Order { .. }, Some(JsonValue::Array(items))) => items
                .iter()
                .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_owned))
                .collect::<Vec<_>>()
                .join(", "),
            (FieldKind::List { .. }, Some(JsonValue::Array(items))) => {
                format!("{} item(s)", items.len())
            }
            (FieldKind::Dict { .. }, Some(JsonValue::Object(values))) => {
                format!("{} key(s)", values.len())
            }
            (FieldKind::Json { .. }, Some(value)) => {
                serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
            }
            (
                FieldKind::Number {
                    unit: Some(unit), ..
                },
                Some(number),
            ) => {
                let raw = number
                    .as_str()
                    .map_or_else(|| number.to_string(), str::to_owned);
                format!("{raw} {unit}")
            }
            (_, Some(JsonValue::String(s))) => s.clone(),
            (_, Some(other)) => other.to_string(),
        }
    }

    /// Whether the value is edited as inline text (string, number, secret, model).
    #[must_use]
    pub const fn is_text(&self) -> bool {
        matches!(
            self.kind,
            FieldKind::Text { .. }
                | FieldKind::Number { .. }
                | FieldKind::Secret
                | FieldKind::Model { .. }
        )
    }

    /// Whether the row renders at all for the effective values (§7 `visibleIf`).
    #[must_use]
    pub fn is_visible(&self, values: &JsonValue) -> bool {
        self.visible_if
            .as_ref()
            .is_none_or(|cond| cond.eval(values))
    }

    /// Whether the row accepts edits for the effective values (§7 `disabledIf`).
    #[must_use]
    pub fn is_enabled(&self, values: &JsonValue) -> bool {
        self.disabled_if
            .as_ref()
            .is_none_or(|cond| !cond.eval(values))
    }

    /// The next value when toggling/cycling (`back` = previous option); `None` for kinds
    /// that are edited as text, run commands, live in a sub-page or are read-only.
    #[must_use]
    pub fn cycle(&self, current: Option<&JsonValue>, back: bool) -> Option<JsonValue> {
        let current = current.or(self.default.as_ref());
        match &self.kind {
            FieldKind::Boolean => Some(JsonValue::Bool(
                !current.and_then(JsonValue::as_bool).unwrap_or(false),
            )),
            FieldKind::Enum { choices, .. } if !choices.is_empty() => {
                let raw = current.and_then(JsonValue::as_str).unwrap_or_default();
                let len = choices.len();
                let pos = choices
                    .iter()
                    .position(|choice| choice.value.as_str() == Some(raw));
                let next = match (pos, back) {
                    (None, _) => 0,
                    (Some(p), false) => (p + 1) % len,
                    (Some(p), true) => (p + len - 1) % len,
                };
                Some(choices[next].value.clone())
            }
            _ => None,
        }
    }

    /// Parses edited text into the value to write (`Ok(None)` = unset).
    ///
    /// # Errors
    ///
    /// A message for invalid numbers, out-of-range values, malformed JSON and kinds that are
    /// edited elsewhere (sub-page list/dict, command actions, read-only order).
    pub fn parse(&self, input: &str) -> Result<Option<JsonValue>, String> {
        let trimmed = input.trim();
        match &self.kind {
            FieldKind::Number { min, max, .. } => {
                if trimmed.is_empty() {
                    return Ok(None);
                }
                let number: f64 = trimmed
                    .parse()
                    .map_err(|_| format!("{} must be a number", self.label))?;
                if min.is_some_and(|m| number < m) || max.is_some_and(|m| number > m) {
                    return Err(format!("{} is out of range", self.label));
                }
                let value = if number.fract() == 0.0 && number.abs() < 9.0e15 {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "integral and within the exact f64 range"
                    )]
                    JsonValue::from(number as i64)
                } else {
                    JsonValue::from(number)
                };
                Ok(Some(value))
            }
            FieldKind::Secret => Ok(Some(JsonValue::String(trimmed.to_owned()))),
            FieldKind::Json { .. } => {
                if trimmed.is_empty() {
                    return Ok(None);
                }
                serde_json::from_str(trimmed)
                    .map(Some)
                    .map_err(|error| format!("{}: {error}", self.label))
            }
            FieldKind::Text { .. } | FieldKind::Model { .. } => {
                if trimmed.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(JsonValue::String(input.to_owned())))
                }
            }
            FieldKind::Boolean
            | FieldKind::Enum { .. }
            | FieldKind::MultiSelect { .. }
            | FieldKind::Order { .. }
            | FieldKind::Action { .. }
            | FieldKind::List { .. }
            | FieldKind::Dict { .. } => Err(format!("{} is edited elsewhere", self.label)),
        }
    }

    /// Whether the value is a `list` / `dict` edited row by row in a sub-page (§7).
    #[must_use]
    pub const fn is_container(&self) -> bool {
        matches!(self.kind, FieldKind::List { .. } | FieldKind::Dict { .. })
    }

    /// Whether the value is edited as JSON in the external editor / code editor (§7 `json`).
    #[must_use]
    pub const fn is_json(&self) -> bool {
        matches!(self.kind, FieldKind::Json { .. })
    }

    /// The row template of a `list` (`item`) or `dict` (`value`) field.
    #[must_use]
    pub fn row_template(&self) -> Option<&Self> {
        match &self.kind {
            FieldKind::List { item, .. } => item.as_deref(),
            FieldKind::Dict { value, .. } => value.as_deref(),
            _ => None,
        }
    }

    /// Rows of a `list` value (array indices as keys) or a `dict` value (object keys), in
    /// stored order; empty for other kinds and unset values.
    #[must_use]
    pub fn rows(&self, value: Option<&JsonValue>) -> Vec<(String, JsonValue)> {
        match (self.kind(), value) {
            (FieldKind::List { .. }, Some(JsonValue::Array(items))) => items
                .iter()
                .enumerate()
                .map(|(index, value)| (index.to_string(), value.clone()))
                .collect(),
            (FieldKind::Dict { .. }, Some(JsonValue::Object(entries))) => entries
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The `list` array / `dict` object built from `rows` (dict keys keep their order).
    #[must_use]
    pub fn container(&self, rows: &[(String, JsonValue)]) -> JsonValue {
        match self.kind() {
            FieldKind::Dict { .. } => JsonValue::Object(
                rows.iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            _ => JsonValue::Array(rows.iter().map(|(_, value)| value.clone()).collect()),
        }
    }

    /// A new, empty row value: the `item` / `value` template default, else a per-kind zero.
    #[must_use]
    pub fn new_row(&self) -> JsonValue {
        if let Some(default) = self.row_template().and_then(|row| row.default.clone()) {
            return default;
        }
        match self.row_template().map(|row| &row.kind) {
            Some(FieldKind::Boolean) => JsonValue::Bool(false),
            Some(FieldKind::Number { .. }) => JsonValue::from(0),
            Some(
                FieldKind::MultiSelect { .. } | FieldKind::List { .. } | FieldKind::Order { .. },
            ) => JsonValue::Array(Vec::new()),
            Some(FieldKind::Dict { .. }) => JsonValue::Object(serde_json::Map::new()),
            Some(FieldKind::Enum { choices, .. }) => choices
                .first()
                .map_or(JsonValue::String(String::new()), |choice| {
                    choice.value.clone()
                }),
            Some(FieldKind::Json { .. } | FieldKind::Action { .. }) => JsonValue::Null,
            Some(FieldKind::Text { .. } | FieldKind::Secret | FieldKind::Model { .. }) | None => {
                JsonValue::String(String::new())
            }
        }
    }

    /// The editable field of one row: the `item` / `value` template, or a kind inferred from
    /// the stored row value (rows without a template are edited as text / JSON).
    #[must_use]
    pub fn row(&self, value: &JsonValue) -> Self {
        if let Some(template) = self.row_template() {
            return template.clone();
        }
        let kind = match value {
            JsonValue::Bool(_) => FieldKind::Boolean,
            JsonValue::Number(_) => FieldKind::Number {
                min: None,
                max: None,
                step: None,
                unit: None,
                integer: false,
            },
            JsonValue::Array(_) | JsonValue::Object(_) => FieldKind::Json { schema: None },
            _ => FieldKind::Text {
                multiline: false,
                placeholder: None,
                format: None,
            },
        };
        Self {
            key: self.key.clone(),
            label: self.label.clone(),
            description: None,
            hint: None,
            badge: None,
            section: String::new(),
            kind,
            scope: self.scope,
            default: None,
            advanced: false,
            restart: None,
            visible_if: None,
            disabled_if: None,
            dynamic: None,
        }
    }
}

impl HubField {
    /// The field kind, shared by the row helpers.
    const fn kind(&self) -> &FieldKind {
        &self.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Vec<JsonValue> {
        vec![
            json!({
                "namespace": "notify", "title": "Notify", "extensionId": "acme.notify",
                "icon": "dot", "order": 5, "description": "Where finished turns go",
                "sections": [{"id": "events", "title": "Events", "fields": [
                    {"key": "events.agentEnd", "type": "boolean", "label": "Agent finished", "default": true},
                    {"key": "platform", "type": "enum", "label": "Platform", "options": [{"value": "native", "label": "Native", "description": "Built in"}, "ntfy"]},
                    {"key": "ntfy.token", "type": "secret", "label": "ntfy token", "scope": "global"},
                    {"key": "limit", "type": "number", "label": "Limit", "min": 1, "max": 10, "unit": "s"},
                    {"key": "test", "type": "action", "label": "Send test", "command": "notify-test"},
                    {"key": "servers", "type": "list", "label": "Servers", "addLabel": "Add server",
                     "item": {"key": "url", "type": "string", "label": "URL"}},
                    {"key": "env", "type": "dict", "label": "Env", "keyLabel": "Name",
                     "value": {"type": "string", "label": "Value"}},
                    {"key": "raw", "type": "json", "label": "Raw", "badge": "unsafe"},
                    {"key": "advanced", "type": "page", "label": "Advanced", "sections": [
                        {"title": "Retry", "advanced": true, "fields": [
                            {"key": "retry.count", "type": "number", "label": "Retries", "advanced": true,
                             "visibleIf": {"key": "events.agentEnd", "eq": true},
                             "disabledIf": {"key": "platform", "in": ["ntfy"]},
                             "restart": "host", "hint": "0 disables"}
                        ]}
                    ]}
                ]}]
            }),
            json!({"namespace": "a", "sections": [], "icon": "not-an-icon"}),
            json!({"namespace": "z", "title": "Aaa", "order": 5, "sections": []}),
        ]
    }

    #[test]
    fn namespaces_and_fields_flatten_pages() {
        let schema = schema();
        let names = namespaces(&schema);
        let ids: Vec<&str> = names.iter().map(|n| n.namespace.as_str()).collect();
        assert_eq!(
            ids,
            ["z", "notify", "a"],
            "order desc, then title A→Z (§6.4)"
        );
        assert_eq!(names[1].title, "Notify");
        assert_eq!(names[1].icon, Some(Icon::Dot));
        assert_eq!(names[1].order, Some(5));
        assert_eq!(
            names[1].description.as_deref(),
            Some("Where finished turns go")
        );
        assert_eq!(names[2].icon, None, "an unknown icon name is dropped");
        let fields = fields(&schema, "notify");
        assert_eq!(fields.len(), 9);
        assert_eq!(fields[2].scope, FieldScope::Global);
        assert_eq!(fields[8].section, "Events › Advanced › Retry");
        assert!(fields[8].advanced, "an advanced section marks its fields");
        assert_eq!(
            fields[1].kind,
            FieldKind::Enum {
                choices: vec![
                    HubChoice {
                        value: json!("native"),
                        label: "Native".into(),
                        description: Some("Built in".into()),
                    },
                    HubChoice {
                        value: json!("ntfy"),
                        label: "ntfy".into(),
                        description: None,
                    },
                ],
                allow_custom: false,
                display: EnumDisplay::Select,
            },
            "v1 bare-string options still load"
        );
        assert!(matches!(
            &fields[5].kind,
            FieldKind::List { item: Some(item), add_label: Some(label), .. }
                if item.label == "URL" && label == "Add server"
        ));
        assert!(matches!(
            &fields[6].kind,
            FieldKind::Dict { value: Some(value), key_label: Some(label), .. }
                if value.label == "Value" && label == "Name"
        ));
        assert_eq!(fields[7].badge.as_deref(), Some("unsafe"));
        assert_eq!(fields[8].restart, Some(Restart::Host));
        assert_eq!(fields[8].hint.as_deref(), Some("0 disables"));
        assert!(fields[8].visible_if.is_some() && fields[8].disabled_if.is_some());
    }

    #[test]
    fn display_cycle_and_parse() {
        let schema = schema();
        let fields = fields(&schema, "notify");
        let values = json!({
            "events": {"agentEnd": false},
            "ntfy": {"token": SECRET_MASK},
            "servers": [{"url": "a"}],
            "env": {"A": "1", "B": "2"},
            "raw": {"x": 1},
        });
        assert_eq!(
            fields[0].display(value_at(&values, "events.agentEnd")),
            "off"
        );
        assert_eq!(fields[0].display(None), "on");
        assert_eq!(
            fields[2].display(value_at(&values, "ntfy.token")),
            SECRET_MASK
        );
        assert_eq!(fields[1].display(None), "(default)");
        assert_eq!(fields[1].display(Some(&json!("ntfy"))), "ntfy");
        assert_eq!(fields[3].display(Some(&json!(5))), "5 s");
        assert_eq!(fields[4].display(None), "run /notify-test");
        assert_eq!(fields[5].display(value_at(&values, "servers")), "1 item(s)");
        assert_eq!(fields[6].display(value_at(&values, "env")), "2 key(s)");
        assert_eq!(fields[7].display(value_at(&values, "raw")), r#"{"x":1}"#);
        assert_eq!(fields[7].display(None), "(default)");
        assert_eq!(
            fields[0].cycle(Some(&json!(false)), false),
            Some(json!(true))
        );
        assert_eq!(fields[1].cycle(None, false), Some(json!("native")));
        assert_eq!(
            fields[1].cycle(Some(&json!("native")), true),
            Some(json!("ntfy"))
        );
        assert_eq!(
            fields[1].cycle(Some(&json!("custom")), false),
            Some(json!("native"))
        );
        assert_eq!(fields[3].parse("5"), Ok(Some(json!(5))));
        assert!(fields[3].parse("50").is_err());
        assert!(fields[3].parse("x").is_err());
        assert_eq!(fields[3].parse(""), Ok(None));
        assert_eq!(fields[7].parse(r#"{"y":2}"#), Ok(Some(json!({"y": 2}))));
        assert!(fields[7].parse("{oops").is_err());
        assert!(
            fields[5].parse("x").is_err(),
            "lists are edited in a sub-page"
        );
        assert!(fields[3].is_text() && !fields[0].is_text());
        assert!(!fields[5].is_text() && !fields[7].is_text());
    }

    fn descriptor() -> SettingsNamespace {
        let mut servers = Field::new("servers", FieldType::List, "Servers");
        servers.props.add_label = Some("Add".into());
        servers.props.item = Some(Box::new(Field::string("url", "URL")));
        let mut namespace = SettingsNamespace {
            namespace: "typed".into(),
            title: "Typed".into(),
            extension_id: "ext".into(),
            icon: Some(Icon::Gear),
            order: Some(900),
            description: Some("typed descriptor".into()),
            defaults: Some(json!({"limit": 5})),
            sections: vec![SettingsSection {
                id: "main".into(),
                title: "Main".into(),
                description: None,
                advanced: None,
                fields: vec![
                    Field::boolean("on", "On"),
                    Field::number("limit", "Limit"),
                    servers,
                ],
            }],
        };
        namespace.sections[0].fields[1].visible_if = Some(Cond {
            key: "on".into(),
            eq: Some(json!(true)),
            ..Cond::default()
        });
        namespace
    }

    #[test]
    fn typed_descriptors_drive_the_hub() {
        let descriptor = descriptor();
        let names = namespaces_of([&descriptor]);
        assert_eq!(
            names,
            vec![HubNamespace {
                namespace: "typed".into(),
                title: "Typed".into(),
                extension_id: "ext".into(),
                icon: Some(Icon::Gear),
                order: Some(900),
                description: Some("typed descriptor".into()),
                defaults: Some(json!({"limit": 5})),
            }]
        );
        let rows = fields_of(&descriptor);
        let labels: Vec<&str> = rows.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(labels, ["On", "Limit", "Servers"]);
        assert!(
            matches!(&rows[2].kind, FieldKind::List { item: Some(item), .. } if item.key == "url")
        );
        assert_eq!(
            rows[1].visible_if.as_ref().map(|c| c.key.as_str()),
            Some("on")
        );

        // The typed path and the `get_settings_schema` JSON path agree.
        let json = vec![serde_json::to_value(&descriptor).unwrap_or_default()];
        assert_eq!(namespaces(&json), names);
        assert_eq!(fields(&json, "typed"), rows);
    }

    #[test]
    fn conditions_gate_rows() {
        let values = json!({"mode": "auto", "count": 0, "names": ["a"], "empty": []});
        let holds = |cond: JsonValue| {
            serde_json::from_value::<Cond>(cond)
                .unwrap_or_default()
                .eval(&values)
        };
        assert!(holds(json!({"key": "mode", "eq": "auto"})));
        assert!(!holds(json!({"key": "mode", "eq": "manual"})));
        assert!(holds(json!({"key": "mode", "ne": "manual"})));
        assert!(!holds(json!({"key": "mode", "ne": "auto"})));
        assert!(holds(json!({"key": "mode", "in": ["auto", "manual"]})));
        assert!(!holds(json!({"key": "mode", "in": ["manual"]})));
        assert!(!holds(json!({"key": "count", "truthy": true})));
        assert!(holds(json!({"key": "names", "truthy": true})));
        assert!(!holds(json!({"key": "empty", "truthy": true})));
        assert!(holds(json!({"key": "missing", "eq": null})));
        assert!(!holds(json!({"key": "missing", "truthy": true})));
        assert!(holds(json!({"key": "mode"})));

        let field = HubField {
            key: "limit".into(),
            label: "Limit".into(),
            description: None,
            hint: None,
            badge: None,
            section: String::new(),
            kind: FieldKind::Number {
                min: None,
                max: None,
                step: None,
                unit: None,
                integer: false,
            },
            scope: FieldScope::Both,
            default: None,
            advanced: false,
            restart: None,
            visible_if: Some(
                serde_json::from_value(json!({"key": "mode", "eq": "auto"})).unwrap_or_default(),
            ),
            disabled_if: Some(
                serde_json::from_value(json!({"key": "count", "truthy": false}))
                    .unwrap_or_default(),
            ),
            dynamic: None,
        };
        assert!(field.is_visible(&values));
        assert!(!field.is_enabled(&values));
        assert!(!field.is_visible(&json!({"mode": "manual"})));
    }

    #[test]
    fn registry_settings_entries_become_namespaces() {
        let raw: JsonValue = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../oino-ui/fixtures/registry/snapshot.json"
        )))
        .unwrap_or_default();
        let snapshot: oino_ui::slot::UiSnapshot =
            serde_json::from_value(raw).expect("snapshot fixture parses");
        let names: Vec<HubNamespace> = snapshot
            .entries
            .iter()
            .filter_map(HubNamespace::from_entry)
            .collect();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].namespace, "ext");
        assert_eq!(names[0].title, "Ext");
        assert_eq!(names[0].extension_id, "ext");
        let UiView::Settings { namespace } = &snapshot.entries[2].view else {
            panic!("the fixture's third entry is a settings section");
        };
        assert_eq!(fields_of(namespace).len(), 0);
    }

    #[test]
    fn list_and_dict_rows_edit_in_a_sub_page() {
        let schema = schema();
        let fields = fields(&schema, "notify");
        let servers = &fields[5];
        assert!(servers.is_container() && !servers.is_json());
        assert!(fields[7].is_json() && !fields[7].is_container());
        let rows = servers.rows(Some(&json!([{"url": "a"}, {"url": "b"}])));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0, "1", "list rows are indexed");
        assert_eq!(
            servers.container(&rows),
            json!([{"url": "a"}, {"url": "b"}])
        );
        assert_eq!(servers.new_row(), json!(""), "item template default");
        let row = servers.row(&json!("a"));
        assert_eq!(row.label, "URL");
        assert_eq!(row.parse("https://x"), Ok(Some(json!("https://x"))));

        let env = &fields[6];
        let rows = env.rows(Some(&json!({"B": "2", "A": "1"})));
        assert_eq!(
            rows,
            [("B".to_owned(), json!("2")), ("A".to_owned(), json!("1"))],
            "dict rows keep the stored key order"
        );
        assert_eq!(env.container(&rows), json!({"B": "2", "A": "1"}));
        assert_eq!(env.rows(None), Vec::new());

        // Rows without a template infer an editor from the stored value.
        let mut loose = HubField {
            key: "loose".into(),
            label: "Loose".into(),
            description: None,
            hint: None,
            badge: None,
            section: String::new(),
            kind: FieldKind::List {
                item: None,
                min: None,
                max: None,
                add_label: None,
            },
            scope: FieldScope::Both,
            default: None,
            advanced: false,
            restart: None,
            visible_if: None,
            disabled_if: None,
            dynamic: None,
        };
        assert!(matches!(loose.row(&json!(true)).kind, FieldKind::Boolean));
        assert!(matches!(
            loose.row(&json!(3)).kind,
            FieldKind::Number { .. }
        ));
        assert!(matches!(
            loose.row(&json!({"a": 1})).kind,
            FieldKind::Json { .. }
        ));
        assert!(matches!(
            loose.row(&json!("x")).kind,
            FieldKind::Text { .. }
        ));
        loose.kind = FieldKind::Dict {
            value: None,
            key_pattern: None,
            key_label: None,
        };
        assert_eq!(loose.new_row(), json!(""));
    }
}
