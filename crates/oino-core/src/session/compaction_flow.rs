//! Manual and automatic compaction plus overflow recovery (pi `compact`, `_checkCompaction`,
//! `_runAutoCompaction`, `_compactBeforeNextAssistantResponse`, `_omitRecoveryAttempt`).

use std::sync::{Arc, Weak};

use oino_ai::{AgentMessage, AssistantContent, AssistantMessage, Model, StopReason};
use oino_session::{CompactionDraft, EntryId, EntryKind, SessionEntry, SessionProjection};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, Inner, SessionOpError, lock};
use crate::compaction::{
    self, CompactionPreparation, CompactionResult, CompactionSettings, RetryPolicy,
    SummarizationRetryObserver, Summarizer, calculate_context_tokens, estimate_context_tokens,
    estimate_projected_context_tokens, estimate_tokens, is_context_overflow, is_recoverable_length,
    prepare_compaction, should_compact,
};
use crate::events::{AgentSessionEvent, CompactionReason, SummarizationSource};

/// Maps `summarization_retry_*` callbacks to session events.
pub(crate) struct RetryEvents {
    pub(crate) inner: Weak<Inner>,
    pub(crate) source: SummarizationSource,
}

impl SummarizationRetryObserver for RetryEvents {
    fn retry_scheduled(&self, attempt: u32, max_attempts: u32, delay_ms: u64, error_message: &str) {
        if let Some(inner) = self.inner.upgrade() {
            AgentSession::from_inner(inner).emit(AgentSessionEvent::SummarizationRetryScheduled {
                attempt,
                max_attempts,
                delay_ms,
                error_message: error_message.to_owned(),
            });
        }
    }

    fn retry_attempt_start(&self) {
        if let Some(inner) = self.inner.upgrade() {
            AgentSession::from_inner(inner).emit(
                AgentSessionEvent::SummarizationRetryAttemptStart {
                    source: self.source,
                },
            );
        }
    }

    fn retry_finished(&self, _success: bool, _attempt: u32, _final_error: Option<&str>) {
        if let Some(inner) = self.inner.upgrade() {
            AgentSession::from_inner(inner).emit(AgentSessionEvent::SummarizationRetryFinished);
        }
    }
}

fn reason_str(reason: CompactionReason) -> &'static str {
    match reason {
        CompactionReason::Manual => "manual",
        CompactionReason::Threshold => "threshold",
        CompactionReason::Overflow => "overflow",
    }
}

fn messages_tokens(messages: &[AgentMessage]) -> u64 {
    messages.iter().map(estimate_tokens).sum()
}

impl AgentSession {
    pub(crate) fn compaction_settings(&self, model: Option<&Model>) -> CompactionSettings {
        let settings = lock(&self.inner.settings)
            .settings()
            .compaction_settings(model.map(|m| (m.provider.as_str(), m.id.as_str())))
            .unwrap_or_else(|error| {
                tracing::warn!("invalid compaction settings: {error}");
                oino_settings::CompactionSettings {
                    enabled: true,
                    reserve_tokens: compaction::DEFAULT_COMPACTION_SETTINGS.reserve_tokens,
                    keep_recent_tokens: compaction::DEFAULT_COMPACTION_SETTINGS.keep_recent_tokens,
                }
            });
        CompactionSettings {
            enabled: settings.enabled,
            reserve_tokens: settings.reserve_tokens,
            keep_recent_tokens: settings.keep_recent_tokens,
        }
    }

    pub(crate) fn retry_policy(&self) -> RetryPolicy {
        let retry = lock(&self.inner.settings).settings().retry_settings();
        RetryPolicy {
            enabled: retry.enabled,
            max_retries: u32::try_from(retry.max_retries).unwrap_or(u32::MAX),
            base_delay_ms: retry.base_delay_ms,
            max_agent_delay_ms: Some(retry.max_agent_delay_ms),
        }
    }

    /// pi `_getSummarizationRequestAuth`: routes a virtual model, resolves credentials.
    pub(crate) async fn summarizer(
        &self,
        model: &Model,
        cancel: CancellationToken,
        source: SummarizationSource,
    ) -> Result<Summarizer, String> {
        let thinking = self.thinking_level();
        let (model, thinking) = if oino_models::virtual_models::is_virtual_api(&model.api) {
            let llm = oino_session::convert_to_llm(&self.messages());
            let route = self
                .route_virtual(
                    model,
                    llm,
                    oino_models::RouteReason::Direct,
                    thinking,
                    None,
                    &cancel,
                )
                .await?;
            (route.model, route.thinking_level)
        } else {
            (model.clone(), thinking)
        };
        let auth = self
            .inner
            .models
            .get_api_key_and_headers(&model, &oino_models::AuthOverrides::default())
            .await
            .map_err(|e| e.message)?;
        Ok(Summarizer {
            stream: self.inner.agent_stream(),
            model,
            api_key: auth.api_key,
            headers: auth
                .headers
                .map(|h| h.into_iter().map(|(k, v)| (k, Some(v))).collect()),
            thinking_level: Some(thinking),
            session_id: None,
            retry: Some(self.retry_policy()),
            observer: Some(Arc::new(RetryEvents {
                inner: Arc::downgrade(&self.inner),
                source,
            })),
            cancel,
        })
    }

    async fn run_default_compaction(
        &self,
        preparation: &CompactionPreparation,
        model: &Model,
        custom_instructions: Option<&str>,
        cancel: &CancellationToken,
        reason: CompactionReason,
    ) -> Result<CompactionResult, String> {
        let summarizer = self
            .summarizer(
                model,
                cancel.clone(),
                SummarizationSource::Compaction { reason },
            )
            .await?;
        compaction::compact(preparation, &summarizer, custom_instructions)
            .await
            .map_err(|e| e.to_string())
    }

    async fn emit_session_compact_failed(
        &self,
        reason: CompactionReason,
        error_message: Option<&str>,
        aborted: bool,
        will_retry: bool,
        from_extension: bool,
    ) {
        let runner = self.runner();
        if runner.has_handlers("session_compact_failed") {
            let mut payload = json!({"type": "session_compact_failed", "reason": reason_str(reason), "aborted": aborted, "willRetry": will_retry, "fromExtension": from_extension});
            if let Some(message) = error_message {
                payload["errorMessage"] = json!(message);
            }
            runner.emit(payload).await;
        }
    }

    /// Runs `session_before_compact`; `Err(true)` = cancelled.
    async fn before_compact(
        &self,
        preparation: &CompactionPreparation,
        branch: &[SessionEntry],
        custom_instructions: Option<&str>,
        reason: CompactionReason,
        will_retry: bool,
    ) -> Result<Option<CompactionResult>, ()> {
        let runner = self.runner();
        if !runner.has_handlers("session_before_compact") {
            return Ok(None);
        }
        let mut payload = json!({
            "type": "session_before_compact",
            "preparation": preparation,
            "branchEntries": branch,
            "reason": reason_str(reason),
            "willRetry": will_retry,
        });
        if let Some(instructions) = custom_instructions {
            payload["customInstructions"] = json!(instructions);
        }
        let Some(result) = runner.emit(payload).await else {
            return Ok(None);
        };
        if result.get("cancel") == Some(&serde_json::Value::Bool(true)) {
            return Err(());
        }
        Ok(result
            .get("compaction")
            .and_then(|c| serde_json::from_value(c.clone()).ok()))
    }

    /// Appends the compaction and emits `session_compact`; returns the result.
    async fn commit_compaction(
        &self,
        result: CompactionResult,
        from_extension: bool,
        reason: CompactionReason,
        will_retry: bool,
    ) -> Result<CompactionResult, String> {
        let entry_id = lock(&self.inner.manager)
            .append_compaction(CompactionDraft {
                summary: result.summary.clone(),
                first_kept_entry_id: Some(result.first_kept_entry_id.clone()),
                tokens_before: result.tokens_before,
                details: result.details.clone(),
                from_hook: Some(from_extension),
                usage: result.usage,
                system_message: None,
            })
            .map_err(|e| e.to_string())?;
        self.refresh_context();
        let after = messages_tokens(
            &lock(&self.inner.manager)
                .build_session_projection()
                .messages,
        );
        let entry = lock(&self.inner.manager).entry(&entry_id).cloned();
        let runner = self.runner();
        if let (Some(entry), true) = (entry, runner.has_handlers("session_compact")) {
            runner
                .emit(json!({"type": "session_compact", "compactionEntry": entry, "fromExtension": from_extension, "reason": reason_str(reason), "willRetry": will_retry}))
                .await;
        }
        Ok(CompactionResult {
            estimated_tokens_after: Some(after),
            ..result
        })
    }

    /// pi `compact`: manual compaction; aborts the current run first, never retries.
    ///
    /// # Errors
    ///
    /// `Already compacted`, `Nothing to compact (session too small)`, `Compaction cancelled`,
    /// or the summariser failure.
    pub async fn compact(
        &self,
        custom_instructions: Option<String>,
    ) -> Result<CompactionResult, SessionOpError> {
        self.abort().await;
        let cancel = CancellationToken::new();
        lock(&self.inner.cancels).compaction = Some(cancel.clone());
        self.mark_busy();
        self.emit(AgentSessionEvent::CompactionStart {
            reason: CompactionReason::Manual,
        });
        let mut from_extension = false;
        let mut cancelled_by_extension = false;
        let outcome: Result<CompactionResult, String> = async {
            let model = self.model().ok_or_else(|| {
                oino_models::auth::guidance::no_model_selected_message(
                    &self.inner.docs_root.join("docs"),
                )
            })?;
            let settings = self.compaction_settings(Some(&model));
            let branch: Vec<SessionEntry> = lock(&self.inner.manager)
                .branch(None)
                .into_iter()
                .cloned()
                .collect();
            let refs: Vec<&SessionEntry> = branch.iter().collect();
            let Some(preparation) = prepare_compaction(&refs, &settings) else {
                if branch
                    .last()
                    .is_some_and(|e| matches!(e.kind, EntryKind::Compaction(_)))
                {
                    return Err("Already compacted".to_owned());
                }
                return Err("Nothing to compact (session too small)".to_owned());
            };
            let Ok(supplied) = self
                .before_compact(
                    &preparation,
                    &branch,
                    custom_instructions.as_deref(),
                    CompactionReason::Manual,
                    false,
                )
                .await
            else {
                cancelled_by_extension = true;
                return Err("Compaction cancelled".to_owned());
            };
            let result = if let Some(result) = supplied {
                from_extension = true;
                result
            } else {
                self.run_default_compaction(
                    &preparation,
                    &model,
                    custom_instructions.as_deref(),
                    &cancel,
                    CompactionReason::Manual,
                )
                .await?
            };
            if cancel.is_cancelled() {
                return Err("Compaction cancelled".to_owned());
            }
            self.commit_compaction(result, from_extension, CompactionReason::Manual, false)
                .await
        }
        .await;
        lock(&self.inner.cancels).compaction = None;
        self.set_idle_if_idle();
        match outcome {
            Ok(result) => {
                self.emit(AgentSessionEvent::CompactionEnd {
                    reason: CompactionReason::Manual,
                    result: Some(result.clone()),
                    aborted: false,
                    will_retry: false,
                    error_message: None,
                });
                Ok(result)
            }
            Err(message) => {
                let aborted = cancel.is_cancelled() || cancelled_by_extension;
                let error_message = (!aborted).then(|| format!("Compaction failed: {message}"));
                self.emit(AgentSessionEvent::CompactionEnd {
                    reason: CompactionReason::Manual,
                    result: None,
                    aborted,
                    will_retry: false,
                    error_message: error_message.clone(),
                });
                self.emit_session_compact_failed(
                    CompactionReason::Manual,
                    error_message.as_deref(),
                    aborted,
                    false,
                    from_extension,
                )
                .await;
                Err(SessionOpError::Failed(message))
            }
        }
    }

    /// Enables or disables automatic compaction (settings).
    pub fn set_auto_compaction_enabled(&self, enabled: bool) {
        if let Err(error) = self.with_settings(|s| s.set_compaction_enabled(enabled)) {
            tracing::warn!("saving compaction.enabled failed: {error}");
        }
    }

    /// pi `_exceedsCompactionThreshold`.
    pub(crate) fn exceeds_compaction_threshold(
        &self,
        model: &Model,
        projection: &SessionProjection,
    ) -> bool {
        if model.context_window == 0 {
            return false;
        }
        let tokens = {
            let manager = lock(&self.inner.manager);
            estimate_projected_context_tokens(projection, &manager.branch(None)).tokens
        };
        should_compact(
            tokens,
            model.context_window,
            &self.compaction_settings(self.model().as_ref()),
        )
    }

    /// pi `_compactBeforeNextAssistantResponse`; returns the projected messages.
    pub(crate) async fn compact_before_next_assistant_response(&self) -> Vec<AgentMessage> {
        let projection = lock(&self.inner.manager).build_session_projection();
        let Some(model) = self.model() else {
            return projection.messages;
        };
        if oino_models::virtual_models::is_virtual_api(&model.api)
            || !self.exceeds_compaction_threshold(&model, &projection)
        {
            return projection.messages;
        }
        self.run_auto_compaction(CompactionReason::Threshold, false)
            .await;
        lock(&self.inner.manager)
            .build_session_projection()
            .messages
    }

    /// Entry id of a persisted assistant message (latest match on the branch).
    fn assistant_entry_id(&self, message: &AssistantMessage) -> Option<EntryId> {
        let manager = lock(&self.inner.manager);
        manager
            .branch(None)
            .into_iter()
            .rev()
            .find_map(|entry| match entry.message() {
                Some(AgentMessage::Assistant(a))
                    if a.timestamp == message.timestamp && a.model == message.model =>
                {
                    Some(entry.id.clone())
                }
                _ => None,
            })
    }

    /// pi `_checkCompaction`; returns whether the post-run loop should continue.
    #[allow(
        clippy::too_many_lines,
        reason = "one-to-one port of pi _checkCompaction"
    )]
    pub(crate) async fn check_compaction(
        &self,
        assistant: &AssistantMessage,
        skip_aborted: bool,
    ) -> bool {
        let model = self.model();
        let settings = self.compaction_settings(model.as_ref());
        if !settings.enabled || (skip_aborted && assistant.stop_reason == StopReason::Aborted) {
            return false;
        }
        let message_model = self.model_for_message(assistant);
        let same_model = message_model.is_some();
        let context_window = message_model
            .as_ref()
            .or(model.as_ref())
            .map_or(0, |m| m.context_window);
        let (branch, projection) = {
            let manager = lock(&self.inner.manager);
            (
                manager
                    .branch(None)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<SessionEntry>>(),
                manager.build_session_projection(),
            )
        };
        let latest_compaction = branch
            .iter()
            .rev()
            .find(|e| matches!(e.kind, EntryKind::Compaction(_)));
        if let Some(compaction) = latest_compaction
            && compaction
                .timestamp
                .to_ms()
                .is_some_and(|ms| assistant.timestamp <= ms)
        {
            return false;
        }
        let entry_id = self.assistant_entry_id(assistant);
        let projected = entry_id.as_ref().is_none_or(|id| {
            projection.entries.iter().any(|e| {
                &e.source_entry.id == id
                    && e.messages
                        .iter()
                        .any(|m| matches!(m, AgentMessage::Assistant(_)))
            })
        });
        let index = entry_id
            .as_ref()
            .and_then(|id| branch.iter().position(|e| &e.id == id));
        let after: &[SessionEntry] = index.map_or(&[], |i| &branch[i + 1..]);
        let post_edit = after
            .iter()
            .any(|e| matches!(e.kind, EntryKind::ContextEdit { .. }));
        let latest_edit = after.iter().rev().find_map(|e| match &e.kind {
            EntryKind::ContextEdit {
                target_id,
                replacement,
            } if Some(target_id) == entry_id.as_ref() => Some(replacement),
            _ => None,
        });
        let retained = entry_id.is_none()
            || (!after
                .iter()
                .any(|e| matches!(e.kind, EntryKind::Compaction(_)))
                && latest_edit.is_none_or(Option::is_some));
        let usage_matches = projected && !post_edit;
        let explicit =
            assistant.stop_reason == StopReason::Error && is_context_overflow(assistant, 0);
        let overflow = same_model
            && ((explicit && retained)
                || (usage_matches && is_context_overflow(assistant, context_window)));
        let recoverable = same_model
            && projected
            && message_model
                .as_ref()
                .is_some_and(|m| is_recoverable_length(assistant, m.max_tokens));
        if overflow || recoverable {
            let will_retry = assistant.stop_reason != StopReason::Stop;
            if !will_retry {
                return self
                    .run_auto_compaction(CompactionReason::Overflow, false)
                    .await;
            }
            if lock(&self.inner.book).overflow_recovery_attempted {
                let error_message = if overflow {
                    "Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model."
                } else {
                    "Truncated response recovery failed after one compact-and-retry attempt."
                };
                self.emit(AgentSessionEvent::CompactionEnd {
                    reason: CompactionReason::Overflow,
                    result: None,
                    aborted: false,
                    will_retry: false,
                    error_message: Some(error_message.to_owned()),
                });
                self.emit_session_compact_failed(
                    CompactionReason::Overflow,
                    Some(error_message),
                    false,
                    false,
                    false,
                )
                .await;
                return false;
            }
            lock(&self.inner.book).overflow_recovery_attempted = true;
            self.omit_recovery_attempt(assistant);
            let retry = self
                .run_auto_compaction(CompactionReason::Overflow, will_retry)
                .await;
            if retry {
                lock(&self.inner.book).failed_response = Some(assistant.clone());
            }
            return retry;
        }
        let has_edits = projection
            .entries
            .iter()
            .any(|e| matches!(e.source_entry.kind, EntryKind::ContextEdit { .. }));
        let direct = calculate_context_tokens(&assistant.usage);
        let context_tokens = if has_edits {
            let refs: Vec<&SessionEntry> = branch.iter().collect();
            estimate_projected_context_tokens(&projection, &refs).tokens
        } else if assistant.stop_reason == StopReason::Error || direct == 0 {
            let messages = self.messages();
            let estimate = estimate_context_tokens(&messages);
            if let (Some(usage_index), Some(compaction)) =
                (estimate.last_usage_index, latest_compaction)
                && let Some(AgentMessage::Assistant(usage_message)) = messages.get(usage_index)
                && compaction
                    .timestamp
                    .to_ms()
                    .is_some_and(|ms| usage_message.timestamp <= ms)
            {
                return false;
            }
            estimate.tokens
        } else {
            direct
        };
        if should_compact(context_tokens, context_window, &settings) {
            return self
                .run_auto_compaction(CompactionReason::Threshold, false)
                .await;
        }
        false
    }

    /// pi `_modelForMessage`.
    fn model_for_message(&self, message: &AssistantMessage) -> Option<Model> {
        let model = self.model()?;
        if oino_models::virtual_models::is_virtual_api(&model.api) {
            return self
                .inner
                .models
                .find_physical(&message.provider, &message.model);
        }
        (model.provider == message.provider && model.id == message.model).then_some(model)
    }

    /// pi `_isRetryableError`.
    pub(crate) fn is_retryable_error(&self, message: &AssistantMessage) -> bool {
        let window = self
            .model_for_message(message)
            .or_else(|| self.model())
            .map_or(0, |m| m.context_window);
        if is_context_overflow(message, window) {
            return false;
        }
        compaction::is_retryable_assistant_error(message)
    }

    /// pi `_omitRecoveryAttempt`: omits the failed attempt and its tool results from context.
    pub(crate) fn omit_recovery_attempt(&self, message: &AssistantMessage) {
        let call_ids: Vec<&str> = message
            .content
            .iter()
            .filter_map(|block| match block {
                AssistantContent::ToolCall(call) => Some(call.id.as_str()),
                _ => None,
            })
            .collect();
        let mut targets = Vec::new();
        {
            let manager = lock(&self.inner.manager);
            let branch = manager.branch(None);
            if let Some(index) = branch.iter().rposition(|entry| {
                matches!(entry.message(), Some(AgentMessage::Assistant(a)) if a.timestamp == message.timestamp && a.model == message.model)
            }) {
                targets.push(branch[index].id.clone());
                for entry in &branch[index + 1..] {
                    match entry.message() {
                        Some(AgentMessage::Assistant(_)) => break,
                        Some(AgentMessage::ToolResult(result)) if call_ids.contains(&result.tool_call_id.as_str()) => {
                            targets.push(entry.id.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
        for target in targets {
            let appended = lock(&self.inner.manager).append_context_edit(&target, None);
            match appended {
                Ok(id) => self.publish_entry(&id),
                Err(error) => tracing::error!("omitting recovery attempt failed: {error}"),
            }
        }
        self.refresh_context();
    }

    /// pi `_runAutoCompaction`; returns whether the post-run loop should continue.
    pub(crate) async fn run_auto_compaction(
        &self,
        reason: CompactionReason,
        will_retry: bool,
    ) -> bool {
        let Some(model) = self.model() else {
            return false;
        };
        let settings = self.compaction_settings(Some(&model));
        let branch: Vec<SessionEntry> = lock(&self.inner.manager)
            .branch(None)
            .into_iter()
            .cloned()
            .collect();
        let refs: Vec<&SessionEntry> = branch.iter().collect();
        let Some(preparation) = prepare_compaction(&refs, &settings) else {
            return false;
        };
        let cancel = CancellationToken::new();
        lock(&self.inner.cancels).auto_compaction = Some(cancel.clone());
        self.mark_busy();
        self.emit(AgentSessionEvent::CompactionStart { reason });
        let mut from_extension = false;
        let mut cancelled_by_extension = false;
        let outcome: Result<CompactionResult, String> = async {
            if cancel.is_cancelled() {
                return Err("aborted".to_owned());
            }
            let Ok(supplied) = self
                .before_compact(&preparation, &branch, None, reason, will_retry)
                .await
            else {
                cancelled_by_extension = true;
                return Err("Compaction cancelled".to_owned());
            };
            if cancel.is_cancelled() {
                return Err("aborted".to_owned());
            }
            let result = if let Some(result) = supplied {
                from_extension = true;
                result
            } else {
                self.run_default_compaction(&preparation, &model, None, &cancel, reason)
                    .await?
            };
            if cancel.is_cancelled() {
                return Err("aborted".to_owned());
            }
            self.commit_compaction(result, from_extension, reason, will_retry)
                .await
        }
        .await;
        lock(&self.inner.cancels).auto_compaction = None;
        let resume = match outcome {
            Ok(result) => {
                self.emit(AgentSessionEvent::CompactionEnd {
                    reason,
                    result: Some(result),
                    aborted: false,
                    will_retry,
                    error_message: None,
                });
                will_retry || self.inner.agent.has_queued_messages()
            }
            Err(message) => {
                let aborted = cancel.is_cancelled() || cancelled_by_extension;
                let error_message = (!aborted).then(|| match reason {
                    CompactionReason::Overflow => {
                        format!("Context overflow recovery failed: {message}")
                    }
                    _ => format!("Auto-compaction failed: {message}"),
                });
                self.emit(AgentSessionEvent::CompactionEnd {
                    reason,
                    result: None,
                    aborted,
                    will_retry: false,
                    error_message: error_message.clone(),
                });
                self.emit_session_compact_failed(
                    reason,
                    error_message.as_deref(),
                    aborted,
                    false,
                    from_extension,
                )
                .await;
                false
            }
        };
        self.set_idle_if_idle();
        resume
    }
}

impl Inner {
    /// The agent's stream function (summaries reuse the provider dispatch).
    pub(crate) fn agent_stream(&self) -> Arc<dyn oino_agent::StreamFn> {
        Arc::clone(&self.stream)
    }
}
