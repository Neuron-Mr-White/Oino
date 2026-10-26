//! Golden tests for `oino_core::compaction` pure logic against pi 1.4.0.
//! Fixtures: `scripts/pi-oracle/core-runtime/compaction/oracle.mjs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use oino_ai::json::json_semantic_eq;
use oino_ai::{AgentMessage, AssistantMessage, Message, Usage};
use oino_core::compaction::errors::{
    CEREBRAS_BODYLESS_OVERFLOW_PATTERN, NON_OVERFLOW_PATTERNS,
    NON_RETRYABLE_PROVIDER_LIMIT_PATTERNS, OVERFLOW_PATTERNS, RETRYABLE_PROVIDER_ERROR_PATTERNS,
    pattern_matches,
};
use oino_core::compaction::{
    CompactionSettings, CutPointResult, DEFAULT_COMPACTION_SETTINGS, FileOperations,
    SUMMARIZATION_SYSTEM_PROMPT, calculate_context_tokens, collect_entries_for_branch_summary,
    estimate_context_tokens, estimate_projected_context_tokens, estimate_tokens, find_cut_point,
    find_turn_start_index, format_file_operations, is_context_overflow,
    is_retryable_assistant_error, last_assistant_usage, prepare_branch_entries, prepare_compaction,
    retry_delay_ms, serialize_conversation, should_compact,
};
use oino_session::{EntryId, EntryIndex, SessionEntry, build_session_projection, convert_to_llm};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/compaction/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_json(label: &str, actual: &Value, expected: &Value) {
    assert!(
        json_semantic_eq(actual, expected),
        "{label}\n  actual:   {actual}\n  expected: {expected}"
    );
}

fn pi_cut(value: &Value) -> CutPointResult {
    CutPointResult {
        first_kept_entry_index: usize::try_from(value["firstKeptEntryIndex"].as_u64().unwrap())
            .unwrap(),
        turn_start_index: usize::try_from(value["turnStartIndex"].as_i64().unwrap()).ok(),
        is_split_turn: value["isSplitTurn"].as_bool().unwrap(),
    }
}

fn file_ops_json(ops: &FileOperations) -> Value {
    serde_json::to_value(ops).unwrap()
}

#[test]
fn sessions_match_pi() {
    let sessions = fixture("sessions.json");
    let mut checked = 0;
    for (name, scenario) in sessions.as_object().unwrap() {
        let entries: Vec<SessionEntry> =
            serde_json::from_value(scenario["entries"].clone()).unwrap();
        let index = EntryIndex::new(entries);
        let branch: Vec<&SessionEntry> = scenario["branchIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| index.get(&EntryId::new(id.as_str().unwrap())).unwrap())
            .collect();
        let leaf = scenario["leafId"].as_str().map(EntryId::new);
        let projection = build_session_projection(&index, leaf.as_ref());

        for case in scenario["compaction"].as_array().unwrap() {
            let keep = case["keepRecentTokens"].as_u64().unwrap();
            let settings = CompactionSettings {
                enabled: true,
                reserve_tokens: 16384,
                keep_recent_tokens: keep,
            };
            let label = format!("{name} keep={keep}");
            let prepared = prepare_compaction(&branch, &settings);
            assert_json(
                &format!("{label} prepareCompaction"),
                &serde_json::to_value(&prepared).unwrap(),
                &case["prepare"],
            );
            assert_eq!(
                find_cut_point(&branch, 0, branch.len(), keep),
                pi_cut(&case["findCutPoint"]),
                "{label} findCutPoint"
            );
            checked += 1;
        }
        let turn_starts: Vec<Option<usize>> = (0..branch.len())
            .map(|index| find_turn_start_index(&branch, index, 0))
            .collect();
        let expected: Vec<Option<usize>> = scenario["turnStarts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| usize::try_from(value.as_i64().unwrap()).ok())
            .collect();
        assert_eq!(turn_starts, expected, "{name} findTurnStartIndex");
        assert_json(
            &format!("{name} getLastAssistantUsage"),
            &serde_json::to_value(last_assistant_usage(&branch)).unwrap(),
            &scenario["lastAssistantUsage"],
        );
        assert_json(
            &format!("{name} estimateContextTokens"),
            &serde_json::to_value(estimate_context_tokens(&projection.messages)).unwrap(),
            &scenario["estimate"],
        );
        assert_json(
            &format!("{name} estimateProjectedContextTokens"),
            &serde_json::to_value(estimate_projected_context_tokens(&projection, &branch)).unwrap(),
            &scenario["projectedEstimate"],
        );
        let tokens: Vec<u64> = projection.messages.iter().map(estimate_tokens).collect();
        assert_json(
            &format!("{name} estimateTokens"),
            &json!(tokens),
            &scenario["messageTokens"],
        );
        assert_eq!(
            serialize_conversation(&convert_to_llm(&projection.messages)),
            scenario["serialized"].as_str().unwrap(),
            "{name} serializeConversation"
        );
        check_branch_cases(name, scenario, &index);
    }
    assert!(checked > 100, "only {checked} compaction cases");
}

fn check_branch_cases(name: &str, scenario: &Value, index: &EntryIndex) {
    for case in scenario["branchCases"].as_array().unwrap() {
        let old_leaf = case["oldLeaf"].as_str().map(EntryId::new);
        let target = EntryId::new(case["target"].as_str().unwrap());
        let label = format!("{name} {old_leaf:?} -> {target}");
        let collected = collect_entries_for_branch_summary(index, old_leaf.as_ref(), &target);
        let ids: Vec<&str> = collected.entries.iter().map(|e| e.id.as_str()).collect();
        assert_json(&format!("{label} entries"), &json!(ids), &case["entryIds"]);
        assert_json(
            &format!("{label} commonAncestorId"),
            &json!(collected.common_ancestor_id),
            &case["commonAncestorId"],
        );
        for prepared in case["prepared"].as_array().unwrap() {
            let budget = prepared["budget"].as_u64().unwrap();
            let actual = prepare_branch_entries(&collected.entries, budget);
            let label = format!("{label} budget={budget}");
            assert_json(
                &format!("{label} messages"),
                &serde_json::to_value(&actual.messages).unwrap(),
                &prepared["messages"],
            );
            assert_json(
                &format!("{label} fileOps"),
                &file_ops_json(&actual.file_ops),
                &prepared["fileOps"],
            );
            assert_eq!(
                actual.total_tokens,
                prepared["totalTokens"].as_u64().unwrap(),
                "{label} totalTokens"
            );
        }
    }
}

#[test]
fn utils_match_pi() {
    let utils = fixture("utils.json");
    assert_eq!(
        SUMMARIZATION_SYSTEM_PROMPT,
        utils["systemPrompt"].as_str().unwrap()
    );
    for (index, case) in utils["serialize"].as_array().unwrap().iter().enumerate() {
        let messages: Vec<Message> = serde_json::from_value(case["messages"].clone()).unwrap();
        assert_eq!(
            serialize_conversation(&messages),
            case["text"].as_str().unwrap(),
            "serialize case {index}"
        );
    }
    for case in utils["estimate"].as_array().unwrap() {
        let message: AgentMessage = serde_json::from_value(case["message"].clone()).unwrap();
        assert_eq!(
            estimate_tokens(&message),
            case["tokens"].as_u64().unwrap(),
            "estimateTokens {}",
            case["message"]
        );
    }
    for case in utils["fileLists"].as_array().unwrap() {
        let ops: FileOperations = serde_json::from_value(case["ops"].clone()).unwrap();
        let lists = ops.file_lists();
        assert_json(
            "computeFileLists",
            &serde_json::to_value(&lists).unwrap(),
            &json!({"readFiles": case["readFiles"], "modifiedFiles": case["modifiedFiles"]}),
        );
        assert_eq!(
            format_file_operations(&lists.read_files, &lists.modified_files),
            case["formatted"].as_str().unwrap()
        );
    }
    for case in utils["shouldCompact"].as_array().unwrap() {
        let settings: CompactionSettings =
            serde_json::from_value(case["settings"].clone()).unwrap();
        assert_eq!(
            should_compact(
                case["tokens"].as_u64().unwrap(),
                case["window"].as_u64().unwrap(),
                &settings
            ),
            case["result"].as_bool().unwrap(),
            "shouldCompact {case}"
        );
    }
    for case in utils["calculateContextTokens"].as_array().unwrap() {
        let usage: Usage = serde_json::from_value(case["usage"].clone()).unwrap();
        assert_eq!(
            calculate_context_tokens(&usage),
            case["tokens"].as_u64().unwrap()
        );
    }
    assert_json(
        "DEFAULT_COMPACTION_SETTINGS",
        &serde_json::to_value(DEFAULT_COMPACTION_SETTINGS).unwrap(),
        &utils["defaults"],
    );
}

#[test]
fn error_tables_match_pi_sources() {
    let errors = fixture("errors.json");
    let sources = &errors["sources"];
    assert_json("overflow", &json!(OVERFLOW_PATTERNS), &sources["overflow"]);
    assert_json(
        "cerebras",
        &json!(CEREBRAS_BODYLESS_OVERFLOW_PATTERN),
        &sources["cerebras"],
    );
    assert_json(
        "nonOverflow",
        &json!(NON_OVERFLOW_PATTERNS),
        &sources["nonOverflow"],
    );
    assert_json(
        "nonRetryable",
        &json!(NON_RETRYABLE_PROVIDER_LIMIT_PATTERNS),
        &sources["nonRetryable"],
    );
    assert_json(
        "retryable",
        &json!(RETRYABLE_PROVIDER_ERROR_PATTERNS),
        &sources["retryable"],
    );
}

#[test]
fn pattern_engine_matches_js_regexp() {
    let errors = fixture("errors.json");
    let all: Vec<&str> = OVERFLOW_PATTERNS
        .iter()
        .chain([&CEREBRAS_BODYLESS_OVERFLOW_PATTERN])
        .chain(NON_OVERFLOW_PATTERNS)
        .chain(NON_RETRYABLE_PROVIDER_LIMIT_PATTERNS)
        .chain(RETRYABLE_PROVIDER_ERROR_PATTERNS)
        .copied()
        .collect();
    for case in errors["patterns"].as_array().unwrap() {
        let text = case["message"].as_str().unwrap();
        let matches: Vec<usize> = all
            .iter()
            .enumerate()
            .filter(|(_, pattern)| pattern_matches(pattern, text).unwrap())
            .map(|(index, _)| index)
            .collect();
        let expected: Vec<usize> = serde_json::from_value(case["matches"].clone()).unwrap();
        let names = |list: &[usize]| list.iter().map(|i| all[*i]).collect::<Vec<_>>();
        assert_eq!(
            names(&matches),
            names(&expected),
            "patterns matching {text:?}"
        );
    }
}

#[test]
fn classification_matches_pi() {
    let errors = fixture("errors.json");
    let cases = errors["cases"].as_array().unwrap();
    for case in cases {
        let message: AssistantMessage = serde_json::from_value(case["message"].clone()).unwrap();
        let window = case["contextWindow"].as_u64().unwrap();
        assert_eq!(
            is_context_overflow(&message, window),
            case["overflow"].as_bool().unwrap(),
            "isContextOverflow {:?} {} window={window}",
            message.error_message,
            message.provider
        );
        assert_eq!(
            is_retryable_assistant_error(&message),
            case["retryable"].as_bool().unwrap(),
            "isRetryableAssistantError {:?}",
            message.error_message
        );
    }
    for case in errors["retryDelay"].as_array().unwrap() {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "fixture values are small non-negative integers or 1e20"
        )]
        let cap = case["maxAgentDelayMs"].as_f64().map(|cap| cap as u64);
        let attempt = u32::try_from(case["attempt"].as_u64().unwrap()).unwrap();
        assert_eq!(
            retry_delay_ms(case["baseDelayMs"].as_u64().unwrap(), cap, attempt),
            case["delay"].as_u64().unwrap(),
            "retryDelayMs {case}"
        );
    }
}
