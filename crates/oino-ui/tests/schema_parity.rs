//! Schema parity (WP-U1 acceptance): the exported `schema/node.schema.json` must
//! stay identical to the Appendix A block in the spec document (normalized =
//! structural JSON equality), and the registry envelopes must validate the
//! shared `fixtures/registry/*.json` seeds (WP-U5 / WP-U9 reuse them).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]
use std::fs;

use serde_json::Value as Json;

const SPEC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/dev/v2/specs/ui-protocol.md"
);

fn schema_file(name: &str) -> Json {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/schema/").to_owned() + name;
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {path}: {e}"))
}

/// Extracts the Appendix A fenced JSON block from the spec document.
fn appendix_a_from_spec() -> Json {
    let doc = fs::read_to_string(SPEC).unwrap_or_else(|e| panic!("read {SPEC}: {e}"));
    let marker = "## Appendix A — JSON Schema";
    let start = doc
        .find(marker)
        .unwrap_or_else(|| panic!("spec is missing {marker}"));
    let rest = &doc[start..];
    let fence = rest
        .find("```json")
        .unwrap_or_else(|| panic!("no ```json after Appendix A"));
    let body = &rest[fence + "```json".len()..];
    let end = body
        .find("```")
        .unwrap_or_else(|| panic!("unterminated json fence in Appendix A"));
    let raw = body[..end].trim();
    serde_json::from_str(raw).unwrap_or_else(|e| panic!("Appendix A block is not valid JSON: {e}"))
}

#[test]
fn exported_node_schema_equals_appendix_a() {
    let exported = schema_file("node.schema.json");
    let appendix = appendix_a_from_spec();
    assert_eq!(
        exported, appendix,
        "schema/node.schema.json drifted from the spec's Appendix A"
    );
}

#[test]
fn exported_schemas_parse() {
    let node = schema_file("node.schema.json");
    let registry = schema_file("registry.schema.json");
    assert_eq!(
        node["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        registry["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(node["title"], "Oino UI protocol v2 node");
}

/// The registry schema references `node.schema.json` relatively; merge the node
/// `$defs` in and rewrite the refs so a plain validator can check the fixtures.
fn merged_registry_schema() -> Json {
    let mut registry = schema_file("registry.schema.json");
    let node_defs = schema_file("node.schema.json")["$defs"]
        .as_object()
        .unwrap_or_else(|| panic!("node defs"))
        .clone();
    let defs = registry
        .get_mut("$defs")
        .and_then(|d| d.as_object_mut())
        .unwrap_or_else(|| panic!("registry defs"));
    for (key, value) in node_defs {
        defs.entry(key).or_insert(value);
    }
    rewrite_refs(&mut registry);
    registry
}

fn rewrite_refs(value: &mut Json) {
    match value {
        Json::String(s) => {
            if s == "node.schema.json" {
                "#/$defs/Node".clone_into(s);
            } else if let Some(target) = s.strip_prefix("node.schema.json#/$defs/") {
                *s = format!("#/$defs/{target}");
            }
        }
        Json::Array(items) => {
            for item in items {
                rewrite_refs(item);
            }
        }
        Json::Object(map) => {
            for (_key, item) in map.iter_mut() {
                rewrite_refs(item);
            }
        }
        _ => {}
    }
}

#[test]
fn registry_fixtures_validate() {
    let validator = jsonschema::validator_for(&merged_registry_schema())
        .unwrap_or_else(|e| panic!("merged registry schema compiles: {e}"));
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/registry");
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {dir}: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert!(
        names.contains(&"snapshot.json".to_owned()) && names.contains(&"patch.json".to_owned()),
        "shared registry fixtures missing (used by WP-U5 / WP-U9)"
    );
    for name in names {
        let raw = fs::read_to_string(format!("{dir}/{name}")).unwrap_or_else(|e| panic!("{e}"));
        let value: Json = serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{name}: {e}"));
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| format!("{} (at {})", e, e.instance_path))
            .collect();
        assert!(errors.is_empty(), "registry fixture {name}: {errors:?}");
    }
}
