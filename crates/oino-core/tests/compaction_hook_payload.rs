//! `session_before_compact` payload bounds (compactor-fix): pi's payload, unchanged, on every
//! golden session; bounded on a large synthetic session shaped like the 213 MB chat that
//! trapped `unipi.compactor` (tool results with megabyte `details`, images, 9 compactions).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use oino_core::compaction::hook_payload::json_size;
use oino_core::compaction::{
    CompactionSettings, HOOK_PAYLOAD_LIMITS, bound_branch_entries, bound_preparation,
    prepare_compaction,
};
use oino_session::{EntryId, EntryIndex, SessionEntry};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/compaction/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

const SETTINGS: CompactionSettings = CompactionSettings {
    enabled: true,
    reserve_tokens: 16384,
    keep_recent_tokens: 20_000,
};

#[test]
fn golden_sessions_get_pis_payload_unchanged() {
    let sessions = fixture("sessions.json");
    let mut checked = 0;
    for (name, scenario) in sessions.as_object().unwrap() {
        let entries: Vec<SessionEntry> =
            serde_json::from_value(scenario["entries"].clone()).unwrap();
        let index = EntryIndex::new(entries);
        let branch: Vec<SessionEntry> = scenario["branchIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| {
                index
                    .get(&EntryId::new(id.as_str().unwrap()))
                    .unwrap()
                    .clone()
            })
            .collect();
        let bounded = bound_branch_entries(&branch, &HOOK_PAYLOAD_LIMITS);
        assert_eq!(bounded.omitted, 0, "{name}");
        assert!(!bounded.slimmed, "{name}");
        assert_eq!(
            Value::Array(bounded.entries),
            serde_json::to_value(&branch).unwrap(),
            "{name}"
        );
        for keep in [1, 50, 20_000] {
            let settings = CompactionSettings {
                keep_recent_tokens: keep,
                ..SETTINGS
            };
            let refs: Vec<&SessionEntry> = branch.iter().collect();
            if let Some(prep) = prepare_compaction(&refs, &settings) {
                let (value, slimmed) = bound_preparation(&prep, &HOOK_PAYLOAD_LIMITS);
                assert!(!slimmed, "{name}");
                assert_eq!(value, serde_json::to_value(&prep).unwrap(), "{name}");
                checked += 1;
            }
        }
    }
    assert!(checked > 5, "only {checked} preparations checked");
}

/// A chat shaped like the owner's: per turn a user prompt, an assistant tool call and an
/// `edit` result whose `details.oldContent` is a whole file; every 8th result carries an
/// image; a compaction every 200 turns.
fn large_session(turns: usize, details: usize, user_pad: usize) -> Vec<SessionEntry> {
    let pad = "x".repeat(user_pad);
    let file = "fn main() { println!(\"hello\"); }\n".repeat(details / 34 + 1);
    let image = "iVBORw0KGgo".repeat(4096);
    let mut out = Vec::with_capacity(turns * 3 + turns / 200);
    let mut parent: Option<String> = None;
    let mut push = |v: Value, parent: &mut Option<String>| {
        let id = v["id"].as_str().unwrap().to_owned();
        let mut v = v;
        v["parentId"] = json!(parent.clone());
        v["timestamp"] = json!("2026-11-03T00:00:00.000Z");
        out.push(serde_json::from_value::<SessionEntry>(v).unwrap());
        *parent = Some(id);
    };
    for t in 0..turns {
        if t > 0 && t % 200 == 0 {
            push(
                json!({"type": "compaction", "id": format!("c{t}"), "summary": format!("summary up to turn {t}"),
                    "firstKeptEntryId": format!("u{}", t - 10), "tokensBefore": 150_000}),
                &mut parent,
            );
        }
        push(
            json!({"type": "message", "id": format!("u{t}"),
                "message": {"role": "user", "content": format!("turn {t}: fix the router{pad}"), "timestamp": 1}}),
            &mut parent,
        );
        push(
            json!({"type": "message", "id": format!("a{t}"),
                "message": {"role": "assistant", "content": [
                    {"type": "text", "text": format!("Editing router.tsx for turn {t}.")},
                    {"type": "toolCall", "id": format!("call{t}"), "name": "edit", "arguments": {"path": "src/router.tsx"}}],
                    "api": "faux", "provider": "faux", "model": "faux-1",
                    "usage": {"input": 1, "output": 1, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 2,
                        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                    "stopReason": "toolUse", "timestamp": 1}}),
            &mut parent,
        );
        let mut content =
            vec![json!({"type": "text", "text": "Successfully edited src/router.tsx (8 edits)"})];
        if t % 8 == 0 {
            content.push(json!({"type": "image", "data": image, "mimeType": "image/png"}));
        }
        push(
            json!({"type": "message", "id": format!("r{t}"),
                "message": {"role": "toolResult", "toolCallId": format!("call{t}"), "toolName": "edit",
                    "content": content, "details": {"oldContent": file, "diff": file},
                    "isError": false, "timestamp": 1}}),
            &mut parent,
        );
    }
    out
}

#[test]
fn large_synthetic_session_payload_is_bounded() {
    // ≈ 1,850 turns × 2 × 24 KiB of details ≈ 90 MB serialized.
    let branch = large_session(1_850, 24 << 10, 0);
    let raw: usize = branch.iter().map(json_size).sum();
    assert!(raw > 80 << 20, "synthetic session only {raw} bytes");

    let bounded = bound_branch_entries(&branch, &HOOK_PAYLOAD_LIMITS);
    let size = json_size(&bounded.entries);
    assert!(
        size <= HOOK_PAYLOAD_LIMITS.branch_bytes,
        "branchEntries {size} bytes"
    );
    assert!(bounded.slimmed);
    assert_eq!(bounded.omitted + bounded.entries.len(), branch.len());
    // The newest entries, contiguous and oldest first, ending at the leaf.
    let ids: Vec<&str> = bounded
        .entries
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    let tail: Vec<String> = branch[bounded.omitted..]
        .iter()
        .map(|e| e.id.to_string())
        .collect();
    assert_eq!(ids, tail);
    // The live context (newest compaction + its kept tail) is inside the window.
    assert!(ids.contains(&"c1800"));
    assert!(ids.contains(&"u1790"));
    // Text survives slimming; details and image bytes do not.
    let result = bounded
        .entries
        .iter()
        .rev()
        .find(|e| e["message"]["role"] == "toolResult")
        .unwrap();
    assert_eq!(
        result["message"]["content"][0]["text"],
        "Successfully edited src/router.tsx (8 edits)"
    );
    assert!(result["message"].get("details").is_none());
    for e in &bounded.entries {
        if let Some(parts) = e["message"]["content"].as_array() {
            for p in parts.iter().filter(|p| p["type"] == "image") {
                assert_eq!(p["data"], "");
            }
        }
    }

    // The 50 turns after the newest compaction hold few tokens (details are not context).
    let settings = CompactionSettings {
        keep_recent_tokens: 2_000,
        ..SETTINGS
    };
    let refs: Vec<&SessionEntry> = branch.iter().collect();
    let prep = prepare_compaction(&refs, &settings).expect("something to compact");
    assert!(
        json_size(&prep) > 1 << 20,
        "preparation too small to matter"
    );
    let (value, slimmed) = bound_preparation(&prep, &HOOK_PAYLOAD_LIMITS);
    assert!(json_size(&value) <= HOOK_PAYLOAD_LIMITS.preparation_bytes);
    assert_eq!(value["firstKeptEntryId"], json!(prep.first_kept_entry_id));
    assert_eq!(value["tokensBefore"], json!(prep.tokens_before));
    let _ = slimmed;
}

#[test]
fn no_compaction_huge_session_preparation_drops_oldest_messages() {
    // One long run without a compaction: everything before the kept tail is summarised.
    // Text-heavy user turns that slimming cannot shrink.
    let branch = large_session(199, 64 << 10, 48 << 10);
    let refs: Vec<&SessionEntry> = branch.iter().collect();
    let prep = prepare_compaction(&refs, &SETTINGS).expect("something to compact");
    let limits = oino_core::compaction::HookPayloadLimits {
        preparation_bytes: 4 << 20,
        ..HOOK_PAYLOAD_LIMITS
    };
    let (value, slimmed) = bound_preparation(&prep, &limits);
    assert!(slimmed);
    assert!(json_size(&value) <= limits.preparation_bytes);
    let omitted = value["messagesToSummarizeOmitted"].as_u64().unwrap();
    let kept = value["messagesToSummarize"].as_array().unwrap().len();
    assert_eq!(
        usize::try_from(omitted).unwrap() + kept,
        prep.messages_to_summarize.len()
    );
}
