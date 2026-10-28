//! [`FrontendApi`]: typed `_oino/*` frontend commands over any [`SessionBackend`].
//!
//! The same calls work in-process ([`crate::InProcessBackend`]) and remotely (`oino-rpc`'s
//! `RpcBackend`), so the TUI and the desktop app share one surface.

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_ai::JsonValue;
use oino_core::extensions::settings::SettingsLayer;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::backend::{ClientError, SessionBackend};
use crate::protocol::{
    AgentInfo, AuthProviderInfo, Checkpoint, CompletionGroup, EditorEvent, ExtensionInfo,
    OinoCommand, QueueEdit, QueueKind, RewindResult, RpcResponseData, RunningBash, SettingsScope,
    TopicValue,
};

/// An extension shortcut (`_oino/get_shortcuts`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutInfo {
    /// Action id.
    pub id: String,
    /// Default key ids (pi `KeyId` syntax).
    #[serde(default)]
    pub default_keys: Vec<String>,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Owning extension.
    #[serde(default)]
    pub extension_id: String,
}

/// What `_oino/rewind` restores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewindOptions {
    /// Restore the files changed at and after the message.
    pub files: bool,
    /// Fork the conversation before the message (pi `fork`).
    pub conversation: bool,
}

impl Default for RewindOptions {
    fn default() -> Self {
        Self {
            files: true,
            conversation: true,
        }
    }
}

/// `_oino/agents` reply.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct AgentTree {
    /// Open agents, parents before children.
    pub agents: Vec<AgentInfo>,
    /// Live `agent_event` forwarding is on.
    pub follow: bool,
}

fn whole<T: DeserializeOwned>(data: RpcResponseData) -> Result<T, ClientError> {
    serde_json::from_value(data.unwrap_or(JsonValue::Null))
        .map_err(|e| ClientError::Protocol(e.to_string()))
}

fn field<T: DeserializeOwned>(data: RpcResponseData, name: &str) -> Result<T, ClientError> {
    let value = data
        .and_then(|mut d| d.get_mut(name).map(JsonValue::take))
        .unwrap_or(JsonValue::Null);
    serde_json::from_value(value).map_err(|e| ClientError::Protocol(format!("{name}: {e}")))
}

/// Typed frontend commands (modes.md §4); implemented for every [`SessionBackend`].
pub trait FrontendApi {
    /// `_oino/get_completions`.
    fn completions(
        &self,
        text: &str,
        cursor: usize,
    ) -> BoxFuture<'_, Result<Vec<CompletionGroup>, ClientError>>;
    /// `_oino/ui_action`.
    fn ui_action(
        &self,
        panel_id: &str,
        action_id: &str,
        value: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/editor_event` → whether an extension consumed it.
    fn editor_event(&self, event: EditorEvent) -> BoxFuture<'_, Result<bool, ClientError>>;
    /// `_oino/invoke_shortcut`.
    fn invoke_shortcut(&self, id: &str) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/get_shortcuts`.
    fn shortcuts(&self) -> BoxFuture<'_, Result<Vec<ShortcutInfo>, ClientError>>;
    /// `_oino/get_settings_schema` (ui-protocol §6.6 namespaces).
    fn settings_schema(&self) -> BoxFuture<'_, Result<Vec<JsonValue>, ClientError>>;
    /// `_oino/get_extension_settings`.
    fn extension_settings(
        &self,
        namespace: &str,
        layer: SettingsLayer,
    ) -> BoxFuture<'_, Result<JsonValue, ClientError>>;
    /// `_oino/set_extension_setting` (`None` deletes).
    fn set_extension_setting(
        &self,
        namespace: &str,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/get_settings` (merged core settings).
    fn settings(&self) -> BoxFuture<'_, Result<JsonValue, ClientError>>;
    /// `_oino/set_setting` (`None` deletes).
    fn set_setting(
        &self,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/service_call`.
    fn service_call(
        &self,
        target: Option<&str>,
        name: &str,
        payload: JsonValue,
    ) -> BoxFuture<'_, Result<JsonValue, ClientError>>;
    /// `_oino/auth_providers`.
    fn auth_providers(&self) -> BoxFuture<'_, Result<Vec<AuthProviderInfo>, ClientError>>;
    /// `_oino/login` (runs the OAuth flow; resolves when it finished).
    fn login(&self, provider: &str) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/logout`.
    fn logout(&self, provider: &str) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/set_api_key`.
    fn set_api_key(&self, provider: &str, key: &str) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/topics`: changes the subscription; returns the sticky values it now matches.
    fn topics(
        &self,
        subscribe: &[&str],
        unsubscribe: &[&str],
    ) -> BoxFuture<'_, Result<Vec<TopicValue>, ClientError>>;
    /// `_oino/bus_get`: last sticky value of `topic`.
    fn bus_get(&self, topic: &str) -> BoxFuture<'_, Result<Option<JsonValue>, ClientError>>;
    /// `_oino/agents`: the nested-agent tree; `follow` toggles `agent_event` forwarding.
    fn agents(&self, follow: Option<bool>) -> BoxFuture<'_, Result<AgentTree, ClientError>>;
    /// `_oino/agent_messages`: transcript of one nested agent.
    fn agent_messages(
        &self,
        agent_id: &str,
    ) -> BoxFuture<'_, Result<Vec<oino_ai::AgentMessage>, ClientError>>;
    /// `_oino/agent_abort`.
    fn agent_abort(&self, agent_id: &str) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/queue_edit`.
    fn queue_edit(&self, edit: QueueEdit) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/queue_edit{op: remove}`.
    fn queue_remove(
        &self,
        queue: QueueKind,
        index: usize,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/queue_edit{op: sendNow}`.
    fn queue_send_now(
        &self,
        queue: QueueKind,
        index: usize,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/queue_edit{op: move}`.
    fn queue_move(
        &self,
        queue: QueueKind,
        from: usize,
        to: usize,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/queue_edit{op: edit}`.
    fn queue_set_text(
        &self,
        queue: QueueKind,
        index: usize,
        text: &str,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/checkpoints`.
    fn checkpoints(&self) -> BoxFuture<'_, Result<Vec<Checkpoint>, ClientError>>;
    /// `_oino/rewind`.
    fn rewind(
        &self,
        entry_id: &str,
        options: RewindOptions,
    ) -> BoxFuture<'_, Result<RewindResult, ClientError>>;
    /// `_oino/running_bash`.
    fn running_bash(&self) -> BoxFuture<'_, Result<Vec<RunningBash>, ClientError>>;
    /// `_oino/extensions`.
    fn extensions(&self) -> BoxFuture<'_, Result<Vec<ExtensionInfo>, ClientError>>;
    /// `_oino/set_extension_enabled`.
    fn set_extension_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> BoxFuture<'_, Result<(), ClientError>>;
    /// `_oino/render`: the OUS node an extension renders for one transcript row, or `None`
    /// (ui-protocol §6.4; built-in tool rows included).
    fn render(&self, request: RenderRow) -> BoxFuture<'_, Result<Option<JsonValue>, ClientError>>;
}

/// `_oino/render` request as frontends build it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRow {
    /// `customMessage` | `customEntry` | `tool`.
    pub target: String,
    /// Custom type or tool name.
    pub key: String,
    /// Entry or tool-call id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// `simple` | `regular` | `advanced`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<String>,
    /// Expanded row.
    #[serde(default)]
    pub expanded: bool,
    /// `narrow` | `normal` | `wide`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    /// Row payload.
    #[serde(default)]
    pub data: JsonValue,
}

impl<B: SessionBackend + ?Sized> FrontendApi for B {
    fn completions(
        &self,
        text: &str,
        cursor: usize,
    ) -> BoxFuture<'_, Result<Vec<CompletionGroup>, ClientError>> {
        let command = OinoCommand::GetCompletions {
            text: text.to_owned(),
            cursor,
        };
        self.send(command.into())
            .map(|r| field(r?, "groups"))
            .boxed()
    }

    fn ui_action(
        &self,
        panel_id: &str,
        action_id: &str,
        value: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::UiAction {
            panel_id: panel_id.to_owned(),
            action_id: action_id.to_owned(),
            value,
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn editor_event(&self, event: EditorEvent) -> BoxFuture<'_, Result<bool, ClientError>> {
        self.send(OinoCommand::EditorEvent(event).into())
            .map(|r| field(r?, "consumed"))
            .boxed()
    }

    fn invoke_shortcut(&self, id: &str) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::InvokeShortcut {
            shortcut_id: id.to_owned(),
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn shortcuts(&self) -> BoxFuture<'_, Result<Vec<ShortcutInfo>, ClientError>> {
        self.send(OinoCommand::GetShortcuts.into())
            .map(|r| field(r?, "shortcuts"))
            .boxed()
    }

    fn settings_schema(&self) -> BoxFuture<'_, Result<Vec<JsonValue>, ClientError>> {
        self.send(OinoCommand::GetSettingsSchema.into())
            .map(|r| field(r?, "namespaces"))
            .boxed()
    }

    fn extension_settings(
        &self,
        namespace: &str,
        layer: SettingsLayer,
    ) -> BoxFuture<'_, Result<JsonValue, ClientError>> {
        let command = OinoCommand::GetExtensionSettings {
            namespace: namespace.to_owned(),
            scope: layer,
        };
        self.send(command.into())
            .map(|r| field(r?, "values"))
            .boxed()
    }

    fn set_extension_setting(
        &self,
        namespace: &str,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = match value {
            Some(value) => OinoCommand::SetExtensionSetting {
                namespace: namespace.to_owned(),
                key: key.to_owned(),
                value: Some(value),
                scope,
            },
            None => OinoCommand::UnsetExtensionSetting {
                namespace: namespace.to_owned(),
                key: key.to_owned(),
                scope,
            },
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn settings(&self) -> BoxFuture<'_, Result<JsonValue, ClientError>> {
        self.send(OinoCommand::GetSettings { key: None }.into())
            .map(|r| field(r?, "settings"))
            .boxed()
    }

    fn set_setting(
        &self,
        key: &str,
        value: Option<JsonValue>,
        scope: SettingsScope,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::SetSetting {
            key: key.to_owned(),
            value,
            scope,
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn service_call(
        &self,
        target: Option<&str>,
        name: &str,
        payload: JsonValue,
    ) -> BoxFuture<'_, Result<JsonValue, ClientError>> {
        let command = OinoCommand::ServiceCall {
            target: target.map(str::to_owned),
            name: name.to_owned(),
            payload,
        };
        self.send(command.into())
            .map(|r| r.map(Option::unwrap_or_default))
            .boxed()
    }

    fn auth_providers(&self) -> BoxFuture<'_, Result<Vec<AuthProviderInfo>, ClientError>> {
        self.send(OinoCommand::AuthProviders.into())
            .map(|r| field(r?, "providers"))
            .boxed()
    }

    fn login(&self, provider: &str) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::Login {
            provider: provider.to_owned(),
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn logout(&self, provider: &str) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::Logout {
            provider: provider.to_owned(),
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn set_api_key(&self, provider: &str, key: &str) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::SetApiKey {
            provider: provider.to_owned(),
            key: key.to_owned(),
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn topics(
        &self,
        subscribe: &[&str],
        unsubscribe: &[&str],
    ) -> BoxFuture<'_, Result<Vec<TopicValue>, ClientError>> {
        let command = OinoCommand::Topics {
            subscribe: subscribe.iter().map(|s| (*s).to_owned()).collect(),
            unsubscribe: unsubscribe.iter().map(|s| (*s).to_owned()).collect(),
        };
        self.send(command.into())
            .map(|r| field(r?, "topics"))
            .boxed()
    }

    fn bus_get(&self, topic: &str) -> BoxFuture<'_, Result<Option<JsonValue>, ClientError>> {
        let command = OinoCommand::BusGet {
            topic: topic.to_owned(),
        };
        self.send(command.into())
            .map(|r| field::<JsonValue>(r?, "data").map(|v| (!v.is_null()).then_some(v)))
            .boxed()
    }

    fn agents(&self, follow: Option<bool>) -> BoxFuture<'_, Result<AgentTree, ClientError>> {
        self.send(OinoCommand::Agents { follow }.into())
            .map(|r| whole(r?))
            .boxed()
    }

    fn agent_messages(
        &self,
        agent_id: &str,
    ) -> BoxFuture<'_, Result<Vec<oino_ai::AgentMessage>, ClientError>> {
        let command = OinoCommand::AgentMessages {
            agent_id: agent_id.to_owned(),
        };
        self.send(command.into())
            .map(|r| field(r?, "messages"))
            .boxed()
    }

    fn agent_abort(&self, agent_id: &str) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::AgentAbort {
            agent_id: agent_id.to_owned(),
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }

    fn queue_edit(&self, edit: QueueEdit) -> BoxFuture<'_, Result<(), ClientError>> {
        self.send(OinoCommand::QueueEdit(edit).into())
            .map(|r| r.map(drop))
            .boxed()
    }

    fn queue_remove(
        &self,
        queue: QueueKind,
        index: usize,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        self.queue_edit(QueueEdit::Remove { queue, index })
    }

    fn queue_send_now(
        &self,
        queue: QueueKind,
        index: usize,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        self.queue_edit(QueueEdit::SendNow { queue, index })
    }

    fn queue_move(
        &self,
        queue: QueueKind,
        from: usize,
        to: usize,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        self.queue_edit(QueueEdit::Move { queue, from, to })
    }

    fn queue_set_text(
        &self,
        queue: QueueKind,
        index: usize,
        text: &str,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        self.queue_edit(QueueEdit::Edit {
            queue,
            index,
            text: text.to_owned(),
        })
    }

    fn checkpoints(&self) -> BoxFuture<'_, Result<Vec<Checkpoint>, ClientError>> {
        self.send(OinoCommand::Checkpoints.into())
            .map(|r| field(r?, "checkpoints"))
            .boxed()
    }

    fn rewind(
        &self,
        entry_id: &str,
        options: RewindOptions,
    ) -> BoxFuture<'_, Result<RewindResult, ClientError>> {
        let command = OinoCommand::Rewind {
            entry_id: entry_id.to_owned(),
            files: options.files,
            conversation: options.conversation,
        };
        self.send(command.into()).map(|r| whole(r?)).boxed()
    }

    fn running_bash(&self) -> BoxFuture<'_, Result<Vec<RunningBash>, ClientError>> {
        self.send(OinoCommand::RunningBash.into())
            .map(|r| field(r?, "calls"))
            .boxed()
    }

    fn extensions(&self) -> BoxFuture<'_, Result<Vec<ExtensionInfo>, ClientError>> {
        self.send(OinoCommand::Extensions.into())
            .map(|r| field(r?, "extensions"))
            .boxed()
    }

    fn render(&self, request: RenderRow) -> BoxFuture<'_, Result<Option<JsonValue>, ClientError>> {
        let command = OinoCommand::Render {
            target: request.target,
            key: request.key,
            entry_id: request.entry_id,
            density: request.density,
            expanded: request.expanded,
            width: request.width,
            data: request.data,
        };
        self.send(command.into())
            .map(|r| field::<JsonValue>(r?, "node").map(|v| (!v.is_null()).then_some(v)))
            .boxed()
    }

    fn set_extension_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> BoxFuture<'_, Result<(), ClientError>> {
        let command = OinoCommand::SetExtensionEnabled {
            id: id.to_owned(),
            enabled,
        };
        self.send(command.into()).map(|r| r.map(drop)).boxed()
    }
}
