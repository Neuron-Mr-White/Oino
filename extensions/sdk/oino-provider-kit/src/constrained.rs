//! pi-ai `api/constrained-sampling.js`: strict JSON-schema conversion and grammar tools.

use std::collections::{BTreeMap, HashSet};

use oino_ai::{ConstrainedSampling, JsonObject, StrictMode, ToolDecl};
use serde_json::{Value, json};

const UNSUPPORTED_STRICT_SCHEMA_KEYS: [&str; 16] = [
    "$ref",
    "$defs",
    "definitions",
    "allOf",
    "oneOf",
    "patternProperties",
    "dependentSchemas",
    "dependencies",
    "unevaluatedProperties",
    "propertyNames",
    "contains",
    "prefixItems",
    "not",
    "if",
    "then",
    "else",
];

/// Provider-specific keyword filter (`isUnsupportedKeyword`).
pub type KeywordFilter = fn(&str, &Value) -> bool;

/// A schema strict mode cannot express.
#[derive(Debug)]
struct Unsupported(String);

fn is_structured(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let types: Vec<&str> = match object.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    types.contains(&"object")
        || types.contains(&"array")
        || object.contains_key("properties")
        || object.contains_key("items")
}

fn allows_null(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    let ty = object.get("type");
    if ty == Some(&json!("null"))
        || ty
            .and_then(Value::as_array)
            .is_some_and(|list| list.contains(&json!("null")))
    {
        return true;
    }
    if object.get("const") == Some(&Value::Null)
        || object
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|list| list.contains(&Value::Null))
    {
        return true;
    }
    object
        .get("anyOf")
        .and_then(Value::as_array)
        .is_some_and(|variants| variants.iter().any(allows_null))
}

fn make_strict_node(schema: &mut Value, filter: Option<KeywordFilter>) -> Result<(), Unsupported> {
    let Some(object) = schema.as_object_mut() else {
        return Err(Unsupported("boolean schemas are unsupported".into()));
    };
    for key in UNSUPPORTED_STRICT_SCHEMA_KEYS {
        if object.contains_key(key) {
            return Err(Unsupported(format!("{key} schemas are unsupported")));
        }
    }
    if let Some(filter) = filter {
        for (key, value) in object.iter() {
            if filter(key, value) {
                return Err(Unsupported(format!("{key}: {value} is unsupported")));
            }
        }
    }
    if let Some(any_of) = object.get_mut("anyOf") {
        let Some(variants) = any_of.as_array_mut().filter(|v| !v.is_empty()) else {
            return Err(Unsupported("anyOf must contain at least one schema".into()));
        };
        for variant in variants {
            if is_structured(variant) {
                return Err(Unsupported(
                    "object and array unions are unsupported".into(),
                ));
            }
            make_strict_node(variant, filter)?;
        }
    }
    if let Some(items) = object.get_mut("items") {
        if items.is_array() {
            return Err(Unsupported("tuple schemas are unsupported".into()));
        }
        make_strict_node(items, filter)?;
    }
    let is_object = object.get("type") == Some(&json!("object"));
    if object.contains_key("properties") && !is_object {
        return Err(Unsupported("properties require type object".into()));
    }
    if !is_object {
        return Ok(());
    }
    if object
        .get("additionalProperties")
        .is_some_and(|v| v != &Value::Bool(false))
    {
        return Err(Unsupported(
            "schema-valued or true additionalProperties is unsupported".into(),
        ));
    }
    if object.get("properties").is_some_and(|p| !p.is_object()) {
        return Err(Unsupported("object properties must be a schema map".into()));
    }
    let required: Vec<String> = match object.get("required") {
        None => Vec::new(),
        Some(Value::Array(list)) if list.iter().all(Value::is_string) => list
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(_) => {
            return Err(Unsupported("object required must be a string array".into()));
        }
    };
    let mut properties = object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let names: Vec<String> = properties.keys().cloned().collect();
    if required.iter().any(|key| !names.contains(key)) {
        return Err(Unsupported("required contains an unknown property".into()));
    }
    let required: HashSet<String> = required.into_iter().collect();
    for (key, property) in &mut properties {
        make_strict_node(property, filter)?;
        if !required.contains(key) && !allows_null(property) {
            *property = json!({"anyOf": [property.clone(), {"type": "null"}]});
        }
    }
    if object.contains_key("properties") {
        object.insert("properties".into(), Value::Object(properties));
    }
    object.insert("required".into(), json!(names));
    object.insert("additionalProperties".into(), Value::Bool(false));
    Ok(())
}

fn make_strict(schema: &Value, filter: Option<KeywordFilter>) -> Result<Value, Unsupported> {
    let mut cloned = schema.clone();
    if !cloned.is_object() {
        return Err(Unsupported("root schema must have type object".into()));
    }
    make_strict_node(&mut cloned, filter)?;
    if cloned.get("type") != Some(&json!("object")) {
        return Err(Unsupported("root schema must have type object".into()));
    }
    Ok(cloned)
}

/// pi `getJsonSchemaToolParameters`.
///
/// # Errors
/// Only when `strict` is set and the schema cannot be made strict (callers resolve strictness
/// first with [`resolve_json_schema_strict`], so this does not happen in practice).
pub fn json_schema_tool_parameters(tool: &ToolDecl, strict: bool) -> Result<Value, String> {
    if strict {
        make_strict(&tool.parameters, None).map_err(|e| e.0)
    } else {
        Ok(tool.parameters.clone())
    }
}

#[allow(
    clippy::option_option,
    reason = "absent config vs json_schema config without strictness"
)]
fn json_schema_config(tool: &ToolDecl) -> Option<Option<StrictMode>> {
    match &tool.constrained_sampling {
        Some(ConstrainedSampling::Config(config)) if config.kind == "json_schema" => {
            Some(config.strict)
        }
        _ => None,
    }
}

/// Whether the tool asks for `json_schema` with `strict: "prefer"`.
#[must_use]
pub fn prefers_strict(tool: &ToolDecl) -> bool {
    json_schema_config(tool) == Some(Some(StrictMode::Prefer))
}

/// pi `resolveJsonSchemaStrictSampling`: `Some(true)` when the tool is sent strict.
///
/// # Errors
/// When the tool requires strict sampling the provider or schema cannot honour.
pub fn resolve_json_schema_strict(
    tool: &ToolDecl,
    supports_strict: bool,
    filter: Option<KeywordFilter>,
) -> Result<Option<bool>, String> {
    let Some(strict) = json_schema_config(tool) else {
        return Ok(None);
    };
    let require = strict == Some(StrictMode::Require);
    if supports_strict {
        return match make_strict(&tool.parameters, filter) {
            Ok(_) => Ok(Some(true)),
            Err(_) if !require => Ok(None),
            Err(error) => Err(format!(
                "Tool \"{}\" requires JSON-schema constrained sampling, but {}.",
                tool.name, error.0
            )),
        };
    }
    if require {
        return Err(format!(
            "Tool \"{}\" requires JSON-schema constrained sampling, but strict tools are unsupported.",
            tool.name
        ));
    }
    Ok(None)
}

/// A resolved grammar tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grammar {
    /// `lark` or `regex`.
    pub format: &'static str,
    /// Grammar text.
    pub definition: String,
    /// The single string argument carrying the input.
    pub input_property: String,
}

fn infer_grammar_input_property(tool: &ToolDecl) -> Result<String, String> {
    let schema = &tool.parameters;
    if schema.get("type") != Some(&json!("object")) {
        return Err("grammar constrained sampling requires an object parameter schema".into());
    }
    let required = schema.get("required").and_then(Value::as_array);
    let Some(property) = required
        .filter(|list| list.len() == 1)
        .and_then(|list| list[0].as_str())
    else {
        return Err(
            "grammar constrained sampling requires exactly one required string property".into(),
        );
    };
    let Some(definition) = schema.pointer(&format!("/properties/{property}")) else {
        return Err(format!(
            "grammar constrained sampling requires a properties entry for {property}"
        ));
    };
    if definition.get("type") != Some(&json!("string")) {
        return Err(format!(
            "grammar constrained sampling property {property} must have type string"
        ));
    }
    Ok(property.to_owned())
}

/// pi `resolveGrammarConstrainedSampling`.
///
/// # Errors
/// When a grammar tool has no usable variant or an unsuitable schema.
pub fn resolve_grammar(tool: &ToolDecl, supported: bool) -> Result<Option<Grammar>, String> {
    let Some(ConstrainedSampling::Config(config)) = &tool.constrained_sampling else {
        return Ok(None);
    };
    if config.kind != "grammar" || !supported {
        return Ok(None);
    }
    let variant = |name: &str| {
        config
            .variants
            .as_ref()
            .and_then(|v| v.get(name))
            .filter(|text| !text.trim().is_empty())
            .cloned()
    };
    let (format, definition) = match (variant("openai_lark"), variant("openai_regex")) {
        (Some(lark), _) => ("lark", lark),
        (None, Some(regex)) => ("regex", regex),
        (None, None) => {
            return Err(format!(
                "Tool \"{}\" cannot use grammar constrained sampling: no supported grammar variant was provided.",
                tool.name
            ));
        }
    };
    let input_property = infer_grammar_input_property(tool).map_err(|message| {
        format!(
            "Tool \"{}\" cannot use grammar constrained sampling: {message}.",
            tool.name
        )
    })?;
    Ok(Some(Grammar {
        format,
        definition,
        input_property,
    }))
}

/// pi `createGrammarToolInputProperties`: tool name → input property.
///
/// # Errors
/// Propagates [`resolve_grammar`] errors.
pub fn grammar_input_properties(
    tools: &[ToolDecl],
    supported: bool,
) -> Result<BTreeMap<String, String>, String> {
    let mut properties = BTreeMap::new();
    for tool in tools {
        if let Some(grammar) = resolve_grammar(tool, supported)? {
            properties.insert(tool.name.clone(), grammar.input_property);
        }
    }
    Ok(properties)
}

/// pi `getGrammarToolInput`.
///
/// # Errors
/// When the argument is not a string.
pub fn grammar_tool_input(
    name: &str,
    arguments: &JsonObject,
    property: &str,
) -> Result<String, String> {
    match arguments.get(property) {
        Some(Value::String(input)) => Ok(input.clone()),
        _ => Err(format!(
            "Grammar tool call \"{name}\" requires argument \"{property}\" to be a string."
        )),
    }
}

/// Streaming state of a grammar tool's JSON argument rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GrammarBuffer {
    input: String,
    started: bool,
    closed: bool,
}

/// pi `appendGrammarToolInputJsonDelta`: the JSON text to append for `next` input.
///
/// # Errors
/// When the input changes non-monotonically or after closing.
pub fn append_grammar_delta(
    buffer: &mut GrammarBuffer,
    property: &str,
    next: &str,
    close: bool,
) -> Result<Option<String>, String> {
    if buffer.closed {
        if close && next == buffer.input {
            return Ok(None);
        }
        return Err(format!(
            "grammar tool input for property \"{property}\" changed after it was closed"
        ));
    }
    let Some(input_delta) = next.strip_prefix(buffer.input.as_str()) else {
        return Err(format!(
            "grammar tool input for property \"{property}\" changed non-monotonically"
        ));
    };
    if !close && input_delta.is_empty() {
        return Ok(None);
    }
    let mut delta = String::new();
    if !buffer.started {
        delta.push('{');
        delta.push_str(&Value::String(property.to_owned()).to_string());
        delta.push_str(":\"");
        buffer.started = true;
    }
    let quoted = Value::String(input_delta.to_owned()).to_string();
    delta.push_str(&quoted[1..quoted.len() - 1]);
    next.clone_into(&mut buffer.input);
    if close {
        delta.push_str("\"}");
        buffer.closed = true;
    }
    Ok(Some(delta))
}
