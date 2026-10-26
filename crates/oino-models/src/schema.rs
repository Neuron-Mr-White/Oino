//! A small JSON-schema validator reproducing the typebox checks and error texts pi applies to
//! `models.json` (`model-config.ts` `ModelsConfigSchema`).
//!
//! Errors are reported like pi: required properties first, then properties in schema order;
//! paths are dotted (`providers.a.models.0.id`), a missing required property is reported at
//! `<object path>.<first missing>` with every missing name in the message.

use serde_json::Value;

/// A schema node.
#[derive(Debug)]
pub(crate) enum Schema {
    /// Object with known properties; additional properties are allowed.
    Object(&'static [Prop]),
    /// Object whose every value matches the schema.
    Record(&'static Schema),
    /// Any JSON object (`Record<string, unknown>`).
    AnyObject,
    /// String with a minimum length (in UTF-16 code units, as JavaScript counts).
    String { min_len: usize },
    /// Finite number, optionally `> exclusive_min`.
    Number { exclusive_min: Option<f64> },
    /// Integer with optional inclusive bounds.
    Integer { min: Option<f64>, max: Option<f64> },
    /// Boolean.
    Bool,
    /// One of the strings.
    Enum(&'static [&'static str]),
    /// Exactly this string.
    Const(&'static str),
    /// Array of items.
    Array(&'static Schema),
    /// `string | null`.
    StringOrNull,
}

/// One object property.
#[derive(Debug)]
pub(crate) struct Prop {
    name: &'static str,
    schema: &'static Schema,
    required: bool,
}

const fn opt(name: &'static str, schema: &'static Schema) -> Prop {
    Prop {
        name,
        schema,
        required: false,
    }
}

const fn req(name: &'static str, schema: &'static Schema) -> Prop {
    Prop {
        name,
        schema,
        required: true,
    }
}

static STRING: Schema = Schema::String { min_len: 0 };
static NON_EMPTY: Schema = Schema::String { min_len: 1 };
static NUMBER: Schema = Schema::Number {
    exclusive_min: None,
};
static POSITIVE: Schema = Schema::Number {
    exclusive_min: Some(0.0),
};
static POSITIVE_INT: Schema = Schema::Integer {
    min: Some(1.0),
    max: None,
};
static JPEG_QUALITY: Schema = Schema::Integer {
    min: Some(1.0),
    max: Some(100.0),
};
static BOOL: Schema = Schema::Bool;
static ANY_OBJECT: Schema = Schema::AnyObject;
static STRING_OR_NULL: Schema = Schema::StringOrNull;
static MODALITY: Schema = Schema::Enum(&["text", "image"]);
static MODALITIES: Schema = Schema::Array(&MODALITY);
static HEADERS: Schema = Schema::Record(&STRING);
static RADIUS: Schema = Schema::Const("radius");

static THINKING_LEVEL_MAP: Schema = Schema::Object(&[
    opt("off", &STRING_OR_NULL),
    opt("minimal", &STRING_OR_NULL),
    opt("low", &STRING_OR_NULL),
    opt("medium", &STRING_OR_NULL),
    opt("high", &STRING_OR_NULL),
    opt("xhigh", &STRING_OR_NULL),
    opt("max", &STRING_OR_NULL),
]);

static PROMPT_CACHE: Schema = Schema::Object(&[opt("short", &POSITIVE), opt("long", &POSITIVE)]);

static COST_TIER: Schema = Schema::Object(&[
    req("inputTokensAbove", &NUMBER),
    req("input", &NUMBER),
    req("output", &NUMBER),
    req("cacheRead", &NUMBER),
    req("cacheWrite", &NUMBER),
]);
static COST_TIERS: Schema = Schema::Array(&COST_TIER);

static COST: Schema = Schema::Object(&[
    req("input", &NUMBER),
    req("output", &NUMBER),
    req("cacheRead", &NUMBER),
    req("cacheWrite", &NUMBER),
    opt("tiers", &COST_TIERS),
]);

static PARTIAL_COST: Schema = Schema::Object(&[
    opt("input", &NUMBER),
    opt("output", &NUMBER),
    opt("cacheRead", &NUMBER),
    opt("cacheWrite", &NUMBER),
    opt("tiers", &COST_TIERS),
]);

static IMAGE_RESIZE: Schema = Schema::Object(&[
    opt("maxWidth", &POSITIVE_INT),
    opt("maxHeight", &POSITIVE_INT),
    opt("maxBytes", &POSITIVE_INT),
    opt("jpegQuality", &JPEG_QUALITY),
]);

static IMAGE_LIMITS: Schema = Schema::Object(&[
    opt("resize", &IMAGE_RESIZE),
    opt("maxPerMessage", &POSITIVE_INT),
    opt("maxPerRequest", &POSITIVE_INT),
]);

static INPUT_LIMITS: Schema = Schema::Object(&[
    opt("maxRequestBytes", &POSITIVE_INT),
    opt("images", &IMAGE_LIMITS),
]);

static SAMPLING_BY_LEVEL: Schema = Schema::Object(&[
    opt("off", &ANY_OBJECT),
    opt("minimal", &ANY_OBJECT),
    opt("low", &ANY_OBJECT),
    opt("medium", &ANY_OBJECT),
    opt("high", &ANY_OBJECT),
    opt("xhigh", &ANY_OBJECT),
    opt("max", &ANY_OBJECT),
]);

static MODEL_DEFINITION: Schema = Schema::Object(&[
    req("id", &NON_EMPTY),
    opt("name", &NON_EMPTY),
    opt("api", &NON_EMPTY),
    opt("baseUrl", &NON_EMPTY),
    opt("reasoning", &BOOL),
    opt("thinkingLevelMap", &THINKING_LEVEL_MAP),
    opt("input", &MODALITIES),
    opt("inputLimits", &INPUT_LIMITS),
    opt("cost", &COST),
    opt("promptCache", &PROMPT_CACHE),
    opt("contextWindow", &POSITIVE),
    opt("maxTokens", &POSITIVE),
    opt("samplingParams", &ANY_OBJECT),
    opt("samplingParamsByThinkingLevel", &SAMPLING_BY_LEVEL),
    opt("headers", &HEADERS),
    opt("compat", &ANY_OBJECT),
]);
static MODEL_DEFINITIONS: Schema = Schema::Array(&MODEL_DEFINITION);

static MODEL_OVERRIDE: Schema = Schema::Object(&[
    opt("name", &NON_EMPTY),
    opt("reasoning", &BOOL),
    opt("thinkingLevelMap", &THINKING_LEVEL_MAP),
    opt("input", &MODALITIES),
    opt("inputLimits", &INPUT_LIMITS),
    opt("cost", &PARTIAL_COST),
    opt("promptCache", &PROMPT_CACHE),
    opt("contextWindow", &POSITIVE),
    opt("maxTokens", &POSITIVE),
    opt("samplingParams", &ANY_OBJECT),
    opt("samplingParamsByThinkingLevel", &SAMPLING_BY_LEVEL),
    opt("headers", &HEADERS),
    opt("compat", &ANY_OBJECT),
]);
static MODEL_OVERRIDES: Schema = Schema::Record(&MODEL_OVERRIDE);

static PROVIDER: Schema = Schema::Object(&[
    opt("name", &NON_EMPTY),
    opt("baseUrl", &NON_EMPTY),
    opt("apiKey", &NON_EMPTY),
    opt("api", &NON_EMPTY),
    opt("oauth", &RADIUS),
    opt("headers", &HEADERS),
    opt("compat", &ANY_OBJECT),
    opt("authHeader", &BOOL),
    opt("models", &MODEL_DEFINITIONS),
    opt("modelOverrides", &MODEL_OVERRIDES),
]);
static PROVIDERS: Schema = Schema::Record(&PROVIDER);

/// pi `ModelsConfigSchema` (`compat` is checked only for being an object: compatibility flags
/// are interpreted by provider extensions).
pub(crate) static MODELS_CONFIG: Schema =
    Schema::Object(&[opt("$schema", &STRING), req("providers", &PROVIDERS)]);

/// One validation failure: dotted path and typebox message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaError {
    pub(crate) path: String,
    pub(crate) message: String,
}

/// Validates `value` against `schema`, returning every error in pi order.
pub(crate) fn validate(schema: &Schema, value: &Value) -> Vec<SchemaError> {
    let mut errors = Vec::new();
    check(schema, value, &mut Vec::new(), &mut errors);
    errors
}

fn escape_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn join_path(path: &[String]) -> String {
    if path.is_empty() {
        "root".to_owned()
    } else {
        path.join(".")
    }
}

fn push(errors: &mut Vec<SchemaError>, path: &[String], message: impl Into<String>) {
    errors.push(SchemaError {
        path: join_path(path),
        message: message.into(),
    });
}

fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

#[allow(
    clippy::float_cmp,
    reason = "integer check mirrors JavaScript Number.isInteger"
)]
fn is_integer(number: f64) -> bool {
    number.trunc() == number
}

fn check(schema: &Schema, value: &Value, path: &mut Vec<String>, errors: &mut Vec<SchemaError>) {
    match schema {
        Schema::Object(props) => check_object(props, value, path, errors),
        Schema::Record(item) => {
            let Value::Object(map) = value else {
                return push(errors, path, "must be object");
            };
            for (key, entry) in map {
                path.push(escape_segment(key));
                check(item, entry, path, errors);
                path.pop();
            }
        }
        Schema::AnyObject => {
            if !value.is_object() {
                push(errors, path, "must be object");
            }
        }
        Schema::String { min_len } => match value {
            Value::String(text) if js_length(text) < *min_len => push(
                errors,
                path,
                format!("must not have fewer than {min_len} characters"),
            ),
            Value::String(_) => {}
            _ => push(errors, path, "must be string"),
        },
        Schema::Number { exclusive_min } => match value.as_f64() {
            None => push(errors, path, "must be number"),
            Some(number) => {
                if let Some(min) = exclusive_min
                    && number <= *min
                {
                    push(errors, path, format!("must be > {min}"));
                }
            }
        },
        Schema::Integer { min, max } => match value.as_f64() {
            Some(number) if is_integer(number) => {
                if let Some(min) = min
                    && number < *min
                {
                    push(errors, path, format!("must be >= {min}"));
                }
                if let Some(max) = max
                    && number > *max
                {
                    push(errors, path, format!("must be <= {max}"));
                }
            }
            _ => push(errors, path, "must be integer"),
        },
        Schema::Bool => {
            if !value.is_boolean() {
                push(errors, path, "must be boolean");
            }
        }
        Schema::Enum(allowed) => {
            if !value.as_str().is_some_and(|text| allowed.contains(&text)) {
                push(errors, path, "must be equal to one of the allowed values");
            }
        }
        Schema::Const(expected) => {
            if value.as_str() != Some(expected) {
                push(errors, path, "must be equal to constant");
            }
        }
        Schema::Array(item) => {
            let Value::Array(items) = value else {
                return push(errors, path, "must be array");
            };
            for (index, entry) in items.iter().enumerate() {
                path.push(index.to_string());
                check(item, entry, path, errors);
                path.pop();
            }
        }
        Schema::StringOrNull => {
            if !(value.is_string() || value.is_null()) {
                push(errors, path, "must be string");
                push(errors, path, "must be null");
                push(errors, path, "must match a schema in anyOf");
            }
        }
    }
}

fn check_object(
    props: &[Prop],
    value: &Value,
    path: &mut Vec<String>,
    errors: &mut Vec<SchemaError>,
) {
    let Value::Object(map) = value else {
        return push(errors, path, "must be object");
    };
    let missing: Vec<&str> = props
        .iter()
        .filter(|prop| prop.required && !map.contains_key(prop.name))
        .map(|prop| prop.name)
        .collect();
    if let Some(first) = missing.first() {
        path.push((*first).to_owned());
        push(
            errors,
            path,
            format!("must have required properties {}", missing.join(", ")),
        );
        path.pop();
    }
    for prop in props {
        if let Some(entry) = map.get(prop.name) {
            path.push(escape_segment(prop.name));
            check(prop.schema, entry, path, errors);
            path.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn messages(value: &Value) -> Vec<String> {
        validate(&MODELS_CONFIG, value)
            .into_iter()
            .map(|e| format!("{}: {}", e.path, e.message))
            .collect()
    }

    #[test]
    fn reports_like_typebox() {
        assert_eq!(messages(&json!([])), vec!["root: must be object"]);
        assert_eq!(
            messages(&json!({})),
            vec!["providers: must have required properties providers"]
        );
        assert_eq!(
            messages(&json!({"providers": {"a": {"models": [{"name": 5}]}}})),
            vec![
                "providers.a.models.0.id: must have required properties id",
                "providers.a.models.0.name: must be string"
            ]
        );
        assert_eq!(
            messages(&json!({"providers": {"a": {"models": [{"id": "m", "contextWindow": 0}]}}})),
            vec!["providers.a.models.0.contextWindow: must be > 0"]
        );
    }
}
