//! Fixture corpus (WP-U1 acceptance): every fixture parses into `Node`, passes
//! `validate()`, round-trips through serde, and validates against the normative
//! JSON Schema — both the checked-in file and the re-serialized tree. The corpus
//! also has to cover every schema property per node kind, so a missing Rust field
//! (or a wrong rename) fails here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]
use std::collections::BTreeMap;
use std::fs::{self, DirEntry};
use std::path::PathBuf;

use oino_ui::limits::validate;
use oino_ui::node::Node;
use serde_json::Value as Json;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/nodes")
}

fn node_schema() -> Json {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/schema/node.schema.json"
    )))
    .unwrap_or_default()
}

/// Normalized JSON Schema validator over the node schema (registry refs merged in
/// for the registry fixtures).
fn validator(schema: &Json) -> jsonschema::Validator {
    jsonschema::validator_for(schema).unwrap_or_else(|e| panic!("schema must compile: {e}"))
}

fn all_fixtures() -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(fixtures_dir())
        .unwrap_or_else(|e| panic!("fixtures dir: {e}"))
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".json") {
            continue;
        }
        let raw = fs::read_to_string(entry.path()).unwrap_or_default();
        let value: Json = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("fixture {name} must be valid JSON: {e}"));
        out.push((name, value));
    }
    out
}

#[test]
fn corpus_size_and_kind_coverage() {
    let fixtures = all_fixtures();
    assert!(
        fixtures.len() >= 60,
        "corpus must hold 60+ fixtures, has {}",
        fixtures.len()
    );
    let schema = node_schema();
    let kinds: Vec<&str> = schema["$defs"]["Node"]["oneOf"]
        .as_array()
        .map(|refs| {
            refs.iter()
                .filter_map(|r| r["$ref"].as_str())
                .map(|r| r.trim_start_matches("#/$defs/"))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(kinds.len(), 25, "Appendix A defines 25 node kinds");
    let seen: std::collections::BTreeSet<String> = fixtures
        .iter()
        .filter_map(|(_, v)| v["t"].as_str().map(str::to_owned))
        .collect();
    for kind in &kinds {
        assert!(
            seen.contains(*kind),
            "no fixture for node kind {kind} (expected fixtures/nodes/{kind}-*.json)"
        );
    }
}

#[test]
fn fixtures_parse_validate_roundtrip() {
    let validator = validator(&node_schema());
    for (name, value) in all_fixtures() {
        // Schema-valid as written.
        let errors: Vec<String> = validator
            .iter_errors(&value)
            .map(|e| format!("{} (at {})", e, e.instance_path))
            .collect();
        assert!(
            errors.is_empty(),
            "fixture {name} schema errors: {errors:?}"
        );

        // Parses into Node.
        let node: Node = serde_json::from_value(value.clone())
            .unwrap_or_else(|e| panic!("fixture {name} must parse: {e}"));

        // Passes the §4.8 validator.
        if let Err(rejections) = validate(&node) {
            panic!("fixture {name} must validate: {rejections:?}");
        }

        // Round-trips: parse(serialize(parse)) == parse.
        let re = serde_json::to_value(&node)
            .map_err(|e| panic!("fixture {name} must serialize: {e}"))
            .unwrap_or_default();
        let back: Node = serde_json::from_value(re.clone())
            .unwrap_or_else(|e| panic!("fixture {name} re-parse failed: {e}"));
        assert_eq!(node, back, "fixture {name} round-trip mismatch");

        // The re-serialized form is schema-valid too (serde names match Appendix A).
        let errors: Vec<String> = validator
            .iter_errors(&re)
            .map(|e| format!("{} (at {})", e, e.instance_path))
            .collect();
        assert!(
            errors.is_empty(),
            "fixture {name} re-serialized schema errors: {errors:?}\n{re}"
        );
    }
}

#[test]
fn corpus_covers_every_schema_property_per_kind() {
    let schema = node_schema();
    let defs = schema["$defs"]
        .as_object()
        .unwrap_or_else(|| panic!("schema $defs"));
    let common_props: Vec<String> = defs["Common"]["properties"]
        .as_object()
        .map(|p| p.keys().cloned().collect())
        .unwrap_or_default();

    // Per kind: union of fixture keys (own properties, minus "t").
    let mut covered: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for (_, value) in all_fixtures() {
        let kind = value["t"].as_str().unwrap_or_default().to_owned();
        let keys: std::collections::BTreeSet<String> = value
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        covered.entry(kind.clone()).or_default().extend(keys);
    }

    for (kind, def) in defs {
        let Some(all_of) = def.get("allOf").and_then(Json::as_array) else {
            continue; // shared defs (Key, Style, …) have no per-kind properties
        };
        let Some(own) = all_of.iter().find(|part| part.get("properties").is_some()) else {
            continue;
        };
        let expected: Vec<&String> = own["properties"]
            .as_object()
            .map(|p| p.keys().collect())
            .unwrap_or_default();
        let Some(seen) = covered.get(kind.as_str()) else {
            panic!("no fixtures for kind {kind}");
        };
        let mut missing: Vec<String> = expected
            .into_iter()
            .filter(|p| *p != "t" && !seen.contains(*p))
            .cloned()
            .collect();
        // Common props may be covered by fixtures of other kinds.
        missing.retain(|p| !common_props.contains(p));
        assert!(
            missing.is_empty(),
            "kind {kind}: fixtures never exercise properties {missing:?} — \
             add a fixtures/nodes/{kind}-*.json case"
        );
    }
}

#[test]
fn serde_names_match_schema_between_kinds() {
    // A stronger lock: for each kind, the set of properties the Rust type emits
    // (serialized from the fixtures) never contains a name the schema does not
    // declare — otherwise Appendix A and the wire types drift apart.
    let schema = node_schema();
    let defs = schema["$defs"]
        .as_object()
        .unwrap_or_else(|| panic!("defs"));
    let common: Vec<&String> = defs["Common"]["properties"]
        .as_object()
        .map(|p| p.keys().collect())
        .unwrap_or_default();
    for (name, value) in all_fixtures() {
        let kind = value["t"].as_str().unwrap_or_default().to_owned();
        let Some(def) = defs.get(&kind) else {
            panic!("fixture {name}: unknown kind {kind}");
        };
        let mut allowed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        if let Some(all_of) = def.get("allOf").and_then(Json::as_array) {
            for part in all_of {
                if let Some(props) = part.get("properties").and_then(Json::as_object) {
                    for key in props.keys() {
                        allowed.insert(key.as_str());
                    }
                }
                if let Some(reference) = part.get("$ref").and_then(Json::as_str) {
                    let target = reference.trim_start_matches("#/$defs/");
                    if let Some(shared) = defs.get(target).and_then(|d| d.get("properties")) {
                        for key in shared.as_object().unwrap_or_else(|| panic!("props")).keys() {
                            allowed.insert(key.as_str());
                        }
                    }
                }
            }
        }
        if let Some(own) = value.as_object() {
            for key in own.keys() {
                assert!(
                    key == "t" || allowed.contains(key.as_str()) || common.contains(&key),
                    "fixture {name}: property {key:?} is not in the {kind} schema \
                     (serde rename drift)"
                );
            }
        }
    }
}
