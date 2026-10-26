//! [`FrontendApi`]: typed `_oino/*` frontend commands over any [`SessionBackend`].
//!
//! The same calls work in-process ([`crate::InProcessBackend`]) and remotely (`oino-rpc`'s
//! `RpcBackend`), so the TUI and the desktop app share one surface.

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_ai::JsonValue;
use oino_core::extensions::settings::SettingsLayer;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::backend::{ClientError, SessionBackend};
use crate::protocol::{
    AuthProviderInfo, CompletionGroup, EditorEvent, OinoCommand, RpcResponseData, SettingsScope,
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
}
