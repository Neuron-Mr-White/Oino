//! Test doubles built into the binary with feature `faux` (never in release builds):
//!
//! - `builtin:faux` — a scripted provider component (provider `faux`, model `faux-1`, API
//!   `faux`). Responses come from the JSON array in the file named by `OINO_FAUX_SCRIPT`
//!   (or inline in `OINO_FAUX_RESPONSES`); credentials from `OINO_FAUX_API_KEY`.
//!   It also registers provider `faux-oauth` (model `faux-oauth-1`, OAuth only): its
//!   `oauth-login` asks for a code with an `input` dialog and returns it as the access token.
//! - `builtin:ui-probe` — an extension with a `/probe <method> [title]` command that raises
//!   the extension UI method and reports the answer with `notify` (`probe:<method>:<json>`),
//!   and `/probe-fail` whose command fails (an `extension_error`). It also exercises the
//!   frontend surface: `/probe` argument completion, shortcut `probe.ping`, panel actions
//!   and editor intents (reported as `probe:action:…` / `probe:editor:…`; `emptyArrowDown`
//!   is consumed), service `probe.echo`, settings namespace `probe` and a `complete-input`
//!   contributor for `#` tokens.
//!
//! Script steps: `{"text"?, "thinking"?, "toolCalls"?: [{id, name, arguments}], "error"?,
//! "stopReason"?, "delayMs"?}`; `delayMs` is slept before every streamed event, so a step
//! can stay "streaming" while a test steers or aborts.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{
    AssistantContent, AssistantMessage, DoneReason, ErrorReason, JsonObject, JsonValue, Model,
    ProviderEvent, StopReason, ToolCall,
};
use oino_core::extensions::api::{
    CallContext, CommandSpec, CompletionItem, ExtensionCallError, ExtensionInstance, ExtensionKind,
    ProviderInstance, Registration, ShortcutSpec,
};
use oino_core::extensions::host::{HostApi, HostOp};
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// The faux provider API.
pub const FAUX_API: &str = "faux";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One scripted tool call.
#[derive(Clone, Debug, Deserialize)]
struct ScriptCall {
    id: String,
    name: String,
    #[serde(default)]
    arguments: JsonObject,
}

/// One scripted response.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptStep {
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ScriptCall>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    stop_reason: Option<StopReason>,
    #[serde(default)]
    delay_ms: u64,
}

/// The faux model.
fn faux_model() -> Option<Model> {
    model_of("faux", "faux-1")
}

fn model_of(provider: &str, id: &str) -> Option<Model> {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": FAUX_API, "provider": provider, "baseUrl": "",
        "reasoning": true, "input": ["text", "image"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 128_000, "maxTokens": 16_384
    }))
    .ok()
}

fn parse_script(text: &str) -> VecDeque<ScriptStep> {
    serde_json::from_str::<Vec<ScriptStep>>(text)
        .unwrap_or_default()
        .into()
}

fn events(step: ScriptStep) -> Vec<ProviderEvent> {
    let mut message = AssistantMessage::empty(FAUX_API, "faux", "faux-1", oino_agent::now_ms());
    let mut content = Vec::new();
    if let Some(thinking) = step.thinking {
        content.push(AssistantContent::Thinking(oino_ai::ThinkingContent {
            thinking,
            ..oino_ai::ThinkingContent::default()
        }));
    }
    if let Some(text) = step.text {
        content.push(AssistantContent::text(text));
    }
    for call in step.tool_calls {
        content.push(AssistantContent::ToolCall(ToolCall {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
            ..ToolCall::default()
        }));
    }
    let stop_reason = step.stop_reason.unwrap_or(if step.error.is_some() {
        StopReason::Error
    } else if content
        .iter()
        .any(|c| matches!(c, AssistantContent::ToolCall(_)))
    {
        StopReason::ToolUse
    } else {
        StopReason::Stop
    });
    let mut out = vec![ProviderEvent::Start {}];
    for (index, block) in content.iter().enumerate() {
        out.extend(block_events(index, block));
    }
    message.content = content;
    message.stop_reason = stop_reason;
    message.error_message = step.error;
    out.push(match stop_reason {
        StopReason::Error => ProviderEvent::Error {
            reason: ErrorReason::Error,
            error: message,
        },
        StopReason::Aborted => ProviderEvent::Error {
            reason: ErrorReason::Aborted,
            error: message,
        },
        StopReason::ToolUse => ProviderEvent::Done {
            reason: DoneReason::ToolUse,
            message,
        },
        StopReason::Length => ProviderEvent::Done {
            reason: DoneReason::Length,
            message,
        },
        _ => ProviderEvent::Done {
            reason: DoneReason::Stop,
            message,
        },
    });
    out
}

/// `*_start`, `*_delta`, `*_end` of one content block.
fn block_events(index: usize, block: &AssistantContent) -> [ProviderEvent; 3] {
    match block {
        AssistantContent::Text(text) => [
            ProviderEvent::TextStart {
                content_index: index,
            },
            ProviderEvent::TextDelta {
                content_index: index,
                delta: text.text.clone(),
            },
            ProviderEvent::TextEnd {
                content_index: index,
                content: text.text.clone(),
            },
        ],
        AssistantContent::Thinking(thinking) => [
            ProviderEvent::ThinkingStart {
                content_index: index,
            },
            ProviderEvent::ThinkingDelta {
                content_index: index,
                delta: thinking.thinking.clone(),
            },
            ProviderEvent::ThinkingEnd {
                content_index: index,
                content: thinking.thinking.clone(),
            },
        ],
        AssistantContent::ToolCall(call) => [
            ProviderEvent::ToolcallStart {
                content_index: index,
                id: Some(call.id.clone()),
                tool_name: Some(call.name.clone()),
            },
            ProviderEvent::ToolcallDelta {
                content_index: index,
                delta: serde_json::to_string(&call.arguments).unwrap_or_default(),
            },
            ProviderEvent::ToolcallEnd {
                content_index: index,
                tool_call: call.clone(),
            },
        ],
    }
}

fn aborted() -> ProviderEvent {
    let mut message = AssistantMessage::empty(FAUX_API, "faux", "faux-1", oino_agent::now_ms());
    message.stop_reason = StopReason::Aborted;
    message.error_message = Some("Request was aborted".to_owned());
    ProviderEvent::Error {
        reason: ErrorReason::Aborted,
        error: message,
    }
}

/// `builtin:faux`.
pub struct FauxProvider {
    steps: Mutex<VecDeque<ScriptStep>>,
    host: Mutex<Option<Weak<dyn HostApi>>>,
}

impl std::fmt::Debug for FauxProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FauxProvider").finish_non_exhaustive()
    }
}

impl FauxProvider {
    /// Loads the script from `OINO_FAUX_RESPONSES` or the file in `OINO_FAUX_SCRIPT`.
    #[must_use]
    pub fn from_env() -> Self {
        let text = std::env::var("OINO_FAUX_RESPONSES").ok().or_else(|| {
            std::env::var_os("OINO_FAUX_SCRIPT").and_then(|path| std::fs::read_to_string(path).ok())
        });
        Self::from_script(text.as_deref().unwrap_or("[]"))
    }

    /// A provider answering with the steps of a JSON script (invalid scripts are empty).
    #[must_use]
    pub fn from_script(script: &str) -> Self {
        Self {
            steps: Mutex::new(parse_script(script)),
            host: Mutex::new(None),
        }
    }

    /// The host OAuth dialogs go to.
    #[must_use]
    pub fn with_host(self, host: &Arc<dyn HostApi>) -> Self {
        *lock(&self.host) = Some(Arc::downgrade(host));
        self
    }
}

impl ProviderInstance for FauxProvider {
    fn stream(
        &self,
        _request: JsonValue,
        cancel: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<BoxStream<'static, Result<ProviderEvent, ExtensionCallError>>, ExtensionCallError>,
    > {
        let step = lock(&self.steps).pop_front().unwrap_or_else(|| ScriptStep {
            text: Some("(no more responses)".to_owned()),
            ..ScriptStep::default()
        });
        let delay = Duration::from_millis(step.delay_ms);
        let queue: VecDeque<ProviderEvent> = events(step).into();
        let stream = futures::stream::unfold(Some(queue), move |state| {
            let cancel = cancel.clone();
            async move {
                let mut queue = state?;
                if !delay.is_zero() {
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        () = cancel.cancelled() => return Some((Ok(aborted()), None)),
                    }
                }
                if cancel.is_cancelled() {
                    return Some((Ok(aborted()), None));
                }
                let event = queue.pop_front()?;
                Some((Ok(event), Some(queue)))
            }
        });
        Box::pin(async move { Ok(stream.boxed()) })
    }

    fn list_models(
        &self,
        _provider_id: &str,
        _credential: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<Vec<Model>, ExtensionCallError>> {
        Box::pin(async { Ok(faux_model().into_iter().collect()) })
    }

    fn oauth_login(
        &self,
        provider_id: &str,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        let host = lock(&self.host).as_ref().and_then(Weak::upgrade);
        let provider_id = provider_id.to_owned();
        Box::pin(async move {
            let host = host.ok_or_else(|| ExtensionCallError::new("host gone"))?;
            let params: JsonObject = serde_json::from_value(
                json!({"title": format!("Sign in to {provider_id}"), "placeholder": "Paste the code"}),
            )
            .unwrap_or_default();
            let answer = host
                .call(
                    "faux",
                    &cx,
                    HostOp::UiDialog {
                        method: "input".to_owned(),
                        params,
                        cancel: None,
                    },
                )
                .await
                .map_err(|e| ExtensionCallError::new(e.to_string()))?;
            let code = answer["value"]
                .as_str()
                .filter(|c| !c.is_empty())
                .ok_or_else(|| ExtensionCallError::new("Login cancelled"))?;
            Ok(
                json!({"type": "oauth", "access": code, "refresh": "faux-refresh", "expires": 4_102_444_800_000_i64}),
            )
        })
    }

    fn oauth_refresh(
        &self,
        _provider_id: &str,
        mut credential: JsonValue,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        credential["expires"] = json!(4_102_444_800_000_i64);
        Box::pin(async move { Ok(credential) })
    }
}

/// The faux provider as a component.
#[derive(Debug)]
pub struct FauxComponent(pub FauxProvider);

impl ExtensionInstance for FauxComponent {
    fn id(&self) -> &'static str {
        "faux"
    }

    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Provider
    }

    fn path(&self) -> String {
        "builtin:faux".to_owned()
    }

    fn init(&self, _cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
        Box::pin(async {
            Ok(Registration {
                apis: vec![FAUX_API.into()],
                providers: vec![
                    json!({
                        "id": "faux", "name": "Faux", "api": FAUX_API,
                        "authEnv": ["OINO_FAUX_API_KEY"],
                        "models": faux_model().into_iter().collect::<Vec<_>>(),
                    }),
                    json!({
                        "id": "faux-oauth", "name": "Faux OAuth", "api": FAUX_API,
                        "oauth": true,
                        "models": model_of("faux-oauth", "faux-oauth-1").into_iter().collect::<Vec<_>>(),
                    }),
                ],
                ..Registration::default()
            })
        })
    }

    fn on_event(
        &self,
        _kind: &str,
        _payload: JsonValue,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async { Ok(None) })
    }

    fn run_tool(
        &self,
        tool: &str,
        _invocation: ToolInvocation,
        _updates: UpdateSink,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>> {
        let message = format!("unknown tool {tool}");
        Box::pin(async move { Err(ExtensionCallError::new(message)) })
    }

    fn run_command(
        &self,
        _name: &str,
        _args: &str,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async { Ok(None) })
    }

    fn provider(&self) -> Option<&dyn ProviderInstance> {
        Some(&self.0)
    }
}

/// `builtin:ui-probe`.
pub struct UiProbe {
    host: Weak<dyn HostApi>,
}

impl std::fmt::Debug for UiProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiProbe").finish_non_exhaustive()
    }
}

impl UiProbe {
    /// A probe calling back into `host`.
    #[must_use]
    pub fn new(host: &Arc<dyn HostApi>) -> Self {
        Self {
            host: Arc::downgrade(host),
        }
    }

    async fn probe(&self, args: &str, cx: &CallContext) -> Result<(), ExtensionCallError> {
        let host = self
            .host
            .upgrade()
            .ok_or_else(|| ExtensionCallError::new("host gone"))?;
        let mut words = args.split_whitespace();
        let method = words.next().unwrap_or("confirm").to_owned();
        let title = words.collect::<Vec<_>>().join(" ");
        let title = if title.is_empty() {
            "Probe".to_owned()
        } else {
            title
        };
        let params =
            |value: JsonValue| -> JsonObject { serde_json::from_value(value).unwrap_or_default() };
        let call = |op| host.call("ui-probe", cx, op);
        let (op, dialog) = match method.as_str() {
            "select" => (params(json!({"title": title, "options": ["a", "b"]})), true),
            "confirm" => (params(json!({"title": title, "message": "Sure?"})), true),
            "input" => (params(json!({"title": title, "placeholder": "type"})), true),
            "editor" => (params(json!({"title": title, "prefill": "draft"})), true),
            "timeout" => (
                params(json!({"title": title, "options": ["a"], "timeout": 50})),
                true,
            ),
            "form" => (
                params(
                    json!({"title": title, "questions": [{"id": "q", "question": "Q?", "options": []}]}),
                ),
                true,
            ),
            "notify" => (
                params(json!({"message": title, "notifyType": "warning"})),
                false,
            ),
            "setStatus" => (
                params(json!({"statusKey": "probe", "statusText": title})),
                false,
            ),
            "setWidget" => (
                params(
                    json!({"widgetKey": "probe", "widgetLines": [title], "widgetPlacement": "belowEditor"}),
                ),
                false,
            ),
            "setTitle" => (params(json!({"title": title})), false),
            "panel" | "editor-text" => return self.frontend_probe(&method, &title, cx).await,
            "set_editor_text" => (params(json!({"text": title})), false),
            "setFrame" => (
                params(
                    json!({"frameKey": "probe", "top": [{"text": title, "priority": 1, "align": "left"}]}),
                ),
                false,
            ),
            other => return Err(ExtensionCallError::new(format!("unknown probe {other}"))),
        };
        let wire_method = if method == "timeout" {
            "select".to_owned()
        } else {
            method.clone()
        };
        if dialog {
            let answer = call(HostOp::UiDialog {
                method: wire_method,
                params: op,
                cancel: None,
            })
            .await
            .map_err(|e| ExtensionCallError::new(e.to_string()))?;
            let report = params(json!({"message": format!("probe:{method}:{answer}")}));
            call(HostOp::UiFire {
                method: "notify".to_owned(),
                params: report,
            })
            .await
            .map_err(|e| ExtensionCallError::new(e.to_string()))?;
        } else {
            call(HostOp::UiFire {
                method: wire_method,
                params: op,
            })
            .await
            .map_err(|e| ExtensionCallError::new(e.to_string()))?;
        }
        Ok(())
    }
}

/// `/probe` methods offered by argument completion.
const PROBE_METHODS: [&str; 14] = [
    "select",
    "confirm",
    "input",
    "editor",
    "timeout",
    "form",
    "notify",
    "setStatus",
    "setWidget",
    "setTitle",
    "set_editor_text",
    "setFrame",
    "panel",
    "editor-text",
];

impl UiProbe {
    /// `/probe panel` opens a panel with a button; `/probe editor-text` reports
    /// `ui.editor-text`.
    async fn frontend_probe(
        &self,
        method: &str,
        title: &str,
        cx: &CallContext,
    ) -> Result<(), ExtensionCallError> {
        let host = self
            .host
            .upgrade()
            .ok_or_else(|| ExtensionCallError::new("host gone"))?;
        let failed =
            |e: oino_core::extensions::host::HostError| ExtensionCallError::new(e.to_string());
        if method == "editor-text" {
            let text = host
                .call("ui-probe", cx, HostOp::EditorText)
                .await
                .map_err(failed)?;
            return self.notify(cx, format!("probe:editor-text:{text}")).await;
        }
        let spec = json!({"type": "button", "label": "Go", "actionId": "go"});
        let params: JsonObject = serde_json::from_value(
            json!({"panelId": "ui-probe/panel-0", "title": title, "placement": "overlay", "spec": spec}),
        )
        .unwrap_or_default();
        host.call(
            "ui-probe",
            cx,
            HostOp::UiFire {
                method: "openPanel".to_owned(),
                params,
            },
        )
        .await
        .map(drop)
        .map_err(failed)
    }

    async fn notify(&self, cx: &CallContext, message: String) -> Result<(), ExtensionCallError> {
        let host = self
            .host
            .upgrade()
            .ok_or_else(|| ExtensionCallError::new("host gone"))?;
        let params: JsonObject =
            serde_json::from_value(json!({"message": message})).unwrap_or_default();
        host.call(
            "ui-probe",
            cx,
            HostOp::UiFire {
                method: "notify".to_owned(),
                params,
            },
        )
        .await
        .map(drop)
        .map_err(|e| ExtensionCallError::new(e.to_string()))
    }
}

impl ExtensionInstance for UiProbe {
    fn id(&self) -> &'static str {
        "ui-probe"
    }

    fn path(&self) -> String {
        "builtin:ui-probe".to_owned()
    }

    fn init(&self, _cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
        Box::pin(async {
            Ok(Registration {
                commands: vec![
                    CommandSpec {
                        name: "probe".into(),
                        description: Some("Raise an extension UI request".into()),
                        hidden: false,
                        completions: true,
                    },
                    CommandSpec {
                        name: "probe-fail".into(),
                        description: Some("Fail".into()),
                        hidden: false,
                        completions: false,
                    },
                ],
                shortcuts: vec![ShortcutSpec {
                    id: "probe.ping".into(),
                    default_keys: vec!["ctrl+alt+p".into()],
                    description: "Ping the probe".into(),
                }],
                events: vec!["oino_ui_action".into(), "oino_editor".into()],
                settings: vec![json!({
                    "namespace": "probe", "title": "Probe",
                    "sections": [{"id": "main", "title": "Main", "fields": [
                        {"key": "enabled", "type": "boolean", "label": "Enabled", "default": true},
                        {"key": "mode", "type": "enum", "label": "Mode", "default": "a",
                         "options": [{"value": "a", "label": "A"}, {"value": "b", "label": "B"}]},
                        {"key": "token", "type": "secret", "label": "Token", "scope": "global"}
                    ]}]
                })],
                ..Registration::default()
            })
        })
    }

    fn on_event(
        &self,
        kind: &str,
        payload: JsonValue,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        let kind = kind.to_owned();
        Box::pin(async move {
            match kind.as_str() {
                "oino_ui_action" => {
                    let value = payload.get("value").cloned().unwrap_or(JsonValue::Null);
                    let message = format!(
                        "probe:action:{}:{value}",
                        payload["actionId"].as_str().unwrap_or("")
                    );
                    self.notify(&cx, message).await?;
                    Ok(None)
                }
                "oino_editor" => {
                    let intent = payload["intent"].as_str().unwrap_or_default();
                    if intent == "emptyArrowDown" {
                        return Ok(Some(json!({"consume": true})));
                    }
                    if intent == "submit" {
                        let message = format!(
                            "probe:editor:{intent}:{}",
                            payload["text"].as_str().unwrap_or("")
                        );
                        self.notify(&cx, message).await?;
                    }
                    Ok(None)
                }
                _ => Ok(None),
            }
        })
    }

    fn complete_command(
        &self,
        name: &str,
        prefix: &str,
        _cx: CallContext,
    ) -> BoxFuture<'_, Vec<CompletionItem>> {
        let items = if name == "probe" {
            PROBE_METHODS
                .iter()
                .filter(|m| m.starts_with(prefix))
                .map(|m| CompletionItem {
                    value: (*m).to_owned(),
                    label: (*m).to_owned(),
                    description: None,
                })
                .collect()
        } else {
            Vec::new()
        };
        Box::pin(async move { items })
    }

    fn run_shortcut(
        &self,
        id: &str,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<(), ExtensionCallError>> {
        let message = format!("probe:shortcut:{id}");
        Box::pin(async move { self.notify(&cx, message).await })
    }

    fn complete_input(
        &self,
        request: JsonValue,
        _cx: CallContext,
    ) -> BoxFuture<'_, Option<JsonValue>> {
        let text = request["text"].as_str().unwrap_or_default().to_owned();
        Box::pin(async move {
            text.split_whitespace().last().filter(|t| t.starts_with('#')).map(|_| {
                json!([{"source": "probe", "items": [{"value": "#probe", "description": "probe tag"}]}])
            })
        })
    }

    fn services(&self) -> Vec<String> {
        vec!["probe.echo".to_owned()]
    }

    fn handle_service(
        &self,
        name: &str,
        payload: JsonValue,
        caller: &str,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        let result = (name == "probe.echo")
            .then(|| json!({"echo": payload, "caller": caller}))
            .ok_or_else(|| ExtensionCallError::new(format!("unknown service {name}")));
        Box::pin(async move { result })
    }

    fn run_tool(
        &self,
        tool: &str,
        _invocation: ToolInvocation,
        _updates: UpdateSink,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>> {
        let message = format!("unknown tool {tool}");
        Box::pin(async move { Err(ExtensionCallError::new(message)) })
    }

    fn run_command(
        &self,
        name: &str,
        args: &str,
        _cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        let name = name.to_owned();
        let args = args.to_owned();
        Box::pin(async move {
            if name == "probe-fail" {
                return Err(ExtensionCallError::new("probe failed"));
            }
            self.probe(&args, &cx).await.map(|()| None)
        })
    }
}
