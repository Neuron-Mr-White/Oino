//! Prompt pipeline and run lifecycle (`prompt`, `steer`, `followUp`, `sendCustomMessage`,
//! `sendUserMessage`, `_startRun`, `_prepareRun`, `_handlePostAgentRun`, `_emitAgentSettled`).

use futures::future::BoxFuture;
use oino_ai::{
    AgentMessage, CustomMessage, ImageContent, JsonValue, UserContent, UserContentField,
    UserMessage,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, IsolatedTask, SessionOpError, lock};
use crate::events::AgentSessionEvent;
use crate::extensions::api::CallKind;
use crate::extensions::host::{CustomMessageDraft, Delivery};
use crate::extensions::runner::InputOutcome;

/// How an accepted prompt was dispatched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptDisposition {
    /// An extension command or input handler consumed it.
    Handled,
    /// An extension refused the command without consuming the input.
    Refused(String),
    /// Queued (steering / follow-up).
    Queued,
    /// A run started.
    Started,
}

/// Queueing while streaming.
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

/// The input source record.
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

/// What starts a run.
#[derive(Clone, Debug)]
pub(crate) enum Trigger {
    User {
        prompt: String,
        images: Vec<ImageContent>,
        author: Option<String>,
    },
    Custom(CustomMessage),
}

fn user_message(text: String, images: Vec<ImageContent>, author: Option<String>) -> AgentMessage {
    let mut content = vec![UserContent::text(text)];
    content.extend(images.into_iter().map(UserContent::Image));
    AgentMessage::User(UserMessage {
        author,
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
    /// Sends a prompt: extension commands run immediately, `input` handlers
    /// may transform or handle it, skills and templates expand, and while streaming it is
    /// queued per `streaming_behavior`. Returns once the run started (await
    /// [`AgentSession::wait_for_idle`] for completion).
    ///
    /// # Errors
    ///
    /// Fixed errors: streaming without behaviour, no model, no API key, compaction running.
    pub fn prompt(
        &self,
        text: String,
        options: PromptOptions,
    ) -> BoxFuture<'_, Result<PromptDisposition, SessionOpError>> {
        // An extension's message is a new input, not the host command that caused it.
        // Clear admission identity before any deferral captures the task-local value.
        if options.source == InputSource::Extension {
            Box::pin(
                super::delivery::MESSAGE_ID.scope(
                    super::delivery::EXTENSION_MESSAGE_ID
                        .try_with(Clone::clone)
                        .unwrap_or_default(),
                    self.prompt_inner(text, options),
                ),
            )
        } else {
            Box::pin(self.prompt_inner(text, options))
        }
    }

    async fn prompt_inner(
        &self,
        text: String,
        options: PromptOptions,
    ) -> Result<PromptDisposition, SessionOpError> {
        if lock(&self.inner.book).emitting_settled {
            let session = self.clone();
            let message_id = super::delivery::MESSAGE_ID
                .try_with(Clone::clone)
                .unwrap_or_default();
            self.defer(Box::pin(super::delivery::MESSAGE_ID.scope(
                message_id,
                async move {
                    session.run_deferred_prompt(text, options).await;
                },
            )));
            return Ok(PromptDisposition::Queued);
        }
        if options.expand_prompt_templates && text.starts_with('/') {
            lock(&self.inner.book)
                .command_images
                .clone_from(&options.images);
            let handled = self.try_execute_extension_command(&text).await;
            lock(&self.inner.book).command_images.clear();
            if let Some(disposition) = handled? {
                return Ok(disposition);
            }
        }
        if lock(&self.inner.cancels).compaction.is_some() {
            self.with_manager(oino_session::SessionManager::begin_run)
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
            // Never drop the owner's message: hold it until the compaction ends (or is
            // stopped), then send it as a normal prompt.
            let session = self.clone();
            let message_id = super::delivery::MESSAGE_ID
                .try_with(Clone::clone)
                .unwrap_or_default();
            self.spawn_isolated(
                IsolatedTask::Drive,
                super::delivery::MESSAGE_ID.scope(message_id, async move {
                    while lock(&session.inner.cancels).compaction.is_some() {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    session.run_deferred_prompt(text, options).await;
                }),
            );
            return Ok(PromptDisposition::Queued);
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
            self.queue_input(behavior, expanded, images, options.source);
            return Ok(PromptDisposition::Queued);
        }
        self.validate_model_and_auth()?;
        self.start_run(Trigger::User {
            prompt: expanded,
            images,
            author: (options.source == InputSource::Extension).then(|| "extension".into()),
        })
        .await?;
        Ok(PromptDisposition::Started)
    }

    async fn run_deferred_prompt(&self, text: String, options: PromptOptions) {
        match super::delivery::EXTENSION_MESSAGE_ID
            .scope(
                super::delivery::MESSAGE_ID
                    .try_with(Clone::clone)
                    .unwrap_or_default(),
                self.prompt(text, options),
            )
            .await
        {
            Ok(PromptDisposition::Handled) => {
                if let Ok(id) = super::delivery::MESSAGE_ID.try_with(Clone::clone)
                    && let Err(error) = self.finish_delivery(&id)
                {
                    self.runner()
                        .emit_error("<runtime>", "prompt", error.to_string());
                }
            }
            Ok(_) => {}
            Err(error) => self
                .runner()
                .emit_error("<runtime>", "prompt", error.to_string()),
        }
    }

    /// Queues a steering message.
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

    /// Queues a follow-up message.
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
        self.queue_input(behavior, expanded, images, source);
        Ok(PromptDisposition::Queued)
    }

    fn queue_input(
        &self,
        behavior: StreamingBehavior,
        text: String,
        images: Vec<ImageContent>,
        source: InputSource,
    ) {
        {
            let mut book = lock(&self.inner.book);
            match behavior {
                StreamingBehavior::Steer => book.steering.push(text.clone()),
                StreamingBehavior::FollowUp => book.follow_up.push(text.clone()),
            }
        }
        self.emit_queue_update();
        let mut message = user_message(
            text,
            images,
            (source == InputSource::Extension).then(|| "extension".into()),
        );
        if source != InputSource::Extension
            || super::delivery::EXTENSION_MESSAGE_ID
                .try_with(|id| !id.is_empty())
                .unwrap_or(false)
        {
            self.identify_delivery(&mut message);
        }
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

    /// E x p a n d   S k i l l   C o m m a n d.
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

    /// Runs an extension command if `text` names one.
    async fn try_execute_extension_command(
        &self,
        text: &str,
    ) -> Result<Option<PromptDisposition>, SessionOpError> {
        let name = command_name(text);
        let args = text.split_once(' ').map_or("", |(_, rest)| rest);
        let runner = self.runner();
        let Some(command) = runner.command(name) else {
            return Ok(None);
        };
        self.record_command_echo(&command.spec.name, args);
        let cx = runner.call_context().with_kind(CallKind::Command);
        let parent = super::delivery::MESSAGE_ID
            .try_with(Clone::clone)
            .unwrap_or_default();
        let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result = super::delivery::COMMAND_HANDOFF
            .scope(
                (parent, std::sync::Arc::default(), failed.clone()),
                command.extension.instance.run_command(
                    &command.spec.name,
                    args,
                    CancellationToken::new(),
                    cx,
                ),
            )
            .await;
        if let Ok(Some(value)) = &result
            && value.get("disposition").and_then(serde_json::Value::as_str) == Some("refused")
        {
            return Ok(Some(PromptDisposition::Refused(
                value
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Command refused")
                    .to_owned(),
            )));
        }
        if let Err(error) = result {
            // A command echo alone is not an outcome. Keep failures (including
            // denied process approvals) in the durable conversation, so a queued
            // command cannot be acknowledged and vanish without an explanation.
            self.append_custom_message(CustomMessage {
                custom_type: "oino.command-error".into(),
                content: UserContentField::Text(format!(
                    "/{name} did not complete: {}",
                    error.message
                )),
                display: true,
                details: None,
                timestamp: oino_agent::now_ms(),
            });
            runner.emit_error(&format!("command:{name}"), "command", error.message);
        }
        if failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(SessionOpError::Failed(
                "extension handoff was not durably admitted".into(),
            ));
        }
        Ok(Some(PromptDisposition::Handled))
    }

    /// Oino command echo: every extension command run is recorded as a custom entry.
    fn record_command_echo(&self, name: &str, args: &str) {
        // G3-2 (beqa): the originating host message id (when the command was admitted by a host)
        // lets a frontend pair its pending bubble with this echo by identity, not by text and time.
        let mut data = json!({"name": name, "args": args});
        if let Ok(id) = super::delivery::MESSAGE_ID.try_with(Clone::clone)
            && !id.is_empty()
        {
            data["messageId"] = json!(id);
        }
        let appended =
            lock(&self.inner.manager).append_custom_entry("oino.command-echo", Some(data));
        if let Ok(id) = appended {
            self.publish_entry(&id);
        }
    }

    /// `sendCustomMessage` (four cases, see runtime §2).
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
        if matches!(draft.custom_type.as_str(), REMINDER_TYPE | START_TYPE) {
            return Err(SessionOpError::Failed(
                "reserved host reminder type; use append_reminder".into(),
            ));
        }
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
            let follow = deliver_as == Some(Delivery::FollowUp);
            let queued = AgentMessage::Custom(message.clone());
            if follow {
                self.inner.agent.follow_up(queued.clone());
            } else {
                self.inner.agent.steer(queued.clone());
            }
            // Lost-wake guard: a trigger queued while the run is ending can
            // land after the run's last queue drain — the run then settles
            // and nothing ever consumes it (a fast background subagent
            // finishing exactly as the parent's turn ends loses its
            // completion notification). If the run settled underneath the
            // push, take the message back and give it a run of its own; when
            // a newer run already drained it, the take-back finds nothing
            // and the trigger is not duplicated.
            if !self.is_streaming() {
                let rescued = self
                    .inner
                    .agent
                    .update_queue(follow, |queue| {
                        queue
                            .iter()
                            .rposition(|m| *m == queued)
                            .map(|at| queue.remove(at))
                    })
                    .is_some();
                if rescued {
                    return self.start_run(Trigger::Custom(message)).await;
                }
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

    /// Always triggers a turn (queued while streaming).
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
        if pending.is_empty() {
            return;
        }
        for message in pending {
            self.append_custom_message(message);
        }
        // D-FLUSH barrier 3: flushed custom messages are durable before the run continues.
        self.boundary_sync("flush pending custom messages");
    }

    fn defer(&self, action: Deferred) {
        let session = self.clone();
        self.spawn_isolated(IsolatedTask::Drive, async move {
            // Runs once the current agent_settled emission finished (deferred actions).
            while lock(&session.inner.book).emitting_settled {
                tokio::task::yield_now().await;
            }
            action.await;
        });
    }

    /// Prepares the run inline, then drives it in the background.
    pub(crate) async fn start_run(&self, trigger: Trigger) -> Result<(), SessionOpError> {
        {
            let mut book = lock(&self.inner.book);
            book.run_active = true;
            book.abort_requested = false;
        }
        let started = std::time::Instant::now();
        if let Err(error) = self.with_manager(oino_session::SessionManager::begin_run) {
            lock(&self.inner.book).run_active = false;
            return Err(SessionOpError::Failed(error.to_string()));
        }
        tracing::debug!(
            phase = "begin_run",
            elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            "prompt ACK phase"
        );
        self.clear_waiting(crate::events::RunState::Running);
        self.mark_busy();
        let preparing = std::time::Instant::now();
        let messages = match self.prepare_run(trigger).await {
            Ok(messages) => messages,
            Err(error) => {
                let _ = self.with_manager(oino_session::SessionManager::end_run);
                lock(&self.inner.book).run_active = false;
                self.flush_pending_bash_messages();
                self.flush_pending_custom_messages();
                self.set_idle_if_idle();
                return Err(error);
            }
        };
        tracing::debug!(
            phase = "prepare_run",
            elapsed_us = u64::try_from(preparing.elapsed().as_micros()).unwrap_or(u64::MAX),
            "prompt ACK phase"
        );
        self.flush_pending_bash_messages();
        self.flush_pending_custom_messages();
        lock(&self.inner.book).failed_response = None;
        self.record_selection();
        self.clear_pending_tools();
        let session = self.clone();
        self.spawn_isolated(IsolatedTask::Drive, async move {
            session.drive_run(messages).await;
        });
        Ok(())
    }

    async fn drive_run(self, messages: Vec<AgentMessage>) {
        if let Err(error) = self.inner.agent.prompt(messages).await {
            tracing::error!("agent prompt failed: {error}");
        }
        // The settle tail runs once the decision loop breaks — unless a
        // queued trigger arrived while it was deciding (lost-wake guard,
        // below), in which case the run drives another round that drains it.
        // The tail guard is bounded: a queue that somehow never drains must
        // not keep the run from settling forever.
        let mut tail_redrives = 0;
        loop {
            // Every exit from this decision loop is a settle decision.
            let settle = loop {
                if self.abort_requested() {
                    break true;
                }
                if self.handle_post_agent_run().await {
                    if self.abort_requested() {
                        break true;
                    }
                    if self.inner.agent.continue_run().await.is_err() {
                        break true;
                    }
                    continue;
                }
                let boundary_ok =
                    !self.abort_requested() && self.run_before_settle_boundary().await;
                if !boundary_ok
                    && (self.abort_requested() || !self.inner.agent.has_queued_messages())
                {
                    // Plain settle: aborted, or nothing pending. (When the
                    // boundary said "nothing pending" but the lost-wake
                    // guard below finds a queued trigger — one that landed
                    // in the awaits since that read — we fall through to
                    // `continue_run` and drive another round instead.)
                    break true;
                }
                if self.abort_requested() || self.inner.agent.continue_run().await.is_err() {
                    break true;
                }
            };
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
            // Lost-wake guard (settle tail): the flushes above await, so a
            // trigger may have queued since the decision. Re-check with
            // nothing between it and the flip in `emit_agent_settled` — a
            // queued trigger drives another round instead of being buried.
            if settle
                && !self.abort_requested()
                && self.inner.agent.has_queued_messages()
                && tail_redrives < 3
                && self.inner.agent.continue_run().await.is_ok()
            {
                tail_redrives += 1;
                continue;
            }
            self.emit_agent_settled().await;
            return;
        }
    }

    pub(crate) fn abort_requested(&self) -> bool {
        lock(&self.inner.book).abort_requested
    }

    /// P r e p a r e   R u n.
    async fn prepare_run(&self, trigger: Trigger) -> Result<Vec<AgentMessage>, SessionOpError> {
        let started = std::time::Instant::now();
        if let Some(last) = self.last_assistant_message() {
            self.check_compaction(&last, false).await;
        }
        let mut base = self.system_prompt_options();
        self.apply_skill_opt_in(&mut base.skills);
        let runner = self.runner();
        let mut trigger_json = serde_json::Map::new();
        match &trigger {
            Trigger::User { prompt, images, .. } => {
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
        // Compatibility hooks still run, but cannot mutate the frozen system slots.
        let mut options = base.clone();
        let mut injected = Vec::new();
        if runner.has_handlers("before_agent_start") {
            let frozen = self.system_prompt();
            let render = move |_: &JsonValue| frozen.clone();
            let outcome = runner
                .emit_before_agent_start(trigger_json, json!(base), &render)
                .await;
            injected = outcome.messages;
        }
        tracing::debug!(
            phase = "before_agent_start",
            elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
            "prompt ACK phase"
        );
        options.selected_tools = self.active_tool_names();
        self.ensure_prompt_prefix()?;
        self.announce_tool_catalog();
        let mut messages = Vec::new();
        match trigger {
            Trigger::Custom(message) => messages.push(AgentMessage::Custom(message)),
            Trigger::User {
                prompt,
                images,
                author,
            } => messages.push(user_message(prompt, images, author)),
        }
        for message in &mut messages {
            self.identify_delivery(message);
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
        let update = self.prepare_prompt_and_tool_loadout(&mut options);
        lock(&self.inner.book).run_prompt_options = Some(options);
        if let Some(update) = update {
            messages.insert(0, AgentMessage::System(update));
        }
        Ok(messages)
    }

    /// The last assistant message of the transcript.
    #[must_use]
    pub fn last_assistant_message(&self) -> Option<oino_ai::AssistantMessage> {
        self.inner.agent.with_state(|state| {
            state
                .messages
                .iter()
                .rev()
                .find_map(|message| match message {
                    AgentMessage::Assistant(assistant) => Some(assistant.clone()),
                    _ => None,
                })
        })
    }

    /// Retry, overflow recovery, threshold compaction, queued work.
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

    /// E m i t   A g e n t   S e t t l e d.
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
        if let Err(error) = self.with_manager(oino_session::SessionManager::end_run) {
            tracing::error!("persisting run-ended failed: {error}");
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

/// Result of a durable reminder append.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderAppend {
    /// Persisted session entry id.
    pub id: String,
    /// True if a new entry was appended; false if the latest visible reminder matches.
    pub appended: bool,
}

/// Only host-created custom messages with this type carry trusted envelopes.
pub(crate) const REMINDER_TYPE: &str = "oino.system-reminder";
pub(crate) const START_TYPE: &str = "oino.session-start";
/// The owner's notes: shown in the timeline, never sent to the model.
pub(crate) const NOTE_TYPE: &str = "oino.note";

impl AgentSession {
    /// Append plain text as trusted harness context. `key` must start with
    /// `extension_id/` and have a nonempty topic. Superseded undelivered
    /// reminders with the same key are removed from the context projection.
    /// The manager lock serializes appends. Persistence errors reach the caller.
    ///
    /// # Errors
    /// Invalid ownership, or a session persistence failure.
    pub fn append_reminder(
        &self,
        extension_id: &str,
        key: &str,
        text: &str,
    ) -> Result<ReminderAppend, SessionOpError> {
        if extension_id.is_empty()
            || key
                .strip_prefix(extension_id)
                .and_then(|s| s.strip_prefix('/'))
                .is_none_or(str::is_empty)
        {
            return Err(SessionOpError::Failed(
                "reminder key must belong to the originating extension".into(),
            ));
        }
        let mut manager = lock(&self.inner.manager);
        let content = UserContentField::Text(reminder(text));
        let (latest, superseded) = manager.lookup_custom_key(REMINDER_TYPE, key);
        if let Some((id, previous)) = latest
            && previous == content
        {
            return Ok(ReminderAppend {
                id: id.to_string(),
                appended: false,
            });
        }
        for target in superseded {
            manager
                .append_context_edit(&target, None)
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        }
        let id = manager
            .append_custom_message_entry(REMINDER_TYPE, content, false, Some(json!({"key":key})))
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        manager
            .sync()
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        Ok(ReminderAppend {
            id: id.to_string(),
            appended: true,
        })
    }

    /// Save an owner's note: a `custom` entry of type `oino.note` with `{text, images?}`.
    /// A note is shown in the timeline and never reaches the model (custom entries are
    /// not context, and compaction skips them). It is written at once, also mid-run, and
    /// published so live clients see it. Returns the new entry id.
    ///
    /// # Errors
    /// An empty note, or a session persistence failure.
    pub fn append_note(
        &self,
        text: &str,
        images: &[ImageContent],
    ) -> Result<String, SessionOpError> {
        if text.trim().is_empty() && images.is_empty() {
            return Err(SessionOpError::Failed(
                "a note needs text or an image".into(),
            ));
        }
        let mut data = json!({ "text": text });
        if !images.is_empty() {
            data["images"] = images
                .iter()
                .map(|i| json!({ "type": "image", "data": i.data, "mimeType": i.mime_type }))
                .collect();
        }
        let id = lock(&self.inner.manager)
            .append_custom_entry(NOTE_TYPE, Some(data))
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        self.publish_entry(&id);
        Ok(id.to_string())
    }

    pub(crate) fn reminder_messages(&self) -> Vec<AgentMessage> {
        lock(&self.inner.manager)
            .build_session_projection()
            .messages
            .into_iter()
            .filter(|message| {
                matches!(message, AgentMessage::Custom(custom)
                if custom.custom_type == REMINDER_TYPE
                    && !custom.details.as_ref().and_then(|d| d["key"].as_str()).is_some_and(|key| {
                        matches!(key, "oino/todo-missing" | "oino/todo-stale" | "oino/todo-settle")
                    }))
            })
            .collect()
    }

    pub(crate) fn frozen_prompt(&self) -> Option<String> {
        lock(&self.inner.manager)
            .wake_branch()
            .iter()
            .find_map(|e| match &e.kind {
                oino_session::EntryKind::SystemSnapshot { text, version: 3 } => Some(text.clone()),
                _ => None,
            })
    }

    /// Capture once, before any user turn. Legacy history receives start context
    /// at its tail rather than an inserted synthetic message.
    pub(crate) fn ensure_prompt_prefix(&self) -> Result<(), SessionOpError> {
        if self.frozen_prompt().is_some()
            && lock(&self.inner.manager).wake_branch().iter().any(|entry|
                matches!(&entry.kind, oino_session::EntryKind::CustomMessage { custom_type, .. } if custom_type == START_TYPE)) {
            self.restore_announced_tool_catalog();
            return Ok(());
        }
        let text = self.system_prompt();
        let options = self.system_prompt_options();
        let workspace_free = self.cwd().as_os_str().is_empty() || self.with_manager(|m| m.wake_branch().iter().any(|e|
            matches!(&e.kind, oino_session::EntryKind::Custom { custom_type, .. } if custom_type == "oino:superagent")));
        let cwd = self.work_cwd();
        let mut blocks = vec![reminder(&self.tool_catalog())];
        if !workspace_free {
            blocks.push(reminder(&format!(
                "Available subagents:\n{}",
                self.agents_list()
            )));
        }
        if workspace_free {
            blocks.push(reminder("Superagent session: no workspace or cwd. Resolve an explicit project before board changes; ask the user when ambiguous. No filesystem or shell tools."));
        } else {
            blocks.push(reminder(&format!(
                "User system info ({}/{})\nSession working directory: {}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                cwd.display()
            )));
        }
        let date = oino_session::IsoTimestamp::now().to_string();
        blocks.push(reminder(&format!(
            "Current date: {}",
            date.get(..10).unwrap_or(&date)
        )));
        if !workspace_free {
            let mut listing = std::fs::read_dir(&cwd)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            listing.sort();
            let git = std::process::Command::new("git")
                .args(["status", "--short", "--branch"])
                .current_dir(&cwd)
                .output()
                .map_or_else(
                    |e| e.to_string(),
                    |o| {
                        format!(
                            "{}{}",
                            String::from_utf8_lossy(&o.stdout),
                            String::from_utf8_lossy(&o.stderr)
                        )
                    },
                );
            blocks.push(reminder(&format!("Workspace snapshot captured once at session start:\n% ls\n{}\n% git status --short --branch\n{git}", listing.join("\n"))));
            if !options.context_files.is_empty() {
                blocks.push(reminder(
                    &oino_resources::system_prompt::render_project_context(&options.context_files),
                ));
            }
        }
        let mut manager = lock(&self.inner.manager);
        // Complete a partially persisted prefix without ever replacing its bytes.
        if manager.wake_branch().iter().any(|entry|
            matches!(&entry.kind, oino_session::EntryKind::CustomMessage {custom_type, ..} if custom_type == START_TYPE)) {
            drop(manager);
            self.restore_announced_tool_catalog();
            return Ok(());
        }
        if !manager.wake_branch().iter().any(|entry| {
            matches!(
                entry.kind,
                oino_session::EntryKind::SystemSnapshot { version: 3, .. }
            )
        }) {
            manager
                .append_entry(oino_session::EntryKind::SystemSnapshot { text, version: 3 })
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        }
        manager
            .append_custom_message_entry(
                START_TYPE,
                UserContentField::Text(blocks.join("\n\n")),
                false,
                None,
            )
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        manager
            .sync()
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        drop(manager);
        self.mark_tool_catalog_announced();
        self.refresh_context();
        Ok(())
    }

    fn scope_start_catalog(&self, content: &mut UserContentField) {
        let disabled = self.disabled_tool_names();
        let scope_catalog = |text: &mut String| {
            if text.contains("Unified tool catalog:\n") {
                *text = text
                    .lines()
                    .filter(|line| !disabled.iter().any(|n| n == line.trim()))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        };
        match content {
            UserContentField::Text(text) => scope_catalog(text),
            UserContentField::Blocks(blocks) => {
                for block in blocks {
                    if let UserContent::Text(block) = block {
                        scope_catalog(&mut block.text);
                    }
                }
            }
        }
    }

    /// Restore the immutable slots around the model's tail view without changing
    /// stored source messages. Legacy system patches remain in the UI only.
    pub(crate) fn prefix_view(&self, messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        let manager = lock(&self.inner.manager);
        let branch = manager.wake_branch();
        let snapshot = branch.iter().find_map(|e| match &e.kind {
            oino_session::EntryKind::SystemSnapshot { text, version: 3 } => {
                Some((text.clone(), e.timestamp.to_ms().unwrap_or(0)))
            }
            _ => None,
        });
        let Some((text, timestamp)) = snapshot else {
            return messages;
        };
        let start = branch.iter().find_map(|e| match &e.kind {
            oino_session::EntryKind::CustomMessage {
                custom_type,
                content,
                ..
            } if custom_type == START_TYPE => Some((e, content.clone())),
            _ => None,
        });
        // A legacy conversation already had real history before its snapshot.
        let legacy = branch
            .iter()
            .take_while(|e| {
                !matches!(
                    e.kind,
                    oino_session::EntryKind::SystemSnapshot { version: 3, .. }
                )
            })
            .any(|e| {
                matches!(
                    e.kind,
                    oino_session::EntryKind::Message { .. }
                        | oino_session::EntryKind::Compaction(_)
                )
            });
        let trusted = branch
            .iter()
            .filter_map(|e| match &e.kind {
                oino_session::EntryKind::CustomMessage {
                    custom_type,
                    content,
                    ..
                } if custom_type == REMINDER_TYPE || custom_type == START_TYPE => {
                    Some((custom_type, content))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut tail: Vec<_> = messages
            .into_iter()
            .filter(|m| !matches!(m, AgentMessage::System(_)))
            .map(|mut m| {
                if let AgentMessage::Custom(c) = &mut m
                    && matches!(c.custom_type.as_str(), REMINDER_TYPE | START_TYPE)
                    && !trusted
                        .iter()
                        .any(|(kind, content)| **kind == c.custom_type && **content == c.content)
                {
                    c.custom_type = "oino.untrusted-context".into();
                }
                m
            })
            .collect();
        let mut out = vec![AgentMessage::System(oino_ai::SystemMessage {
            content: oino_ai::UserTextField::Text(text),
            sections: None,
            tools_added: Some(
                self.inner
                    .agent
                    .with_state(|s| s.tools.iter().map(|t| t.meta().declaration()).collect()),
            ),
            tools_removed: None,
            timestamp,
        })];
        if !legacy && let Some((entry, mut content)) = start {
            // Runtime-scoped tools must not be advertised by the stored start catalog.
            // Keep the durable transcript untouched; only its provider projection is scoped.
            self.scope_start_catalog(&mut content);
            tail.retain(|m| !matches!(m, AgentMessage::Custom(c) if c.custom_type == START_TYPE));
            out.push(AgentMessage::Custom(CustomMessage {
                custom_type: START_TYPE.into(),
                content,
                display: false,
                details: None,
                timestamp: entry.timestamp.to_ms().unwrap_or(0),
            }));
        }
        out.extend(tail);
        out
    }
}

fn reminder(text: &str) -> String {
    format!(
        "<system-reminder>\n{}\n</system-reminder>",
        oino_ai::transcript::escape_reminder_tags(text)
    )
}
