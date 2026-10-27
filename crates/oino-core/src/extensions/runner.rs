//! Extension runner: registries and event dispatch with pi composition semantics
//! (pi `ExtensionRunner`, runtime.md §7.2).
//!
//! Handlers run sequentially in load order. Notification and transform handlers that fail
//! are reported as `extension_error` and skipped (the chain keeps the previous value);
//! `tool_call` and `user_bash` failures block. Every call has a deadline.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use indexmap::{IndexMap, IndexSet};
use oino_ai::{AgentMessage, ImageContent, JsonObject, JsonValue};
use serde_json::json;

use super::api::{
    ArbiterRoles, CallContext, CommandSpec, ExtensionCallError, ExtensionInstance, FlagSpec,
    Registration, ShortcutSpec, ToolSpec,
};
use crate::events::ExtensionErrorEvent;

/// Default handler deadline.
pub const HOOK_DEADLINE: Duration = Duration::from_secs(5);
/// `tool_call` deadline.
pub const TOOL_CALL_DEADLINE: Duration = Duration::from_secs(30);
/// `session_before_*` deadline.
pub const SESSION_BEFORE_DEADLINE: Duration = Duration::from_secs(60);
/// Upper bound of a manifest-raised handler deadline (runtime §7.2).
pub const MAX_MANIFEST_DEADLINE: Duration = Duration::from_secs(300);
/// Deadline of one `wait-reason` / `propose-nudge` call (runtime §7.6).
pub const ARBITER_DEADLINE: Duration = Duration::from_secs(2);

/// Deadline for one handler of `kind`.
#[must_use]
pub fn deadline_for(kind: &str) -> Duration {
    if kind == "tool_call" {
        TOOL_CALL_DEADLINE
    } else if kind.starts_with("session_before_") {
        SESSION_BEFORE_DEADLINE
    } else {
        HOOK_DEADLINE
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Receives extension failures (published as `extension_error`).
pub trait ErrorSink: Send + Sync {
    /// Reports one failure.
    fn report(&self, error: ExtensionErrorEvent);
}

/// Mutable registrations of one extension (static + dynamic).
#[derive(Clone, Debug, Default)]
pub struct ExtensionState {
    /// Tools by name.
    pub tools: IndexMap<String, ToolSpec>,
    /// Commands in registration order.
    pub commands: Vec<CommandSpec>,
    /// Shortcuts by id.
    pub shortcuts: IndexMap<String, ShortcutSpec>,
    /// Flags by name.
    pub flags: IndexMap<String, FlagSpec>,
    /// Subscribed event kinds.
    pub events: IndexSet<String>,
    /// Render specs.
    pub renderers: Vec<JsonValue>,
    /// Implemented provider APIs.
    pub apis: Vec<String>,
    /// Turn arbiter roles.
    pub arbiter: ArbiterRoles,
}

impl From<Registration> for ExtensionState {
    fn from(registration: Registration) -> Self {
        Self {
            tools: registration
                .tools
                .into_iter()
                .map(|tool| (tool.name.clone(), tool))
                .collect(),
            commands: registration.commands,
            shortcuts: registration
                .shortcuts
                .into_iter()
                .map(|s| (s.id.clone(), s))
                .collect(),
            flags: registration
                .flags
                .into_iter()
                .map(|f| (f.name.clone(), f))
                .collect(),
            events: registration.events.into_iter().collect(),
            renderers: registration.renderers,
            apis: registration.apis,
            arbiter: registration.arbiter,
        }
    }
}

/// A loaded extension and its registrations.
pub struct LoadedExtension {
    /// The instance.
    pub instance: Arc<dyn ExtensionInstance>,
    state: Mutex<ExtensionState>,
}

impl std::fmt::Debug for LoadedExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedExtension")
            .field("id", &self.instance.id())
            .finish_non_exhaustive()
    }
}

impl LoadedExtension {
    /// Wraps an initialised instance.
    #[must_use]
    pub fn new(instance: Arc<dyn ExtensionInstance>, registration: Registration) -> Self {
        Self {
            instance,
            state: Mutex::new(registration.into()),
        }
    }

    /// Copy of the registrations.
    #[must_use]
    pub fn state(&self) -> ExtensionState {
        lock(&self.state).clone()
    }

    /// Edits the registrations.
    pub fn update<R>(&self, f: impl FnOnce(&mut ExtensionState) -> R) -> R {
        f(&mut lock(&self.state))
    }

    fn subscribed(&self, kind: &str) -> bool {
        lock(&self.state).events.contains(kind)
    }
}

/// Every loaded extension in load order (shared by the sessions of one cwd).
#[derive(Debug, Default)]
pub struct ExtensionSet {
    extensions: Vec<Arc<LoadedExtension>>,
}

impl ExtensionSet {
    /// A set in load order.
    #[must_use]
    pub fn new(extensions: Vec<Arc<LoadedExtension>>) -> Self {
        Self { extensions }
    }

    /// Extensions in load order.
    #[must_use]
    pub fn extensions(&self) -> &[Arc<LoadedExtension>] {
        &self.extensions
    }

    /// Extension by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Arc<LoadedExtension>> {
        self.extensions.iter().find(|e| e.instance.id() == id)
    }

    /// Number of extensions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    /// Whether no extension is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
}

/// A command with its resolved invocation name (pi `ResolvedCommand`).
#[derive(Clone, Debug)]
pub struct ResolvedCommand {
    /// Owning extension.
    pub extension: Arc<LoadedExtension>,
    /// Spec.
    pub spec: CommandSpec,
    /// `name`, or `name:N` on collisions.
    pub invocation_name: String,
}

/// A registered tool and its owner.
#[derive(Clone, Debug)]
pub struct RegisteredTool {
    /// Owning extension.
    pub extension: Arc<LoadedExtension>,
    /// Spec.
    pub spec: ToolSpec,
}

/// Result of [`ExtensionRunner::emit_input`].
#[derive(Clone, Debug, PartialEq)]
pub enum InputOutcome {
    /// Continue with the original input.
    Continue,
    /// Continue with replaced input.
    Transform {
        /// New text.
        text: String,
        /// New images.
        images: Option<Vec<ImageContent>>,
    },
    /// An extension handled the input.
    Handled,
}

/// Combined `before_agent_start` result.
#[derive(Clone, Debug, PartialEq)]
pub struct BeforeAgentStartOutcome {
    /// Messages to inject (`{customType, content, display, details?}`), in order.
    pub messages: Vec<JsonValue>,
    /// Final `systemPromptOptions`.
    pub system_prompt_options: JsonValue,
    /// Forced system prompt (last wins).
    pub system_prompt: Option<String>,
}

/// Result of a boundary dispatch (`turn_end`, `agent_before_settle`).
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryOutcome {
    /// Drafts to commit.
    pub entries: Vec<JsonValue>,
    /// Ensure one more request.
    pub continue_run: bool,
}

/// Collected `resources_discover` paths with their extension.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredResources {
    /// `(path, extension id)` skill paths.
    pub skill_paths: Vec<(String, String)>,
    /// Prompt template paths.
    pub prompt_paths: Vec<(String, String)>,
    /// Theme paths.
    pub theme_paths: Vec<(String, String)>,
}

/// Per-session dispatcher over the shared [`ExtensionSet`].
#[derive(Clone)]
pub struct ExtensionRunner {
    set: Arc<ExtensionSet>,
    cx: CallContext,
    errors: Arc<dyn ErrorSink>,
    flag_values: Arc<Mutex<IndexMap<String, JsonValue>>>,
    stale: Arc<Mutex<Option<String>>>,
    dialogs: Option<crate::ui::UiBridge>,
}

impl std::fmt::Debug for ExtensionRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionRunner")
            .field("extensions", &self.set.len())
            .field("session", &self.cx.session_id)
            .finish_non_exhaustive()
    }
}

/// pi `ExtensionRunner.invalidate` default message (verbatim, product name substituted).
pub const STALE_CONTEXT_MESSAGE: &str = "This extension ctx is stale after session replacement or reload. Do not use a captured Oino or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload().";

impl ExtensionRunner {
    /// A runner for one session.
    #[must_use]
    pub fn new(
        set: Arc<ExtensionSet>,
        cx: CallContext,
        errors: Arc<dyn ErrorSink>,
        flag_values: IndexMap<String, JsonValue>,
    ) -> Self {
        Self {
            set,
            cx,
            errors,
            flag_values: Arc::new(Mutex::new(flag_values)),
            stale: Arc::new(Mutex::new(None)),
            dialogs: None,
        }
    }

    /// Pauses handler deadlines while the handler's extension has a dialog open on `ui`,
    /// and withdraws its dialogs when a handler times out (GA-42).
    #[must_use]
    pub fn with_dialogs(mut self, ui: crate::ui::UiBridge) -> Self {
        self.dialogs = Some(ui);
        self
    }

    /// The extension set.
    #[must_use]
    pub fn set(&self) -> &Arc<ExtensionSet> {
        &self.set
    }

    /// Base call context of this session.
    #[must_use]
    pub fn call_context(&self) -> &CallContext {
        &self.cx
    }

    /// Replaces the base call context (UI binding, mode).
    pub fn set_call_context(&mut self, cx: CallContext) {
        self.cx = cx;
    }

    /// Marks contexts of this runner stale (pi `invalidate`).
    pub fn invalidate(&self, message: Option<&str>) {
        let mut stale = lock(&self.stale);
        if stale.is_none() {
            *stale = Some(message.unwrap_or(STALE_CONTEXT_MESSAGE).to_owned());
        }
    }

    /// The stale message once invalidated.
    #[must_use]
    pub fn stale_message(&self) -> Option<String> {
        lock(&self.stale).clone()
    }

    /// Reports an error as `extension_error`.
    pub fn emit_error(&self, extension_path: &str, event: &str, error: impl Into<String>) {
        self.errors.report(ExtensionErrorEvent {
            extension_path: extension_path.to_owned(),
            event: event.to_owned(),
            error: error.into(),
            stack: None,
        });
    }

    fn active(&self) -> impl Iterator<Item = &Arc<LoadedExtension>> {
        self.set.extensions().iter()
    }

    fn handlers(&self, kind: &str) -> Vec<Arc<LoadedExtension>> {
        self.active()
            .filter(|extension| extension.subscribed(kind))
            .cloned()
            .collect()
    }

    /// pi `hasHandlers`.
    #[must_use]
    pub fn has_handlers(&self, kind: &str) -> bool {
        self.active().any(|extension| extension.subscribed(kind))
    }

    async fn call(
        &self,
        extension: &LoadedExtension,
        kind: &str,
        payload: JsonValue,
    ) -> Result<Option<JsonValue>, ExtensionCallError> {
        let deadline = extension.instance.hook_deadline(kind).map_or_else(
            || deadline_for(kind),
            |d| d.min(MAX_MANIFEST_DEADLINE).max(deadline_for(kind)),
        );
        let call = extension.instance.on_event(kind, payload, self.cx.clone());
        let id = extension.instance.id();
        let waiting = || {
            self.dialogs
                .as_ref()
                .is_some_and(|ui| ui.pending_for(id) > 0)
        };
        if let Some(result) =
            crate::extensions::deadline::with_deadline(call, deadline, waiting).await
        {
            return result;
        }
        if let Some(ui) = &self.dialogs {
            ui.cancel_for(id);
        }
        Err(ExtensionCallError::new(format!(
            "Extension handler for {kind} timed out after {}ms",
            deadline.as_millis()
        )))
    }

    fn report(&self, extension: &LoadedExtension, kind: &str, error: &ExtensionCallError) {
        self.emit_error(&extension.instance.path(), kind, error.message.clone());
    }

    /// Generic dispatch (pi `emit`): notification events, plus the `session_before_*` gates
    /// where the first result with `cancel: true` stops dispatch and the last result wins.
    pub async fn emit(&self, payload: JsonValue) -> Option<JsonValue> {
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        let gate = kind.starts_with("session_before_");
        let mut result = None;
        for extension in self.handlers(&kind) {
            match self.call(&extension, &kind, payload.clone()).await {
                Ok(Some(value)) if gate && !value.is_null() => {
                    let cancel = value.get("cancel") == Some(&JsonValue::Bool(true));
                    result = Some(value);
                    if cancel {
                        return result;
                    }
                }
                Ok(_) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
        }
        result
    }

    /// Consumable dispatch (`oino_editor` intents, ui-protocol §7.2): `true` once a handler
    /// returns `{consume: true}`; later handlers are not called.
    pub async fn emit_consumable(&self, payload: JsonValue) -> bool {
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        for extension in self.handlers(&kind) {
            match self.call(&extension, &kind, payload.clone()).await {
                Ok(Some(value)) if value.get("consume") == Some(&JsonValue::Bool(true)) => {
                    return true;
                }
                Ok(_) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
        }
        false
    }

    /// Chain over one payload field: each handler's returned `{field: value}` replaces the
    /// field for the next handler (`before_provider_headers`, `before_provider_request`).
    /// Returns the final field value.
    pub async fn emit_chain(&self, mut payload: JsonValue, field: &str) -> JsonValue {
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        for extension in self.handlers(&kind) {
            match self.call(&extension, &kind, payload.clone()).await {
                Ok(Some(value)) => {
                    if let Some(next) = value.get(field) {
                        payload[field] = next.clone();
                    } else if !value.is_object() {
                        payload[field] = value;
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
        }
        payload[field].take()
    }

    /// `tool_call` gate: the first `block` result wins; a failing handler blocks with
    /// `Extension error: {message}`. Returned `input` replaces the arguments (Oino).
    pub async fn emit_tool_call(&self, mut payload: JsonValue) -> Option<JsonValue> {
        let mut result: Option<JsonValue> = None;
        for extension in self.handlers("tool_call") {
            match self.call(&extension, "tool_call", payload.clone()).await {
                Ok(Some(value)) if value.is_object() => {
                    if let Some(input) = value.get("input").filter(|v| v.is_object()) {
                        payload["input"] = input.clone();
                    }
                    let block = value.get("block") == Some(&JsonValue::Bool(true));
                    result = Some(value);
                    if block {
                        return result;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.report(&extension, "tool_call", &error);
                    return Some(json!({
                        "block": true,
                        "reason": format!("Extension error: {}", error.message),
                    }));
                }
            }
        }
        if let (Some(result), Some(input)) = (result.as_mut(), payload.get("input"))
            && result.get("input").is_some()
        {
            result["input"] = input.clone();
        }
        result
    }

    /// `tool_result` chain (pi `emitToolResult`): returns the combined override, if any.
    pub async fn emit_tool_result(&self, mut event: JsonObject) -> Option<JsonObject> {
        let mut modified = false;
        for extension in self.handlers("tool_result") {
            let result = self
                .call(&extension, "tool_result", JsonValue::Object(event.clone()))
                .await;
            let update = match result {
                Ok(Some(JsonValue::Object(update))) => update,
                Ok(_) => continue,
                Err(error) => {
                    self.report(&extension, "tool_result", &error);
                    continue;
                }
            };
            if let Some(content) = update.get("content").filter(|v| !v.is_null()) {
                event.insert("content".into(), content.clone());
                if update
                    .get("structuredContent")
                    .is_none_or(JsonValue::is_null)
                {
                    event.shift_remove("structuredContent");
                }
                modified = true;
            }
            for key in ["details", "structuredContent", "isError", "usage"] {
                if let Some(value) = update.get(key).filter(|v| !v.is_null()) {
                    event.insert(key.into(), value.clone());
                    modified = true;
                }
            }
        }
        modified.then(|| {
            [
                "content",
                "details",
                "structuredContent",
                "isError",
                "usage",
            ]
            .into_iter()
            .filter_map(|key| event.get(key).map(|v| (key.to_owned(), v.clone())))
            .collect()
        })
    }

    /// `message_end` chain: handlers may return a replacement with the same role.
    pub async fn emit_message_end(&self, message: &AgentMessage) -> Option<AgentMessage> {
        let mut current = message.clone();
        let mut modified = false;
        for extension in self.handlers("message_end") {
            let payload = json!({"type": "message_end", "message": current});
            match self.call(&extension, "message_end", payload).await {
                Ok(Some(value)) => {
                    let Some(replacement) = value.get("message").filter(|m| !m.is_null()) else {
                        continue;
                    };
                    let mut replacement = replacement.clone();
                    let role = replacement.get("role").and_then(JsonValue::as_str);
                    if role != Some(role_of(&current)) {
                        self.emit_error(
                            &extension.instance.path(),
                            "message_end",
                            "message_end handlers must return a message with the same role",
                        );
                        continue;
                    }
                    if replacement.get("content").is_none_or(JsonValue::is_null)
                        && matches!(role, Some("user" | "assistant" | "toolResult" | "custom"))
                    {
                        replacement["content"] = json!([]);
                    }
                    match serde_json::from_value::<AgentMessage>(replacement) {
                        Ok(parsed) => {
                            current = parsed;
                            modified = true;
                        }
                        Err(error) => {
                            self.emit_error(
                                &extension.instance.path(),
                                "message_end",
                                error.to_string(),
                            );
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "message_end", &error),
            }
        }
        modified.then_some(current)
    }

    /// `input` chain: `transform` replaces text/images for the next handler, `handled` stops.
    pub async fn emit_input(
        &self,
        text: &str,
        images: Option<&[ImageContent]>,
        source: &str,
        streaming_behavior: Option<&str>,
    ) -> InputOutcome {
        let mut current_text = text.to_owned();
        let mut current_images: Option<Vec<ImageContent>> = images.map(<[ImageContent]>::to_vec);
        let original_images = current_images.clone();
        for extension in self.handlers("input") {
            let mut payload = json!({"type": "input", "text": current_text, "source": source});
            if let Some(images) = &current_images {
                payload["images"] = images_json(images);
            }
            if let Some(behavior) = streaming_behavior {
                payload["streamingBehavior"] = json!(behavior);
            }
            match self.call(&extension, "input", payload).await {
                Ok(Some(value)) => match value.get("action").and_then(JsonValue::as_str) {
                    Some("handled") => return InputOutcome::Handled,
                    Some("transform") => {
                        if let Some(text) = value.get("text").and_then(JsonValue::as_str) {
                            text.clone_into(&mut current_text);
                        }
                        if let Some(images) = value
                            .get("images")
                            .filter(|v| !v.is_null())
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                        {
                            current_images = Some(images);
                        }
                    }
                    _ => {}
                },
                Ok(None) => {}
                Err(error) => self.report(&extension, "input", &error),
            }
        }
        if current_text != text || current_images != original_images {
            InputOutcome::Transform {
                text: current_text,
                images: current_images,
            }
        } else {
            InputOutcome::Continue
        }
    }

    /// `context` then `context_with_system` (pi `emitContext`).
    pub async fn emit_context(&self, messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        let mut current = messages;
        for extension in self.handlers("context") {
            let visible: Vec<AgentMessage> = current
                .iter()
                .filter(|m| !matches!(m, AgentMessage::System(_)))
                .cloned()
                .collect();
            let payload = json!({"type": "context", "messages": visible});
            match self.call(&extension, "context", payload).await {
                Ok(Some(value)) => {
                    let Some(returned) = value
                        .get("messages")
                        .filter(|v| !v.is_null())
                        .and_then(|v| serde_json::from_value::<Vec<AgentMessage>>(v.clone()).ok())
                    else {
                        continue;
                    };
                    if returned == visible {
                        continue;
                    }
                    let head = crate::session::current_system_message(&current);
                    current = head
                        .map(AgentMessage::System)
                        .into_iter()
                        .chain(returned)
                        .collect();
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "context", &error),
            }
        }
        for extension in self.handlers("context_with_system") {
            let had_head = matches!(current.first(), Some(AgentMessage::System(_)));
            let payload = json!({"type": "context_with_system", "messages": current});
            match self.call(&extension, "context_with_system", payload).await {
                Ok(Some(value)) => {
                    if let Some(returned) = value
                        .get("messages")
                        .filter(|v| !v.is_null())
                        .and_then(|v| serde_json::from_value::<Vec<AgentMessage>>(v.clone()).ok())
                    {
                        current = returned;
                    }
                    if had_head && !matches!(current.first(), Some(AgentMessage::System(_))) {
                        self.emit_error(&extension.instance.path(), "context_with_system", "Handler removed the leading system message; the request has no prompt or initial tool declarations. Keep it at index 0 or replace a dropped prefix with getCurrentSystemMessage().");
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "context_with_system", &error),
            }
        }
        current
    }

    /// `before_agent_start`: `systemPromptOptions` chain (returned options replace the
    /// current ones), `message`s accumulate, `systemPrompt` last wins. `render` renders the
    /// current options for the event's `systemPrompt` field.
    pub async fn emit_before_agent_start(
        &self,
        trigger: JsonObject,
        options: JsonValue,
        render: &(dyn Fn(&JsonValue) -> String + Send + Sync),
    ) -> BeforeAgentStartOutcome {
        let mut current = options;
        let mut messages = Vec::new();
        let mut forced = None;
        for extension in self.handlers("before_agent_start") {
            let mut payload = trigger.clone();
            payload.insert("type".into(), json!("before_agent_start"));
            payload.insert(
                "systemPrompt".into(),
                json!(forced.clone().unwrap_or_else(|| render(&current))),
            );
            payload.insert("systemPromptOptions".into(), current.clone());
            match self
                .call(&extension, "before_agent_start", JsonValue::Object(payload))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(message) = value.get("message").filter(|m| m.is_object()) {
                        messages.push(message.clone());
                    }
                    if let Some(options) =
                        value.get("systemPromptOptions").filter(|o| o.is_object())
                    {
                        current = options.clone();
                    }
                    if let Some(prompt) = value.get("systemPrompt").and_then(JsonValue::as_str) {
                        forced = Some(prompt.to_owned());
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "before_agent_start", &error),
            }
        }
        BeforeAgentStartOutcome {
            messages,
            system_prompt_options: current,
            system_prompt: forced,
        }
    }

    /// `resources_discover`: every result merged in order.
    pub async fn emit_resources_discover(&self, cwd: &str, reason: &str) -> DiscoveredResources {
        let mut found = DiscoveredResources::default();
        for extension in self.handlers("resources_discover") {
            let payload = json!({"type": "resources_discover", "cwd": cwd, "reason": reason});
            match self.call(&extension, "resources_discover", payload).await {
                Ok(Some(value)) => {
                    let id = extension.instance.id().to_owned();
                    let collect = |key: &str, into: &mut Vec<(String, String)>| {
                        for path in value
                            .get(key)
                            .and_then(JsonValue::as_array)
                            .into_iter()
                            .flatten()
                        {
                            if let Some(path) = path.as_str() {
                                into.push((path.to_owned(), id.clone()));
                            }
                        }
                    };
                    collect("skillPaths", &mut found.skill_paths);
                    collect("promptPaths", &mut found.prompt_paths);
                    collect("themePaths", &mut found.theme_paths);
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "resources_discover", &error),
            }
        }
        found
    }

    /// Boundary dispatch (pi `emitBoundary`). `context` rebuilds the preview after each
    /// handler; an invalid draft list discards everything.
    pub async fn emit_boundary(
        &self,
        base: JsonObject,
        context: &(dyn Fn(&[JsonValue]) -> Result<JsonValue, String> + Send + Sync),
    ) -> BoundaryOutcome {
        let kind = base
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut entries: Vec<JsonValue> = Vec::new();
        let mut continue_run = false;
        let mut preview = context(&entries).unwrap_or(JsonValue::Null);
        let mut valid = true;
        for extension in self.handlers(&kind) {
            let mut payload = base.clone();
            payload.insert("entries".into(), JsonValue::Array(entries.clone()));
            payload.insert("continue".into(), JsonValue::Bool(continue_run));
            payload.insert("context".into(), preview.clone());
            match self
                .call(&extension, &kind, JsonValue::Object(payload))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(list) = value.get("entries").and_then(JsonValue::as_array) {
                        entries.clone_from(list);
                    }
                    if let Some(flag) = value.get("continue").and_then(JsonValue::as_bool) {
                        continue_run = flag;
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
            match context(&entries) {
                Ok(next) => {
                    preview = next;
                    valid = true;
                }
                Err(error) => {
                    valid = false;
                    self.emit_error(
                        &extension.instance.path(),
                        &kind,
                        format!("Invalid boundary entries: {error}"),
                    );
                }
            }
        }
        if valid {
            BoundaryOutcome {
                entries,
                continue_run,
            }
        } else {
            BoundaryOutcome {
                entries: Vec::new(),
                continue_run: false,
            }
        }
    }

    /// `user_bash` gate: the first `{result}` wins; failures block (`Err`). `{operations}`
    /// cannot be supplied by WASM extensions and is treated as no result (Oino delta).
    ///
    /// # Errors
    ///
    /// The failing handler's error text.
    pub async fn emit_user_bash(&self, payload: JsonValue) -> Result<Option<JsonValue>, String> {
        for extension in self.handlers("user_bash") {
            match self.call(&extension, "user_bash", payload.clone()).await {
                Ok(Some(value)) => {
                    if let Some(result) = value.get("result").filter(|r| r.is_object()) {
                        return Ok(Some(result.clone()));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.report(&extension, "user_bash", &error);
                    return Err(error.message);
                }
            }
        }
        Ok(None)
    }

    /// `cache_warming_decision`: the last returned `action` wins.
    pub async fn emit_cache_warming_decision(&self, payload: JsonValue, action: String) -> String {
        let mut action = action;
        for extension in self.handlers("cache_warming_decision") {
            match self
                .call(&extension, "cache_warming_decision", payload.clone())
                .await
            {
                Ok(Some(value)) => {
                    if let Some(next) = value.get("action").and_then(JsonValue::as_str) {
                        next.clone_into(&mut action);
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "cache_warming_decision", &error),
            }
        }
        action
    }

    /// Arbiter wait sources (runtime §7.6): asks every `wait-source` concurrently (2 s each)
    /// and returns the reasons in load order.
    pub async fn arbiter_wait_reasons(&self) -> Vec<String> {
        let sources: Vec<Arc<LoadedExtension>> = self
            .active()
            .filter(|e| e.state().arbiter.wait_source)
            .cloned()
            .collect();
        let calls = sources.iter().map(|extension| {
            let cx = self.cx.clone();
            async move {
                tokio::time::timeout(ARBITER_DEADLINE, extension.instance.wait_reason(cx))
                    .await
                    .ok()
                    .flatten()
            }
        });
        futures::future::join_all(calls)
            .await
            .into_iter()
            .flatten()
            .filter(|reason| !reason.trim().is_empty())
            .collect()
    }

    /// Arbiter nudge (runtime §7.6): asks every `nudge-provider` concurrently (2 s each),
    /// picks the highest `priority` (ties: earliest loaded), tells the winner
    /// `nudge-delivered(id)` and returns its proposal.
    pub async fn arbiter_nudge(&self) -> Option<JsonValue> {
        let providers: Vec<Arc<LoadedExtension>> = self
            .active()
            .filter(|e| e.state().arbiter.nudge_provider)
            .cloned()
            .collect();
        let calls = providers.iter().map(|extension| {
            let cx = self.cx.clone();
            async move {
                tokio::time::timeout(ARBITER_DEADLINE, extension.instance.propose_nudge(cx))
                    .await
                    .ok()
                    .flatten()
                    .filter(|proposal| proposal.get("customType").is_some_and(JsonValue::is_string))
            }
        });
        let proposals = futures::future::join_all(calls).await;
        let priority = |p: &JsonValue| p.get("priority").and_then(JsonValue::as_f64).unwrap_or(0.0);
        let mut winner: Option<(usize, JsonValue)> = None;
        for (index, proposal) in proposals.into_iter().enumerate() {
            let Some(proposal) = proposal else { continue };
            if winner
                .as_ref()
                .is_none_or(|(_, best)| priority(&proposal) > priority(best))
            {
                winner = Some((index, proposal));
            }
        }
        let (index, proposal) = winner?;
        let id = proposal
            .get("id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        providers[index]
            .instance
            .nudge_delivered(&id, self.cx.clone());
        Some(proposal)
    }

    /// Registered tools across extensions; the first registration per name wins.
    #[must_use]
    pub fn tools(&self) -> Vec<RegisteredTool> {
        let mut seen = HashSet::new();
        let mut tools = Vec::new();
        for extension in self.active() {
            for spec in extension.state().tools.into_values() {
                if seen.insert(spec.name.clone()) {
                    tools.push(RegisteredTool {
                        extension: Arc::clone(extension),
                        spec,
                    });
                }
            }
        }
        tools
    }

    /// Commands with invocation names (pi `resolveRegisteredCommands`).
    #[must_use]
    pub fn commands(&self) -> Vec<ResolvedCommand> {
        let mut all = Vec::new();
        for extension in self.active() {
            for spec in extension.state().commands {
                all.push((Arc::clone(extension), spec));
            }
        }
        let mut counts: IndexMap<String, usize> = IndexMap::new();
        for (_, spec) in &all {
            *counts.entry(spec.name.clone()).or_default() += 1;
        }
        let mut seen: IndexMap<String, usize> = IndexMap::new();
        let mut taken = HashSet::new();
        all.into_iter()
            .map(|(extension, spec)| {
                let occurrence = {
                    let slot = seen.entry(spec.name.clone()).or_default();
                    *slot += 1;
                    *slot
                };
                let mut invocation_name = if counts.get(&spec.name).copied().unwrap_or(0) > 1 {
                    format!("{}:{occurrence}", spec.name)
                } else {
                    spec.name.clone()
                };
                let mut suffix = occurrence;
                while taken.contains(&invocation_name) {
                    suffix += 1;
                    invocation_name = format!("{}:{suffix}", spec.name);
                }
                taken.insert(invocation_name.clone());
                ResolvedCommand {
                    extension,
                    spec,
                    invocation_name,
                }
            })
            .collect()
    }

    /// pi `getCommand(invocationName)`.
    #[must_use]
    pub fn command(&self, invocation_name: &str) -> Option<ResolvedCommand> {
        self.commands()
            .into_iter()
            .find(|command| command.invocation_name == invocation_name)
    }

    /// Flags (first registration per name wins).
    #[must_use]
    pub fn flags(&self) -> IndexMap<String, FlagSpec> {
        let mut flags = IndexMap::new();
        for extension in self.active() {
            for (name, flag) in extension.state().flags {
                flags.entry(name).or_insert(flag);
            }
        }
        flags
    }

    /// Flag value: CLI value, else the flag default.
    #[must_use]
    pub fn flag_value(&self, name: &str) -> Option<JsonValue> {
        lock(&self.flag_values)
            .get(name)
            .cloned()
            .or_else(|| self.flags().get(name).and_then(|f| f.default.clone()))
    }

    /// Sets a flag value.
    pub fn set_flag_value(&self, name: &str, value: JsonValue) {
        lock(&self.flag_values).insert(name.to_owned(), value);
    }

    /// Every flag value set on the CLI.
    #[must_use]
    pub fn flag_values(&self) -> IndexMap<String, JsonValue> {
        lock(&self.flag_values).clone()
    }

    /// Shortcuts with their owners (later registrations of the same id win, like pi keys).
    #[must_use]
    pub fn shortcuts(&self) -> Vec<(String, ShortcutSpec)> {
        let mut shortcuts: IndexMap<String, (String, ShortcutSpec)> = IndexMap::new();
        for extension in self.active() {
            for (id, spec) in extension.state().shortcuts {
                shortcuts.insert(id, (extension.instance.id().to_owned(), spec));
            }
        }
        shortcuts.into_values().collect()
    }

    /// Every render spec in load order.
    #[must_use]
    pub fn renderers(&self) -> Vec<(String, JsonValue)> {
        self.active()
            .flat_map(|extension| {
                let id = extension.instance.id().to_owned();
                extension
                    .state()
                    .renderers
                    .into_iter()
                    .map(move |spec| (id.clone(), spec))
            })
            .collect()
    }

    /// The provider component implementing `api` (first in load order).
    #[must_use]
    pub fn provider_for_api(&self, api: &str) -> Option<Arc<LoadedExtension>> {
        self.set
            .extensions()
            .iter()
            .find(|extension| {
                extension.instance.provider().is_some()
                    && extension.state().apis.iter().any(|a| a == api)
            })
            .cloned()
    }
}

/// pi role string of a message.
#[must_use]
pub fn role_of(message: &AgentMessage) -> &'static str {
    match message {
        AgentMessage::System(_) => "system",
        AgentMessage::User(_) => "user",
        AgentMessage::Assistant(_) => "assistant",
        AgentMessage::ToolResult(_) => "toolResult",
        AgentMessage::BashExecution(_) => "bashExecution",
        AgentMessage::Custom(_) => "custom",
        AgentMessage::BranchSummary(_) => "branchSummary",
        AgentMessage::CompactionSummary(_) => "compactionSummary",
    }
}

fn images_json(images: &[ImageContent]) -> JsonValue {
    JsonValue::Array(
        images
            .iter()
            .map(|image| json!({"type": "image", "data": image.data, "mimeType": image.mime_type}))
            .collect(),
    )
}
