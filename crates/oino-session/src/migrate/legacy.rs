//! Pre-v1 Oino session files (`{"record":"header"|"entry",…}`) → v3 (session-v3 §5.2).
//!
//! The mapping is pure: [`migrate_oino_legacy_text`] takes the file text and the file mtime
//! and returns v3 lines. Legacy files are `BTreeMap` dumps ordered by UUID, so entries are
//! re-ordered parents-first (pre-order, siblings in file order) before timestamps are
//! assigned; see `deviations/core-types-session.md`.

use std::collections::{HashMap, HashSet};

use oino_ai::{AgentMessage, JsonObject};
use serde_json::{Value, json};

use crate::context::{EntryIndex, build_session_projection, current_system_message};
use crate::entry::{
    CURRENT_SESSION_VERSION, EntryId, EntryKind, IsoTimestamp, SessionEntry, SessionHeader,
};
use crate::error::SessionError;

/// Non-fatal problem found while migrating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationWarning {
    /// 1-based line in the legacy file (0 when not line specific).
    pub line: usize,
    /// What was dropped or approximated.
    pub message: String,
}

/// Result of mapping a legacy file.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyMigration {
    /// v3 header.
    pub header: SessionHeader,
    /// v3 entries (ending with the leaf-restoring `session_info`).
    pub entries: Vec<SessionEntry>,
    /// Dropped/approximated records.
    pub warnings: Vec<MigrationWarning>,
    /// Whether any `message` entry exists (sessions without messages are not migrated).
    pub has_messages: bool,
}

struct LegacyEntry {
    line: usize,
    uuid: String,
    parent: Option<String>,
    kind: Value,
}

struct Legacy {
    session_id: String,
    name: String,
    cwd: String,
    leaf_id: Option<String>,
    entries: Vec<LegacyEntry>,
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn parse_records(text: &str, warnings: &mut Vec<MigrationWarning>) -> Result<Legacy, SessionError> {
    let mut legacy = Legacy {
        session_id: String::new(),
        name: String::new(),
        cwd: String::new(),
        leaf_id: None,
        entries: Vec::new(),
    };
    let mut saw_header = false;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            warnings.push(MigrationWarning {
                line: line_number,
                message: "malformed JSON line dropped".into(),
            });
            continue;
        };
        match str_field(&record, "record") {
            Some("header") if !saw_header => {
                saw_header = true;
                let header = record.get("header").cloned().unwrap_or(Value::Null);
                legacy.session_id = header
                    .get("session_id")
                    .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_owned))
                    .unwrap_or_default();
                str_field(&header, "name")
                    .unwrap_or_default()
                    .clone_into(&mut legacy.name);
                str_field(&header, "cwd")
                    .unwrap_or_default()
                    .clone_into(&mut legacy.cwd);
                legacy.leaf_id = str_field(&record, "leaf_id").map(str::to_owned);
            }
            Some("entry") => {
                let entry = record.get("entry");
                let uuid = entry.and_then(|e| str_field(e, "id"));
                let kind = entry
                    .and_then(|e| e.get("kind"))
                    .filter(|k| k.get("kind").is_some());
                match (uuid, kind) {
                    (Some(uuid), Some(kind)) => legacy.entries.push(LegacyEntry {
                        line: line_number,
                        uuid: uuid.to_owned(),
                        parent: entry
                            .and_then(|e| str_field(e, "parent"))
                            .map(str::to_owned),
                        kind: kind.clone(),
                    }),
                    _ => warnings.push(MigrationWarning {
                        line: line_number,
                        message: "entry without id/kind dropped".into(),
                    }),
                }
            }
            _ => warnings.push(MigrationWarning {
                line: line_number,
                message: "unknown record dropped".into(),
            }),
        }
    }
    if !saw_header || legacy.session_id.is_empty() {
        return Err(SessionError::MissingHeader(std::path::PathBuf::new()));
    }
    Ok(legacy)
}

/// Pre-order (parents first, siblings in file order); orphans and cycle members become roots.
fn topological(entries: Vec<LegacyEntry>) -> Vec<LegacyEntry> {
    let known: HashSet<String> = entries.iter().map(|e| e.uuid.clone()).collect();
    let mut children: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        let parent = entry
            .parent
            .clone()
            .filter(|p| known.contains(p) && *p != entry.uuid);
        children.entry(parent).or_default().push(index);
    }
    let mut order = Vec::with_capacity(entries.len());
    let mut visited = vec![false; entries.len()];
    let mut stack: Vec<usize> = children.get(&None).cloned().unwrap_or_default();
    stack.reverse();
    loop {
        while let Some(index) = stack.pop() {
            if std::mem::replace(&mut visited[index], true) {
                continue;
            }
            order.push(index);
            if let Some(kids) = children.get(&Some(entries[index].uuid.clone())) {
                stack.extend(kids.iter().rev());
            }
        }
        match visited.iter().position(|seen| !seen) {
            Some(index) => stack.push(index),
            None => break,
        }
    }
    let mut slots: Vec<Option<LegacyEntry>> = entries.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

fn map_usage(usage: &Value) -> Value {
    let get = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let (input, output) = (get("input_tokens"), get("output_tokens"));
    let (cache_read, cache_write) = (get("cache_read_tokens"), get("cache_write_tokens"));
    let total = usage
        .get("cost")
        .filter(|cost| str_field(cost, "currency") == Some("USD"))
        .and_then(|cost| cost.get("amount"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let mut mapped = json!({"input": input, "output": output, "cacheRead": cache_read,
        "cacheWrite": cache_write});
    if let Some(write_1h) = usage.get("cache_write_1h_tokens").and_then(Value::as_u64) {
        mapped["cacheWrite1h"] = json!(write_1h);
    }
    mapped["reasoning"] = json!(get("reasoning_tokens"));
    mapped["totalTokens"] = json!(input + output + cache_read + cache_write);
    mapped["cost"] =
        json!({"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": total});
    mapped
}

fn zero_usage() -> Value {
    json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
        "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}})
}

/// Outcome of mapping one legacy message.
enum Mapped {
    /// pi message JSON.
    Message(Value),
    /// The message becomes a non-message entry (non-model-visible custom).
    CustomEntry(Box<EntryKind>),
    /// Dropped (warning recorded).
    Dropped,
}

struct Mapper<'a> {
    ids: HashMap<String, EntryId>,
    message_ids: HashMap<String, EntryId>,
    tool_ids: HashMap<String, String>,
    warnings: &'a mut Vec<MigrationWarning>,
}

impl Mapper<'_> {
    fn warn(&mut self, line: usize, message: impl Into<String>) {
        self.warnings.push(MigrationWarning {
            line,
            message: message.into(),
        });
    }

    /// `assistant`: `Some(true)` assistant blocks, `Some(false)` user blocks, `None` any.
    fn map_block(&mut self, block: &Value, assistant: Option<bool>, line: usize) -> Option<Value> {
        match str_field(block, "kind") {
            Some("text") => {
                let mut out =
                    json!({"type": "text", "text": str_field(block, "text").unwrap_or_default()});
                if let Some(signature) = str_field(block, "signature") {
                    out["textSignature"] = json!(signature);
                }
                Some(out)
            }
            Some("image") if assistant != Some(true) => Some(json!({"type": "image",
                "data": str_field(block, "data").unwrap_or_default(),
                "mimeType": str_field(block, "media_type").unwrap_or_default()})),
            Some("thinking") if assistant != Some(false) => {
                let mut out = json!({"type": "thinking", "thinking": str_field(block, "text").unwrap_or_default()});
                let signature = str_field(block, "signature")
                    .or_else(|| block.get("provider_data").and_then(Value::as_str));
                if let Some(signature) = signature {
                    out["thinkingSignature"] = json!(signature);
                }
                if block.get("redacted").and_then(Value::as_bool) == Some(true) {
                    out["redacted"] = json!(true);
                }
                Some(out)
            }
            Some("tool_call") if assistant != Some(false) => {
                let legacy_id = block
                    .get("id")
                    .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_owned))
                    .unwrap_or_default();
                let id =
                    str_field(block, "native_id").map_or_else(|| legacy_id.clone(), str::to_owned);
                self.tool_ids.insert(legacy_id, id.clone());
                let mut out = json!({"type": "toolCall", "id": id, "name": str_field(block, "name").unwrap_or_default()});
                if let Some(Value::Object(arguments)) = block.get("arguments") {
                    out["arguments"] = Value::Object(arguments.clone());
                } else {
                    out["arguments"] = json!({});
                    out["argumentsError"] = json!("legacy arguments were not an object");
                }
                Some(out)
            }
            other => {
                self.warn(line, format!("content block {other:?} dropped"));
                None
            }
        }
    }

    fn map_blocks(
        &mut self,
        blocks: Option<&Value>,
        assistant: Option<bool>,
        line: usize,
    ) -> Value {
        Value::Array(
            blocks
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|block| self.map_block(block, assistant, line))
                .collect(),
        )
    }

    /// Maps a legacy message to a pi message value, a `custom` entry for non-model-visible
    /// customs, or drops it.
    fn map_message(&mut self, message: &Value, ts: i64, line: usize) -> Mapped {
        match str_field(message, "kind") {
            Some("system") => {
                let mut out = json!({"role": "system", "content": str_field(message, "content").unwrap_or_default()});
                let sections: JsonObject = message
                    .get("sections")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|s| {
                        Some((
                            str_field(s, "name")?.to_owned(),
                            s.get("text").cloned().unwrap_or(Value::Null),
                        ))
                    })
                    .collect();
                if !sections.is_empty() {
                    out["sections"] = Value::Object(sections);
                }
                let added: Vec<Value> = message
                    .get("tools_added")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|t| {
                        json!({"name": str_field(t, "name").unwrap_or_default(),
                        "description": str_field(t, "description").unwrap_or_default(),
                        "parameters": t.get("input_schema").cloned().unwrap_or_else(|| json!({}))})
                    })
                    .collect();
                if !added.is_empty() {
                    out["toolsAdded"] = Value::Array(added);
                }
                let removed: Vec<Value> = message
                    .get("tools_removed")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|name| json!({"name": name}))
                    .collect();
                if !removed.is_empty() {
                    out["toolsRemoved"] = Value::Array(removed);
                }
                out["timestamp"] = json!(ts);
                Mapped::Message(out)
            }
            Some("user") => Mapped::Message(json!({"role": "user",
                "content": self.map_blocks(message.get("content"), Some(false), line), "timestamp": ts})),
            Some("assistant") => Mapped::Message(self.map_assistant(message, ts, line)),
            Some("tool_result") => {
                let legacy_call = message
                    .get("tool_call_id")
                    .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_owned))
                    .unwrap_or_default();
                let call_id = self
                    .tool_ids
                    .get(&legacy_call)
                    .cloned()
                    .unwrap_or(legacy_call);
                let mut out = json!({"role": "toolResult", "toolCallId": call_id,
                    "toolName": str_field(message, "tool_name").unwrap_or_default(),
                    "content": self.map_blocks(message.get("content"), Some(false), line)});
                if let Some(details) = message.get("details").filter(|d| !d.is_null()) {
                    out["details"] = details.clone();
                }
                if let Some(usage) = message.get("usage").filter(|u| u.is_object()) {
                    out["usage"] = map_usage(usage);
                }
                out["isError"] = json!(
                    message
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                );
                out["timestamp"] = json!(ts);
                Mapped::Message(out)
            }
            Some("custom") => map_custom(message, ts),
            other => {
                self.warn(line, format!("legacy message kind {other:?} dropped"));
                Mapped::Dropped
            }
        }
    }

    fn map_assistant(&mut self, message: &Value, ts: i64, line: usize) -> Value {
        let model = message.pointer("/provider/model");
        let mut out = json!({"role": "assistant",
            "content": self.map_blocks(message.get("content"), Some(true), line),
            "api": "oino-legacy",
            "provider": model.and_then(|m| str_field(m, "provider")).unwrap_or("unknown"),
            "model": model.and_then(|m| str_field(m, "name")).unwrap_or("unknown")});
        if let Some(response_model) = str_field(message, "response_model") {
            out["responseModel"] = json!(response_model);
        }
        if let Some(request_id) = message
            .pointer("/provider/request_id")
            .and_then(Value::as_str)
        {
            out["responseId"] = json!(request_id);
        }
        if let Some(level) = str_field(message, "provider_thinking_level") {
            out["providerThinkingLevel"] = json!(level);
        }
        out["usage"] = message
            .get("usage")
            .filter(|u| u.is_object())
            .map_or_else(zero_usage, map_usage);
        let (stop, raw) = match str_field(message, "stop_reason") {
            Some("end_turn") => ("stop", None),
            Some("tool_use") => ("toolUse", None),
            Some("length") => ("length", None),
            Some("error") => ("error", None),
            Some("aborted") => ("aborted", None),
            _ => ("stop", Some("unknown")),
        };
        out["stopReason"] = json!(stop);
        if let Some(error) = str_field(message, "error_message") {
            out["errorMessage"] = json!(error);
        }
        if let Some(raw) = raw {
            out["rawStopReason"] = json!(raw);
        }
        out["timestamp"] = json!(ts);
        out
    }
}

/// `Custom{name,payload,model_visible}` → custom message value (`Ok`) or `custom` entry (`Err`).
fn map_custom(message: &Value, ts: i64) -> Mapped {
    let name = str_field(message, "name").unwrap_or_default().to_owned();
    let payload = message.get("payload").cloned().unwrap_or(Value::Null);
    if message.get("model_visible").and_then(Value::as_bool) != Some(true) {
        return Mapped::CustomEntry(Box::new(EntryKind::Custom {
            custom_type: name,
            data: Some(payload),
        }));
    }
    let mut out = json!({"role": "custom", "customType": name});
    if let Value::String(text) = &payload {
        out["content"] = json!(text);
    } else {
        out["content"] = json!(payload.to_string());
        out["details"] = payload;
    }
    out["display"] = json!(true);
    out["timestamp"] = json!(ts);
    Mapped::Message(out)
}

fn from_value<T: serde::de::DeserializeOwned>(
    value: Value,
    line: usize,
) -> Result<T, SessionError> {
    serde_json::from_value(value).map_err(|source| SessionError::Json {
        path: std::path::PathBuf::new(),
        line,
        source,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "linear pipeline of the spec's numbered steps; splitting hides the order"
)]
/// Maps legacy file text to v3 lines. `base_ms` is the file mtime: entry *i* of *N* gets
/// `base − (N − 1 − i)` ms.
///
/// # Errors
///
/// [`SessionError::MissingHeader`] when no header record exists; [`SessionError::Json`] if
/// a mapped value fails to deserialise (indicates a mapping bug).
pub fn migrate_oino_legacy_text(text: &str, base_ms: i64) -> Result<LegacyMigration, SessionError> {
    let mut warnings = Vec::new();
    let legacy = parse_records(text, &mut warnings)?;
    let header_leaf = legacy.leaf_id.clone();
    let last_leaf_move = legacy.entries.iter().rev().find_map(|e| {
        (str_field(&e.kind, "kind") == Some("leaf_move"))
            .then(|| str_field(&e.kind, "leaf_id").map(str::to_owned))
            .flatten()
    });
    let last_in_file = legacy.entries.last().map(|e| e.uuid.clone());
    let entries = topological(legacy.entries);
    let count = i64::try_from(entries.len()).unwrap_or(i64::MAX);
    let timestamp_of = |index: usize| base_ms - (count - 1 - i64::try_from(index).unwrap_or(0));

    let mut taken = HashSet::new();
    let mut mapper = Mapper {
        ids: HashMap::new(),
        message_ids: HashMap::new(),
        tool_ids: HashMap::new(),
        warnings: &mut warnings,
    };
    let parents: HashMap<String, Option<String>> = entries
        .iter()
        .map(|e| (e.uuid.clone(), e.parent.clone()))
        .collect();
    for entry in &entries {
        let id = EntryId::generate(|candidate| taken.contains(candidate));
        taken.insert(id.to_string());
        if let Some(message_id) = entry.kind.pointer("/message/id").and_then(Value::as_str) {
            mapper.message_ids.insert(message_id.to_owned(), id.clone());
        }
        mapper.ids.insert(entry.uuid.clone(), id);
    }

    // First pass: map kinds (drops decided here).
    let mut mapped_kinds: Vec<(usize, Option<EntryKind>)> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let kind = map_kind(&mut mapper, entry, &parents, timestamp_of(index))?;
        mapped_kinds.push((index, kind));
    }
    let kept: HashSet<String> = mapped_kinds
        .iter()
        .filter(|(_, kind)| kind.is_some())
        .map(|(index, _)| entries[*index].uuid.clone())
        .collect();
    let resolve = |mut uuid: Option<String>| -> Option<EntryId> {
        let mut seen = HashSet::new();
        while let Some(current) = uuid {
            if !seen.insert(current.clone()) {
                return None;
            }
            if kept.contains(&current) {
                return mapper.ids.get(&current).cloned();
            }
            uuid = parents.get(&current).cloned().flatten();
        }
        None
    };

    let mut index = EntryIndex::default();
    for (position, kind) in mapped_kinds {
        let Some(mut kind) = kind else { continue };
        let entry = &entries[position];
        let id = mapper
            .ids
            .get(&entry.uuid)
            .cloned()
            .unwrap_or_else(|| EntryId::new(entry.uuid.clone()));
        let parent_id = resolve(entry.parent.clone());
        // Labels and branch summaries point at the parent; skip dropped ancestors.
        match &mut kind {
            EntryKind::Label { target_id, .. } => match &parent_id {
                Some(parent) => target_id.clone_from(parent),
                None => continue,
            },
            EntryKind::BranchSummary(summary) => {
                summary.from_id = parent_id.clone().unwrap_or_else(|| id.clone());
            }
            _ => {}
        }
        index.push(SessionEntry {
            id,
            parent_id,
            timestamp: IsoTimestamp::from_ms(timestamp_of(position)),
            kind,
        });
    }
    let has_messages = index
        .entries()
        .iter()
        .any(|e| matches!(e.kind, EntryKind::Message { .. }));
    let mut entries_out = fill_compaction_checkpoints(index);

    let leaf = [header_leaf, last_leaf_move, last_in_file]
        .into_iter()
        .flatten()
        .find(|uuid| parents.contains_key(uuid))
        .and_then(|uuid| resolve(Some(uuid)));
    let info_id = EntryId::generate(|candidate| taken.contains(candidate));
    let name = legacy.name.trim();
    entries_out.push(SessionEntry {
        id: info_id,
        parent_id: leaf,
        timestamp: IsoTimestamp::from_ms(base_ms),
        kind: EntryKind::SessionInfo {
            name: (!name.is_empty()).then(|| name.to_owned()),
        },
    });
    let header_timestamp = entries_out
        .first()
        .map_or_else(|| IsoTimestamp::from_ms(base_ms), |e| e.timestamp.clone());
    Ok(LegacyMigration {
        header: SessionHeader {
            version: CURRENT_SESSION_VERSION,
            id: legacy.session_id,
            timestamp: header_timestamp,
            cwd: legacy.cwd,
            parent_session: None,
        },
        entries: entries_out,
        warnings,
        has_messages,
    })
}

/// Stores the folded system checkpoint on migrated compactions (what pi's
/// `appendCompaction` would have recorded), so pi-exact projection keeps the prompt.
fn fill_compaction_checkpoints(mut index: EntryIndex) -> Vec<SessionEntry> {
    let compactions: Vec<(usize, Option<EntryId>, i64)> = index
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e.kind, EntryKind::Compaction(_)))
        .map(|(i, e)| (i, e.parent_id.clone(), e.timestamp.to_ms().unwrap_or(0)))
        .collect();
    for (position, parent, ts) in compactions {
        let checkpoint = parent.and_then(|parent| {
            let projection = build_session_projection(&index, Some(&parent));
            current_system_message(&projection.messages)
        });
        let mut entries = index.into_entries();
        if let EntryKind::Compaction(data) = &mut entries[position].kind {
            data.system_message = checkpoint.map(|mut system| {
                system.timestamp = ts;
                system
            });
        }
        index = EntryIndex::new(entries);
    }
    index.into_entries()
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per legacy kind keeps the spec table 1:1"
)]
fn map_kind(
    mapper: &mut Mapper<'_>,
    entry: &LegacyEntry,
    parents: &HashMap<String, Option<String>>,
    ts: i64,
) -> Result<Option<EntryKind>, SessionError> {
    let kind = &entry.kind;
    let line = entry.line;
    let mapped_parent = |mapper: &Mapper<'_>| {
        entry
            .parent
            .as_ref()
            .and_then(|p| mapper.ids.get(p))
            .cloned()
    };
    let result = match str_field(kind, "kind").unwrap_or_default() {
        "message" => {
            match mapper.map_message(kind.get("message").unwrap_or(&Value::Null), ts, line) {
                Mapped::Message(message) => Some(EntryKind::Message {
                    message: from_value::<AgentMessage>(message, line)?,
                }),
                Mapped::Dropped => None,
                Mapped::CustomEntry(custom) => Some(*custom),
            }
        }
        "model_change" => Some(EntryKind::ModelChange {
            provider: kind
                .pointer("/model/provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model_id: kind
                .pointer("/model/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
        "thinking_level_change" => {
            let level = str_field(kind, "thinking_level").unwrap_or("off");
            Some(EntryKind::ThinkingLevelChange {
                thinking_level: if level == "x_high" {
                    "xhigh".into()
                } else {
                    level.to_owned()
                },
            })
        }
        "compaction" => {
            let own = mapper.ids.get(&entry.uuid).cloned();
            let first_kept = match str_field(kind, "first_kept") {
                Some(first) => mapper.ids.get(first).cloned(),
                None => first_after_replaces(kind, entry, parents)
                    .and_then(|u| mapper.ids.get(&u).cloned()),
            }
            .or(own);
            let mut out = json!({"summary": str_field(kind, "summary").unwrap_or_default(),
                "firstKeptEntryId": first_kept.map(|id| id.to_string()).unwrap_or_default(),
                "tokensBefore": kind.get("tokens_before").and_then(Value::as_u64).unwrap_or(0)});
            if let Some(details) = kind.get("details").filter(|d| !d.is_null()) {
                out["details"] = details.clone();
            }
            Some(EntryKind::Compaction(from_value(out, line)?))
        }
        "context_edit" => {
            let target = str_field(kind, "target").and_then(|t| {
                mapper
                    .message_ids
                    .get(t)
                    .or_else(|| mapper.ids.get(t))
                    .cloned()
            });
            match target {
                None => {
                    mapper.warn(line, "context_edit with unresolved target dropped");
                    None
                }
                Some(target_id) => {
                    let replacement = match kind.get("replacement") {
                        Some(Value::Array(_)) => {
                            let blocks = mapper.map_blocks(kind.get("replacement"), None, line);
                            Some(from_value(json!({"content": blocks}), line)?)
                        }
                        _ => None,
                    };
                    Some(EntryKind::ContextEdit {
                        target_id,
                        replacement,
                    })
                }
            }
        }
        "usage" => Some(EntryKind::Usage {
            kind: str_field(kind, "usage_kind").unwrap_or_default().to_owned(),
            provider: kind
                .pointer("/model/provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model: kind
                .pointer("/model/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            usage: from_value(map_usage(kind.get("usage").unwrap_or(&Value::Null)), line)?,
            note: None,
        }),
        "branch_summary" => {
            let from_id = mapped_parent(mapper)
                .or_else(|| mapper.ids.get(&entry.uuid).cloned())
                .unwrap_or_else(|| EntryId::new("root"));
            Some(EntryKind::BranchSummary(from_value(
                json!({"fromId": from_id, "summary": str_field(kind, "summary").unwrap_or_default()}),
                line,
            )?))
        }
        "custom" => Some(EntryKind::Custom {
            custom_type: str_field(kind, "name").unwrap_or_default().to_owned(),
            data: Some(kind.get("payload").cloned().unwrap_or(Value::Null)),
        }),
        "extension_custom" => {
            let inner = kind.get("entry").unwrap_or(&Value::Null);
            let owner = inner
                .get("owner_extension_id")
                .map(|o| o.as_str().map_or_else(|| o.to_string(), str::to_owned))
                .unwrap_or_default();
            Some(EntryKind::Custom {
                custom_type: format!(
                    "oino.legacy/{owner}/{}",
                    str_field(inner, "key").unwrap_or_default()
                ),
                data: Some(inner.get("payload").cloned().unwrap_or(Value::Null)),
            })
        }
        "custom_message" => {
            match mapper.map_message(kind.get("message").unwrap_or(&Value::Null), ts, line) {
                Mapped::Message(message) => {
                    let message: AgentMessage = from_value(message, line)?;
                    match message {
                        AgentMessage::Custom(custom) => Some(EntryKind::CustomMessage {
                            custom_type: custom.custom_type,
                            content: custom.content,
                            display: custom.display,
                            details: custom.details,
                        }),
                        other => Some(EntryKind::Message { message: other }),
                    }
                }
                Mapped::Dropped => None,
                Mapped::CustomEntry(custom) => Some(*custom),
            }
        }
        "label" => {
            if let Some(target_id) = mapped_parent(mapper) {
                Some(EntryKind::Label {
                    target_id,
                    label: str_field(kind, "label")
                        .filter(|l| !l.is_empty())
                        .map(str::to_owned),
                })
            } else {
                mapper.warn(line, "label on a root entry dropped");
                None
            }
        }
        "session_info" => Some(EntryKind::SessionInfo {
            name: str_field(kind, "name").map(str::to_owned),
        }),
        "leaf_move" => None,
        other => {
            mapper.warn(line, format!("legacy entry kind {other:?} dropped"));
            None
        }
    };
    Ok(result)
}

/// v1 compactions: first entry on the compaction's path after the last id in `replaces`.
fn first_after_replaces(
    kind: &Value,
    entry: &LegacyEntry,
    parents: &HashMap<String, Option<String>>,
) -> Option<String> {
    let last_replaced = kind.get("replaces")?.as_array()?.last()?.as_str()?;
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut current = entry.parent.clone();
    while let Some(uuid) = current {
        if !seen.insert(uuid.clone()) {
            break;
        }
        current = parents.get(&uuid).cloned().flatten();
        path.push(uuid);
    }
    path.reverse();
    let position = path.iter().position(|uuid| uuid == last_replaced)?;
    path.get(position + 1).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const U1: &str = "00000000-0000-0000-0000-000000000001";

    fn line(entry: &Value) -> String {
        json!({"record": "entry", "entry": entry}).to_string()
    }

    #[test]
    fn maps_messages_tool_links_and_restores_leaf() {
        let header = json!({"record":"header","header":{"version":2,"session_id":U1,"name":"named","cwd":"/w"},
            "leaf_id":"u-a"}).to_string();
        let text = [
            header,
            line(&json!({"id":"u-r","parent":"u-a","kind":{"kind":"message","message":{"kind":"tool_result","id":"m3",
                "tool_call_id":"tc-uuid","tool_name":"read","content":[{"kind":"text","text":"ok"}],"is_error":false,"terminate":false,"details":null}}})),
            line(&json!({"id":"u-u","parent":null,"kind":{"kind":"message","message":{"kind":"user","id":"m1","content":[{"kind":"text","text":"hi"}]}}})),
            line(&json!({"id":"u-a","parent":"u-u","kind":{"kind":"message","message":{"kind":"assistant","id":"m2",
                "content":[{"kind":"tool_call","id":"tc-uuid","name":"read","arguments":{"path":"a"},"native_id":"call_9"}],
                "stop_reason":"tool_use","usage":{"input_tokens":3,"output_tokens":2,"cache_read_tokens":0,"cache_write_tokens":0,"cost":{"amount":0.5,"currency":"USD"}},
                "provider":{"request_id":"req","model":{"provider":"router","name":"m"}}}}})),
            line(&json!({"id":"u-e","parent":"u-r","kind":{"kind":"context_edit","target":"m3","replacement":null}})),
            line(&json!({"id":"u-l","parent":"u-e","kind":{"kind":"leaf_move","leaf_id":"u-a"}})),
        ]
        .join("\n");
        let migration = migrate_oino_legacy_text(&text, 10_000).unwrap();
        assert!(migration.has_messages);
        let entries = &migration.entries;
        assert_eq!(entries.len(), 5, "{entries:#?}");
        // Parents first: user, assistant, tool result, edit, session_info.
        let roles: Vec<&str> = entries.iter().map(SessionEntry::type_name).collect();
        assert_eq!(
            roles,
            [
                "message",
                "message",
                "message",
                "context_edit",
                "session_info"
            ]
        );
        let wire = serde_json::to_value(&entries[1]).unwrap();
        assert_eq!(wire["message"]["content"][0]["id"], json!("call_9"));
        assert_eq!(wire["message"]["usage"]["cost"]["total"], json!(0.5));
        assert_eq!(wire["message"]["responseId"], json!("req"));
        let result = serde_json::to_value(&entries[2]).unwrap();
        assert_eq!(result["message"]["toolCallId"], json!("call_9"));
        let edit = serde_json::to_value(&entries[3]).unwrap();
        assert_eq!(edit["targetId"], json!(entries[2].id.as_str()));
        let info = &entries[4];
        assert_eq!(info.parent_id.as_ref(), Some(&entries[1].id));
        assert_eq!(
            info.kind,
            EntryKind::SessionInfo {
                name: Some("named".into())
            }
        );
        assert_eq!(entries[0].timestamp, IsoTimestamp::from_ms(10_000 - 4));
        assert_eq!(migration.header.id, U1);
    }

    #[test]
    fn non_visible_custom_messages_become_custom_entries() {
        let header = json!({"record":"header","header":{"version":1,"session_id":U1,"name":"","cwd":"/w"},"leaf_id":null}).to_string();
        let text = [
            header,
            line(&json!({"id":"a","parent":null,"kind":{"kind":"custom_message","message":{"kind":"custom","id":"x","name":"n","payload":{"k":1},"model_visible":false}}})),
            line(&json!({"id":"b","parent":"a","kind":{"kind":"custom_message","message":{"kind":"custom","id":"y","name":"v","payload":{"k":2},"model_visible":true}}})),
        ]
        .join("\n");
        let migration = migrate_oino_legacy_text(&text, 0).unwrap();
        assert!(!migration.has_messages);
        assert_eq!(migration.entries[0].type_name(), "custom");
        let visible = serde_json::to_value(&migration.entries[1]).unwrap();
        assert_eq!(visible["content"], json!("{\"k\":2}"));
        assert_eq!(visible["details"], json!({"k": 2}));
        assert_eq!(
            migration.entries[2].kind,
            EntryKind::SessionInfo { name: None }
        );
    }
}
