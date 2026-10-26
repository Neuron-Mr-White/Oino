//! Coding-agent message helpers (pi `core/messages.js`): LLM conversion and summary wrappers.

use std::fmt::Write as _;

use oino_ai::{
    AgentMessage, BashExecutionMessage, BranchSummaryMessage, CompactionSummaryMessage,
    CustomMessage, JsonValue, Message, TimestampMs, UserContent, UserContentField, UserMessage,
};

/// Text before a compaction summary in LLM context.
pub const COMPACTION_SUMMARY_PREFIX: &str = "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";
/// Text after a compaction summary in LLM context.
pub const COMPACTION_SUMMARY_SUFFIX: &str = "\n</summary>";
/// Text before a branch summary in LLM context.
pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";
/// Text after a branch summary in LLM context.
pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";

/// pi `bashExecutionToText`.
#[must_use]
pub fn bash_execution_to_text(message: &BashExecutionMessage) -> String {
    let mut text = format!("Ran `{}`\n", message.command);
    if message.output.is_empty() {
        text.push_str("(no output)");
    } else {
        let _ = write!(text, "```\n{}\n```", message.output);
    }
    if message.cancelled {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(code) = message.exit_code.filter(|code| *code != 0) {
        let _ = write!(text, "\n\nCommand exited with code {code}");
    }
    if let (true, Some(path)) = (message.truncated, &message.full_output_path) {
        let _ = write!(text, "\n\n[Output truncated. Full output: {path}]");
    }
    text
}

/// pi `createBranchSummaryMessage`.
#[must_use]
pub fn branch_summary_message(
    summary: &str,
    from_id: Option<&str>,
    timestamp: TimestampMs,
) -> AgentMessage {
    AgentMessage::BranchSummary(BranchSummaryMessage {
        summary: summary.to_owned(),
        from_id: from_id.map(str::to_owned),
        timestamp,
    })
}

/// pi `createCompactionSummaryMessage`.
#[must_use]
pub fn compaction_summary_message(
    summary: &str,
    tokens_before: u64,
    timestamp: TimestampMs,
) -> AgentMessage {
    AgentMessage::CompactionSummary(CompactionSummaryMessage {
        summary: summary.to_owned(),
        tokens_before,
        timestamp,
    })
}

/// pi `createCustomMessage`.
#[must_use]
pub fn custom_message(
    custom_type: &str,
    content: UserContentField,
    display: bool,
    details: Option<JsonValue>,
    timestamp: TimestampMs,
) -> AgentMessage {
    AgentMessage::Custom(CustomMessage {
        custom_type: custom_type.to_owned(),
        content,
        display,
        details,
        timestamp,
    })
}

fn user_text(text: String, timestamp: TimestampMs) -> Message {
    Message::User(UserMessage {
        content: UserContentField::Blocks(vec![UserContent::text(text)]),
        timestamp,
    })
}

/// Converts one agent message for the LLM (`None` when it is excluded).
#[must_use]
pub fn to_llm(message: &AgentMessage) -> Option<Message> {
    match message {
        AgentMessage::BashExecution(bash) => (bash.exclude_from_context != Some(true))
            .then(|| user_text(bash_execution_to_text(bash), bash.timestamp)),
        AgentMessage::Custom(custom) => Some(Message::User(UserMessage {
            content: UserContentField::Blocks(custom.content.clone().into_blocks()),
            timestamp: custom.timestamp,
        })),
        AgentMessage::BranchSummary(summary) => Some(user_text(
            format!(
                "{BRANCH_SUMMARY_PREFIX}{}{BRANCH_SUMMARY_SUFFIX}",
                summary.summary
            ),
            summary.timestamp,
        )),
        AgentMessage::CompactionSummary(summary) => Some(user_text(
            format!(
                "{COMPACTION_SUMMARY_PREFIX}{}{COMPACTION_SUMMARY_SUFFIX}",
                summary.summary
            ),
            summary.timestamp,
        )),
        other => other.as_llm(),
    }
}

/// pi `convertToLlm`.
#[must_use]
pub fn convert_to_llm(messages: &[AgentMessage]) -> Vec<Message> {
    messages.iter().filter_map(to_llm).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bash(raw: serde_json::Value) -> BashExecutionMessage {
        serde_json::from_value(raw).unwrap()
    }

    #[test]
    fn bash_text_matches_pi() {
        assert_eq!(
            bash_execution_to_text(&bash(
                json!({"command":"ls","output":"a","exitCode":2,"cancelled":false,
                "truncated":true,"fullOutputPath":"/t.log","timestamp":1})
            )),
            "Ran `ls`\n```\na\n```\n\nCommand exited with code 2\n\n[Output truncated. Full output: /t.log]"
        );
        assert_eq!(
            bash_execution_to_text(&bash(
                json!({"command":"x","output":"","cancelled":true,"truncated":false,"timestamp":1})
            )),
            "Ran `x`\n(no output)\n\n(command cancelled)"
        );
    }

    #[test]
    fn convert_wraps_summaries_and_skips_excluded() {
        let messages: Vec<AgentMessage> = serde_json::from_value(json!([
            {"role":"bashExecution","command":"x","output":"","cancelled":false,"truncated":false,"timestamp":1,"excludeFromContext":true},
            {"role":"custom","customType":"c","content":"hi","display":false,"timestamp":2},
            {"role":"compactionSummary","summary":"S","tokensBefore":1,"timestamp":3},
            {"role":"branchSummary","summary":"B","fromId":"x","timestamp":4}
        ]))
        .unwrap();
        let out = serde_json::to_value(convert_to_llm(&messages)).unwrap();
        assert_eq!(
            out[0],
            json!({"role":"user","content":[{"type":"text","text":"hi"}],"timestamp":2})
        );
        assert_eq!(
            out[1]["content"][0]["text"],
            json!(format!(
                "{COMPACTION_SUMMARY_PREFIX}S{COMPACTION_SUMMARY_SUFFIX}"
            ))
        );
        assert_eq!(
            out[2]["content"][0]["text"],
            json!(format!("{BRANCH_SUMMARY_PREFIX}B</summary>"))
        );
        assert_eq!(out.as_array().map(Vec::len), Some(3));
    }
}
