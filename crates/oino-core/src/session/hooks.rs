//! Agent hooks installed by the session (pi `_installAgent*` methods and the `sdk.ts` agent
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
    StopReason, SystemMessage, UserContent,
};
use oino_models::AuthOverrides;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, Inner, current_system_message, lock};

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
            session.before_tool_call(cx.tool_call, cx.args, None).await
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
    /// `tool_call` handlers (pi `_beforeToolCall`).
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

    /// `tool_result` handlers (pi `_afterToolCall`).
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

    /// pi `_installAgentRequestProjection`.
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

    /// pi `_installAgentNextTurnRefresh`.
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
        let hidden = self.hidden_declarations();
        if !hidden.is_empty() {
            for message in &mut messages {
                if let AgentMessage::System(system) = message {
                    if let Some(added) = &mut system.tools_added {
                        added.retain(|t| !hidden.contains(&t.name));
                        if added.is_empty() {
                            system.tools_added = None;
                        }
                    }
                    if let Some(removed) = &mut system.tools_removed {
                        removed.retain(|t| !hidden.contains(&t.name));
                        if removed.is_empty() {
                            system.tools_removed = None;
                        }
                    }
                }
            }
        }
        let forced = lock(&self.inner.book)
            .run_prompt_options
            .as_ref()
            .and_then(|o| o.force_system_prompt.clone());
        if let Some(forced) = forced {
            let current = current_system_message(&messages);
            let head = SystemMessage {
                content: oino_ai::UserTextField::Text(forced),
                sections: None,
                tools_added: current.as_ref().and_then(|c| c.tools_added.clone()),
                tools_removed: None,
                timestamp: current.map_or_else(now_ms, |c| c.timestamp),
            };
            messages = std::iter::once(AgentMessage::System(head))
                .chain(
                    messages
                        .into_iter()
                        .filter(|m| !matches!(m, AgentMessage::System(_))),
                )
                .collect();
        }
        messages
    }
}

/// Content blocks with their `type` tags (pi shapes).
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
                let model = request.model.clone();
                match session.authorize(request).await {
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
    /// pi `_getRequiredRequestAuth` + header transforms.
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

/// pi OAuth failure text.
pub(crate) fn oauth_failed_message(provider: &str) -> String {
    format!(
        "Authentication failed for \"{provider}\". Credentials may have expired or network is unavailable. Run '/login {provider}' to re-authenticate."
    )
}
