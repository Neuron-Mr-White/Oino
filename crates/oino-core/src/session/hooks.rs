//! Agent hooks installed by the session (`_installAgent*` methods and the agent
//! wiring): tool interception, request projection + virtual routing, turn boundaries,
//! between-turn compaction and prompt refresh, context transforms, per-request auth.

use std::sync::{Arc, OnceLock, Weak};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use oino_agent::{
    AfterToolCall, AfterToolCallContext, AfterToolCallResult, AgentContext, AgentHooks,
    AgentLoopTurnUpdate, AgentRequestUpdate, AgentTurnContext, BeforeToolCall,
    BeforeToolCallContext, BeforeToolCallResult, FinishTurn, HookError, PrepareNextTurn,
    PrepareRequest, PrepareRequestContext, StreamFn, StreamRequest, TransformContext, TurnDecision,
    now_ms,
};
use oino_ai::{
    AgentMessage, AssistantMessage, AssistantMessageEvent, ErrorReason, JsonObject, JsonValue,
    StopReason, UserContent,
};
use oino_models::AuthOverrides;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, Inner, lock};

/// Where the session log must be durable before the run proceeds (D-FLUSH, core spec
/// §2.1): the three fsync barriers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Barrier {
    /// Before each model request (the failed request never leaves the process).
    Request,
    /// Before a tool with side effects runs (its body must be attributable to a record).
    Tool,
    /// At a step boundary (diagnostics only; the next request barrier fails closed).
    Step,
}

impl Barrier {
    const fn name(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Tool => "tool",
            Self::Step => "step",
        }
    }
}

/// Built-in tools with no side effects: they are exempt from the tool barrier (D-FLUSH
/// barrier 2; read-only set `read`, `grep`, `find`, `ls`) — nothing to repair and no
/// effect to attribute, so no fsync and no dispatch record (D-DISPATCH).
const READ_ONLY_BUILTIN_TOOLS: [&str; 4] = ["read", "grep", "find", "ls"];

pub(crate) struct SessionHooks {
    inner: Weak<Inner>,
}

impl SessionHooks {
    fn session(&self) -> Result<AgentSession, HookError> {
        self.inner
            .upgrade()
            .map(AgentSession::from_inner)
            .ok_or_else(|| HookError::new("Session disposed"))
    }
}

/// Builds the hook set for the agent of `inner`.
pub(crate) fn install(inner: &Arc<Inner>) -> AgentHooks {
    let hooks = Arc::new(SessionHooks {
        inner: Arc::downgrade(inner),
    });
    AgentHooks {
        transform_context: Some(Arc::clone(&hooks) as _),
        get_api_key: None,
        before_tool_call: Some(Arc::clone(&hooks) as _),
        after_tool_call: Some(Arc::clone(&hooks) as _),
        finish_turn: Some(Arc::clone(&hooks) as _),
        prepare_request: Some(Arc::clone(&hooks) as _),
        prepare_next_turn: Some(hooks as _),
    }
}

impl BeforeToolCall for SessionHooks {
    fn before<'a>(
        &'a self,
        cx: BeforeToolCallContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<BeforeToolCallResult>, HookError>> {
        Box::pin(async move {
            let session = self.session()?;
            let result = session
                .before_tool_call(cx.tool_call, cx.args, None)
                .await?;
            if !result.as_ref().is_some_and(|r| r.block) {
                let args = result
                    .as_ref()
                    .and_then(|r| r.input.as_ref())
                    .unwrap_or(cx.args);
                session.checkpoint_tool_call(&cx.tool_call.name, args);
                // D-DISPATCH + D-FLUSH barrier 2: the call is on the durable log before
                // the tool body runs — the record attributes an interrupted effect, the
                // barrier makes the whole log durable. Either failure becomes the tool's
                // error result (the hook-failure path), so the body never runs
                // unpersisted. Read-only tools skip both.
                session.tool_dispatch_barrier(cx.tool_call, args)?;
            }
            Ok(result)
        })
    }
}

impl AfterToolCall for SessionHooks {
    fn after<'a>(
        &'a self,
        cx: AfterToolCallContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AfterToolCallResult>, HookError>> {
        Box::pin(async move {
            let session = self.session()?;
            Ok(session.after_tool_call(&cx, None).await)
        })
    }
}

impl FinishTurn for SessionHooks {
    fn finish<'a>(
        &'a self,
        turn: AgentTurnContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<TurnDecision>, HookError>> {
        Box::pin(async move {
            let session = self.session()?;
            lock(&session.inner.book).boundary_dispatched = true;
            let extension_continue = session
                .dispatch_turn_end_boundary(turn.message, turn.tool_results)
                .await;
            Ok(extension_continue.then_some(TurnDecision::Continue))
        })
    }
}

impl PrepareRequest for SessionHooks {
    fn prepare<'a>(
        &'a self,
        request: PrepareRequestContext<'a>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AgentRequestUpdate>, HookError>> {
        Box::pin(async move {
            let session = self.session()?;
            session
                .prepare_request(request.context, cancel)
                .await
                .map(Some)
        })
    }
}

impl PrepareNextTurn for SessionHooks {
    fn prepare<'a>(
        &'a self,
        turn: AgentTurnContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AgentLoopTurnUpdate>, HookError>> {
        Box::pin(async move {
            let session = self.session()?;
            Ok(Some(session.prepare_next_turn(turn.context).await))
        })
    }
}

impl TransformContext for SessionHooks {
    fn transform<'a>(
        &'a self,
        messages: Vec<AgentMessage>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Vec<AgentMessage>, HookError>> {
        Box::pin(async move {
            let session = self.session()?;
            Ok(session.transform_context(messages).await)
        })
    }
}

impl AgentSession {
    /// Fsync barrier (D-FLUSH): flushes the session file; `Err` is the fail-closed signal
    /// the caller turns into a request error event (barrier 1), a `HookError` tool result
    /// (barrier 2) or a diagnostic log (barrier 3). In-memory sessions always pass.
    pub(crate) fn durability_barrier(&self, barrier: Barrier) -> Result<(), String> {
        lock(&self.inner.manager)
            .sync()
            .map_err(|error| format!("session durability barrier failed: {error}"))
            .inspect_err(|error| {
                tracing::error!(barrier = barrier.name(), session = %self.session_id(), "{error}");
            })
    }

    /// D-DISPATCH + D-FLUSH barrier 2: append the dispatch record of a tool with side
    /// effects, then make the log durable — the tool body runs only after both
    /// succeeded. Read-only tools (`read`/`grep`/`find`/`ls` built-ins and
    /// `annotations.readOnly == true` extension tools) skip the record and the fsync:
    /// nothing to repair and no effect to attribute.
    pub(crate) fn tool_dispatch_barrier(
        &self,
        call: &oino_ai::ToolCall,
        args: &JsonObject,
    ) -> Result<(), HookError> {
        if self.tool_is_read_only(&call.name) {
            return Ok(());
        }
        let recorded =
            self.with_manager(|manager| manager.append_tool_dispatch(&call.id, &call.name, args));
        if let Err(error) = recorded {
            tracing::error!(session = %self.session_id(), "tool dispatch record failed: {error}");
            return Err(HookError::new(format!(
                "session dispatch record failed: {error}"
            )));
        }
        self.durability_barrier(Barrier::Tool)
            .map_err(HookError::new)
    }

    /// Side-effect classification of a tool (D-FLUSH barrier 2).
    fn tool_is_read_only(&self, tool_name: &str) -> bool {
        if READ_ONLY_BUILTIN_TOOLS.contains(&tool_name) {
            return true;
        }
        self.runner().tools().iter().any(|tool| {
            tool.spec.name == tool_name
                && tool
                    .spec
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.get("readOnly"))
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
        })
    }

    /// D-FLUSH barrier 3: sync at a step boundary. Failures are diagnostics — the already
    /// finished turn stands; the next request barrier fails closed on the same error.
    pub(crate) fn boundary_sync(&self, at: &'static str) {
        if let Err(error) = self.durability_barrier(Barrier::Step) {
            tracing::error!(boundary = at, session = %self.session_id(), "{error}");
        }
    }

    /// `tool_call` handlers.
    pub(crate) async fn before_tool_call(
        &self,
        call: &oino_ai::ToolCall,
        args: &JsonObject,
        parent_tool_call_id: Option<&str>,
    ) -> Result<Option<BeforeToolCallResult>, HookError> {
        let runner = self.runner();
        if !runner.has_handlers("tool_call") {
            return Ok(None);
        }
        let mut payload = json!({
            "type": "tool_call",
            "toolName": call.name,
            "toolCallId": call.id,
            "input": args,
        });
        if let Some(parent) = parent_tool_call_id {
            payload["parentToolCallId"] = json!(parent);
        }
        let Some(result) = runner.emit_tool_call(payload).await else {
            return Ok(None);
        };
        Ok(Some(BeforeToolCallResult {
            block: result.get("block") == Some(&JsonValue::Bool(true)),
            reason: result
                .get("reason")
                .and_then(JsonValue::as_str)
                .map(str::to_owned),
            terminate: result.get("terminate") == Some(&JsonValue::Bool(true)),
            input: result.get("input").and_then(JsonValue::as_object).cloned(),
        }))
    }

    /// `tool_result` handlers.
    pub(crate) async fn after_tool_call(
        &self,
        cx: &AfterToolCallContext<'_>,
        parent_tool_call_id: Option<&str>,
    ) -> Option<AfterToolCallResult> {
        let runner = self.runner();
        if !runner.has_handlers("tool_result") {
            return None;
        }
        let mut event = JsonObject::new();
        event.insert("type".into(), json!("tool_result"));
        event.insert("toolName".into(), json!(cx.tool_call.name));
        event.insert("toolCallId".into(), json!(cx.tool_call.id));
        if let Some(parent) = parent_tool_call_id {
            event.insert("parentToolCallId".into(), json!(parent));
        }
        event.insert("input".into(), JsonValue::Object(cx.args.clone()));
        event.insert("content".into(), json!(tagged_content(&cx.result.content)));
        event.insert(
            "details".into(),
            cx.result.details.clone().unwrap_or(JsonValue::Null),
        );
        if let Some(structured) = &cx.result.structured_content {
            event.insert("structuredContent".into(), structured.clone());
        }
        event.insert("isError".into(), json!(cx.is_error));
        if let Some(usage) = &cx.result.usage {
            event.insert("usage".into(), json!(usage));
        }
        let update = runner.emit_tool_result(event).await?;
        let content: Option<Vec<UserContent>> = update
            .get("content")
            .and_then(|c| serde_json::from_value(c.clone()).ok());
        Some(AfterToolCallResult {
            content: content.or_else(|| Some(cx.result.content.clone())),
            details: update.get("details").cloned().filter(|d| !d.is_null()),
            structured_content: update.get("structuredContent").cloned(),
            is_error: update.get("isError").and_then(JsonValue::as_bool),
            usage: update
                .get("usage")
                .and_then(|u| serde_json::from_value(u.clone()).ok()),
            terminate: None,
        })
    }

    /// Installs the agent request projection.
    async fn prepare_request(
        &self,
        context: &AgentContext,
        cancel: &CancellationToken,
    ) -> Result<AgentRequestUpdate, HookError> {
        let failed = lock(&self.inner.book).failed_response.take();
        let projection = lock(&self.inner.manager).build_session_projection();
        let mut next = AgentContext {
            messages: projection.messages.clone(),
            tools: context.tools.clone(),
        };
        let model = self.inner.agent.model();
        let thinking = self.inner.agent.thinking_level();
        if !oino_models::virtual_models::is_virtual_api(&model.api) {
            return Ok(AgentRequestUpdate {
                context: Some(next),
                model: Some(model),
                thinking_level: Some(thinking),
            });
        }
        let llm = oino_session::convert_to_llm(&next.messages);
        let last_response = llm
            .iter()
            .rposition(|m| matches!(m, oino_ai::Message::Assistant(_)));
        let user_turn = llm
            .iter()
            .skip(last_response.map_or(0, |i| i + 1))
            .any(|m| matches!(m, oino_ai::Message::User(_)));
        let reason = if failed.is_some() {
            oino_models::virtual_models::RouteReason::Retry
        } else if user_turn {
            oino_models::virtual_models::RouteReason::User
        } else {
            oino_models::virtual_models::RouteReason::Continuation
        };
        let route = self
            .route_virtual(&model, llm, reason, thinking, failed, cancel)
            .await
            .map_err(HookError::new)?;
        if self.exceeds_compaction_threshold(&route.model, &projection) {
            self.run_auto_compaction(crate::events::CompactionReason::Threshold, false)
                .await;
            next.messages = lock(&self.inner.manager)
                .build_session_projection()
                .messages;
        }
        Ok(AgentRequestUpdate {
            context: Some(next),
            model: Some(route.model),
            thinking_level: Some(route.thinking_level),
        })
    }

    /// Installs the next-turn refresh.
    async fn prepare_next_turn(&self, _context: &AgentContext) -> AgentLoopTurnUpdate {
        let messages = self.compact_before_next_assistant_response().await;
        let base = self.system_prompt_options();
        let run = lock(&self.inner.book).run_prompt_options.clone();
        let mut options = run.unwrap_or_else(|| base.clone());
        options.selected_tools = self.active_tool_names();
        let mut snippets = base.tool_snippets.clone();
        snippets.extend(options.tool_snippets.clone());
        options.tool_snippets = snippets;
        let mut guidelines = base.tool_guidelines.clone();
        guidelines.extend(options.tool_guidelines.clone());
        options.tool_guidelines = guidelines;
        let update = self.prepare_prompt_and_tool_loadout(&mut options, &messages);
        lock(&self.inner.book).run_prompt_options = Some(options);
        AgentLoopTurnUpdate {
            context: Some(AgentContext {
                messages,
                tools: self.inner.agent.with_state(|s| s.tools.clone()),
            }),
            messages: update.map(|m| vec![AgentMessage::System(m)]),
            model: Some(self.inner.agent.model()),
            thinking_level: Some(self.inner.agent.thinking_level()),
        }
    }

    /// Agent `transformContext`: `context` handlers, hidden declarations, forced prompt.
    async fn transform_context(&self, messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        let runner = self.runner();
        let mut messages =
            if runner.has_handlers("context") || runner.has_handlers("context_with_system") {
                runner.emit_context(messages).await
            } else {
                messages
            };
        messages = self.prefix_view(messages);

        messages
    }
}

/// Content blocks with their `type` tags (wire shapes).
pub(crate) fn tagged_content(content: &[UserContent]) -> JsonValue {
    serde_json::to_value(content).unwrap_or(JsonValue::Array(Vec::new()))
}

/// The session's provider stream: resolves credentials per request, applies
/// `before_provider_headers`, then dispatches to the provider components.
pub(crate) struct SessionStream {
    pub(crate) inner: OnceLock<Weak<Inner>>,
    pub(crate) base: Arc<dyn StreamFn>,
}

impl StreamFn for SessionStream {
    fn stream(&self, request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        let inner = self.inner.get().and_then(Weak::upgrade);
        let base = Arc::clone(&self.base);
        Box::pin(
            stream::once(async move {
                let Some(inner) = inner else {
                    return base.stream(request);
                };
                let session = AgentSession::from_inner(inner);
                let mut request = request;
                let model = request.model.clone();
                // Append model changes before this request, never into either slot.
                let selection = format!("{}/{}", model.provider, model.id);
                let changed = lock(&session.inner.book)
                    .reminder_model
                    .as_ref()
                    .is_some_and(|old| old != &selection);
                let reminder_error = if changed {
                    match session.append_reminder(
                        "oino",
                        "oino/model",
                        &format!("Model changed to {selection}."),
                    ) {
                        Ok(_) => {
                            lock(&session.inner.book).reminder_model = Some(selection);
                            None
                        }
                        Err(error) => Some(error.to_string()),
                    }
                } else {
                    None
                };
                // A reminder may arrive after context transformation. Add only
                // missing host envelopes, preserving extension context transforms.
                let reminders = session.reminder_messages();
                let serialized = convert_provider_view(&session, &reminders);
                for reminder in serialized {
                    if !request.context.messages.contains(&reminder) {
                        request.context.messages.push(reminder);
                    }
                }
                // D-FLUSH barrier 1: the log is durable before each model request (first
                // request, steering, retries, compaction continuations). A failure returns
                // a single error event shaped like an authorize failure, so the request
                // never leaves the process.
                let authorized = match reminder_error {
                    Some(error) => Err(error),
                    None => match session.durability_barrier(Barrier::Request) {
                        Ok(()) => session.authorize(request).await,
                        Err(message) => Err(message),
                    },
                };
                match authorized {
                    Ok(request) => base.stream(request),
                    Err(message) => {
                        let mut error = AssistantMessage::empty(
                            &model.api,
                            &model.provider,
                            &model.id,
                            now_ms(),
                        );
                        error.stop_reason = StopReason::Error;
                        error.error_message = Some(message);
                        Box::pin(stream::once(async move {
                            AssistantMessageEvent::Error {
                                reason: ErrorReason::Error,
                                error,
                            }
                        }))
                    }
                }
            })
            .flatten(),
        )
    }
}

impl AgentSession {
    /// `_getRequiredRequestAuth` + header transforms.
    async fn authorize(&self, mut request: StreamRequest) -> Result<StreamRequest, String> {
        let auth = self
            .inner
            .models
            .get_api_key_and_headers(&request.model, &AuthOverrides::default())
            .await;
        let auth = match auth {
            Ok(auth) => auth,
            Err(error) => return Err(error.message),
        };
        if auth.api_key.is_none()
            && auth.headers.is_none()
            && !self
                .inner
                .models
                .has_configured_auth(&request.model.provider)
        {
            let provider = request.model.provider.clone();
            let message = if self.inner.models.is_using_oauth(&provider) {
                oauth_failed_message(&provider)
            } else {
                oino_models::auth::guidance::no_api_key_found_message(
                    &provider,
                    &self.inner.docs_root.join("docs"),
                )
            };
            return Err(message);
        }
        request.api_key = auth.api_key;
        let mut headers: indexmap::IndexMap<String, Option<String>> = auth
            .headers
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, Some(v)))
            .collect();
        if let Some(extra) = request.options.headers.take() {
            headers.extend(extra);
        }
        let runner = self.runner();
        if runner.has_handlers("before_provider_headers") {
            let payload = json!({"type": "before_provider_headers", "headers": headers});
            let result = runner.emit_chain(payload, "headers").await;
            if let Ok(next) = serde_json::from_value(result) {
                headers = next;
            }
        }
        request.options.headers = (!headers.is_empty()).then_some(headers);
        let settings = lock(&self.inner.settings).snapshot();
        let retry = settings.provider_retry_settings();
        let idle = settings.http_idle_timeout_ms().unwrap_or(300_000);
        request.options.timeout_ms = request
            .options
            .timeout_ms
            .or(retry.timeout_ms)
            .or(Some(if idle == 0 { 2_147_483_647 } else { idle }));
        request.options.max_retries = request
            .options
            .max_retries
            .or_else(|| retry.max_retries.and_then(|r| u32::try_from(r).ok()));
        Ok(request)
    }
}

/// OAuth failure text.
pub(crate) fn oauth_failed_message(provider: &str) -> String {
    format!(
        "Authentication failed for \"{provider}\". Credentials may have expired or network is unavailable. Run '/login {provider}' to re-authenticate."
    )
}

fn convert_provider_view(
    session: &AgentSession,
    messages: &[AgentMessage],
) -> Vec<oino_ai::Message> {
    (super::convert_with_block_images(Arc::clone(&session.inner.settings)))(messages)
}
