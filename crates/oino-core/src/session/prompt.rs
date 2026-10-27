//! Prompt pipeline and run lifecycle (pi `prompt`, `steer`, `followUp`, `sendCustomMessage`,
//! `sendUserMessage`, `_startRun`, `_prepareRun`, `_handlePostAgentRun`, `_emitAgentSettled`).

use futures::future::BoxFuture;
use oino_ai::{
    AgentMessage, CustomMessage, ImageContent, JsonValue, UserContent, UserContentField,
    UserMessage,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, SessionOpError, lock};
use crate::events::AgentSessionEvent;
use crate::extensions::api::CallKind;
use crate::extensions::host::{CustomMessageDraft, Delivery};
use crate::extensions::runner::InputOutcome;

/// How an accepted prompt was dispatched (pi `PromptDisposition`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptDisposition {
    /// An extension command or input handler consumed it.
    Handled,
    /// Queued (steering / follow-up).
    Queued,
    /// A run started.
    Started,
}

/// Queueing while streaming (pi `streamingBehavior`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamingBehavior {
    /// Steering queue.
    Steer,
    /// Follow-up queue.
    FollowUp,
}

impl StreamingBehavior {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Steer => "steer",
            Self::FollowUp => "followUp",
        }
    }
}

/// pi `InputSource`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputSource {
    /// A frontend user.
    #[default]
    Interactive,
    /// RPC client.
    Rpc,
    /// An extension.
    Extension,
}

impl InputSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Rpc => "rpc",
            Self::Extension => "extension",
        }
    }
}

/// Options of [`AgentSession::prompt`].
#[derive(Clone, Debug, PartialEq)]
pub struct PromptOptions {
    /// Dispatch extension commands, expand skills and templates (default `true`).
    pub expand_prompt_templates: bool,
    /// Image attachments.
    pub images: Vec<ImageContent>,
    /// Queueing while streaming.
    pub streaming_behavior: Option<StreamingBehavior>,
    /// Input source.
    pub source: InputSource,
}

impl Default for PromptOptions {
    fn default() -> Self {
        Self {
            expand_prompt_templates: true,
            images: Vec::new(),
            streaming_behavior: None,
            source: InputSource::Interactive,
        }
    }
}

/// What starts a run (pi `BeforeAgentStartTrigger`).
#[derive(Clone, Debug)]
pub(crate) enum Trigger {
    User {
        prompt: String,
        images: Vec<ImageContent>,
    },
    Custom(CustomMessage),
}

fn user_message(text: String, images: Vec<ImageContent>) -> AgentMessage {
    let mut content = vec![UserContent::text(text)];
    content.extend(images.into_iter().map(UserContent::Image));
    AgentMessage::User(UserMessage {
        content: UserContentField::Blocks(content),
        timestamp: oino_agent::now_ms(),
    })
}

fn images_json(images: &[ImageContent]) -> JsonValue {
    JsonValue::Array(
        images
            .iter()
            .map(|i| json!({"type": "image", "data": i.data, "mimeType": i.mime_type}))
            .collect(),
    )
}

type Deferred = BoxFuture<'static, ()>;

impl AgentSession {
    /// Sends a prompt (pi `prompt`): extension commands run immediately, `input` handlers
    /// may transform or handle it, skills and templates expand, and while streaming it is
    /// queued per `streaming_behavior`. Returns once the run started (await
    /// [`AgentSession::wait_for_idle`] for completion).
    ///
    /// # Errors
    ///
    /// pi's errors: streaming without behaviour, no model, no API key, compaction running.
    pub fn prompt(
        &self,
        text: String,
        options: PromptOptions,
    ) -> BoxFuture<'_, Result<PromptDisposition, SessionOpError>> {
        Box::pin(self.prompt_inner(text, options))
    }

    async fn prompt_inner(
        &self,
        text: String,
        options: PromptOptions,
    ) -> Result<PromptDisposition, SessionOpError> {
        if lock(&self.inner.book).emitting_settled {
            let session = self.clone();
            self.defer(Box::pin(async move {
                if let Err(error) = session.prompt(text, options).await {
                    session
                        .runner()
                        .emit_error("<runtime>", "prompt", error.to_string());
                }
            }));
            return Ok(PromptDisposition::Queued);
        }
        if options.expand_prompt_templates
            && text.starts_with('/')
            && self.try_execute_extension_command(&text).await
        {
            return Ok(PromptDisposition::Handled);
        }
        if lock(&self.inner.cancels).compaction.is_some() {
            return Err(SessionOpError::CompactionInProgress);
        }
        let streaming = self.is_streaming();
        let Some((text, images)) = self
            .run_input_handlers(
                text,
                options.images,
                options.source,
                options.streaming_behavior.filter(|_| streaming),
            )
            .await
        else {
            return Ok(PromptDisposition::Handled);
        };
        let expanded = if options.expand_prompt_templates {
            self.expand_templates(&text)
        } else {
            text
        };
        if self.is_streaming() {
            let behavior = options
                .streaming_behavior
                .ok_or(SessionOpError::StreamingWithoutBehavior)?;
            self.queue_input(behavior, expanded, images);
            return Ok(PromptDisposition::Queued);
        }
        self.validate_model_and_auth()?;
        self.start_run(Trigger::User {
            prompt: expanded,
            images,
        })
        .await?;
        Ok(PromptDisposition::Started)
    }

    /// pi `steer`: queues a steering message.
    ///
    /// # Errors
    ///
    /// [`SessionOpError::CommandNotAllowed`] for extension commands.
    pub async fn steer(
        &self,
        text: String,
        images: Vec<ImageContent>,
        source: InputSource,
    ) -> Result<PromptDisposition, SessionOpError> {
        self.queue_user_input(text, images, StreamingBehavior::Steer, source)
            .await
    }

    /// pi `followUp`: queues a follow-up message.
    ///
    /// # Errors
    ///
    /// [`SessionOpError::CommandNotAllowed`] for extension commands.
    pub async fn follow_up(
        &self,
        text: String,
        images: Vec<ImageContent>,
        source: InputSource,
    ) -> Result<PromptDisposition, SessionOpError> {
        self.queue_user_input(text, images, StreamingBehavior::FollowUp, source)
            .await
    }

    async fn queue_user_input(
        &self,
        text: String,
        images: Vec<ImageContent>,
        behavior: StreamingBehavior,
        source: InputSource,
    ) -> Result<PromptDisposition, SessionOpError> {
        if text.starts_with('/') {
            let name = command_name(&text);
            if self.runner().command(name).is_some() {
                return Err(SessionOpError::CommandNotAllowed(name.to_owned()));
            }
        }
        let streaming = self.is_streaming();
        let Some((text, images)) = self
            .run_input_handlers(text, images, source, streaming.then_some(behavior))
            .await
        else {
            return Ok(PromptDisposition::Handled);
        };
        let expanded = self.expand_templates(&text);
        self.queue_input(behavior, expanded, images);
        Ok(PromptDisposition::Queued)
    }

    fn queue_input(&self, behavior: StreamingBehavior, text: String, images: Vec<ImageContent>) {
        {
            let mut book = lock(&self.inner.book);
            match behavior {
                StreamingBehavior::Steer => book.steering.push(text.clone()),
                StreamingBehavior::FollowUp => book.follow_up.push(text.clone()),
            }
        }
        self.emit_queue_update();
        let message = user_message(text, images);
        match behavior {
            StreamingBehavior::Steer => self.inner.agent.steer(message),
            StreamingBehavior::FollowUp => self.inner.agent.follow_up(message),
        }
    }

    fn expand_templates(&self, text: &str) -> String {
        let expanded = self.expand_skill_command(text);
        let resources = self
            .inner
            .resources
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        oino_resources::templates::expand_prompt_template(&expanded, resources.prompts())
            .unwrap_or(expanded)
    }

    /// pi `_expandSkillCommand`.
    fn expand_skill_command(&self, text: &str) -> String {
        let resources = self
            .inner
            .resources
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = oino_resources::skills::parse_skill_command(text).and_then(|command| {
            resources
                .skills()
                .iter()
                .find(|skill| skill.name == command.name)
                .map(|skill| skill.file_path.display().to_string())
        });
        match oino_resources::skills::expand_skill_command(text, resources.skills()) {
            Ok(Some(block)) => block,
            Ok(None) => text.to_owned(),
            Err(error) => {
                drop(resources);
                self.runner().emit_error(
                    path.as_deref().unwrap_or("<skill>"),
                    "skill_expansion",
                    error.to_string(),
                );
                text.to_owned()
            }
        }
    }

    async fn run_input_handlers(
        &self,
        text: String,
        images: Vec<ImageContent>,
        source: InputSource,
        streaming: Option<StreamingBehavior>,
    ) -> Option<(String, Vec<ImageContent>)> {
        let runner = self.runner();
        if !runner.has_handlers("input") {
            return Some((text, images));
        }
        let images_opt = (!images.is_empty()).then_some(images.as_slice());
        match runner
            .emit_input(
                &text,
                images_opt,
                source.as_str(),
                streaming.map(StreamingBehavior::as_str),
            )
            .await
        {
            InputOutcome::Handled => None,
            InputOutcome::Transform { text, images: next } => Some((text, next.unwrap_or(images))),
            InputOutcome::Continue => Some((text, images)),
        }
    }

    fn validate_model_and_auth(&self) -> Result<(), SessionOpError> {
        let docs = self.inner.docs_root.join("docs");
        let Some(model) = self.model() else {
            return Err(SessionOpError::NoModel(
                oino_models::auth::guidance::no_model_selected_message(&docs),
            ));
        };
        let models = &self.inner.models;
        if models.has_configured_auth(&model.provider)
            || models.check_auth(&model.provider).is_some()
        {
            return Ok(());
        }
        let message = if models.is_using_oauth(&model.provider) {
            super::hooks::oauth_failed_message(&model.provider)
        } else {
            oino_models::auth::guidance::no_api_key_found_message(&model.provider, &docs)
        };
        Err(SessionOpError::NoApiKey {
            provider: model.provider,
            message,
        })
    }

    /// Runs an extension command if `text` names one (pi `_tryExecuteExtensionCommand`).
    async fn try_execute_extension_command(&self, text: &str) -> bool {
        let name = command_name(text);
        let args = text.split_once(' ').map_or("", |(_, rest)| rest);
        let runner = self.runner();
        let Some(command) = runner.command(name) else {
            return false;
        };
        self.record_command_echo(&command.spec.name, args);
        let cx = runner.call_context().with_kind(CallKind::Command);
        let result = command
            .extension
            .instance
            .run_command(&command.spec.name, args, CancellationToken::new(), cx)
            .await;
        if let Err(error) = result {
            runner.emit_error(&format!("command:{name}"), "command", error.message);
        }
        true
    }

    /// Oino command echo: every extension command run is recorded as a custom entry.
    fn record_command_echo(&self, name: &str, args: &str) {
        let appended = lock(&self.inner.manager).append_custom_entry(
            "oino.command-echo",
            Some(json!({"name": name, "args": args})),
        );
        if let Ok(id) = appended {
            self.publish_entry(&id);
        }
    }

    /// pi `sendCustomMessage` (four cases, see runtime §2).
    ///
    /// # Errors
    ///
    /// Run start failures.
    pub fn send_custom_message(
        &self,
        draft: CustomMessageDraft,
        trigger_turn: bool,
        deliver_as: Option<Delivery>,
    ) -> BoxFuture<'_, Result<(), SessionOpError>> {
        Box::pin(self.send_custom_message_inner(draft, trigger_turn, deliver_as))
    }

    async fn send_custom_message_inner(
        &self,
        draft: CustomMessageDraft,
        trigger_turn: bool,
        deliver_as: Option<Delivery>,
    ) -> Result<(), SessionOpError> {
        let message = CustomMessage {
            custom_type: draft.custom_type,
            content: draft.content,
            display: draft.display,
            details: draft.details,
            timestamp: oino_agent::now_ms(),
        };
        let streaming = self.is_streaming();
        if deliver_as == Some(Delivery::NextTurn) {
            lock(&self.inner.book).pending_next_turn.push(message);
        } else if streaming && trigger_turn {
            if deliver_as == Some(Delivery::FollowUp) {
                self.inner.agent.follow_up(AgentMessage::Custom(message));
            } else {
                self.inner.agent.steer(AgentMessage::Custom(message));
            }
        } else if trigger_turn {
            if lock(&self.inner.book).emitting_settled {
                let session = self.clone();
                let draft = CustomMessageDraft {
                    custom_type: message.custom_type,
                    content: message.content,
                    display: message.display,
                    details: message.details,
                };
                self.defer(Box::pin(async move {
                    if let Err(error) = session
                        .send_custom_message(draft, trigger_turn, deliver_as)
                        .await
                    {
                        session
                            .runner()
                            .emit_error("<runtime>", "send_message", error.to_string());
                    }
                }));
                return Ok(());
            }
            self.start_run(Trigger::Custom(message)).await?;
        } else if streaming {
            lock(&self.inner.book).pending_custom.push(message);
        } else {
            self.append_custom_message(message);
        }
        Ok(())
    }

    /// pi `sendUserMessage`: always triggers a turn (queued while streaming).
    ///
    /// # Errors
    ///
    /// As [`AgentSession::prompt`].
    pub async fn send_user_message(
        &self,
        content: UserContentField,
        deliver_as: Option<Delivery>,
        expand_prompt_templates: bool,
    ) -> Result<PromptDisposition, SessionOpError> {
        let (text, images) = match content {
            UserContentField::Text(text) => (text, Vec::new()),
            UserContentField::Blocks(blocks) => {
                let mut texts = Vec::new();
                let mut images = Vec::new();
                for block in blocks {
                    match block {
                        UserContent::Text(text) => texts.push(text.text),
                        UserContent::Image(image) => images.push(image),
                    }
                }
                (texts.join("\n"), images)
            }
        };
        let streaming_behavior = match deliver_as {
            Some(Delivery::Steer) => Some(StreamingBehavior::Steer),
            Some(Delivery::FollowUp) => Some(StreamingBehavior::FollowUp),
            _ => None,
        };
        self.prompt(
            text,
            PromptOptions {
                expand_prompt_templates,
                images,
                streaming_behavior,
                source: InputSource::Extension,
            },
        )
        .await
    }

    pub(crate) fn append_custom_message(&self, message: CustomMessage) {
        let appended = lock(&self.inner.manager).append_custom_message_entry(
            &message.custom_type,
            message.content.clone(),
            message.display,
            message.details.clone(),
        );
        if let Err(error) = appended {
            tracing::error!("persisting custom message failed: {error}");
        }
        self.refresh_context();
        let message = AgentMessage::Custom(message);
        self.emit(AgentSessionEvent::MessageStart {
            message: message.clone(),
        });
        self.emit(AgentSessionEvent::MessageEnd { message });
    }

    pub(crate) fn flush_pending_custom_messages(&self) {
        let pending = std::mem::take(&mut lock(&self.inner.book).pending_custom);
        for message in pending {
            self.append_custom_message(message);
        }
    }

    fn defer(&self, action: Deferred) {
        let session = self.clone();
        tokio::spawn(async move {
            // Runs once the current agent_settled emission finished (pi deferred actions).
            while lock(&session.inner.book).emitting_settled {
                tokio::task::yield_now().await;
            }
            action.await;
        });
    }

    /// pi `_startRun`: prepares the run inline, then drives it in the background.
    pub(crate) async fn start_run(&self, trigger: Trigger) -> Result<(), SessionOpError> {
        {
            let mut book = lock(&self.inner.book);
            book.run_active = true;
            book.abort_requested = false;
        }
        self.clear_waiting(crate::events::RunState::Running);
        self.mark_busy();
        let messages = match self.prepare_run(trigger).await {
            Ok(messages) => messages,
            Err(error) => {
                lock(&self.inner.book).run_active = false;
                self.flush_pending_bash_messages();
                self.flush_pending_custom_messages();
                self.set_idle_if_idle();
                return Err(error);
            }
        };
        self.flush_pending_bash_messages();
        self.flush_pending_custom_messages();
        lock(&self.inner.book).failed_response = None;
        self.record_selection();
        self.clear_pending_tools();
        let session = self.clone();
        tokio::spawn(async move { session.drive_run(messages).await });
        Ok(())
    }

    async fn drive_run(self, messages: Vec<AgentMessage>) {
        if let Err(error) = self.inner.agent.prompt(messages).await {
            tracing::error!("agent prompt failed: {error}");
        }
        loop {
            if self.abort_requested() {
                break;
            }
            if self.handle_post_agent_run().await {
                if self.abort_requested() {
                    break;
                }
                if self.inner.agent.continue_run().await.is_err() {
                    break;
                }
                continue;
            }
            if self.abort_requested() || !self.run_before_settle_boundary().await {
                break;
            }
            if self.abort_requested() || self.inner.agent.continue_run().await.is_err() {
                break;
            }
        }
        if self.abort_requested() {
            self.finish_cancelled_retry();
        }
        {
            let mut book = lock(&self.inner.book);
            book.failed_response = None;
            book.run_prompt_options = None;
        }
        self.flush_pending_bash_messages();
        self.flush_pending_custom_messages();
        self.emit_agent_settled().await;
    }

    pub(crate) fn abort_requested(&self) -> bool {
        lock(&self.inner.book).abort_requested
    }

    /// pi `_prepareRun`.
    async fn prepare_run(&self, trigger: Trigger) -> Result<Vec<AgentMessage>, SessionOpError> {
        if let Some(last) = self.last_assistant_message() {
            self.check_compaction(&last, false).await;
        }
        let mut base = self.system_prompt_options();
        self.apply_skill_opt_in(&mut base.skills);
        let runner = self.runner();
        let mut trigger_json = serde_json::Map::new();
        match &trigger {
            Trigger::User { prompt, images } => {
                trigger_json.insert("messageType".into(), json!("user"));
                trigger_json.insert("prompt".into(), json!(prompt));
                if !images.is_empty() {
                    trigger_json.insert("images".into(), images_json(images));
                }
            }
            Trigger::Custom(message) => {
                trigger_json.insert("messageType".into(), json!("custom"));
                trigger_json.insert(
                    "message".into(),
                    json!(AgentMessage::Custom(message.clone())),
                );
            }
        }
        let mut options = base.clone();
        let mut injected = Vec::new();
        if runner.has_handlers("before_agent_start") {
            let docs_root = self.inner.docs_root.clone();
            let render = move |options: &JsonValue| -> String {
                serde_json::from_value::<oino_resources::BuildSystemPromptOptions>(options.clone())
                    .ok()
                    .and_then(|o| {
                        oino_resources::system_prompt::build_system_prompt(&o, &docs_root).ok()
                    })
                    .unwrap_or_default()
            };
            let outcome = runner
                .emit_before_agent_start(trigger_json, json!(base), &render)
                .await;
            if let Ok(next) = serde_json::from_value(outcome.system_prompt_options) {
                options = next;
            }
            if outcome.system_prompt.is_some() {
                options.force_system_prompt = outcome.system_prompt;
            }
            injected = outcome.messages;
        }
        if options.selected_tools == base.selected_tools {
            options.selected_tools = self.active_tool_names();
        }
        let mut messages = Vec::new();
        match trigger {
            Trigger::Custom(message) => messages.push(AgentMessage::Custom(message)),
            Trigger::User { prompt, images } => messages.push(user_message(prompt, images)),
        }
        let next_turn = std::mem::take(&mut lock(&self.inner.book).pending_next_turn);
        messages.extend(next_turn.into_iter().map(AgentMessage::Custom));
        for draft in injected {
            let content = draft
                .get("content")
                .filter(|c| !c.is_null())
                .and_then(|c| serde_json::from_value(c.clone()).ok())
                .unwrap_or(UserContentField::Blocks(Vec::new()));
            messages.push(AgentMessage::Custom(CustomMessage {
                custom_type: draft
                    .get("customType")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                content,
                display: draft
                    .get("display")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                details: draft.get("details").cloned(),
                timestamp: oino_agent::now_ms(),
            }));
        }
        let current = self.messages();
        let update = self.prepare_prompt_and_tool_loadout(&mut options, &current);
        lock(&self.inner.book).run_prompt_options = Some(options);
        if let Some(update) = update {
            messages.insert(0, AgentMessage::System(update));
        }
        Ok(messages)
    }

    /// The last assistant message of the transcript.
    #[must_use]
    pub fn last_assistant_message(&self) -> Option<oino_ai::AssistantMessage> {
        self.messages().into_iter().rev().find_map(|m| match m {
            AgentMessage::Assistant(a) => Some(a),
            _ => None,
        })
    }

    /// pi `_handlePostAgentRun`: retry, overflow recovery, threshold compaction, queued work.
    async fn handle_post_agent_run(&self) -> bool {
        let message = lock(&self.inner.book).last_assistant.take();
        if self.abort_requested() {
            self.finish_cancelled_retry();
            return false;
        }
        let Some(message) = message else {
            return self.inner.agent.has_queued_messages();
        };
        if self.is_retryable_error(&message) && self.prepare_retry(&message).await {
            if self.abort_requested() {
                self.finish_cancelled_retry();
            }
            lock(&self.inner.book).failed_response = Some(message);
            return !self.abort_requested();
        }
        if self.abort_requested() {
            self.finish_cancelled_retry();
            return false;
        }
        let final_failure = {
            let mut book = lock(&self.inner.book);
            if message.stop_reason == oino_ai::StopReason::Error && book.retry_attempt > 0 {
                let attempt = book.retry_attempt;
                book.retry_attempt = 0;
                Some(attempt)
            } else {
                None
            }
        };
        if let Some(attempt) = final_failure {
            self.emit(AgentSessionEvent::AutoRetryEnd {
                success: false,
                attempt,
                final_error: message.error_message.clone(),
            });
        }
        if self.check_compaction(&message, true).await {
            return !self.abort_requested();
        }
        !self.abort_requested() && self.inner.agent.has_queued_messages()
    }

    /// pi `_emitAgentSettled`.
    async fn emit_agent_settled(&self) {
        let aborted = {
            let mut book = lock(&self.inner.book);
            book.run_active = false;
            book.emitting_settled = true;
            book.abort_requested
        };
        let runner = self.runner();
        if runner.has_handlers("agent_settled") {
            runner
                .emit(json!({"type": "agent_settled", "aborted": aborted}))
                .await;
        }
        self.emit(AgentSessionEvent::AgentSettled { aborted });
        lock(&self.inner.book).emitting_settled = false;
        self.set_idle_if_idle();
        self.refresh_git_topic_if_followed();
    }
}

/// Command name of `/name args`.
pub(crate) fn command_name(text: &str) -> &str {
    let without_slash = text.strip_prefix('/').unwrap_or(text);
    without_slash
        .split_once(' ')
        .map_or(without_slash, |(name, _)| name)
}
