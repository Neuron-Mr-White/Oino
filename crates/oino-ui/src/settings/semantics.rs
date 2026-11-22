//! OSS-2 semantics (§7.1–7.4): layered resolve, simplify, validation and conditions.
//!
//! Schemastery model, implemented once here and used by core (effective values, "writing a
//! default unsets the key"), the TUI hub (`visibleIf` / `disabledIf`, `restart`) and the app
//! form engine (TS port). `resolve` **never fails**: an invalid stored value falls back to the
//! next layer down and produces a [`Diagnostic`].
//!
//! The crate stays serde-only (§11), so `pattern` / `keyPattern` regexes are not evaluated
//! here: [`check_value`] checks types, enum membership, lengths and ranges, and a host that
//! has a regex engine adds the pattern test through [`resolve_with`].

use serde_json::{Map as JsonMap, Value as Json};

use crate::settings::{Cond, Field, FieldScope, FieldType, SettingsNamespace};

/// A stored value that was ignored by [`resolve`] in favour of the layer below.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Namespace the value belongs to.
    pub namespace: String,
    /// Dot-path key (`compaction.enabled`).
    pub key: String,
    /// Why the value was rejected.
    pub message: String,
}

/// The layers of one namespace, as stored (`{}` when absent).
#[derive(Clone, Copy, Debug)]
pub struct Layers<'a> {
    /// `SettingsNamespace::defaults` (`{}` when absent).
    pub defaults: &'a Json,
    /// `extensionSettings.<namespace>` of the global file (nested).
    pub global: &'a Json,
    /// The same of the project file; `{}` when the project is untrusted.
    pub project: &'a Json,
}

impl Default for Layers<'_> {
    fn default() -> Self {
        Self {
            defaults: &Json::Null,
            global: &Json::Null,
            project: &Json::Null,
        }
    }
}

/// Effective values of a namespace plus the diagnostics of the resolve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    /// Effective object (secrets not filled in — that is the host's job).
    pub values: Json,
    /// One entry per rejected stored value, in field order.
    pub diagnostics: Vec<Diagnostic>,
}

/// One write that reproduces an effective value from the layer below it.
#[derive(Clone, Debug, PartialEq)]
pub struct Write {
    /// Dot-path key.
    pub key: String,
    /// The value to store; `None` unsets the key (it equals the layer below).
    pub value: Option<Json>,
}

/// Which file layer a write targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// `~/.oino/settings.json` (the layer below is the defaults).
    Global,
    /// `<project>/.oino/settings.json` (the layer below is defaults ⊕ global).
    Project,
}

/// Resolve `namespace` over `layers` with [`check_value`].
#[must_use]
pub fn resolve(namespace: &SettingsNamespace, layers: Layers<'_>) -> Resolved {
    resolve_with(namespace, layers, &check_value)
}

/// Resolve `namespace` over `layers`, checking values with `check` (a host with a regex engine
/// passes a checker that also tests `pattern` / `keyPattern`).
#[must_use]
pub fn resolve_with(
    namespace: &SettingsNamespace,
    layers: Layers<'_>,
    check: &dyn Fn(&Field, &Json) -> Result<(), String>,
) -> Resolved {
    let defaults = object_or_empty(layers.defaults);
    let global = object_or_empty(layers.global);
    let project = object_or_empty(layers.project);
    let mut values = merge(&merge(&defaults, &global), &project);
    let mut diagnostics = Vec::new();
    for field in namespace.fields() {
        if !field.stores_value() {
            continue;
        }
        let mut effective = None;
        let stored = stored_layers(field.scope(), &global, &project);
        let stored_count = stored.len();
        for (index, layer) in stored.iter().copied().chain([&defaults]).enumerate() {
            if let Some(value) = lookup_map(layer, &field.key) {
                match check(field, value) {
                    Ok(()) => {
                        effective = Some(value.clone());
                        break;
                    }
                    // Schema defaults are not stored values: no diagnostic for them.
                    Err(message) if index < stored_count => diagnostics.push(Diagnostic {
                        namespace: namespace.namespace.clone(),
                        key: field.key.clone(),
                        message,
                    }),
                    Err(_) => {}
                }
            }
        }
        if effective.is_none()
            && let Some(default) = &field.default
            && check(field, default).is_ok()
        {
            effective = Some(default.clone());
        }
        match effective {
            Some(value) => set_path(&mut values, &field.key, value),
            // Invalid everywhere and no usable default: the key is unset.
            None => remove_path(&mut values, &field.key),
        }
    }
    Resolved {
        values: Json::Object(values),
        diagnostics,
    }
}

/// The writes that turn an effective object into stored layers (`None` = unset, §7.2).
#[must_use]
pub fn simplify(
    namespace: &SettingsNamespace,
    layer: Layer,
    layers: Layers<'_>,
    effective: &Json,
) -> Vec<Write> {
    let defaults = object_or_empty(layers.defaults);
    let global = object_or_empty(layers.global);
    let below = match layer {
        Layer::Global => defaults,
        Layer::Project => merge(&defaults, &global),
    };
    let mut writes = Vec::new();
    for field in namespace.fields() {
        if !field.stores_value() || !writes_scope(field.scope(), layer) {
            continue;
        }
        let current = lookup(effective, &field.key).cloned();
        let base = lookup_map(&below, &field.key)
            .cloned()
            .or_else(|| field.default.clone());
        match current {
            Some(current) if base.as_ref() != Some(&current) => writes.push(Write {
                key: field.key.clone(),
                value: Some(current),
            }),
            Some(_) => writes.push(Write {
                key: field.key.clone(),
                value: None,
            }),
            None => {}
        }
    }
    writes
}

/// Simplify one written value (§7.2): `None` means "unset" because the value equals the layer
/// below the write.
#[must_use]
pub fn simplify_key(field: &Field, below: Option<&Json>, value: &Json) -> Option<Json> {
    let base = below.or(field.default.as_ref());
    if base == Some(value) {
        None
    } else {
        Some(value.clone())
    }
}

/// Whether `layer` may store a field of `scope`.
const fn writes_scope(scope: FieldScope, layer: Layer) -> bool {
    !matches!(
        (scope, layer),
        (FieldScope::Global, Layer::Project) | (FieldScope::Project, Layer::Global)
    )
}

/// Stored layers that contribute to a field of `scope`, highest precedence first.
fn stored_layers<'a>(
    scope: FieldScope,
    global: &'a JsonMap<String, Json>,
    project: &'a JsonMap<String, Json>,
) -> Vec<&'a JsonMap<String, Json>> {
    match scope {
        FieldScope::Both => vec![project, global],
        FieldScope::Global => vec![global],
        FieldScope::Project => vec![project],
    }
}

/// Check `value` against `field` (§7 `Field types`).
///
/// # Errors
///
/// A human-readable reason (`"expected a number"`, `"expected one of a, b"`, …).
pub fn check_value(field: &Field, value: &Json) -> Result<(), String> {
    match field.type_ {
        FieldType::Boolean if !value.is_boolean() => Err("expected a boolean".into()),
        FieldType::Number => check_number(field, value),
        FieldType::String => {
            let Some(text) = value.as_str() else {
                return Err("expected a string".into());
            };
            check_length(field, text)
        }
        FieldType::Secret | FieldType::Model if !value.is_string() => {
            Err("expected a string".into())
        }
        FieldType::Enum => check_enum(field, value),
        FieldType::Multiselect => check_multiselect(field, value),
        FieldType::Order => check_order(field, value),
        FieldType::List => check_list(field, value),
        FieldType::Dict => check_dict(field, value),
        FieldType::Boolean
        | FieldType::Secret
        | FieldType::Model
        | FieldType::Json
        | FieldType::Unknown
        | FieldType::Action
        | FieldType::Page => Ok(()),
    }
}

fn check_number(field: &Field, value: &Json) -> Result<(), String> {
    let Some(number) = value.as_f64() else {
        return Err("expected a number".into());
    };
    if field.props.integer == Some(true) && number.fract() != 0.0 {
        return Err("expected an integer".into());
    }
    if let Some(min) = field
        .props
        .min
        .as_ref()
        .and_then(serde_json::Number::as_f64)
        && number < min
    {
        return Err(format!("must be at least {min}"));
    }
    if let Some(max) = field
        .props
        .max
        .as_ref()
        .and_then(serde_json::Number::as_f64)
        && number > max
    {
        return Err(format!("must be at most {max}"));
    }
    Ok(())
}

fn check_enum(field: &Field, value: &Json) -> Result<(), String> {
    if !value.is_string() {
        return Err("expected a string".into());
    }
    if field.props.allow_custom != Some(true)
        && !field
            .props
            .options
            .iter()
            .any(|option| option.value == *value)
    {
        return Err(one_of(field));
    }
    Ok(())
}

fn check_multiselect(field: &Field, value: &Json) -> Result<(), String> {
    let Some(items) = value.as_array() else {
        return Err("expected a list of strings".into());
    };
    if items.iter().any(|item| !item.is_string()) {
        return Err("expected a list of strings".into());
    }
    if field.props.allow_custom != Some(true)
        && items.iter().any(|item| {
            !field
                .props
                .options
                .iter()
                .any(|option| option.value == *item)
        })
    {
        return Err(one_of(field));
    }
    check_len(field, items.len())
}

fn check_order(field: &Field, value: &Json) -> Result<(), String> {
    let Some(items) = value.as_array() else {
        return Err("expected a list of strings".into());
    };
    if items.iter().any(|item| !item.is_string()) {
        return Err("expected a list of strings".into());
    }
    if !field.props.items.is_empty()
        && let Some(unknown) = items.iter().find(|item| {
            !field
                .props
                .items
                .iter()
                .any(|known| item.as_str() == Some(known))
        })
    {
        return Err(format!(
            "{unknown} is not one of {}",
            field.props.items.join(", ")
        ));
    }
    Ok(())
}

fn check_list(field: &Field, value: &Json) -> Result<(), String> {
    let Some(items) = value.as_array() else {
        return Err("expected a list".into());
    };
    check_len(field, items.len())?;
    if let Some(item) = &field.props.item {
        for entry in items {
            check_value(item, entry)?;
        }
    }
    Ok(())
}

fn check_dict(field: &Field, value: &Json) -> Result<(), String> {
    let Some(entries) = value.as_object() else {
        return Err("expected an object".into());
    };
    if let Some(entry) = &field.props.value {
        for value in entries.values() {
            check_value(entry, value)?;
        }
    }
    Ok(())
}

fn check_length(field: &Field, text: &str) -> Result<(), String> {
    let len = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    if let Some(min) = field.props.min_length
        && len < min
    {
        return Err(format!("must be at least {min} characters"));
    }
    if let Some(max) = field.props.max_length
        && len > max
    {
        return Err(format!("must be at most {max} characters"));
    }
    Ok(())
}

fn check_len(field: &Field, len: usize) -> Result<(), String> {
    if let Some(min) = field
        .props
        .min
        .as_ref()
        .and_then(serde_json::Number::as_u64)
        && len < usize::try_from(min).unwrap_or(usize::MAX)
    {
        return Err(format!("must have at least {min} items"));
    }
    if let Some(max) = field
        .props
        .max
        .as_ref()
        .and_then(serde_json::Number::as_u64)
        && len > usize::try_from(max).unwrap_or(usize::MAX)
    {
        return Err(format!("must have at most {max} items"));
    }
    Ok(())
}

fn one_of(field: &Field) -> String {
    let values: Vec<String> = field
        .props
        .options
        .iter()
        .map(|o| {
            o.value
                .as_str()
                .map_or_else(|| o.value.to_string(), str::to_owned)
        })
        .collect();
    format!("expected one of {}", values.join(", "))
}

impl SettingsNamespace {
    /// Flattened fields in render order (§7 `Section.fields`, `page` drill-ins inlined; `page`
    /// fields themselves are not yielded).
    #[must_use]
    pub fn fields(&self) -> Vec<&Field> {
        let mut out = Vec::new();
        collect_fields(&self.sections, &mut out);
        out
    }

    /// One field by dot-path key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields().into_iter().find(|field| field.key == key)
    }
}

fn collect_fields<'a>(sections: &'a [crate::settings::SettingsSection], out: &mut Vec<&'a Field>) {
    for section in sections {
        for field in &section.fields {
            if field.type_ == FieldType::Page {
                collect_fields(&field.props.sections, out);
                continue;
            }
            out.push(field);
        }
    }
}

impl Cond {
    /// Whether the condition holds on the effective values (§7.4).
    #[must_use]
    pub fn eval(&self, values: &Json) -> bool {
        let value = lookup(values, &self.key);
        if let Some(eq) = &self.eq
            && value != Some(eq)
        {
            return false;
        }
        if let Some(ne) = &self.ne
            && value == Some(ne)
        {
            return false;
        }
        if let Some(one_of) = &self.in_
            && !value.is_some_and(|value| one_of.contains(value))
        {
            return false;
        }
        if let Some(want) = self.truthy
            && value.is_some_and(is_truthy) != want
        {
            return false;
        }
        true
    }
}

/// Schemastery truthiness: `true`, non-zero, non-empty string / array / object.
fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Json::String(text) => !text.is_empty(),
        Json::Array(items) => !items.is_empty(),
        Json::Object(entries) => !entries.is_empty(),
    }
}

impl Field {
    /// Whether the row is rendered for `values` (§7.4 `visibleIf`).
    #[must_use]
    pub fn visible(&self, values: &Json) -> bool {
        self.visible_if
            .as_ref()
            .is_none_or(|cond| cond.eval(values))
    }

    /// Whether the row is editable for `values` (§7.4 `disabledIf`); invisible rows are never
    /// enabled.
    #[must_use]
    pub fn enabled(&self, values: &Json) -> bool {
        self.visible(values)
            && self
                .disabled_if
                .as_ref()
                .is_none_or(|cond| !cond.eval(values))
    }
}

/// Look up a dot path, tolerating v1 flat dotted keys (`{"a.b": 1}`).
pub(crate) fn lookup<'a>(object: &'a Json, key: &str) -> Option<&'a Json> {
    if let Some(flat) = object.get(key) {
        return Some(flat);
    }
    let mut segments = key.split('.');
    let mut cursor = object.get(segments.next()?)?;
    for segment in segments {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

fn lookup_map<'a>(object: &'a JsonMap<String, Json>, key: &str) -> Option<&'a Json> {
    if let Some(flat) = object.get(key) {
        return Some(flat);
    }
    let mut segments = key.split('.');
    let mut cursor = object.get(segments.next()?)?;
    for segment in segments {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

fn object_or_empty(value: &Json) -> JsonMap<String, Json> {
    value.as_object().cloned().unwrap_or_default()
}

/// Objects merge recursively, everything else replaces (`over` wins).
fn merge(base: &JsonMap<String, Json>, over: &JsonMap<String, Json>) -> JsonMap<String, Json> {
    let mut out = base.clone();
    for (key, value) in over {
        match (out.get_mut(key), value) {
            (Some(Json::Object(current)), Json::Object(next)) => {
                *current = merge(current, next);
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    out
}

fn set_path(object: &mut JsonMap<String, Json>, key: &str, value: Json) {
    let mut segments = key.split('.').peekable();
    let mut cursor = object;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            cursor.insert(segment.to_owned(), value);
            return;
        }
        let next = cursor
            .entry(segment.to_owned())
            .or_insert_with(|| Json::Object(JsonMap::new()));
        if !next.is_object() {
            *next = Json::Object(JsonMap::new());
        }
        let Json::Object(map) = next else {
            return;
        };
        cursor = map;
    }
}

fn remove_path(object: &mut JsonMap<String, Json>, key: &str) {
    let segments: Vec<&str> = key.split('.').collect();
    remove_segments(object, &segments);
}

/// Removes the path and prunes objects that became empty; `true` when `object` is empty.
fn remove_segments(object: &mut JsonMap<String, Json>, segments: &[&str]) -> bool {
    match segments {
        [] => object.is_empty(),
        [last] => {
            object.remove(*last);
            object.is_empty()
        }
        [head, rest @ ..] => {
            let empty = match object.get_mut(*head) {
                Some(Json::Object(next)) => remove_segments(next, rest),
                _ => false,
            };
            if empty {
                object.remove(*head);
            }
            object.is_empty()
        }
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
    use crate::settings::{Choice, SettingsSection};

    fn field(key: &str, type_: FieldType) -> Field {
        Field::new(key, type_, key)
    }

    fn section(id: &str, fields: Vec<Field>) -> SettingsSection {
        SettingsSection {
            id: id.into(),
            title: id.into(),
            description: None,
            advanced: None,
            fields,
        }
    }

    /// A namespace covering every §7 layering rule.
    fn namespace() -> SettingsNamespace {
        let mut platform = field("platform", FieldType::Enum);
        platform.props.options = vec![
            Choice {
                value: json!("native"),
                label: "Native".into(),
                description: None,
            },
            Choice {
                value: json!("ntfy"),
                label: "ntfy".into(),
                description: None,
            },
        ];
        let mut retries = field("retry.max", FieldType::Number);
        retries.props.min = Some(serde_json::Number::from(1u64));
        retries.props.max = Some(serde_json::Number::from(9u64));
        retries.props.integer = Some(true);
        let mut global_only = field("globalOnly", FieldType::Boolean);
        global_only.scope = Some(FieldScope::Global);
        let mut project_only = field("projectOnly", FieldType::String);
        project_only.scope = Some(FieldScope::Project);
        let mut with_default = field("withDefault", FieldType::Boolean);
        with_default.default = Some(json!(true));
        let mut page = field("adv", FieldType::Page);
        page.props.sections = vec![section("a", vec![field("advanced.key", FieldType::String)])];
        SettingsNamespace {
            namespace: "demo".into(),
            title: "Demo".into(),
            extension_id: "demo".into(),
            defaults: Some(json!({"platform": "native", "nested": {"a": 1}})),
            sections: vec![section(
                "s",
                vec![
                    field("events.agentEnd", FieldType::Boolean),
                    platform,
                    retries,
                    global_only,
                    project_only,
                    with_default,
                    field("action", FieldType::Action),
                    page,
                ],
            )],
            ..SettingsNamespace::default()
        }
    }

    #[test]
    fn resolve_layers_project_over_global_over_defaults() {
        let namespace = namespace();
        let defaults = namespace.defaults.clone().unwrap_or_default();
        let global = json!({"events": {"agentEnd": false}, "platform": "ntfy", "globalOnly": true});
        let project = json!({"platform": "native", "projectOnly": "p", "retry": {"max": 7}});
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &defaults,
                global: &global,
                project: &project,
            },
        );
        assert_eq!(resolved.diagnostics, vec![]);
        assert_eq!(
            resolved.values,
            json!({
                "events": {"agentEnd": false},
                "platform": "native",
                "nested": {"a": 1},
                "retry": {"max": 7},
                "globalOnly": true,
                "projectOnly": "p",
                "withDefault": true,
            })
        );
    }

    #[test]
    fn resolve_invalid_layer_falls_back_with_a_diagnostic() {
        let namespace = namespace();
        let global = json!({"platform": "ntfy", "retry": {"max": 5}});
        let project = json!({"platform": "carrier-pigeon", "retry": {"max": 12}});
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &Json::Null,
                global: &global,
                project: &project,
            },
        );
        assert_eq!(
            resolved.values["platform"], "ntfy",
            "invalid project enum falls back to global"
        );
        assert_eq!(
            resolved.values["retry"]["max"], 5,
            "out-of-range project number falls back to global"
        );
        assert_eq!(
            resolved.diagnostics,
            vec![
                Diagnostic {
                    namespace: "demo".into(),
                    key: "platform".into(),
                    message: "expected one of native, ntfy".into(),
                },
                Diagnostic {
                    namespace: "demo".into(),
                    key: "retry.max".into(),
                    message: "must be at most 9".into(),
                },
            ]
        );

        // Invalid everywhere, no default: the key is unset, resolve still succeeds.
        let project = json!({"platform": "nope", "retry": {"max": 0}});
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &Json::Null,
                global: &Json::Null,
                project: &project,
            },
        );
        assert!(resolved.values.get("platform").is_none());
        assert!(resolved.values.get("retry").is_none());
        assert_eq!(resolved.diagnostics.len(), 2);
    }

    #[test]
    fn resolve_ignores_the_wrong_scope_layer() {
        let namespace = namespace();
        // A global field never takes the project value; a project field never the global one.
        let global = json!({"projectOnly": "from-global"});
        let project = json!({"globalOnly": false, "projectOnly": "from-project"});
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &Json::Null,
                global: &global,
                project: &project,
            },
        );
        assert!(resolved.values.get("globalOnly").is_none());
        assert_eq!(resolved.values["projectOnly"], "from-project");
    }

    #[test]
    fn resolve_applies_the_field_default_and_keeps_unknown_keys() {
        let namespace = namespace();
        let global = json!({"unlisted": {"kept": true}});
        let defaults = namespace.defaults.clone().unwrap_or_default();
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &defaults,
                global: &global,
                project: &Json::Null,
            },
        );
        assert_eq!(resolved.values["withDefault"], true);
        assert_eq!(resolved.values["platform"], "native");
        assert_eq!(resolved.values["nested"], json!({"a": 1}));
        assert_eq!(
            resolved.values["unlisted"],
            json!({"kept": true}),
            "keys without a field survive"
        );
    }

    #[test]
    fn resolve_with_a_stricter_checker_rejects_more() {
        let namespace = namespace();
        let strict = |field: &Field, value: &Json| {
            check_value(field, value)?;
            if field.key == "platform" && value == "native" {
                return Err("native is disabled here".into());
            }
            Ok(())
        };
        let global = json!({"platform": "ntfy"});
        let project = json!({"platform": "native"});
        let resolved = resolve_with(
            &namespace,
            Layers {
                defaults: &Json::Null,
                global: &global,
                project: &project,
            },
            &strict,
        );
        assert_eq!(resolved.values["platform"], "ntfy");
        assert_eq!(resolved.diagnostics[0].message, "native is disabled here");
    }

    #[test]
    fn simplify_unsets_defaults_and_skips_the_wrong_scope() {
        let namespace = namespace();
        let defaults = namespace.defaults.clone().unwrap_or_default();
        let global = json!({"platform": "ntfy", "projectOnly": "ignored"});
        let layers = Layers {
            defaults: &defaults,
            global: &global,
            project: &Json::Null,
        };
        // Effective values equal to the layer below are written as unsets.
        let effective = json!({
            "platform": "ntfy",
            "events": {"agentEnd": true},
            "retry": {"max": 4},
            "globalOnly": true,
            "projectOnly": "p",
            "withDefault": true,
        });
        let writes = simplify(&namespace, Layer::Global, layers, &effective);
        let by_key = |key: &str| {
            writes
                .iter()
                .find(|write| write.key == key)
                .map(|write| write.value.clone())
        };
        assert_eq!(
            by_key("platform"),
            Some(Some(json!("ntfy"))),
            "a value differing from the default is written"
        );
        assert_eq!(
            by_key("events.agentEnd"),
            Some(Some(json!(true))),
            "no default ⇒ the value is written"
        );
        assert_eq!(by_key("retry.max"), Some(Some(json!(4))));
        assert_eq!(by_key("globalOnly"), Some(Some(json!(true))));
        assert_eq!(
            by_key("projectOnly"),
            None,
            "global writes skip project fields"
        );
        assert_eq!(by_key("withDefault"), Some(None), "field default unsets");

        // A project write is simplified against defaults ⊕ global.
        let effective = json!({"platform": "ntfy", "retry": {"max": 5}});
        let writes = simplify(&namespace, Layer::Project, layers, &effective);
        assert_eq!(
            writes
                .iter()
                .find(|write| write.key == "platform")
                .map(|write| write.value.clone()),
            Some(None),
            "equal to the global value ⇒ unset"
        );
        assert_eq!(
            writes
                .iter()
                .find(|write| write.key == "retry.max")
                .map(|write| write.value.clone()),
            Some(Some(json!(5)))
        );
        assert!(
            !writes.iter().any(|write| write.key == "globalOnly"),
            "project writes skip global fields"
        );
    }

    #[test]
    fn simplify_key_compares_with_the_layer_below() {
        let mut number = field("retry.max", FieldType::Number);
        number.default = Some(json!(3));
        assert_eq!(simplify_key(&number, None, &json!(3)), None);
        assert_eq!(simplify_key(&number, Some(&json!(5)), &json!(5)), None);
        assert_eq!(
            simplify_key(&number, Some(&json!(5)), &json!(6)),
            Some(json!(6))
        );
    }

    #[test]
    fn check_value_covers_every_field_type() {
        let boolean = field("b", FieldType::Boolean);
        assert!(check_value(&boolean, &json!(true)).is_ok());
        assert_eq!(
            check_value(&boolean, &json!("true")).unwrap_err(),
            "expected a boolean"
        );

        let mut number = field("n", FieldType::Number);
        number.props.min = Some(serde_json::Number::from(1u64));
        number.props.integer = Some(true);
        assert!(check_value(&number, &json!(2)).is_ok());
        assert_eq!(
            check_value(&number, &json!(1.5)).unwrap_err(),
            "expected an integer"
        );
        assert_eq!(
            check_value(&number, &json!(0)).unwrap_err(),
            "must be at least 1"
        );

        let mut text = field("t", FieldType::String);
        text.props.min_length = Some(2);
        text.props.max_length = Some(3);
        assert!(check_value(&text, &json!("ab")).is_ok());
        assert_eq!(
            check_value(&text, &json!("a")).unwrap_err(),
            "must be at least 2 characters"
        );

        let mut enums = field("e", FieldType::Enum);
        enums.props.options = namespace().fields()[1].props.options.clone();
        assert!(check_value(&enums, &json!("ntfy")).is_ok());
        assert_eq!(
            check_value(&enums, &json!("x")).unwrap_err(),
            "expected one of native, ntfy"
        );
        let mut custom = enums.clone();
        custom.props.allow_custom = Some(true);
        assert!(check_value(&custom, &json!("x")).is_ok());

        let mut multi = field("m", FieldType::Multiselect);
        multi.props.options = enums.props.options.clone();
        assert!(check_value(&multi, &json!(["native"])).is_ok());
        assert!(check_value(&multi, &json!(["other"])).is_err());
        assert!(check_value(&multi, &json!("native")).is_err());

        let mut order = field("o", FieldType::Order);
        order.props.items = vec!["a".into(), "b".into()];
        assert!(check_value(&order, &json!(["b", "a"])).is_ok());
        assert!(check_value(&order, &json!(["c"])).is_err());

        let mut list = field("l", FieldType::List);
        list.props.item = Some(Box::new(field("item", FieldType::String)));
        list.props.max = Some(serde_json::Number::from(2u64));
        assert!(check_value(&list, &json!(["x"])).is_ok());
        assert!(check_value(&list, &json!(["x", "y", "z"])).is_err());
        assert!(check_value(&list, &json!([1])).is_err());

        let mut dict = field("d", FieldType::Dict);
        dict.props.value = Some(Box::new(field("value", FieldType::Number)));
        assert!(check_value(&dict, &json!({"a": 1})).is_ok());
        assert!(check_value(&dict, &json!({"a": "1"})).is_err());
        assert!(check_value(&dict, &json!(["a"])).is_err());

        assert!(check_value(&field("s", FieldType::Secret), &json!("x")).is_ok());
        assert!(check_value(&field("s", FieldType::Secret), &json!(1)).is_err());
        assert!(check_value(&field("m", FieldType::Model), &json!("a/b")).is_ok());
        assert!(check_value(&field("j", FieldType::Json), &json!({"x": 1})).is_ok());
        assert!(check_value(&field("u", FieldType::Unknown), &json!(true)).is_ok());
    }

    #[test]
    fn conditions_evaluate_on_effective_values() {
        let values = json!({
            "mode": "custom",
            "n": 0,
            "tags": ["a"],
            "flat.key": "x",
            "nested": {"deep": true},
        });
        assert!(
            Cond {
                key: "mode".into(),
                eq: Some(json!("custom")),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            Cond {
                key: "flat.key".into(),
                eq: Some(json!("x")),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            Cond {
                key: "nested.deep".into(),
                eq: Some(json!(true)),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            !Cond {
                key: "mode".into(),
                ne: Some(json!("custom")),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            Cond {
                key: "mode".into(),
                in_: Some(vec![json!("custom"), json!("other")]),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            !Cond {
                key: "n".into(),
                truthy: Some(true),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            Cond {
                key: "tags".into(),
                truthy: Some(true),
                ..Cond::default()
            }
            .eval(&values)
        );
        assert!(
            !Cond {
                key: "missing".into(),
                truthy: Some(true),
                ..Cond::default()
            }
            .eval(&values)
        );

        let mut custom = field("only", FieldType::String);
        custom.visible_if = Some(Cond {
            key: "mode".into(),
            eq: Some(json!("custom")),
            ..Cond::default()
        });
        custom.disabled_if = Some(Cond {
            key: "n".into(),
            eq: Some(json!(0)),
            ..Cond::default()
        });
        assert!(custom.visible(&values));
        assert!(!custom.enabled(&values), "visible but disabled");
        let other = json!({"mode": "simple", "n": 1});
        assert!(!custom.visible(&other));
        assert!(!custom.enabled(&other));
    }

    #[test]
    fn fields_flatten_pages_and_find_by_key() {
        let namespace = namespace();
        let keys: Vec<&str> = namespace.fields().iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "events.agentEnd",
                "platform",
                "retry.max",
                "globalOnly",
                "projectOnly",
                "withDefault",
                "action",
                "advanced.key",
            ]
        );
        assert_eq!(
            namespace.field("advanced.key").map(|field| field.type_),
            Some(FieldType::String)
        );
        assert!(namespace.field("adv").is_none());
    }

    #[test]
    fn list_and_dict_templates_need_no_key() {
        // §7 `FieldWithoutKey`: the row templates carry no `key`.
        let raw = json!({
            "namespace": "mcp", "title": "MCP", "extensionId": "acme.mcp",
            "sections": [{"id": "s", "title": "S", "fields": [
                {"key": "servers", "type": "list", "label": "Servers",
                 "item": {"type": "string", "label": "URL"}},
                {"key": "env", "type": "dict", "label": "Env",
                 "value": {"type": "number", "label": "Value", "min": 0}}
            ]}]
        });
        let namespace: SettingsNamespace =
            serde_json::from_value(raw.clone()).expect("templates parse without keys");
        let list = namespace.field("servers").unwrap();
        assert_eq!(
            list.props.item.as_ref().map(|item| item.key.as_str()),
            Some("")
        );
        assert!(
            check_value(list, &json!(["a"])).is_ok(),
            "the item template validates rows"
        );
        let dict = namespace.field("env").unwrap();
        assert!(check_value(dict, &json!({"A": 1})).is_ok());
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &Json::Null,
                global: &json!({"servers": ["a"], "env": {"A": 1}}),
                project: &Json::Null,
            },
        );
        assert_eq!(resolved.values["servers"], json!(["a"]));
    }

    #[test]
    fn v1_namespaces_parse_unchanged() {
        // The shape the v1 kit's `settings::schema` (v1 §6.6) emits: bare props, `order.items`
        // as `{value, label}` objects, `dynamic` from a service, `allowCustom`, `clearable`.
        let raw = json!({
            "namespace": "notify", "title": "Notify", "extensionId": "acme.notify",
            "defaults": {"events": {"agentEnd": true}},
            "sections": [{"id": "events", "title": "Events", "fields": [
                {"key": "events.agentEnd", "type": "boolean", "label": "Agent"},
                {"key": "platform", "type": "enum", "label": "Platform",
                 "options": [{"value": "native", "label": "Native", "description": "OS"}, "ntfy"],
                 "allowCustom": true, "clearable": true},
                {"key": "tags", "type": "multiselect", "label": "Tags",
                 "options": [{"value": "a", "label": "A"}], "emptyLabel": "none"},
                {"key": "token", "type": "secret", "label": "Token", "scope": "global",
                 "emptyLabel": "unset"},
                {"key": "columns", "type": "order", "label": "Columns",
                 "items": [{"value": "a", "label": "A", "description": "d"}]},
                {"key": "model", "type": "model", "label": "Model",
                 "filter": {"input": ["image"]}, "presets": ["a/b"], "providerKey": "provider",
                 "emptyOption": "auto"},
                {"key": "timeout", "type": "number", "label": "Timeout", "unit": "ms",
                 "min": 0.0, "max": 10.0, "step": 1.0, "zeroLabel": "off"},
                {"key": "path", "type": "string", "label": "Path", "multiline": false,
                 "pattern": "^/", "emptyLabel": "none"},
                {"key": "run", "type": "action", "label": "Run", "command": "run"},
                {"key": "dynamic", "type": "enum", "label": "Dynamic",
                 "options": [], "dynamic": {"service": "models"}},
                {"key": "adv", "type": "page", "label": "Adv", "sections": [
                    {"id": "a", "title": "A", "fields": [
                        {"key": "adv.n", "type": "number", "label": "N"}
                    ]}
                ]}
            ]}]
        });
        let namespace: SettingsNamespace =
            serde_json::from_value(raw.clone()).expect("v1 namespaces parse");
        assert_eq!(namespace.namespace, "notify");
        assert_eq!(namespace.fields().len(), 11);
        let columns = namespace.field("columns").unwrap();
        assert_eq!(columns.props.items, ["a"]);
        let platform = namespace.field("platform").unwrap();
        assert_eq!(platform.props.options[1].value, json!("ntfy"));
        assert!(platform.props.allow_custom.is_some_and(|yes| yes));
        let dynamic = namespace.field("dynamic").unwrap();
        assert_eq!(
            dynamic.dynamic.as_ref().map(|d| d.service.as_str()),
            Some("models")
        );
        let resolved = resolve(
            &namespace,
            Layers {
                defaults: &raw["defaults"],
                global: &json!({"platform": "carrier-pigeon", "events": {"agentEnd": false}}),
                project: &Json::Null,
            },
        );
        // `allowCustom` keeps the custom value; the boolean override wins; defaults survive.
        assert_eq!(resolved.values["platform"], "carrier-pigeon");
        assert_eq!(resolved.values["events"]["agentEnd"], false);
        assert_eq!(resolved.diagnostics, vec![]);
    }

    #[test]
    fn surfaces_filter_per_client() {
        use crate::settings::{Surface, on_surface};
        let tui = Field::boolean("tuiMode", "TUI mode").only_on(&[Surface::Tui]);
        assert!(tui.shown_on(Surface::Tui));
        assert!(!tui.shown_on(Surface::Gui));
        let both = Field::boolean("x", "X");
        assert!(both.shown_on(Surface::Gui) && both.shown_on(Surface::Tui));
        assert!(on_surface(None, Surface::Gui));
        assert!(!on_surface(Some(&[]), Surface::Gui));
        let json = serde_json::to_value(&tui).unwrap();
        assert_eq!(json["surfaces"], serde_json::json!(["tui"]));
        let back: Field = serde_json::from_value(json).unwrap();
        assert_eq!(back.surfaces, Some(vec![Surface::Tui]));
        assert!(
            serde_json::to_value(&both)
                .unwrap()
                .get("surfaces")
                .is_none()
        );
    }
}
