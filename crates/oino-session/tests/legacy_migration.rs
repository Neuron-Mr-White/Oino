//! Golden suite `legacy-migration` (quality §4.4): pre-v1 Oino files → v3.
//!
//! Inputs: `tests/fixtures/oino-legacy/*.jsonl` (scrubbed real files + hand-written edge
//! cases). Goldens: `expected/<name>.jsonl` (ids normalised on comparison). Bless with
//! `OINO_BLESS=1 cargo test -p oino-session --test legacy_migration`, then record pi's view
//! of the goldens with `node scripts/pi-oracle/core-types-session/context.mjs
//! crates/oino-session/tests/fixtures/oino-legacy/expected`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests"
)]

mod common;

use std::fs;
use std::path::PathBuf;

use oino_session::migrate::legacy::migrate_oino_legacy_text;
use oino_session::{EntryKind, SessionManager};
use serde_json::Value;

const BASE_MS: i64 = 1_790_000_000_000;

fn inputs() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(common::fixtures().join("oino-legacy"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files
}

fn as_lines(text: &str) -> Value {
    Value::Array(
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
    )
}

fn migrated_text(path: &PathBuf) -> Option<String> {
    let migration = migrate_oino_legacy_text(&fs::read_to_string(path).unwrap(), BASE_MS).unwrap();
    if !migration.has_messages {
        return None;
    }
    let mut lines = vec![serde_json::to_string(&migration.header).unwrap()];
    lines.extend(
        migration
            .entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap()),
    );
    Some(lines.join("\n") + "\n")
}

#[test]
fn legacy_files_match_goldens() {
    let expected_dir = common::fixtures().join("oino-legacy/expected");
    let bless = std::env::var_os("OINO_BLESS").is_some();
    let mut compared = 0;
    for input in inputs() {
        let golden = expected_dir.join(input.file_name().unwrap());
        let Some(text) = migrated_text(&input) else {
            assert!(
                !golden.exists(),
                "{} has no messages but a golden",
                input.display()
            );
            continue;
        };
        if bless {
            fs::create_dir_all(&expected_dir).unwrap();
            fs::write(&golden, &text).unwrap();
        }
        let mut actual = as_lines(&text);
        let mut expected = as_lines(&fs::read_to_string(&golden).unwrap());
        common::normalize_ids(&mut actual);
        common::normalize_ids(&mut expected);
        assert_eq!(
            common::first_diff(&actual, &expected, ""),
            None,
            "{}",
            input.display()
        );
        compared += 1;
    }
    assert!(compared >= 4, "only {compared} goldens");
}

/// pi 1.4.0 reads the migrated goldens and builds the same context as Oino.
#[test]
fn goldens_match_pi_projection() {
    let dir = common::fixtures().join("oino-legacy/expected");
    for file in fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
    {
        if file.extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        let mut expected: Value = serde_json::from_str(
            &fs::read_to_string(file.with_extension("expected.json")).unwrap(),
        )
        .unwrap();
        let temp = common::tempdir();
        let copy = temp.path().join(file.file_name().unwrap());
        fs::copy(&file, &copy).unwrap();
        let mut manager = SessionManager::open(&copy, temp.path(), None).unwrap();
        let mut actual = common::snapshot(&mut manager);
        common::normalize_ids(&mut actual);
        common::normalize_ids(&mut expected);
        assert_eq!(
            common::first_diff(&actual, &expected, ""),
            None,
            "{}",
            file.display()
        );
    }
}

#[test]
fn rich_fixture_preserves_leaf_name_links_and_edits() {
    let input = common::fixtures().join("oino-legacy/rich-v2.jsonl");
    let migration = migrate_oino_legacy_text(&fs::read_to_string(input).unwrap(), BASE_MS).unwrap();
    let temp = common::tempdir();
    let path = temp.path().join("rich.jsonl");
    fs::write(
        &path,
        migrated_text(&common::fixtures().join("oino-legacy/rich-v2.jsonl")).unwrap(),
    )
    .unwrap();
    let manager = SessionManager::open(&path, temp.path(), None).unwrap();

    // Name and leaf (last leaf_move → "on branch") survive via the trailing session_info.
    assert_eq!(manager.session_name().as_deref(), Some("Rich legacy"));
    let leaf_parent = manager.leaf_entry().unwrap().parent_id.clone().unwrap();
    let leaf_message = manager.entry(&leaf_parent).unwrap().message().unwrap();
    assert_eq!(
        serde_json::to_value(leaf_message).unwrap()["content"][0]["text"],
        "on branch"
    );

    // Context on the leaf: system, user, assistant, two tool results, assistant, branch summary, user.
    let context = manager.build_session_context();
    let roles: Vec<&str> = context
        .messages
        .iter()
        .map(oino_ai::AgentMessage::role)
        .collect();
    assert_eq!(
        roles,
        [
            "system",
            "user",
            "assistant",
            "toolResult",
            "toolResult",
            "assistant",
            "branchSummary",
            "user"
        ]
    );
    let wire = serde_json::to_value(&context.messages).unwrap();
    assert_eq!(wire[2]["content"][3]["id"], "call_native");
    assert_eq!(
        wire[2]["content"][3]["argumentsError"],
        "legacy arguments were not an object"
    );
    assert_eq!(wire[4]["toolCallId"], "call_native");
    assert_eq!(wire[3]["toolCallId"], wire[2]["content"][2]["id"]);
    assert_eq!(wire[2]["content"][0]["thinkingSignature"], "opaque-state");
    assert_eq!(wire[1]["content"][1]["mimeType"], "image/png");
    assert_eq!(context.thinking_level, "xhigh");

    // Compaction keeps the system checkpoint and maps first_kept; the edit targets the tool result.
    let compaction = migration.entries.iter().find_map(|e| match &e.kind {
        EntryKind::Compaction(c) => Some((e.id.clone(), c.clone())),
        _ => None,
    });
    let (_, compaction) = compaction.unwrap();
    assert!(compaction.system_message.is_some());
    let first_kept = migration
        .entries
        .iter()
        .find(|e| e.id == compaction.first_kept_entry_id)
        .unwrap();
    assert_eq!(first_kept.message().unwrap().role(), "toolResult");
    let edit_target = migration.entries.iter().find_map(|e| match &e.kind {
        EntryKind::ContextEdit { target_id, .. } => Some(target_id.clone()),
        _ => None,
    });
    assert_eq!(edit_target, Some(first_kept.id.clone()));
    // Unresolved edit, malformed line, mystery kind, compaction-summary message dropped.
    assert!(migration.warnings.len() >= 4, "{:#?}", migration.warnings);
    assert!(
        migration
            .entries
            .iter()
            .any(|e| matches!(&e.kind, EntryKind::Custom { custom_type, .. } if custom_type == "oino.legacy/oino.vcc/cursor"))
    );
}

#[test]
fn v1_replaces_derives_first_kept() {
    let input = common::fixtures().join("oino-legacy/v1-replaces.jsonl");
    let migration = migrate_oino_legacy_text(&fs::read_to_string(input).unwrap(), BASE_MS).unwrap();
    let compaction = migration
        .entries
        .iter()
        .find_map(|e| match &e.kind {
            EntryKind::Compaction(c) => Some(c.first_kept_entry_id.clone()),
            _ => None,
        })
        .unwrap();
    let kept = migration
        .entries
        .iter()
        .find(|e| e.id == compaction)
        .unwrap();
    // replaces = [a, A] ⇒ first kept is user "b".
    assert_eq!(
        serde_json::to_value(kept.message().unwrap()).unwrap()["content"][0]["text"],
        "b"
    );
    // Header leaf (user "c") restored; the label after it is not on the restored branch.
    let info = migration.entries.last().unwrap();
    let leaf = migration
        .entries
        .iter()
        .find(|e| Some(&e.id) == info.parent_id.as_ref())
        .unwrap();
    assert_eq!(
        serde_json::to_value(leaf.message().unwrap()).unwrap()["content"][0]["text"],
        "c"
    );
    assert_eq!(info.kind, EntryKind::SessionInfo { name: None });
}
