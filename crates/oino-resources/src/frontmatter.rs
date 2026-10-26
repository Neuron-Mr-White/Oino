//! YAML frontmatter (pi `utils/frontmatter.ts`).

use serde_json::{Map, Value};

/// Parsed frontmatter and body.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frontmatter {
    /// Frontmatter fields (empty when there is none or it is not a mapping).
    pub fields: Map<String, Value>,
    /// Body after the closing `---`, trimmed; the whole (normalised) text without frontmatter.
    pub body: String,
}

/// Invalid YAML between the `---` fences.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct FrontmatterError {
    /// Parser message.
    pub message: String,
}

/// pi `parseFrontmatter`: strip a BOM, normalise newlines, and split `---\n…\n---` frontmatter.
///
/// # Errors
///
/// [`FrontmatterError`] when the frontmatter is not valid YAML.
pub fn parse_frontmatter(content: &str) -> Result<Frontmatter, FrontmatterError> {
    let normalized = normalize_newlines(content.strip_prefix('\u{feff}').unwrap_or(content));
    let Some((yaml, body)) = split_frontmatter(&normalized) else {
        return Ok(Frontmatter {
            fields: Map::new(),
            body: normalized,
        });
    };
    if yaml.is_empty() {
        return Ok(Frontmatter {
            fields: Map::new(),
            body,
        });
    }
    let parsed: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(yaml).map_err(|error| FrontmatterError {
            message: error.to_string(),
        })?;
    let fields = match yaml_to_json(parsed) {
        Value::Object(fields) => fields,
        _ => Map::new(),
    };
    Ok(Frontmatter { fields, body })
}

/// pi `stripFrontmatter`: the body only (the whole text when the frontmatter is invalid is
/// **not** returned; invalid frontmatter is an error like in pi).
///
/// # Errors
///
/// See [`parse_frontmatter`].
pub fn strip_frontmatter(content: &str) -> Result<String, FrontmatterError> {
    parse_frontmatter(content).map(|parsed| parsed.body)
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

/// `(yaml, body)` when `text` starts with `---` and a later line starts with `---`.
fn split_frontmatter(text: &str) -> Option<(&str, String)> {
    if !text.starts_with("---") {
        return None;
    }
    let end = text.get(3..)?.find("\n---")? + 3;
    let yaml = text.get(4..end).unwrap_or("");
    let body = text.get(end + 4..).unwrap_or("").trim().to_owned();
    Some((yaml, body))
}

fn yaml_to_json(value: serde_yaml_ng::Value) -> Value {
    use serde_yaml_ng::Value as Yaml;
    match value {
        Yaml::Null => Value::Null,
        Yaml::Bool(b) => Value::Bool(b),
        Yaml::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::from(i)
            } else if let Some(u) = n.as_u64() {
                Value::from(u)
            } else {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Value::Null, Value::Number)
            }
        }
        Yaml::String(s) => Value::String(s),
        Yaml::Sequence(items) => Value::Array(items.into_iter().map(yaml_to_json).collect()),
        Yaml::Mapping(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (yaml_key(key), yaml_to_json(value)))
                .collect(),
        ),
        Yaml::Tagged(tagged) => yaml_to_json(tagged.value),
    }
}

fn yaml_key(key: serde_yaml_ng::Value) -> String {
    match yaml_to_json(key) {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn splits_like_pi() {
        let parsed =
            parse_frontmatter("\u{feff}---\r\nname: x\r\nlist: [1, a]\r\n---\r\n\r\nBody\r\n")
                .unwrap();
        assert_eq!(
            Value::Object(parsed.fields),
            json!({"name": "x", "list": [1, "a"]})
        );
        assert_eq!(parsed.body, "Body");
    }

    #[test]
    fn missing_or_unterminated_frontmatter_is_body() {
        assert_eq!(parse_frontmatter("hi\n").unwrap().body, "hi\n");
        assert_eq!(
            parse_frontmatter("---\nname: x").unwrap().body,
            "---\nname: x"
        );
        let scalar = parse_frontmatter("---\njust text\n---\nb").unwrap();
        assert_eq!(scalar.fields.len(), 0);
        assert_eq!(scalar.body, "b");
    }

    #[test]
    fn invalid_yaml_is_an_error() {
        assert!(parse_frontmatter("---\na: [\n---\n").is_err());
    }
}
