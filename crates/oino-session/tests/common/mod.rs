//! Shared helpers for the oino-session integration tests.
#![allow(
    dead_code,
    unreachable_pub,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers"
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oino_ai::json::json_semantic_eq;
use oino_session::{SessionManager, SessionTreeNode};
use serde_json::{Value, json};

pub fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oino-session-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tree_json(roots: &[SessionTreeNode]) -> Value {
    let mut out = Vec::new();
    let mut stack: Vec<&SessionTreeNode> = roots.iter().rev().collect();
    while let Some(node) = stack.pop() {
        let mut item = json!({"id": node.entry.id});
        if let Some(label) = &node.label {
            item["label"] = json!(label);
        }
        if let Some(ts) = &node.label_timestamp {
            item["labelTimestamp"] = json!(ts);
        }
        item["children"] = json!(
            node.children
                .iter()
                .map(|c| c.entry.id.clone())
                .collect::<Vec<_>>()
        );
        out.push(item);
        stack.extend(node.children.iter().rev());
    }
    Value::Array(out)
}

/// Same shape as `scripts/pi-oracle/core-types-session/lib.mjs` `snapshot`.
pub fn snapshot(manager: &mut SessionManager) -> Value {
    let entries = manager.entries().to_vec();
    let parents: std::collections::HashSet<_> =
        entries.iter().filter_map(|e| e.parent_id.clone()).collect();
    let leaves: Vec<_> = entries
        .iter()
        .filter(|e| !parents.contains(&e.id))
        .map(|e| e.id.clone())
        .collect();
    let original = manager.leaf_id().cloned();
    let per_leaf: Vec<Value> = leaves
        .iter()
        .map(|leaf| {
            manager.set_leaf(leaf).unwrap();
            let projection = manager.build_session_projection();
            json!({
                "leaf": leaf,
                "branchIds": manager.branch(None).iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
                "contextEntryIds": manager.build_context_entries().iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
                "projection": projection.entries.iter().map(|p| json!({"id": p.source_entry.id, "messages": p.messages})).collect::<Vec<_>>(),
                "context": manager.build_session_context(),
            })
        })
        .collect();
    if let Some(original) = original {
        manager.set_leaf(&original).unwrap();
    }
    let labels: Vec<Value> = entries
        .iter()
        .filter_map(|e| manager.label(&e.id).map(|label| json!([e.id, label])))
        .collect();
    json!({
        "leafId": manager.leaf_id(),
        "sessionName": manager.session_name(),
        "entryCount": manager.entry_count(),
        "header": manager.header(),
        "roots": manager.tree().iter().map(|n| n.entry.id.clone()).collect::<Vec<_>>(),
        "tree": tree_json(&manager.tree()),
        "labels": labels,
        "leaves": per_leaf,
    })
}

const ID_KEYS: [&str; 8] = [
    "id",
    "parentId",
    "firstKeptEntryId",
    "targetId",
    "fromId",
    "leafId",
    "leaf",
    "toolCallId",
];
const ID_LIST_KEYS: [&str; 4] = ["branchIds", "contextEntryIds", "roots", "children"];

/// Replaces entry ids by `E1, E2, …` in order of first appearance (object keys visited in
/// sorted order so both sides traverse identically).
pub fn normalize_ids(value: &mut Value) {
    let mut map = HashMap::new();
    walk(value, None, &mut map);
}

fn rename(text: &mut String, map: &mut HashMap<String, String>) {
    let next = map.len() + 1;
    let renamed = map
        .entry(text.clone())
        .or_insert_with(|| format!("E{next}"));
    text.clone_from(renamed);
}

fn walk(value: &mut Value, key: Option<&str>, map: &mut HashMap<String, String>) {
    match value {
        Value::String(text) if key.is_some_and(|k| ID_KEYS.contains(&k)) => rename(text, map),
        Value::Array(items) => {
            let is_id_list = key.is_some_and(|k| ID_LIST_KEYS.contains(&k));
            let is_label_pair = key == Some("labels");
            for item in items {
                match item {
                    Value::String(text) if is_id_list => rename(text, map),
                    Value::Array(pair) if is_label_pair => {
                        if let Some(Value::String(text)) = pair.first_mut() {
                            rename(text, map);
                        }
                    }
                    other => walk(other, key, map),
                }
            }
        }
        Value::Object(object) => {
            let mut keys: Vec<String> = object.keys().cloned().collect();
            keys.sort();
            for k in keys {
                if let Some(child) = object.get_mut(&k) {
                    walk(child, Some(&k), map);
                }
            }
        }
        _ => {}
    }
}

/// First differing JSON path, for readable failures.
pub fn first_diff(left: &Value, right: &Value, path: &str) -> Option<String> {
    if json_semantic_eq(left, right) {
        return None;
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            for key in a.keys().chain(b.keys()) {
                let (x, y) = (
                    a.get(key).unwrap_or(&Value::Null),
                    b.get(key).unwrap_or(&Value::Null),
                );
                if a.contains_key(key) != b.contains_key(key) {
                    return Some(format!("{path}/{key}: present only on one side: {x} | {y}"));
                }
                if let Some(diff) = first_diff(x, y, &format!("{path}/{key}")) {
                    return Some(diff);
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| first_diff(x, y, &format!("{path}/{i}"))),
        _ => {
            let show = |v: &Value| v.to_string().chars().take(300).collect::<String>();
            Some(format!("{path}: {} != {}", show(left), show(right)))
        }
    }
}
