//! Hub-level `_oino/*` commands (app spec §3.14, WP-A24): the settings / auth /
//! extension subset of the session command set, run against core services
//! **without a session runtime**, so the app's one Settings screen can read and
//! write pi agent settings, extension settings and the extension list before
//! any chat exists.
//!
//! `host_cmd{cmd}` arrives on a normal op stream (see `sessions/stream.rs`);
//! the services are the hub's primary-cwd set (cached and idle-evicted like a
//! session's), so a command may spin extensions up once and reuse them. Reply
//! shapes match the session commands exactly (`{settings}`, `{value}`,
//! `{namespaces}`, `{extensions}`, `{providers}`, `{shortcuts}`), so one app
//! wrapper serves both transports.
//!
//! `login` is **not** served here: the OAuth flow raises dialogs on a session's
//! frontend, which a hub-level command has none of (the app opens it from a
//! chat, WP-A25). `set_api_key` and `logout` are pure credential-store writes
//! and work fine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{json, Value};
use unipi_proto::settings::HostCmd;
use unipi_proto::{AppErrorCode, AppErrorDto};

use oino_core::extensions::settings::SettingsLayer;
use oino_core::services::Services;
use oino_settings::SettingsScope;

use crate::sessions::hub::SessionHub;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn failed(message: impl std::fmt::Display) -> AppErrorDto {
    // The session commands surface command failures as plain texts; the wire
    // code for a hub-level command failure is `E_INTERNAL` with the core text.
    AppErrorDto::new(AppErrorCode::E_INTERNAL, message.to_string(), false)
}

fn invalid(message: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message.to_string(), false)
}

fn unsupported(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_UNSUPPORTED, message, false)
}

/// `scope`/`layer` strings → their enums (`global` | `project` / `effective` |
/// `global` | `project`), matching the session commands' serde names.
fn scope_of(raw: &str) -> Result<SettingsScope, AppErrorDto> {
    match raw {
        "global" => Ok(SettingsScope::Global),
        "project" => Ok(SettingsScope::Project),
        other => Err(invalid(format!(
            "unknown scope `{other}` (global | project)"
        ))),
    }
}

fn layer_of(raw: Option<&str>) -> Result<SettingsLayer, AppErrorDto> {
    match raw.unwrap_or("effective") {
        "effective" => Ok(SettingsLayer::Effective),
        "global" => Ok(SettingsLayer::Global),
        "project" => Ok(SettingsLayer::Project),
        other => Err(invalid(format!(
            "unknown layer `{other}` (effective | global | project)"
        ))),
    }
}

/// Runs one hub-level command. `data` is the command's reply payload.
///
/// # Errors
///
/// `E_SCHEMA` for unknown keys/namespaces/scopes, `E_FAILED` for write and
/// credential failures, `E_UNSUPPORTED` for `login` (needs a session frontend).
pub async fn run(hub: &Arc<SessionHub>, cmd: HostCmd) -> Result<Value, AppErrorDto> {
    tracing::info!(op = "host_cmd", kind = cmd.kind(), "request");
    let services = hub
        .core_services()
        .await
        .map_err(|error| failed(error.to_string()))?;
    dispatch(&services, cmd).await
}

async fn dispatch(services: &Arc<Services>, cmd: HostCmd) -> Result<Value, AppErrorDto> {
    let registry = &services.extension_settings;
    match cmd {
        HostCmd::GetSettingsSchema => Ok(json!({ "namespaces": registry.schema() })),
        HostCmd::GetExtensionSettings { namespace, layer } => {
            if registry.namespace(&namespace).is_none() {
                return Err(failed(format!(
                    "Unknown settings namespace: {namespace}"
                )));
            }
            let resolved = registry.resolved(&namespace, layer_of(layer.as_deref())?);
            Ok(json!({
                "values": resolved.values,
                "diagnostics": resolved.diagnostics,
            }))
        }
        HostCmd::SetExtensionSetting {
            namespace,
            key,
            value,
            scope,
        } => {
            let value = value.filter(|value| !value.is_null());
            registry
                .set(&namespace, &key, value, scope_of(&scope)?)
                .map_err(|error| match error {
                    oino_core::extensions::settings::ExtensionSettingsError::Settings(_)
                    | oino_core::extensions::settings::ExtensionSettingsError::Secrets(_) => {
                        failed(error)
                    }
                    other => invalid(other),
                })?;
            Ok(Value::Null)
        }
        HostCmd::UnsetExtensionSetting {
            namespace,
            key,
            scope,
        } => {
            registry
                .set(&namespace, &key, None, scope_of(&scope)?)
                .map_err(|error| match error {
                    oino_core::extensions::settings::ExtensionSettingsError::Settings(_)
                    | oino_core::extensions::settings::ExtensionSettingsError::Secrets(_) => {
                        failed(error)
                    }
                    other => invalid(other),
                })?;
            Ok(Value::Null)
        }
        HostCmd::GetSettings { key } => {
            let merged = registry.merged_settings();
            match key {
                None => Ok(json!({ "settings": merged })),
                Some(key) => {
                    let path: Vec<&str> = key.split('.').collect();
                    let value = oino_settings::json::get_path(&merged, &path).cloned();
                    Ok(json!({ "value": value }))
                }
            }
        }
        HostCmd::SetSetting { key, value, scope } => {
            let path: Vec<&str> = key.split('.').filter(|part| !part.is_empty()).collect();
            if path.is_empty() {
                return Err(invalid("Setting key cannot be empty"));
            }
            let value = value.filter(|value| !value.is_null());
            let mut settings = lock(&services.settings);
            settings
                .set(scope_of(&scope)?, &path, value)
                .map_err(failed)?;
            Ok(Value::Null)
        }
        HostCmd::Extensions => Ok(json!({ "extensions": extension_infos(services) })),
        HostCmd::SetExtensionEnabled { id, enabled } => {
            let mut settings = lock(&services.settings);
            let mut disabled = settings.settings().disabled_extensions();
            disabled.retain(|entry| entry != &id);
            if !enabled {
                disabled.push(id);
            }
            settings
                .set_disabled_extensions(&disabled)
                .map_err(failed)?;
            Ok(Value::Null)
        }
        HostCmd::AuthProviders => Ok(json!({ "providers": auth_providers(services) })),
        HostCmd::Login { .. } => Err(unsupported(
            "OAuth sign-in opens from a chat session (its dialogs need a frontend); set an API key instead",
        )),
        HostCmd::Logout { provider } => {
            services
                .models
                .auth_storage()
                .remove(&provider)
                .map_err(failed)?;
            Ok(Value::Null)
        }
        HostCmd::SetApiKey { provider, key } => {
            let key = key.trim();
            if key.is_empty() {
                return Err(failed("API key cannot be empty"));
            }
            let credential =
                oino_models::auth::credential::AuthCredential::ApiKey(
                    oino_models::auth::credential::ApiKeyCredential {
                        key: Some(key.to_owned()),
                        env: None,
                        extra: serde_json::Map::new(),
                    },
                );
            services
                .models
                .auth_storage()
                .set(&provider, &credential)
                .map_err(failed)?;
            oino_core::services::refresh_provider_models(
                &services.extensions,
                &services.models,
                &provider,
                Some(json!({"type": "api_key", "key": key})),
            )
            .await;
            Ok(Value::Null)
        }
        HostCmd::GetShortcuts => Ok(json!({ "shortcuts": shortcuts(services) })),
    }
}

/// `_oino/extensions`: loaded extensions plus the ids in `disabledExtensions`
/// (the same rows `AgentSession::extension_infos` builds, over the services).
fn extension_infos(services: &Services) -> Vec<Value> {
    let disabled = lock(&services.settings).settings().disabled_extensions();
    let mut rows: Vec<Value> = services
        .extensions
        .extensions()
        .iter()
        .map(|extension| {
            let info = oino_core::session::ExtensionInfo {
                id: extension.instance.id().to_owned(),
                kind: extension.instance.kind(),
                enabled: !disabled.iter().any(|id| id == extension.instance.id()),
                loaded: true,
                path: Some(extension.instance.path()),
            };
            serde_json::to_value(&info).unwrap_or(Value::Null)
        })
        .collect();
    for id in disabled {
        if services.extensions.get(&id).is_none() {
            let info = oino_core::session::ExtensionInfo {
                id,
                kind: oino_core::extensions::api::ExtensionKind::Extension,
                enabled: false,
                loaded: false,
                path: None,
            };
            if let Ok(value) = serde_json::to_value(&info) {
                rows.push(value);
            }
        }
    }
    rows
}

/// `_oino/auth_providers`: the same rows the session command builds, straight
/// from the model registry (`frontend.rs` needs a session only for its runner).
fn auth_providers(services: &Services) -> Vec<Value> {
    let models = &services.models;
    let mut providers: Vec<Value> = models
        .registered_providers()
        .into_iter()
        .filter_map(|id| {
            let registration = models.registration(&id)?;
            let stored = models
                .auth_storage()
                .stored(&id)
                .ok()
                .flatten()
                .map(|credential| {
                    match credential {
                        oino_models::auth::credential::AuthCredential::ApiKey(_) => "api_key",
                        oino_models::auth::credential::AuthCredential::Oauth(_) => "oauth",
                    }
                    .to_owned()
                });
            let info = oino_core::session::AuthProviderInfo {
                name: registration.display_name().to_owned(),
                oauth: registration.oauth,
                configured: models.has_configured_auth(&id),
                stored,
                env: registration.auth_env.clone(),
                id,
            };
            serde_json::to_value(&info).ok()
        })
        .collect();
    // Display order, like the session command.
    providers.sort_by(|a, b| {
        let name = |value: &Value| value["name"].as_str().unwrap_or_default().to_lowercase();
        name(a).cmp(&name(b))
    });
    providers
}

/// `_oino/get_shortcuts`: every loaded extension's registrations (later
/// registrations of the same id win, like the runner's own view).
fn shortcuts(services: &Services) -> Vec<Value> {
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for extension in services.extensions.extensions() {
        let id_of_extension = extension.instance.id();
        for (id, spec) in extension.state().shortcuts {
            by_id.insert(
                id.clone(),
                json!({
                    "id": id,
                    "defaultKeys": spec.default_keys,
                    "description": spec.description,
                    "extensionId": id_of_extension,
                }),
            );
        }
    }
    by_id.into_values().collect()
}
