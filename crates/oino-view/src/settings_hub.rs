//! Extension settings hub model (ui-protocol §6.6): namespaces from `get_settings_schema`,
//! flattened fields per namespace, values from `get_extension_settings`, and the edits a
//! frontend sends back (`set_extension_setting` / `unset_extension_setting`).
//!
//! Pure data: the TUI and the desktop app render it natively.

use oino_ai::JsonValue;

/// Value shown for a set secret (`oino_core::extensions::settings::SECRET_MASK`).
pub const SECRET_MASK: &str = "••••••";

/// A namespace as listed in the hub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubNamespace {
    /// Namespace id.
    pub namespace: String,
    /// Title (defaults to the namespace).
    pub title: String,
    /// Owning extension.
    pub extension_id: String,
}

/// Field type (ui-protocol §6.6).
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    /// On/off.
    Boolean,
    /// One of `options` (`value`, `label`).
    Enum(Vec<(String, String)>),
    /// Several of `options`.
    MultiSelect(Vec<(String, String)>),
    /// Free text.
    Text {
        /// Multi-line editor.
        multiline: bool,
    },
    /// Number with optional bounds.
    Number {
        /// Minimum.
        min: Option<f64>,
        /// Maximum.
        max: Option<f64>,
    },
    /// Secret (masked).
    Secret,
    /// Reorderable list (shown read-only).
    Order,
    /// `provider/id` model.
    Model,
    /// Runs `command` (`/name`).
    Action {
        /// Command to run.
        command: String,
    },
}

/// Hub scope of a field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldScope {
    /// Global and project.
    #[default]
    Both,
    /// Global only.
    Global,
    /// Project only.
    Project,
}

/// One editable row.
#[derive(Clone, Debug, PartialEq)]
pub struct HubField {
    /// Dot-path key.
    pub key: String,
    /// Label.
    pub label: String,
    /// Description or hint.
    pub description: Option<String>,
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
}

fn text(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn options(field: &JsonValue) -> Vec<(String, String)> {
    field
        .get("options")
        .and_then(JsonValue::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|o| match o {
                    JsonValue::String(s) => Some((s.clone(), s.clone())),
                    JsonValue::Object(_) => {
                        let value = o.get("value").map(|v| match v {
                            JsonValue::String(s) => s.clone(),
                            other => other.to_string(),
                        })?;
                        let label = text(o, "label").unwrap_or_else(|| value.clone());
                        Some((value, label))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn field_kind(field: &JsonValue) -> Option<FieldKind> {
    Some(match field.get("type").and_then(JsonValue::as_str)? {
        "boolean" => FieldKind::Boolean,
        "enum" => FieldKind::Enum(options(field)),
        "multiselect" => FieldKind::MultiSelect(options(field)),
        "string" => FieldKind::Text {
            multiline: field.get("multiline") == Some(&JsonValue::Bool(true)),
        },
        "number" => FieldKind::Number {
            min: field.get("min").and_then(JsonValue::as_f64),
            max: field.get("max").and_then(JsonValue::as_f64),
        },
        "secret" => FieldKind::Secret,
        "order" => FieldKind::Order,
        "model" => FieldKind::Model,
        "action" => FieldKind::Action {
            command: text(field, "command")?,
        },
        _ => return None,
    })
}

fn collect(sections: &JsonValue, prefix: &str, out: &mut Vec<HubField>) {
    let Some(sections) = sections.as_array() else {
        return;
    };
    for section in sections {
        let title = text(section, "title")
            .or_else(|| text(section, "id"))
            .unwrap_or_default();
        let path = match (prefix.is_empty(), title.is_empty()) {
            (true, _) => title.clone(),
            (false, true) => prefix.to_owned(),
            (false, false) => format!("{prefix} › {title}"),
        };
        for field in section
            .get("fields")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            if field.get("type").and_then(JsonValue::as_str) == Some("page") {
                let label = text(field, "label").unwrap_or_default();
                let nested = if path.is_empty() {
                    label
                } else {
                    format!("{path} › {label}")
                };
                if let Some(inner) = field.get("sections") {
                    collect(inner, &nested, out);
                }
                continue;
            }
            let (Some(key), Some(kind)) = (text(field, "key"), field_kind(field)) else {
                continue;
            };
            out.push(HubField {
                label: text(field, "label").unwrap_or_else(|| key.clone()),
                description: text(field, "description").or_else(|| text(field, "hint")),
                section: path.clone(),
                kind,
                scope: match field.get("scope").and_then(JsonValue::as_str) {
                    Some("global") => FieldScope::Global,
                    Some("project") => FieldScope::Project,
                    _ => FieldScope::Both,
                },
                default: field.get("default").cloned(),
                advanced: field.get("advanced") == Some(&JsonValue::Bool(true)),
                key,
            });
        }
    }
}

/// Namespaces of a `get_settings_schema` reply, sorted by title.
#[must_use]
pub fn namespaces(schema: &[JsonValue]) -> Vec<HubNamespace> {
    let mut out: Vec<HubNamespace> = schema
        .iter()
        .filter_map(|n| {
            let namespace = text(n, "namespace")?;
            Some(HubNamespace {
                title: text(n, "title").unwrap_or_else(|| namespace.clone()),
                extension_id: text(n, "extensionId").unwrap_or_default(),
                namespace,
            })
        })
        .collect();
    out.sort_by_key(|n| n.title.to_lowercase());
    out
}

/// Flattened fields of `namespace` (pages inlined with a `Page › Section` path).
#[must_use]
pub fn fields(schema: &[JsonValue], namespace: &str) -> Vec<HubField> {
    let mut out = Vec::new();
    if let Some(ns) = schema
        .iter()
        .find(|n| n.get("namespace").and_then(JsonValue::as_str) == Some(namespace))
        && let Some(sections) = ns.get("sections")
    {
        collect(sections, "", &mut out);
    }
    out
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

impl HubField {
    /// Text shown for `value` (the field's default when unset).
    #[must_use]
    pub fn display(&self, value: Option<&JsonValue>) -> String {
        let value = value.or(self.default.as_ref());
        match (&self.kind, value) {
            (FieldKind::Action { command }, _) => {
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
            (FieldKind::Enum(options), Some(v)) => {
                let raw = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                options
                    .iter()
                    .find(|(value, _)| *value == raw)
                    .map_or(raw, |(_, label)| label.clone())
            }
            (FieldKind::MultiSelect(_) | FieldKind::Order, Some(JsonValue::Array(items))) => items
                .iter()
                .map(|i| i.as_str().map_or_else(|| i.to_string(), str::to_owned))
                .collect::<Vec<_>>()
                .join(", "),
            (_, Some(JsonValue::String(s))) => s.clone(),
            (_, Some(other)) => other.to_string(),
        }
    }

    /// Whether the value is edited as text (string, number, secret, model).
    #[must_use]
    pub const fn is_text(&self) -> bool {
        matches!(
            self.kind,
            FieldKind::Text { .. }
                | FieldKind::Number { .. }
                | FieldKind::Secret
                | FieldKind::Model
        )
    }

    /// The next value when toggling/cycling (`back` = previous option); `None` for kinds
    /// that are edited as text, run commands or are read-only.
    #[must_use]
    pub fn cycle(&self, current: Option<&JsonValue>, back: bool) -> Option<JsonValue> {
        let current = current.or(self.default.as_ref());
        match &self.kind {
            FieldKind::Boolean => Some(JsonValue::Bool(
                !current.and_then(JsonValue::as_bool).unwrap_or(false),
            )),
            FieldKind::Enum(options) if !options.is_empty() => {
                let raw = current.and_then(JsonValue::as_str).unwrap_or_default();
                let len = options.len();
                let pos = options.iter().position(|(v, _)| v == raw);
                let next = match (pos, back) {
                    (None, _) => 0,
                    (Some(p), false) => (p + 1) % len,
                    (Some(p), true) => (p + len - 1) % len,
                };
                Some(JsonValue::String(options[next].0.clone()))
            }
            _ => None,
        }
    }

    /// Parses edited text into the value to write (`Ok(None)` = unset).
    ///
    /// # Errors
    ///
    /// A message for invalid numbers or out-of-range values.
    pub fn parse(&self, input: &str) -> Result<Option<JsonValue>, String> {
        let trimmed = input.trim();
        match &self.kind {
            FieldKind::Number { min, max } => {
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
            _ if trimmed.is_empty() => Ok(None),
            _ => Ok(Some(JsonValue::String(input.to_owned()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Vec<JsonValue> {
        vec![
            json!({
                "namespace": "notify", "title": "Notify", "extensionId": "unipi.notify",
                "sections": [{"id": "events", "title": "Events", "fields": [
                    {"key": "events.agentEnd", "type": "boolean", "label": "Agent finished", "default": true},
                    {"key": "platform", "type": "enum", "label": "Platform", "options": [{"value": "native", "label": "Native"}, "ntfy"]},
                    {"key": "ntfy.token", "type": "secret", "label": "ntfy token", "scope": "global"},
                    {"key": "limit", "type": "number", "label": "Limit", "min": 1, "max": 10},
                    {"key": "test", "type": "action", "label": "Send test", "command": "notify-test"},
                    {"key": "advanced", "type": "page", "label": "Advanced", "sections": [
                        {"title": "Retry", "fields": [{"key": "retry.count", "type": "number", "label": "Retries", "advanced": true}]}
                    ]}
                ]}]
            }),
            json!({"namespace": "a", "sections": []}),
        ]
    }

    #[test]
    fn namespaces_and_fields_flatten_pages() {
        let schema = schema();
        let names = namespaces(&schema);
        assert_eq!(names[0].namespace, "a");
        assert_eq!(names[1].title, "Notify");
        let fields = fields(&schema, "notify");
        assert_eq!(fields.len(), 6);
        assert_eq!(fields[2].scope, FieldScope::Global);
        assert_eq!(fields[5].section, "Events › Advanced › Retry");
        assert!(fields[5].advanced);
        assert_eq!(
            fields[1].kind,
            FieldKind::Enum(vec![
                ("native".into(), "Native".into()),
                ("ntfy".into(), "ntfy".into())
            ])
        );
    }

    #[test]
    fn display_cycle_and_parse() {
        let schema = schema();
        let fields = fields(&schema, "notify");
        let values = json!({"events": {"agentEnd": false}, "ntfy.token": SECRET_MASK});
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
        assert_eq!(fields[4].display(None), "run /notify-test");
        assert_eq!(
            fields[0].cycle(Some(&json!(false)), false),
            Some(json!(true))
        );
        assert_eq!(fields[1].cycle(None, false), Some(json!("native")));
        assert_eq!(
            fields[1].cycle(Some(&json!("native")), true),
            Some(json!("ntfy"))
        );
        assert_eq!(fields[3].parse("5"), Ok(Some(json!(5))));
        assert!(fields[3].parse("50").is_err());
        assert!(fields[3].parse("x").is_err());
        assert_eq!(fields[3].parse(""), Ok(None));
        assert!(fields[3].is_text() && !fields[0].is_text());
    }
}
