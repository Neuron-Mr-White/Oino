//! Summary generation (pi `completeSummarization`, `generateSummaryWithUsage`,
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
    SUMMARIZATION_PROMPT, SUMMARIZATION_SYSTEM_PROMPT, TURN_PREFIX_SUMMARIZATION_PROMPT,
    UPDATE_SUMMARIZATION_PROMPT,
};
use super::retry::{RetryPolicy, SummarizationRetryObserver, retry_assistant_call};
use super::utils::{format_file_operations, serialize_conversation};
use super::uuid::uuidv7;

/// Request headers as carried by [`StreamOptions::headers`].
pub type Headers = IndexMap<String, Option<String>>;

/// Everything a summary request needs: provider dispatch, resolved model and credentials,
/// retry policy and observer, cancellation (pi's `model, apiKey, headers, signal,
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
}

/// Text and usage of one generated summary (pi `{text, usage}`; also the `summarize-default`
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
        }
    }

    /// pi `createSummarizationOptions`: headers, routing session id and `reasoning` for
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

    /// pi `completeSummarization`: one request with prompt-cache writes disabled and a routing
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

/// Collects a provider stream to its terminal message (a missing terminal event becomes an
/// error, or `aborted` when cancelled).
async fn final_message(
    mut events: BoxStream<'static, AssistantMessageEvent>,
    model: &Model,
    cancel: &CancellationToken,
) -> AssistantMessage {
    let mut last_partial = None;
    while let Some(event) = events.next().await {
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

/// pi `buildSummarizationContext`: the summarisation system prompt and one user message.
#[must_use]
pub fn summarization_context(prompt_text: &str) -> Context {
    let system = initial_system_message(SUMMARIZATION_SYSTEM_PROMPT, &[]).map(Message::System);
    let user = Message::User(UserMessage {
        content: UserContentField::Blocks(vec![UserContent::text(prompt_text)]),
        timestamp: now_ms(),
    });
    Context {
        messages: system.into_iter().chain(std::iter::once(user)).collect(),
    }
}

/// pi `getSummarizationFailure`: the error for responses that must not become a checkpoint
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

/// pi `contentText(content)`: text blocks joined with newlines.
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

/// Runs one summary request and validates the response.
async fn summarize_prompt(
    summarizer: &Summarizer,
    prompt_text: &str,
    label: &'static str,
) -> Result<SummaryOutput, CompactionError> {
    let response = summarizer
        .complete(
            summarization_context(prompt_text),
            summarizer.summarization_options(),
        )
        .await;
    if let Some(failure) = summarization_failure(&response, label) {
        return Err(CompactionError::Summarization(failure));
    }
    if has_tool_call(&response) {
        return Err(CompactionError::ToolCall(label));
    }
    Ok(SummaryOutput {
        summary: response_text(&response),
        usage: response.usage,
    })
}

/// pi `generateSummaryWithUsage`: summarises `messages` (or updates `previous_summary`) with
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
    let previous_summary = non_empty(previous_summary);
    let mut base_prompt = if previous_summary.is_some() {
        UPDATE_SUMMARIZATION_PROMPT.to_owned()
    } else {
        SUMMARIZATION_PROMPT.to_owned()
    };
    if let Some(instructions) = non_empty(custom_instructions) {
        base_prompt = format!("{base_prompt}\n\nAdditional focus: {instructions}");
    }
    let conversation = serialize_conversation(&convert_to_llm(messages));
    let mut prompt = format!("<conversation>\n{conversation}\n</conversation>\n\n");
    if let Some(previous) = previous_summary {
        let _ = write!(
            prompt,
            "<previous-summary>\n{previous}\n</previous-summary>\n\n"
        );
    }
    prompt.push_str(&base_prompt);
    summarize_prompt(summarizer, &prompt, "Summarization").await
}

/// pi `generateTurnPrefixSummary`: checkpoint of the prefix of a split turn.
async fn generate_turn_prefix_summary(
    messages: &[AgentMessage],
    summarizer: &Summarizer,
    custom_instructions: Option<&str>,
) -> Result<SummaryOutput, CompactionError> {
    let conversation = serialize_conversation(&convert_to_llm(messages));
    let mut prompt = format!(
        "# Conversation\n{conversation}\n\n# Instructions\n{TURN_PREFIX_SUMMARIZATION_PROMPT}"
    );
    if let Some(instructions) = non_empty(custom_instructions) {
        prompt = format!("{prompt}\n\nAdditional focus: {instructions}");
    }
    summarize_prompt(summarizer, &prompt, "Turn prefix summarization").await
}

/// Core's default summariser exposed to extensions (`models.summarize-default`, C9): a
/// summary of `messages` with optional extra focus, updating `previous_summary` when given
/// (pi `generateSummary(…, previousSummary)`).
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

/// pi `compact`: generates the summary for a [`CompactionPreparation`] (history summary,
/// plus a turn-prefix summary for split turns, merged), appends the file lists and returns
/// the [`CompactionResult`] with `details: {readFiles, modifiedFiles}`.
///
/// An aborted request yields the (usually empty) aborted text, as in pi; callers check
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
    let previous = preparation.previous_summary.as_deref();
    let (mut summary, usage) =
        if preparation.is_split_turn && !preparation.turn_prefix_messages.is_empty() {
            let history = if preparation.messages_to_summarize.is_empty() {
                None
            } else {
                Some(
                    generate_summary(
                        &preparation.messages_to_summarize,
                        summarizer,
                        custom_instructions,
                        previous,
                    )
                    .await?,
                )
            };
            let prefix = generate_turn_prefix_summary(
                &preparation.turn_prefix_messages,
                summarizer,
                custom_instructions,
            )
            .await?;
            let (history_text, usage) = match history {
                Some(history) => {
                    let mut usage = history.usage;
                    usage.accumulate(&prefix.usage);
                    (history.summary, usage)
                }
                None => (
                    previous.unwrap_or("No prior history.").to_owned(),
                    prefix.usage,
                ),
            };
            (
                format!(
                    "{history_text}\n\n---\n\n**Turn Context (split turn):**\n\n{}",
                    prefix.summary
                ),
                usage,
            )
        } else {
            let output = generate_summary(
                &preparation.messages_to_summarize,
                summarizer,
                custom_instructions,
                previous,
            )
            .await?;
            (output.summary, output.usage)
        };
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
