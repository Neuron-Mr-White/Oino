//! pi v1/v2 → v3 (`migrateToCurrentVersion`), operating on raw JSON like pi does.

use std::collections::HashSet;

use oino_ai::JsonValue;
use serde_json::Value;

use crate::entry::{CURRENT_SESSION_VERSION, EntryId};

fn is_header(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("session")
}

/// v1 → v2: assign 8-hex ids in file order, link each entry to the previous one and turn
/// `firstKeptEntryIndex` (index into the whole line list, header included) into
/// `firstKeptEntryId`.
fn v1_to_v2(values: &mut [JsonValue]) {
    let mut ids: HashSet<String> = HashSet::new();
    let mut previous: Option<String> = None;
    for index in 0..values.len() {
        if is_header(&values[index]) {
            values[index]["version"] = Value::from(2);
            continue;
        }
        let id = EntryId::generate(|candidate| ids.contains(candidate)).to_string();
        ids.insert(id.clone());
        let first_kept = values[index]
            .get("firstKeptEntryIndex")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok());
        let is_compaction = values[index].get("type").and_then(Value::as_str) == Some("compaction");
        let target_id = first_kept
            .filter(|_| is_compaction)
            .and_then(|target| values.get(target))
            .filter(|target| !is_header(target))
            .and_then(|target| target.get("id").cloned());
        let Some(object) = values[index].as_object_mut() else {
            continue;
        };
        object.insert("id".into(), Value::String(id.clone()));
        object.insert(
            "parentId".into(),
            previous.map_or(Value::Null, Value::String),
        );
        if is_compaction && object.contains_key("firstKeptEntryIndex") {
            if let Some(target_id) = target_id {
                object.insert("firstKeptEntryId".into(), target_id);
            }
            object.remove("firstKeptEntryIndex");
        }
        previous = Some(id);
    }
}

/// v2 → v3: message role `hookMessage` → `custom`.
fn v2_to_v3(values: &mut [JsonValue]) {
    for value in values {
        if is_header(value) {
            value["version"] = Value::from(CURRENT_SESSION_VERSION);
            continue;
        }
        if value.get("type").and_then(Value::as_str) == Some("message")
            && value.pointer("/message/role").and_then(Value::as_str) == Some("hookMessage")
        {
            value["message"]["role"] = Value::String("custom".into());
        }
    }
}

/// Migrates raw pi lines in place; returns `true` when anything changed (pi returns whether
/// the file must be rewritten).
pub fn migrate_pi_values(values: &mut [JsonValue]) -> bool {
    let version = values
        .iter()
        .find(|value| is_header(value))
        .map_or(1, |header| {
            header.get("version").and_then(Value::as_u64).unwrap_or(1)
        });
    if version >= u64::from(CURRENT_SESSION_VERSION) {
        return false;
    }
    if version < 2 {
        v1_to_v2(values);
    }
    v2_to_v3(values);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_gets_linear_ids_and_first_kept_id() {
        let mut values = vec![
            json!({"type":"session","id":"s","timestamp":"t","cwd":"/"}),
            json!({"type":"message","timestamp":"t","message":{"role":"user","content":"a","timestamp":1}}),
            json!({"type":"message","timestamp":"t","message":{"role":"hookMessage","customType":"x","content":"h","display":true,"timestamp":1}}),
            json!({"type":"compaction","timestamp":"t","summary":"s","firstKeptEntryIndex":2,"tokensBefore":1}),
        ];
        assert!(migrate_pi_values(&mut values));
        assert_eq!(values[0]["version"], json!(3));
        assert_eq!(values[1]["parentId"], json!(null));
        assert_eq!(values[2]["parentId"], values[1]["id"]);
        assert_eq!(values[3]["firstKeptEntryId"], values[2]["id"]);
        assert!(values[3].get("firstKeptEntryIndex").is_none());
        assert_eq!(values[2]["message"]["role"], json!("custom"));
        assert!(!migrate_pi_values(&mut values));
    }
}
