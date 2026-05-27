#![forbid(unsafe_code)]

//! LLM-based compaction engine.
//!
//! Produces a structured context summary by sending the conversation history
//! to an LLM model, similar to how the pi coding agent compacts sessions.

use oino_agent_loop::{AbortSignal, StreamProvider, StreamRequest};
use oino_session::{SessionEntry, SessionEntryKind};
use oino_types::{ContentBlock, Message, Model, OinoId, StopReason, ThinkingLevel};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

/// Result of an LLM compaction pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCompaction {
    pub summary: String,
    pub replaces: Vec<OinoId>,
    pub compacted_entries: usize,
    pub kept_entries: usize,
}

/// Compaction using an LLM to summarize session history.
///
/// # Arguments
/// * `branch` - The active branch of session entries.
/// * `stream` - The stream provider for making LLM calls.
/// * `model` - The model to use for summarization.
/// * `custom_prompt` - Optional custom system prompt for the compaction LLM.
/// * `signal` - Abort signal for cancellation.
pub async fn compact_with_llm(
    branch: &[SessionEntry],
    stream: &dyn StreamProvider,
    model: Model,
    custom_prompt: Option<&str>,
    signal: AbortSignal,
) -> Result<LlmCompaction, String> {
    let last_user_index = branch
        .iter()
        .rposition(is_user_message_entry)
        .ok_or("No user message found in branch")?;

    if last_user_index == 0 {
        return Err("Nothing to compact; only one user message at the tail".into());
    }

    let compacted = &branch[..last_user_index];
    if compacted.is_empty() {
        return Err("Nothing to compact".into());
    }

    let kept_entries = branch.len().saturating_sub(compacted.len());

    // Find any previous compaction summary to do iterative update
    let previous_summary = find_previous_summary(compacted);

    // Serialize the conversation to summarize
    let conversation = serialize_conversation(compacted);

    let system_prompt = custom_prompt
        .map(str::to_string)
        .unwrap_or_else(default_compaction_system_prompt);

    let user_prompt = if let Some(prev) = previous_summary {
        iterative_prompt(&prev, &conversation)
    } else {
        initial_prompt(&conversation)
    };

    // Build the request messages
    let messages = vec![Message::user_text(user_prompt)];

    let request = StreamRequest {
        model: model.clone(),
        thinking_level: ThinkingLevel::Low,
        system_prompt: Some(system_prompt),
        messages,
        tools: vec![],
    };

    // Call the LLM
    let events = stream
        .stream(request, signal)
        .await
        .map_err(|e| format!("LLM compaction call failed: {e}"))?;

    // Extract text from events
    let mut summary_text = String::new();
    let mut stop_reason = StopReason::Unknown;
    for event in &events {
        match event {
            oino_types::AssistantStreamEvent::TextDelta { delta } => {
                summary_text.push_str(delta);
            }
            oino_types::AssistantStreamEvent::Done {
                stop_reason: sr, ..
            } => {
                stop_reason = sr.clone();
            }
            oino_types::AssistantStreamEvent::Error { message } => {
                return Err(format!("LLM compaction error: {message}"));
            }
            _ => {}
        }
    }

    if summary_text.trim().is_empty() {
        return Err(format!(
            "LLM compaction produced empty summary (stop_reason: {stop_reason:?})"
        ));
    }

    let replaces = compacted.iter().map(|entry| entry.id).collect();

    Ok(LlmCompaction {
        summary: summary_text,
        replaces,
        compacted_entries: compacted.len(),
        kept_entries,
    })
}

/// Load a custom compaction prompt from a file path.
pub async fn load_custom_prompt(path: &Path) -> Option<String> {
    match fs::read_to_string(path).await {
        Ok(text) if !text.trim().is_empty() => Some(text),
        _ => None,
    }
}

/// Load the compaction prompt from the project's `.oino/prompts/compact.md` file.
pub async fn load_project_compact_prompt(project_dir: &Path) -> Option<String> {
    let prompt_path = project_dir.join(".oino").join("prompts").join("compact.md");
    load_custom_prompt(&prompt_path).await
}

fn default_compaction_system_prompt() -> String {
    "You are a context summarization assistant. Your task is to read a conversation \
     between a user and an AI coding assistant, then produce a structured summary \
     following the exact format specified.\n\n\
     Do NOT continue the conversation. Do NOT respond to any questions in the conversation. \
     ONLY output the structured summary."
        .into()
}

fn initial_prompt(conversation: &str) -> String {
    format!(
        "The messages below are a conversation to summarize. Create a structured \
         context checkpoint summary that another LLM will use to continue the work.\n\n\
         Use this EXACT format:\n\n\
         ## Goal\n\
         [What is the user trying to accomplish?]\n\n\
         ## Constraints & Preferences\n\
         - [Any constraints, preferences, or requirements]\n\
         - (or \"(none)\" if none)\n\n\
         ## Progress\n\
         ### Done\n\
         - [x] [Completed tasks/changes]\n\n\
         ### In Progress\n\
         - [ ] [Current work]\n\n\
         ### Blocked\n\
         - [Issues preventing progress, if any]\n\n\
         ## Key Decisions\n\
         - **[Decision]**: [Brief rationale]\n\n\
         ## Next Steps\n\
         1. [Ordered list of what should happen next]\n\n\
         ## Critical Context\n\
         - [Any data, file paths, function names, error messages needed to continue]\n\n\
         Keep each section concise. Preserve exact file paths, function names, and error messages.\n\n\
         <conversation>\n\
         {conversation}\n\
         </conversation>"
    )
}

fn iterative_prompt(previous_summary: &str, new_conversation: &str) -> String {
    format!(
        "The messages below are NEW conversation messages to incorporate into the existing \
         summary provided in <previous-summary> tags.\n\n\
         Update the existing structured summary with new information. RULES:\n\
         - PRESERVE all existing information from the previous summary\n\
         - ADD new progress, decisions, and context from the new messages\n\
         - UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n\
         - UPDATE \"Next Steps\" based on what was accomplished\n\
         - PRESERVE exact file paths, function names, and error messages\n\
         - If something is no longer relevant, you may remove it\n\n\
         Use this EXACT format:\n\n\
         ## Goal\n\
         [Preserve existing goals, add new ones if the task expanded]\n\n\
         ## Constraints & Preferences\n\
         - [Preserve existing, add new ones discovered]\n\n\
         ## Progress\n\
         ### Done\n\
         - [x] [Include previously done AND newly completed items]\n\n\
         ### In Progress\n\
         - [ ] [Current work]\n\n\
         ### Blocked\n\
         - [Current blockers - remove if resolved]\n\n\
         ## Key Decisions\n\
         - **[Decision]**: [Brief rationale] (preserve all previous, add new)\n\n\
         ## Next Steps\n\
         1. [Update based on current state]\n\n\
         ## Critical Context\n\
         - [Preserve important context, add new if needed]\n\n\
         Keep each section concise. Preserve exact file paths, function names, and error messages.\n\n\
         <previous-summary>\n\
         {previous_summary}\n\
         </previous-summary>\n\n\
         <conversation>\n\
         {new_conversation}\n\
         </conversation>"
    )
}

fn find_previous_summary(entries: &[SessionEntry]) -> Option<String> {
    entries.iter().rev().find_map(|entry| match &entry.kind {
        SessionEntryKind::Compaction { summary, .. } => Some(summary.clone()),
        _ => None,
    })
}

fn serialize_conversation(entries: &[SessionEntry]) -> String {
    let mut output = String::new();
    for entry in entries {
        let message = match &entry.kind {
            SessionEntryKind::Message { message } | SessionEntryKind::CustomMessage { message } => {
                message
            }
            SessionEntryKind::Compaction { summary, .. } => {
                output.push_str(&format!("[Compaction Summary]: {summary}\n\n"));
                continue;
            }
            _ => continue,
        };
        match message {
            Message::User { content, .. } => {
                let text = blocks_to_text(content);
                if !text.is_empty() {
                    output.push_str(&format!("[User]: {text}\n\n"));
                }
            }
            Message::Assistant { content, .. } => {
                let text = blocks_to_text(content);
                let tools = blocks_to_tools(content);
                if !tools.is_empty() {
                    output.push_str(&format!("[Assistant tool calls]: {tools}\n"));
                }
                if !text.is_empty() {
                    output.push_str(&format!("[Assistant]: {text}\n\n"));
                } else if !tools.is_empty() {
                    output.push('\n');
                }
            }
            Message::ToolResult {
                tool_name,
                content,
                is_error,
                ..
            } => {
                let text = truncate_str(&blocks_to_text(content), 2000);
                let prefix = if *is_error { "error" } else { "result" };
                output.push_str(&format!("[Tool {prefix} ({tool_name})]: {text}\n\n"));
            }
            _ => {}
        }
    }
    output
}

fn blocks_to_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn blocks_to_tools(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolCall {
                name, arguments, ..
            } => {
                let args_str = truncate_str(&arguments.to_string(), 200);
                Some(format!("{name}({args_str})"))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

fn is_user_message_entry(entry: &SessionEntry) -> bool {
    matches!(
        &entry.kind,
        SessionEntryKind::Message {
            message: Message::User { .. }
        } | SessionEntryKind::CustomMessage {
            message: Message::User { .. }
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_session::{SessionHeader, SessionManager};

    #[test]
    fn serialize_conversation_formats_messages() {
        let mut session = SessionManager::new(SessionHeader::new("test", "/tmp".into()));
        session.append_message(Message::user_text("Fix the bug in main.rs"));
        session.append_message(Message::assistant_text(
            "I'll check the file",
            StopReason::ToolUse,
        ));
        let branch = session
            .get_branch(session.get_leaf_id())
            .unwrap_or_else(|err| panic!("branch failed: {err}"));

        let output = serialize_conversation(&branch);
        assert!(output.contains("[User]: Fix the bug in main.rs"));
        assert!(output.contains("[Assistant]: I'll check the file"));
    }

    #[test]
    fn initial_prompt_contains_conversation() {
        let prompt = initial_prompt("hello world");
        assert!(prompt.contains("<conversation>"));
        assert!(prompt.contains("hello world"));
        assert!(prompt.contains("## Goal"));
    }

    #[test]
    fn iterative_prompt_contains_previous_summary() {
        let prompt = iterative_prompt("old summary", "new messages");
        assert!(prompt.contains("<previous-summary>"));
        assert!(prompt.contains("old summary"));
        assert!(prompt.contains("<conversation>"));
        assert!(prompt.contains("new messages"));
    }
}
