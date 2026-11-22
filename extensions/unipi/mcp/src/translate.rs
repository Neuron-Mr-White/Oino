//! MCP tool → Oino tool (unipi `src/bridge/translator.ts`).

use std::cmp::Ordering;

use serde_json::{Map, Value, json};
use unipi_kit::bounded::{self, BoundedOutput};

/// unipi `MCP_DEFAULTS.TOOL_NAME_SEPARATOR`.
pub(crate) const SEPARATOR: &str = "__";
/// unipi `MCP_DEFAULTS.MAX_MODEL_OUTPUT_BYTES`.
pub(crate) const MAX_MODEL_OUTPUT_BYTES: usize = 64 * 1024;

/// unipi `compareCodeUnits` (UTF-16 code-unit order, locale independent).
pub(crate) fn compare_code_units(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

const LITERAL_VALUE_KEYWORDS: [&str; 4] = ["const", "default", "enum", "examples"];

fn canonical_value(value: &Value, key: Option<&str>, normalize: bool) -> Value {
    match value {
        Value::Array(items) => {
            if normalize && key == Some("required") && items.iter().all(Value::is_string) {
                let mut names: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
                names.sort_by(|a, b| compare_code_units(a, b));
                names.dedup();
                return json!(names);
            }
            Value::Array(
                items
                    .iter()
                    .map(|i| canonical_value(i, None, normalize))
                    .collect(),
            )
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| compare_code_units(a, b));
            let mut out = Map::new();
            for k in keys {
                let child_normalize = normalize && !LITERAL_VALUE_KEYWORDS.contains(&k.as_str());
                out.insert(
                    k.clone(),
                    canonical_value(&map[k], Some(k), child_normalize),
                );
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// unipi `canonicalizeJsonSchema`.
pub(crate) fn canonicalize(schema: &Value) -> Value {
    canonical_value(schema, None, true)
}

/// Host-safe tool-name characters (letters, digits, `_`, `-`).
pub(crate) fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `<server>__<tool>`.
pub(crate) fn tool_name(server: &str, tool: &str) -> String {
    sanitize(&format!("{server}{SEPARATOR}{tool}"))
}

/// Translated tool definition.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolDef {
    /// Final name.
    pub name: String,
    /// MCP tool name.
    pub mcp_name: String,
    /// Description with the server tag.
    pub description: String,
    /// Canonical parameters (`{properties, required, type}`).
    pub parameters: Value,
    /// MCP annotations (`readOnlyHint`, …).
    pub annotations: Option<Value>,
}

/// unipi `translateMcpTool` (definition part).
pub(crate) fn translate(server: &str, tool: &Value) -> Option<ToolDef> {
    let mcp_name = tool.get("name")?.as_str()?.to_owned();
    let input = tool
        .get("inputSchema")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let properties = input
        .get("properties")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let required = input
        .get("required")
        .filter(|v| v.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let parameters =
        canonicalize(&json!({"type": "object", "properties": properties, "required": required}));
    let desc = tool
        .get("description")
        .and_then(Value::as_str)
        .filter(|d| !d.is_empty())
        .map_or_else(|| format!("MCP tool: {mcp_name}"), ToOwned::to_owned);
    Some(ToolDef {
        name: tool_name(server, &mcp_name),
        description: format!("{desc} [Server: {server}]"),
        mcp_name,
        parameters,
        annotations: tool.get("annotations").filter(|a| a.is_object()).cloned(),
    })
}

/// Model-facing content blocks of a `tools/call` result plus the text that is bounded.
pub(crate) fn result_blocks(result: &Value) -> (String, Vec<Value>) {
    let mut texts: Vec<String> = Vec::new();
    let mut images: Vec<Value> = Vec::new();
    for block in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                {
                    texts.push(t.to_owned());
                }
            }
            Some("image") => {
                if let Some(data) = block.get("data").and_then(Value::as_str) {
                    let mime = block
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    texts.push(format!("[Image: {mime}]"));
                    images.push(json!({"type": "image", "data": data, "mimeType": mime}));
                }
            }
            Some("resource") => {
                let res = block.get("resource").unwrap_or(block);
                let label = res
                    .get("text")
                    .and_then(Value::as_str)
                    .or_else(|| block.get("text").and_then(Value::as_str))
                    .or_else(|| res.get("mimeType").and_then(Value::as_str))
                    .unwrap_or("unknown");
                texts.push(format!("[Resource: {label}]"));
            }
            Some("resource_link") => {
                let uri = block
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                texts.push(format!("[Resource: {uri}]"));
            }
            _ => {}
        }
    }
    let is_error = result.get("isError").and_then(Value::as_bool) == Some(true);
    if texts.is_empty() {
        texts.push(
            if is_error {
                "Unknown error"
            } else {
                "(no output)"
            }
            .to_owned(),
        );
    }
    (texts.join("\n"), images)
}

/// unipi `execute` success path: bounded text, details.
pub(crate) fn format_result(
    server: &str,
    tool: &str,
    result: &Value,
    bound: impl FnOnce(&str, usize, &str) -> BoundedOutput,
) -> (Vec<Value>, Value) {
    let is_error = result.get("isError").and_then(Value::as_bool) == Some(true);
    let (raw, images) = result_blocks(result);
    let wrapper = if is_error {
        format!("MCP tool error from {server}: ")
    } else {
        String::new()
    };
    let max = MAX_MODEL_OUTPUT_BYTES
        .saturating_sub(wrapper.len())
        .max(1024);
    let out = bound(&raw, max, &format!("mcp-{server}-{tool}"));
    let mut content = vec![json!({"type": "text", "text": format!("{wrapper}{}", out.text)})];
    content.extend(images);
    let mut details = json!({
        "server": server,
        "tool": tool,
        "truncated": out.truncated,
        "originalBytes": out.original_bytes,
    });
    if is_error {
        details["error"] = json!(true);
    }
    if let Some(p) = out.artifact_path {
        details["artifactPath"] = json!(p);
    }
    if let Some(sc) = result.get("structuredContent") {
        details["structuredContent"] = sc.clone();
    }
    (content, details)
}

/// Bounds through `unipi_kit::bounded` (artifact in `/tmp/tool-results`).
pub(crate) fn bound_kit(text: &str, max: usize, prefix: &str) -> BoundedOutput {
    bounded::bound_model_output(
        text,
        &bounded::Options {
            max_bytes: max,
            artifact_prefix: prefix,
        },
    )
}

/// unipi failure text.
pub(crate) fn failure_text(server: &str, tool: &str, message: &str) -> String {
    format!(
        "MCP tool \"{tool}\" on server \"{server}\" failed: {message}\nConfigure servers via /settings (MCP)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_like_unipi() {
        let schema = json!({
            "z": 1,
            "required": ["z", "a", "z"],
            "properties": {
                "list": {"anyOf": [{"required": ["b", "a"]}, {"enum": ["z", "a", "z"]}]},
                "invalid": {"required": ["b", 1, "a"]},
            },
            "const": {"required": ["z", "a"]},
            "examples": [{"required": ["z", "a"], "b": 1, "a": 2}],
        });
        let original = schema.clone();
        let c = canonicalize(&schema);
        assert_eq!(schema, original, "input not mutated");
        let keys: Vec<&String> = c
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        assert_eq!(keys, ["const", "examples", "properties", "required", "z"]);
        assert_eq!(c["required"], json!(["a", "z"]));
        assert_eq!(
            c["properties"]["list"]["anyOf"][0]["required"],
            json!(["a", "b"])
        );
        assert_eq!(
            c["properties"]["list"]["anyOf"][1]["enum"],
            json!(["z", "a", "z"])
        );
        assert_eq!(c["properties"]["invalid"]["required"], json!(["b", 1, "a"]));
        assert_eq!(c["const"]["required"], json!(["z", "a"]));
        assert_eq!(c["examples"][0]["required"], json!(["z", "a"]));
        let ex_keys: Vec<&String> = c["examples"][0]
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        assert_eq!(ex_keys, ["a", "b", "required"]);
    }

    #[test]
    fn code_unit_order() {
        let mut v = vec!["😀", "a", "Z", "ä", "A", "\u{FFFF}"];
        v.sort_by(|a, b| compare_code_units(a, b));
        assert_eq!(
            v,
            ["A", "Z", "a", "ä", "😀", "\u{FFFF}"],
            "surrogates sort below U+FFFF like JS"
        );
    }

    #[test]
    fn translated_definition() {
        let t = json!({"name": "find", "inputSchema": {"type": "object", "properties": {"z": {"type": "string"}, "a": {"type": "number"}}, "additionalProperties": false}});
        let d = translate("search", &t).unwrap_or_else(|| unreachable!());
        assert_eq!(d.name, "search__find");
        let keys: Vec<&String> = d
            .parameters
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        assert_eq!(keys, ["properties", "required", "type"]);
        let props: Vec<&String> = d.parameters["properties"]
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        assert_eq!(props, ["a", "z"]);
        assert_eq!(d.parameters["required"], json!([]));
        assert_eq!(d.description, "MCP tool: find [Server: search]");
        let described = translate("s", &json!({"name": "x.y", "description": "Does X"}))
            .unwrap_or_else(|| unreachable!());
        assert_eq!(described.description, "Does X [Server: s]");
        assert_eq!(described.name, "s__x_y", "names are host-safe");
        assert!(translate("s", &json!({})).is_none());
    }

    #[test]
    fn result_mapping() {
        let r = json!({"content": [
            {"type": "text", "text": "hello"},
            {"type": "image", "data": "AAA", "mimeType": "image/png"},
            {"type": "resource", "resource": {"uri": "file:///x", "mimeType": "text/plain"}},
        ]});
        let (text, images) = result_blocks(&r);
        assert_eq!(text, "hello\n[Image: image/png]\n[Resource: text/plain]");
        assert_eq!(images.len(), 1);
        assert_eq!(result_blocks(&json!({})).0, "(no output)");
        assert_eq!(
            result_blocks(&json!({"isError": true, "content": []})).0,
            "Unknown error"
        );
    }

    #[test]
    fn bounds_large_output_and_flags_errors() {
        let raw = "x".repeat(200_000);
        let r = json!({"content": [{"type": "text", "text": raw}]});
        let (content, details) = format_result("srv", "t", &r, |text, max, prefix| {
            bounded::bound_with(
                text,
                &bounded::Options {
                    max_bytes: max,
                    artifact_prefix: prefix,
                },
                |_p, _full| Ok("/host/tmp/a.txt".into()),
            )
        });
        let text = content[0]["text"].as_str().unwrap_or_default();
        assert!(text.len() < 66_000);
        assert!(text.contains("Full output: /host/tmp/a.txt"));
        assert_eq!(details["truncated"], true);
        assert_eq!(details["originalBytes"], 200_000);
        assert_eq!(details["artifactPath"], "/host/tmp/a.txt");
        let err = json!({"isError": true, "content": [{"type": "text", "text": "nope"}]});
        let (content, details) = format_result("srv", "t", &err, |text, max, prefix| {
            bounded::bound_with(
                text,
                &bounded::Options {
                    max_bytes: max,
                    artifact_prefix: prefix,
                },
                |_p, _f| Err("x".into()),
            )
        });
        assert_eq!(content[0]["text"], "MCP tool error from srv: nope");
        assert_eq!(details["error"], true);
        assert_eq!(
            failure_text("srv", "t", "boom"),
            "MCP tool \"t\" on server \"srv\" failed: boom\nConfigure servers via /settings (MCP)"
        );
    }
}
