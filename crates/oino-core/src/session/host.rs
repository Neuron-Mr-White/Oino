//! Host operations performed by extensions against this session (pi `ExtensionActions`,
//! `ExtensionContextActions`, `ExtensionCommandContextActions`, UI context).

use oino_ai::{JsonValue, ThinkingLevel};
use oino_session::EntryId;
use serde_json::json;

use super::{AgentSession, lock};
use crate::events::OinoEvent;
use crate::extensions::api::{CallContext, CallKind};
use crate::extensions::host::{HostError, HostOp, SessionQuery};
use crate::ui::{UiAnswer, is_dialog, prompt_kind};

fn to_json<T: serde::Serialize>(value: T) -> Result<JsonValue, HostError> {
    serde_json::to_value(value).map_err(|e| HostError::Failed(e.to_string()))
}

fn failed(error: impl std::fmt::Display) -> HostError {
    HostError::Failed(error.to_string())
}

impl AgentSession {
    /// Performs one host operation for `extension` (runtime §7.3 context rules apply).
    ///
    /// # Errors
    ///
    /// [`HostError::StaleContext`] after replacement, [`HostError::CommandContextRequired`] for
    /// session-replacing operations outside commands, and operation failures.
    #[allow(clippy::too_many_lines, reason = "one arm per host operation")]
    pub async fn host_call(
        &self,
        extension: &str,
        cx: &CallContext,
        op: HostOp,
    ) -> Result<JsonValue, HostError> {
        let runner = self.runner();
        if let Some(message) = runner.stale_message() {
            return Err(HostError::StaleContext(message));
        }
        let command_context = matches!(cx.call_kind, CallKind::Command | CallKind::Shortcut);
        let loaded = runner.set().get(extension).cloned();
        let registry_changed = matches!(
            op,
            HostOp::RegisterTool(_)
                | HostOp::UnregisterTool(_)
                | HostOp::Subscribe(_)
                | HostOp::Unsubscribe(_)
        );
        let commands_changed = matches!(
            op,
            HostOp::RegisterCommand(_) | HostOp::UnregisterCommand(_)
        );
        let result = match op {
            HostOp::Subscribe(kinds) => {
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                loaded.update(|state| state.events.extend(kinds));
                Ok(JsonValue::Null)
            }
            HostOp::Unsubscribe(kinds) => {
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                loaded.update(|state| state.events.retain(|k| !kinds.contains(k)));
                Ok(JsonValue::Null)
            }
            HostOp::RegisterTool(spec) => {
                let owner = runner
                    .tools()
                    .into_iter()
                    .find(|t| t.spec.name == spec.name)
                    .map(|t| t.extension.instance.id().to_owned());
                let builtin = self
                    .all_tools()
                    .iter()
                    .any(|t| t.name == spec.name && t.source_info.source == "builtin");
                if owner.as_deref().is_some_and(|o| o != extension) || builtin {
                    return Err(HostError::Conflict(format!(
                        "Tool \"{}\" is already registered",
                        spec.name
                    )));
                }
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                loaded.update(|state| state.tools.insert(spec.name.clone(), spec));
                Ok(JsonValue::Null)
            }
            HostOp::UnregisterTool(name) => {
                if let Some(loaded) = loaded {
                    loaded.update(|state| state.tools.shift_remove(&name));
                }
                Ok(JsonValue::Null)
            }
            HostOp::RegisterCommand(spec) => {
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                let name = spec.name.clone();
                loaded.update(|state| {
                    state.commands.retain(|c| c.name != spec.name);
                    state.commands.push(spec);
                });
                let invocation = runner
                    .commands()
                    .into_iter()
                    .find(|c| c.extension.instance.id() == extension && c.spec.name == name)
                    .map_or(name, |c| c.invocation_name);
                Ok(json!(invocation))
            }
            HostOp::UnregisterCommand(name) => {
                if let Some(loaded) = loaded {
                    loaded.update(|state| state.commands.retain(|c| c.name != name));
                }
                Ok(JsonValue::Null)
            }
            HostOp::RegisterShortcut(spec) => {
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                loaded.update(|state| state.shortcuts.insert(spec.id.clone(), spec));
                Ok(JsonValue::Null)
            }
            HostOp::UnregisterShortcut(id) => {
                if let Some(loaded) = loaded {
                    loaded.update(|state| state.shortcuts.shift_remove(&id));
                }
                Ok(JsonValue::Null)
            }
            HostOp::RegisterFlag(spec) => {
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                loaded.update(|state| state.flags.insert(spec.name.clone(), spec));
                Ok(JsonValue::Null)
            }
            HostOp::GetFlag(name) => Ok(runner.flag_value(&name).unwrap_or(JsonValue::Null)),
            HostOp::RegisterRenderer(spec) => {
                let loaded = loaded
                    .ok_or_else(|| HostError::Invalid(format!("Unknown extension {extension}")))?;
                loaded.update(|state| state.renderers.push(spec));
                Ok(JsonValue::Null)
            }
            HostOp::RegisterProvider(config) => {
                let registration: oino_models::ProviderRegistration =
                    serde_json::from_value(config)
                        .map_err(|e| HostError::Invalid(e.to_string()))?;
                self.inner
                    .models
                    .register_provider(extension, registration)
                    .map_err(failed)?;
                self.refresh_current_model();
                Ok(JsonValue::Null)
            }
            HostOp::UnregisterProvider(id) => {
                self.inner.models.unregister_provider(&id);
                self.refresh_current_model();
                Ok(JsonValue::Null)
            }
            HostOp::RegisterVirtualModel(definition) => {
                let definition = serde_json::from_value(definition)
                    .map_err(|e| HostError::Invalid(e.to_string()))?;
                self.inner
                    .models
                    .register_virtual_model(extension, definition)
                    .map_err(failed)?;
                self.refresh_current_model();
                Ok(JsonValue::Null)
            }
            HostOp::UnregisterVirtualModel { provider, id } => {
                self.inner.models.unregister_virtual_model(&provider, &id);
                self.refresh_current_model();
                Ok(JsonValue::Null)
            }
            HostOp::SessionRead(query) => self.session_read(&query),
            HostOp::AppendEntry { custom_type, data } => {
                let id = lock(&self.inner.manager)
                    .append_custom_entry(&custom_type, data)
                    .map_err(failed)?;
                self.publish_entry(&id);
                Ok(json!(id.as_str()))
            }
            HostOp::SendMessage {
                message,
                trigger_turn,
                deliver_as,
            } => {
                let session = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = session
                        .send_custom_message(message, trigger_turn, deliver_as)
                        .await
                    {
                        session
                            .runner()
                            .emit_error("<runtime>", "send_message", error.to_string());
                    }
                });
                Ok(JsonValue::Null)
            }
            HostOp::SendUserMessage {
                content,
                deliver_as,
            } => {
                let session = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = session.send_user_message(content, deliver_as, false).await
                    {
                        session.runner().emit_error(
                            "<runtime>",
                            "send_user_message",
                            error.to_string(),
                        );
                    }
                });
                Ok(JsonValue::Null)
            }
            HostOp::SetSessionName(name) => {
                self.set_session_name(Some(name)).await.map_err(failed)?;
                Ok(JsonValue::Null)
            }
            HostOp::SetLabel { entry_id, label } => {
                self.set_label(&EntryId::new(entry_id), label)
                    .await
                    .map_err(failed)?;
                Ok(JsonValue::Null)
            }
            HostOp::GetActiveTools => to_json(self.active_tool_names()),
            HostOp::GetAllTools => to_json(self.all_tools()),
            HostOp::SetActiveTools(names) => {
                self.set_active_tools_by_name(&names);
                Ok(JsonValue::Null)
            }
            HostOp::GetCommands => to_json(self.commands()),
            HostOp::SetModel { provider, id } => {
                let Some(model) = self.inner.models.find(&provider, &id) else {
                    return Ok(json!(false));
                };
                if !self.inner.models.has_configured_auth(&model.provider) {
                    return Ok(json!(false));
                }
                self.set_model(model, false).await.map_err(failed)?;
                Ok(json!(true))
            }
            HostOp::GetModel => to_json(self.model()),
            HostOp::GetThinkingLevel => to_json(self.thinking_level()),
            HostOp::SetThinkingLevel(level) => {
                let level: ThinkingLevel = level;
                to_json(self.set_thinking_level(level, false).await)
            }
            HostOp::IsIdle => Ok(json!(self.is_idle())),
            HostOp::Abort => {
                let session = self.clone();
                tokio::spawn(async move { session.abort().await });
                Ok(JsonValue::Null)
            }
            HostOp::HasPendingMessages => Ok(json!(self.pending_message_count() > 0)),
            HostOp::GetContextUsage => to_json(self.context_usage()),
            HostOp::Compact {
                custom_instructions,
            } => {
                let session = self.clone();
                tokio::spawn(async move {
                    if let Err(error) = session.compact(custom_instructions).await {
                        tracing::debug!("extension compaction failed: {error}");
                    }
                });
                Ok(JsonValue::Null)
            }
            HostOp::GetSystemPrompt => Ok(json!(self.system_prompt())),
            HostOp::WaitForIdle => {
                if !command_context {
                    return Err(HostError::CommandContextRequired);
                }
                self.wait_for_idle().await;
                Ok(JsonValue::Null)
            }
            HostOp::NavigateTree {
                target_id,
                summarize,
                custom_instructions,
                replace_instructions,
                label,
            } => {
                if !command_context {
                    return Err(HostError::CommandContextRequired);
                }
                let result = self
                    .navigate_tree(
                        &EntryId::new(target_id),
                        super::NavigateTreeOptions {
                            summarize,
                            custom_instructions,
                            replace_instructions,
                            label,
                        },
                    )
                    .await
                    .map_err(failed)?;
                Ok(json!({"cancelled": result.cancelled}))
            }
            HostOp::UiDialog {
                method,
                params,
                cancel,
            } => {
                if !is_dialog(&method) {
                    return Err(HostError::Invalid(format!("{method} is not a dialog")));
                }
                Ok(self.ui_dialog(extension, &method, params, cancel).await)
            }
            HostOp::UiFire { method, params } => {
                self.inner.ui.fire(Some(extension), &method, params);
                Ok(JsonValue::Null)
            }
            HostOp::InvokeCommand { name, args } => {
                self.invoke_command(&name, &args, command_context).await
            }
        };
        if registry_changed {
            self.refresh_tool_registry(None, false);
        }
        if commands_changed {
            self.inner.hub.publish(OinoEvent::CommandsChanged);
        }
        result
    }

    async fn ui_dialog(
        &self,
        extension: &str,
        method: &str,
        params: oino_ai::JsonObject,
        cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> JsonValue {
        let kind = prompt_kind(method);
        let title = params
            .get("title")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let runner = self.runner();
        let prompt_event = |kind_name: &str| {
            let mut payload = json!({"type": kind_name, "reason": "ui_prompt", "kind": kind});
            if let Some(title) = &title {
                payload["title"] = json!(title);
            }
            payload
        };
        if runner.has_handlers("ui_prompt_start") {
            runner.emit(prompt_event("ui_prompt_start")).await;
        }
        let answer = self
            .inner
            .ui
            .dialog(Some(extension), method, params, cancel)
            .await;
        if runner.has_handlers("ui_prompt_end") {
            runner.emit(prompt_event("ui_prompt_end")).await;
        }
        match answer {
            UiAnswer::Value(value) => json!({"value": value}),
            UiAnswer::Confirmed(confirmed) => json!({"confirmed": confirmed}),
            UiAnswer::Cancelled => json!({"cancelled": true}),
        }
    }

    async fn invoke_command(
        &self,
        name: &str,
        args: &str,
        inline: bool,
    ) -> Result<JsonValue, HostError> {
        let runner = self.runner();
        let command = runner
            .command(name)
            .ok_or_else(|| HostError::Invalid(format!("Unknown command {name}")))?;
        if !inline {
            let session = self.clone();
            let text = if args.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {args}")
            };
            tokio::spawn(async move {
                session.wait_for_idle().await;
                let _ = session.prompt(text, super::PromptOptions::default()).await;
            });
            return Ok(JsonValue::Null);
        }
        let cx = runner.call_context().with_kind(CallKind::Command);
        command
            .extension
            .instance
            .run_command(
                &command.spec.name,
                args,
                tokio_util::sync::CancellationToken::new(),
                cx,
            )
            .await
            .map(|value| value.unwrap_or(JsonValue::Null))
            .map_err(|e| HostError::Failed(e.message))
    }

    fn session_read(&self, query: &SessionQuery) -> Result<JsonValue, HostError> {
        let manager = lock(&self.inner.manager);
        match query {
            SessionQuery::Header => to_json(manager.header()),
            SessionQuery::Entries => to_json(manager.entries()),
            SessionQuery::Branch => to_json(manager.branch(None)),
            SessionQuery::Entry { id } => to_json(manager.entry(&EntryId::new(id.clone()))),
            SessionQuery::Tree => to_json(manager.tree()),
            SessionQuery::Projection => to_json(manager.build_session_projection()),
            SessionQuery::ContextEntries => to_json(manager.build_context_entries()),
            SessionQuery::Label { id } => to_json(manager.label(&EntryId::new(id.clone()))),
            SessionQuery::Name => to_json(manager.session_name()),
            SessionQuery::Leaf => to_json(manager.leaf_id().map(EntryId::as_str)),
            SessionQuery::Id => to_json(manager.session_id()),
            SessionQuery::File => to_json(manager.session_file()),
        }
    }

    /// pi `_refreshCurrentModelFromRegistry`.
    pub(crate) fn refresh_current_model(&self) {
        let Some(current) = self.model() else {
            return;
        };
        if let Some(refreshed) = self.inner.models.find(&current.provider, &current.id)
            && refreshed != current
        {
            self.inner.agent.set_model(refreshed);
        }
    }

    /// Slash commands: extension commands, prompt templates, skills (pi `getCommands`).
    #[must_use]
    pub fn commands(&self) -> Vec<SlashCommandInfo> {
        let mut commands: Vec<SlashCommandInfo> = self
            .runner()
            .commands()
            .into_iter()
            .filter(|c| !c.spec.hidden)
            .map(|c| SlashCommandInfo {
                name: c.invocation_name,
                description: c.spec.description,
                source: "extension".to_owned(),
                source_info: json!({"path": c.extension.instance.path(), "source": format!("extension:{}", c.extension.instance.id())}),
            })
            .collect();
        let resources = self
            .inner
            .resources
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        commands.extend(resources.prompts().iter().map(|t| SlashCommandInfo {
            name: t.name.clone(),
            description: Some(t.description.clone()).filter(|d| !d.is_empty()),
            source: "prompt".to_owned(),
            source_info: serde_json::to_value(&t.source_info).unwrap_or(JsonValue::Null),
        }));
        commands.extend(resources.skills().iter().map(|s| SlashCommandInfo {
            name: format!("skill:{}", s.name),
            description: Some(s.description.clone()),
            source: "skill".to_owned(),
            source_info: serde_json::to_value(&s.source_info).unwrap_or(JsonValue::Null),
        }));
        commands
    }
}

/// pi `SlashCommandInfo`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandInfo {
    /// Invocation name.
    pub name: String,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `extension` | `prompt` | `skill`.
    pub source: String,
    /// Provenance.
    pub source_info: JsonValue,
}
