//! Summary generation (`completeSummarization`, `generateSummaryWithUsage`,
//! `generateTurnPrefixSummary`, `getSummarizationFailure`, `compact`) and the extension-facing
//! `summarize-default`.

use std::fmt::Write as _;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::BoxStream;
use indexmap::IndexMap;
use oino_agent::{StreamFn, StreamRequest, now_ms};
use oino_ai::transcript::initial_system_message;
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, AssistantMessageEvent, CacheRetention,
    Context, Message, Model, StopReason, StreamOptions, ThinkingLevel, Usage, UserContent,
    UserContentField, UserMessage,
};
use oino_session::convert_to_llm;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::error::CompactionError;
use super::prepare::{CompactionPreparation, CompactionResult};
use super::prompts::{
    SUMMARIZATION_PROMPT, SUMMARIZATION_SYSTEM_PROMPT, UPDATE_SUMMARIZATION_PROMPT,
};
use super::retry::{RetryPolicy, SummarizationRetryObserver, retry_assistant_call};
use super::utils::{format_file_operations, serialize_conversation};
use super::uuid::uuidv7;

/// Request headers as carried by [`StreamOptions::headers`].
pub type Headers = IndexMap<String, Option<String>>;

/// Everything a summary request needs: provider dispatch, resolved model and credentials,
/// retry policy and observer, cancellation (`model, apiKey, headers, signal,
/// thinkingLevel, streamFn, retry, callbacks, sessionId` parameters).
#[derive(Clone)]
pub struct Summarizer {
    /// Provider dispatch (the session's stream function).
    pub stream: Arc<dyn StreamFn>,
    /// Model to call.
    pub model: Model,
    /// Credential.
    pub api_key: Option<String>,
    /// Extra request headers.
    pub headers: Option<Headers>,
    /// Reasoning level (sent only for reasoning models and levels other than `off`; not used
    /// by branch summaries).
    pub thinking_level: Option<ThinkingLevel>,
    /// Routing session id; a fresh `UUIDv7` per call when `None` (never enables caching).
    pub session_id: Option<String>,
    /// Retry policy (`settings.retry`).
    pub retry: Option<RetryPolicy>,
    /// Retry progress observer.
    pub observer: Option<Arc<dyn SummarizationRetryObserver>>,
    /// Abort signal.
    pub cancel: CancellationToken,
    /// Final escalation sends one model request, without shrink/resample retries.
    pub single_attempt: bool,
}

/// Text and usage of one generated summary (`{text, usage}`; also the `summarize-default`
/// result `{summary, usage?}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryOutput {
    /// Summary text.
    pub summary: String,
    /// Provider usage.
    pub usage: Usage,
}

impl Summarizer {
    /// A summariser without credentials, retries or observer.
    #[must_use]
    pub fn new(stream: Arc<dyn StreamFn>, model: Model) -> Self {
        Self {
            stream,
            model,
            api_key: None,
            headers: None,
            thinking_level: None,
            session_id: None,
            retry: None,
            observer: None,
            cancel: CancellationToken::new(),
            single_attempt: false,
        }
    }

    /// Headers, routing session id and `reasoning` for
    /// reasoning models.
    #[must_use]
    pub fn summarization_options(&self) -> StreamOptions {
        StreamOptions {
            headers: self.headers.clone(),
            session_id: self.session_id.clone(),
            reasoning: self
                .thinking_level
                .filter(|level| self.model.reasoning && *level != ThinkingLevel::Off),
            ..StreamOptions::default()
        }
    }

    /// One request with prompt-cache writes disabled and a routing
    /// session id, retried on transient errors per [`Self::retry`].
    pub async fn complete(&self, context: Context, mut options: StreamOptions) -> AssistantMessage {
        options.cache_retention = Some(CacheRetention::None);
        options.session_id = Some(options.session_id.unwrap_or_else(uuidv7));
        let produce = || {
            let events = self.stream.stream(StreamRequest {
                model: self.model.clone(),
                context: context.clone(),
                options: options.clone(),
                api_key: self.api_key.clone(),
                cancel: self.cancel.clone(),
            });
            final_message(events, &self.model, &self.cancel)
        };
        retry_assistant_call(
            produce,
            self.retry.as_ref(),
            &self.cancel,
            self.observer.as_deref(),
        )
        .await
    }
}

/// The longest a summarisation stream may stay silent before it counts as stalled.
pub const SUMMARY_IDLE: std::time::Duration = std::time::Duration::from_secs(180);

/// Collects a provider stream to its terminal message (a missing terminal event becomes an
/// error, or `aborted` when cancelled).
async fn final_message(
    mut events: BoxStream<'static, AssistantMessageEvent>,
    model: &Model,
    cancel: &CancellationToken,
) -> AssistantMessage {
    let mut last_partial = None;
    loop {
        // A provider stream that goes quiet (a wedged bridge, a dropped connection the
        // transport never notices) must not leave the session "Compacting…" forever
        // (owner 2026-11-06): no event for `SUMMARY_IDLE` fails the call.
        let event = match tokio::time::timeout(SUMMARY_IDLE, events.next()).await {
            Ok(Some(event)) => event,
            Ok(None) => break,
            Err(_) => {
                let mut message = last_partial.unwrap_or_else(|| {
                    AssistantMessage::empty(&model.api, &model.provider, &model.id, now_ms())
                });
                message.stop_reason = StopReason::Error;
                message.error_message = Some(format!(
                    "The model stopped responding (nothing for {} s)",
                    SUMMARY_IDLE.as_secs()
                ));
                return message;
            }
        };
        match event {
            AssistantMessageEvent::Done { message, .. }
            | AssistantMessageEvent::Error { error: message, .. } => return message,
            AssistantMessageEvent::Start { partial } => last_partial = Some(partial),
            _ => {}
        }
    }
    let mut message = last_partial.unwrap_or_else(|| {
        AssistantMessage::empty(&model.api, &model.provider, &model.id, now_ms())
    });
    message.stop_reason = if cancel.is_cancelled() {
        StopReason::Aborted
    } else {
        StopReason::Error
    };
    message
        .error_message
        .get_or_insert_with(|| "Stream ended without a final event".to_owned());
    message
}

/// The summarisation system prompt and one user message.
#[must_use]
pub fn summarization_context(prompt_text: &str) -> Context {
    let system =
        initial_system_message(super::prompts::BRANCH_SYSTEM_PROMPT, &[]).map(Message::System);
    let user = Message::User(UserMessage {
        content: UserContentField::Blocks(vec![UserContent::text(prompt_text)]),
        timestamp: now_ms(),
    });
    Context {
        messages: system.into_iter().chain(std::iter::once(user)).collect(),
    }
}

/// The error for responses that must not become a checkpoint
/// (`error`, or `length` with a partial summary).
#[must_use]
pub fn summarization_failure(response: &AssistantMessage, label: &str) -> Option<String> {
    match response.stop_reason {
        StopReason::Error => Some(format!(
            "{label} failed: {}",
            response
                .error_message
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or("Unknown error")
        )),
        StopReason::Length => Some(format!(
            "{label} failed: generation hit the token cap and the summary is incomplete"
        )),
        _ => None,
    }
}

/// Text blocks joined with newlines.
pub(crate) fn response_text(response: &AssistantMessage) -> String {
    response
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn has_tool_call(response: &AssistantMessage) -> bool {
    response
        .content
        .iter()
        .any(|block| matches!(block, AssistantContent::ToolCall(_)))
}

fn non_empty(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.is_empty())
}

/// Summarises `messages` (or updates `previous_summary`) with
/// the structured checkpoint prompt.
///
/// # Errors
///
/// [`CompactionError::Summarization`] for error/length responses,
/// [`CompactionError::ToolCall`] when the model called a tool.
pub async fn generate_summary(
    messages: &[AgentMessage],
    summarizer: &Summarizer,
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
) -> Result<SummaryOutput, CompactionError> {
    let previous = non_empty(previous_summary).map(strip_todos);
    let context_window = summarizer.model.context_window.max(4096);
    let instructions = custom_instructions.map(|text| truncate_tokens(text, context_window / 4));
    let mut last = "Summarizer produced an empty response".to_owned();
    // Context failures shrink the transcript. Empty/degenerate output is resampled once.
    for percent in [90, 75, 50]
        .into_iter()
        .take(if summarizer.single_attempt { 1 } else { 3 })
    {
        let window = context_window.saturating_mul(percent) / 100;
        let reserved = 4000
            + previous.as_ref().map_or(0, |s| text_tokens(s))
            + instructions.as_ref().map_or(0, |s| text_tokens(s))
            + text_tokens(SUMMARIZATION_SYSTEM_PROMPT);
        let transcript = fit_transcript(messages, window.saturating_sub(reserved));
        let mut prompt = match &previous {
            Some(previous) => format!(
                "Previous summary:\n```\n{previous}\n```\n{}\nThese messages were sent since the previous summary:\n```\n{transcript}\n```\n",
                if text_tokens(previous) > 6000 {
                    super::prompts::SOFT_CAP_INSTRUCTION
                } else {
                    ""
                }
            ),
            None => {
                format!("Please summarize the following conversation:\n```\n{transcript}\n```\n")
            }
        };
        if let Some(instructions) = &instructions {
            let _ = write!(
                prompt,
                "The user has provided the following instructions for this summary:\n{instructions}\n\nIf necessary, you may add new sections or modify existing ones to align with the user's intent.\n\n"
            );
        }
        prompt.push_str(if previous.is_some() {
            "Please incorporate the new messages into the existing summary, preserving important details from both. Remember to wrap your final summary in <summary> tags."
        } else { "Remember to wrap your final summary in <summary> tags." });
        let paragraph = if previous.is_some() {
            UPDATE_SUMMARIZATION_PROMPT
        } else {
            SUMMARIZATION_PROMPT
        };
        let system = SUMMARIZATION_SYSTEM_PROMPT.replace("{continuation}", paragraph);
        for _ in 0..if summarizer.single_attempt { 1 } else { 2 } {
            let mut context = summarization_context(&prompt);
            context.messages[0] =
                Message::System(initial_system_message(&system, &[]).ok_or_else(|| {
                    CompactionError::Summarization("Missing summarizer prompt".into())
                })?);
            let response = summarizer
                .complete(context, summarizer.summarization_options())
                .await;
            if summarizer.cancel.is_cancelled() {
                return Err(CompactionError::Summarization(
                    "Compaction cancelled".into(),
                ));
            }
            if response.stop_reason == StopReason::Error {
                last = response
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "Summary request failed".into());
                if super::errors::is_context_overflow(&response, context_window) {
                    break;
                }
                return Err(CompactionError::Summarization(last));
            }
            let text = response_text(&response);
            let summary = text
                .split_once("<summary>")
                .map_or(text.as_str(), |(_, rest)| rest)
                .split("</summary>")
                .next()
                .unwrap_or_default()
                .trim();
            if !has_tool_call(&response)
                && !summary.is_empty()
                && summary.chars().any(char::is_alphanumeric)
            {
                return Ok(SummaryOutput {
                    summary: summary.to_owned(),
                    usage: response.usage,
                });
            }
            "Summarizer produced empty or degenerate output".clone_into(&mut last);
        }
    }
    Err(CompactionError::Summarization(last))
}

/// Remove the appended TODO snapshot before integrating a delta.
#[must_use]
pub fn strip_todos(text: &str) -> String {
    text.split("\nCurrent TODO List:")
        .next()
        .unwrap_or(text)
        .trim_end()
        .to_owned()
}

fn text_tokens(text: &str) -> u64 {
    u64::try_from(text.chars().count().div_ceil(4)).unwrap_or(u64::MAX)
}
fn truncate_tokens(text: &str, budget: u64) -> String {
    text.chars()
        .take(usize::try_from(budget.saturating_mul(4)).unwrap_or(usize::MAX))
        .collect()
}
fn fit_transcript(messages: &[AgentMessage], budget: u64) -> String {
    let serialized = messages
        .iter()
        .map(|m| serialize_conversation(&convert_to_llm(std::slice::from_ref(m))))
        .collect::<Vec<_>>();
    let mut remaining = budget.saturating_sub(32);
    let mut first = serialized.len();
    for (index, text) in serialized.iter().enumerate().rev() {
        let cost = text_tokens(text);
        if cost > remaining {
            break;
        }
        remaining -= cost;
        first = index;
    }
    let mut result = if first > 0 {
        format!(
            "[{first} earlier messages omitted because they did not fit the summarizer's context window]\n"
        )
    } else {
        String::new()
    };
    if first == serialized.len() && !serialized.is_empty() {
        // Pin the newest message's suffix even when a single tool result exceeds the window.
        let tail = serialized.last().map(String::as_str).unwrap_or_default();
        let chars = usize::try_from(remaining.saturating_mul(4)).unwrap_or(usize::MAX);
        let mut suffix = tail.chars().rev().take(chars).collect::<Vec<_>>();
        suffix.reverse();
        result.extend(suffix);
    } else {
        result.push_str(&serialized[first..].join("\n"));
    }
    result
}

/// Core's default summariser exposed to extensions (`models.summarize-default`, C9): a
/// summary of `messages` with optional extra focus, updating `previous_summary` when given
///.
///
/// # Errors
///
/// As [`generate_summary`].
pub async fn summarize_default(
    messages: &[AgentMessage],
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
    summarizer: &Summarizer,
) -> Result<SummaryOutput, CompactionError> {
    generate_summary(messages, summarizer, custom_instructions, previous_summary).await
}

/// Generates the summary for a [`CompactionPreparation`] (history summary,
/// plus a turn-prefix summary for split turns, merged), appends the file lists and returns
/// the [`CompactionResult`] with `details: {readFiles, modifiedFiles}`.
///
/// An aborted request yields the (usually empty) aborted text; callers check
/// cancellation themselves.
///
/// # Errors
///
/// As [`generate_summary`] (labels `Summarization` / `Turn prefix summarization`), and
/// [`CompactionError::MissingFirstKept`] for an empty first-kept id.
pub async fn compact(
    preparation: &CompactionPreparation,
    summarizer: &Summarizer,
    custom_instructions: Option<&str>,
) -> Result<CompactionResult, CompactionError> {
    let mut messages = preparation.messages_to_summarize.clone();
    messages.extend(preparation.turn_prefix_messages.iter().cloned());
    let output = generate_summary(
        &messages,
        summarizer,
        custom_instructions,
        preparation.previous_summary.as_deref(),
    )
    .await?;
    let mut summary = output.summary;
    let usage = output.usage;
    let lists = preparation.file_ops.file_lists();
    summary.push_str(&format_file_operations(
        &lists.read_files,
        &lists.modified_files,
    ));
    if preparation.first_kept_entry_id.as_str().is_empty() {
        return Err(CompactionError::MissingFirstKept);
    }
    Ok(CompactionResult {
        summary,
        first_kept_entry_id: preparation.first_kept_entry_id.clone(),
        tokens_before: preparation.tokens_before,
        estimated_tokens_after: None,
        usage: Some(usage),
        details: serde_json::to_value(&lists).ok(),
    })
}

#[cfg(test)]
mod idle_tests {
    use super::*;

    /// A provider stream that never says anything more fails after [`SUMMARY_IDLE`] instead of
    /// keeping the session "Compacting…" forever (owner 2026-11-06).
    #[tokio::test(start_paused = true)]
    async fn a_silent_summary_stream_fails_after_the_idle_limit() {
        let model = crate::testing::FauxProvider::model();
        let silent: BoxStream<'static, AssistantMessageEvent> = futures::stream::pending().boxed();
        let started = tokio::time::Instant::now();
        let message = final_message(silent, &model, &CancellationToken::new()).await;
        assert_eq!(message.stop_reason, StopReason::Error);
        assert!(
            message
                .error_message
                .as_deref()
                .is_some_and(|m| m.contains("stopped responding"))
        );
        assert!(started.elapsed() >= SUMMARY_IDLE);
    }
}
