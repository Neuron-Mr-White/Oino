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

fn search_capabilities(agent_dir: &std::path::Path) -> Value {
    oino_core::builtin::search_capabilities(agent_dir)
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
    // Package installs are file operations on the agent dir: no services needed (and a
    // broken extension set must not keep the user from removing the culprit).
    match cmd {
        HostCmd::SearchCapabilities => return Ok(search_capabilities(hub.agent_dir())),
        HostCmd::BuildSearchIndex { cwd, consent } => {
            let root = std::path::Path::new(&cwd);
            if !root.is_absolute() || !root.is_dir() {
                return Err(invalid("Choose an existing project folder"));
            }
            oino_core::builtin::build_search_index(root, hub.agent_dir(), consent)
                .await
                .map_err(failed)?;
            return Ok(json!({ "success": true }));
        }
        HostCmd::InstallExtension { path, replace } => {
            return install_extension(hub, path, replace.unwrap_or(false)).await;
        }
        HostCmd::RemoveExtension { id } => return remove_extension(hub, id).await,
        // The extension packages changed on disk: drop the cached services sets
        // and discover them again, so a freshly installed (or removed) package is
        // live right away — the host does not have to be restarted.
        HostCmd::ReloadExtensions => {
            hub.reset_core_services().await;
            let services = hub
                .core_services()
                .await
                .map_err(|error| failed(error.to_string()))?;
            return Ok(json!({ "extensions": extension_infos(&services, hub.agent_dir()) }));
        }
        // The settings page's "Reload": the same rediscovery, and the model catalogue
        // with it (`models.json` + `auth.json`), so a provider or model edited outside
        // the app is live without restarting it. A live chat reloads itself from the
        // chat command (`reload` on its stream), which also rebuilds its resources.
        HostCmd::Reload => {
            hub.reset_core_services().await;
            let services = hub
                .core_services()
                .await
                .map_err(|error| failed(error.to_string()))?;
            services.models.auth_storage().reload();
            services.models.refresh();
            hub.readiness().invalidate_all();
            return Ok(json!({
                "extensions": extension_infos(&services, hub.agent_dir()),
                "models": services.models.available(),
            }));
        }
        _ => {}
    }
    let services = match settings_cwd(&cmd)? {
        Some(cwd) => hub.core_services_for(&cwd).await,
        None => hub.core_services().await,
    }
    .map_err(|error| failed(error.to_string()))?;
    let global_write = global_settings_write(&cmd);
    let reply = dispatch(hub, &services, cmd).await?;
    if let Some((namespace, key)) = global_write {
        // One services set per project, each with its own copy of the settings files:
        // replay a global write into the others so their extensions apply it from the
        // next turn (no host restart).
        for other in hub.core_services_sets().await {
            if !Arc::ptr_eq(&other, &services) {
                other.extension_settings.reload_external(
                    &namespace,
                    vec![key.clone()],
                    SettingsScope::Global,
                );
            }
        }
    }
    Ok(reply)
}

/// The `cwd` of an extension-settings command (`None` = the primary cwd's services).
fn settings_cwd(cmd: &HostCmd) -> Result<Option<std::path::PathBuf>, AppErrorDto> {
    let cwd = match cmd {
        HostCmd::GetExtensionSettings { cwd, .. }
        | HostCmd::SetExtensionSetting { cwd, .. }
        | HostCmd::UnsetExtensionSetting { cwd, .. } => cwd.as_deref(),
        _ => None,
    };
    let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(cwd);
    if !path.is_absolute() || !path.is_dir() {
        return Err(invalid(format!(
            "not a project folder on this computer: {cwd}"
        )));
    }
    Ok(Some(path))
}

/// `(namespace, key)` of a global extension-settings write.
fn global_settings_write(cmd: &HostCmd) -> Option<(String, String)> {
    match cmd {
        HostCmd::SetExtensionSetting {
            namespace,
            key,
            scope,
            ..
        }
        | HostCmd::UnsetExtensionSetting {
            namespace,
            key,
            scope,
            ..
        } if scope == "global" => Some((namespace.clone(), key.clone())),
        _ => None,
    }
}

/// E-18/F-09: the permission mode is never written into a repo. The workflow extension no
/// longer offers or writes a project `mode`; this refuses the raw settings paths too
/// (`_oino/set_setting key: extensionSettings.permission.mode scope: project`, the same
/// via `_oino/set_extension_setting`), so no UI or runner can reintroduce it.
fn refused_into_repo(cmd: &HostCmd) -> Result<(), AppErrorDto> {
    let project = |scope: &str| scope == "project";
    let banned = match cmd {
        HostCmd::SetSetting { key, scope, .. } => {
            project(scope)
                && (key == "extensionSettings.permission.mode"
                    || key == "extensionSettings.permission.mode.")
        }
        HostCmd::SetExtensionSetting {
            namespace,
            key,
            scope,
            ..
        } => project(scope) && namespace == "permission" && key == "mode",
        _ => false,
    };
    if banned {
        return Err(invalid(
            "the permission mode is per chat; \"make default\" writes the PC default in the agent dir — never a project",
        ));
    }
    Ok(())
}

fn install_error(error: crate::extension_install::InstallError) -> AppErrorDto {
    match error {
        crate::extension_install::InstallError::Invalid(text) => invalid(text),
        crate::extension_install::InstallError::NotInstalled(text) => {
            AppErrorDto::new(AppErrorCode::E_NOT_FOUND, text, false)
        }
        error @ crate::extension_install::InstallError::Io { .. } => failed(error),
    }
}

/// `install_extension{path, replace?}`: validate + copy into `<agent dir>/extensions`.
async fn install_extension(
    hub: &Arc<SessionHub>,
    path: String,
    replace: bool,
) -> Result<Value, AppErrorDto> {
    let path = path.trim();
    if path.is_empty() {
        return Err(invalid("Pick an extension folder or .zip"));
    }
    let source = std::path::PathBuf::from(path);
    let agent_dir = hub.agent_dir().to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || {
        crate::extension_install::install(
            &source,
            &agent_dir,
            &crate::sessions::services::dist_dirs(),
            replace,
        )
    })
    .await
    .map_err(failed)?
    .map_err(install_error)?;
    serde_json::to_value(outcome).map_err(failed)
}

/// `remove_extension{id}`: delete a user-installed package (bundled ones are refused).
async fn remove_extension(hub: &Arc<SessionHub>, id: String) -> Result<Value, AppErrorDto> {
    let agent_dir = hub.agent_dir().to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || {
        crate::extension_install::remove(&id, &agent_dir, &crate::sessions::services::dist_dirs())
    })
    .await
    .map_err(failed)?
    .map_err(install_error)?;
    serde_json::to_value(outcome).map_err(failed)
}

async fn dispatch(
    hub: &Arc<SessionHub>,
    services: &Arc<Services>,
    cmd: HostCmd,
) -> Result<Value, AppErrorDto> {
    refused_into_repo(&cmd)?;
    let registry = &services.extension_settings;
    match cmd {
        HostCmd::GetSettingsSchema => Ok(json!({ "namespaces": registry.schema() })),
        HostCmd::GetExtensionSettings {
            namespace, layer, ..
        } => {
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
            ..
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
            ..
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
        HostCmd::Extensions => Ok(json!({ "extensions": extension_infos(services, hub.agent_dir()) })),
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
        HostCmd::AuthProviders { validate } => Ok(json!({
            "providers": auth_providers(hub, services, validate.unwrap_or(false)).await
        })),
        // `default_model` (D-01): the model a new session will start on, with why.
        HostCmd::DefaultModel => default_model_reply(hub, services).await,
        HostCmd::AvailableModels => {
            let mut data = json!({ "models": services.models.available() });
            crate::sessions::stream::enrich_models_reply(&hub.readiness().snapshot(), &mut data);
            Ok(data)
        }
        HostCmd::Login { .. } => Err(unsupported(
            "OAuth sign-in opens from a chat session (its dialogs need a frontend); set an API key instead",
        )),
        HostCmd::Logout { provider } => {
            services
                .models
                .auth_storage()
                .remove(&provider)
                .map_err(failed)?;
            hub.readiness().invalidate(&provider);
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
            // The new key must be validated afresh (D-37) — and right away (G-05): the
            // bounded probe runs before the reply, so the UI's refetch sees the real
            // answer (a dead key is "Not working · HTTP 401", never "ready" until the
            // first send fails).
            hub.readiness().invalidate(&provider);
            crate::models_readiness::probe_and_cache(hub.readiness(), &services.models, &provider)
                .await;
            Ok(Value::Null)
        }
        HostCmd::GetShortcuts => Ok(json!({ "shortcuts": shortcuts(services) })),
        // Served before the services spin up (`run`).
        HostCmd::SearchCapabilities
        | HostCmd::BuildSearchIndex { .. }
        | HostCmd::InstallExtension { .. }
        | HostCmd::RemoveExtension { .. }
        | HostCmd::ReloadExtensions
        | HostCmd::Reload => Err(unsupported(
            "package installs and reloads are served by `run`",
        )),
    }
}

/// `_oino/extensions`: loaded extensions plus the ids in `disabledExtensions`
/// (the same rows `AgentSession::extension_infos` builds, over the services).
///
/// Each row also says whether Settings may offer "Remove" (`removable`: the package lives
/// in the user dir `<agent dir>/extensions/<id>`, not the app's bundled share dir).
fn extension_infos(services: &Services, agent_dir: &std::path::Path) -> Vec<Value> {
    let disabled = lock(&services.settings).settings().disabled_extensions();
    let mut rows: Vec<Value> = services
        .extensions
        .extensions()
        .iter()
        .map(|extension| {
            let path = extension.instance.path();
            let info = oino_core::session::ExtensionInfo {
                id: extension.instance.id().to_owned(),
                kind: extension.instance.kind(),
                enabled: !disabled.iter().any(|id| id == extension.instance.id()),
                loaded: true,
                path: Some(path.clone()),
            };
            let removable = user_owned(std::path::Path::new(&path), agent_dir);
            with_removable(
                with_manifest(
                    serde_json::to_value(&info).unwrap_or(Value::Null),
                    manifest_of(std::path::Path::new(&path)),
                ),
                removable,
            )
        })
        .collect();
    for id in disabled {
        if services.extensions.get(&id).is_none() {
            let manifest = crate::sessions::services::dist_dirs()
                .into_iter()
                .map(|dir| dir.join(&id))
                .find_map(|dir| manifest_of(&dir));
            let info = oino_core::session::ExtensionInfo {
                id,
                kind: oino_core::extensions::api::ExtensionKind::Extension,
                enabled: false,
                loaded: false,
                path: None,
            };
            let removable = crate::extension_install::is_user_installed(&info.id, agent_dir);
            if let Ok(value) = serde_json::to_value(&info) {
                rows.push(with_removable(with_manifest(value, manifest), removable));
            }
        }
    }
    rows
}

/// Is the package at `path` (its folder or a file in it) one of `<agent dir>/extensions/*`?
fn user_owned(path: &std::path::Path, agent_dir: &std::path::Path) -> bool {
    let root = crate::extension_install::user_root(agent_dir);
    let canon = |p: &std::path::Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let root = canon(&root);
    let path = canon(path);
    path.strip_prefix(&root).is_ok_and(|rest| {
        rest.components().next().is_some_and(|first| {
            root.join(first.as_os_str())
                .join(oino_extension_api::manifest::MANIFEST_FILE)
                .is_file()
        })
    })
}

fn with_removable(mut row: Value, removable: bool) -> Value {
    if let Some(object) = row.as_object_mut() {
        object.insert("removable".to_owned(), Value::Bool(removable));
    }
    row
}

/// The package's `extension.json` (`path` is the package folder or a file in it).
fn manifest_of(path: &std::path::Path) -> Option<Value> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    let text = std::fs::read_to_string(dir.join("extension.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Adds the manifest's display `name`, `version` and `description` to an
/// extension row (Settings › Extensions shows them instead of the raw id).
fn with_manifest(mut row: Value, manifest: Option<Value>) -> Value {
    if let (Some(object), Some(manifest)) = (row.as_object_mut(), manifest) {
        for key in ["name", "version", "description"] {
            if let Some(text) = manifest.get(key).and_then(Value::as_str) {
                object.insert(key.to_owned(), Value::String(text.to_owned()));
            }
        }
    }
    row
}

/// `_oino/auth_providers`: the same rows the session command builds, straight from the
/// model registry (`frontend.rs` needs a session only for its runner), plus the hub's
/// cached credential validation (D-37): `state` (`ready` | `invalid_key` | `unreachable` |
/// `unknown`), `ready` (present iff validated), `reason`, `checkedAt`.
///
/// Validation is lazy by design: rows answer from the cache; configured-but-unvalidated
/// providers are probed in the background (the next read sees the answer). `validate: true`
/// waits for the probes of this call instead — bounded, parallel, once per provider per TTL.
async fn auth_providers(hub: &Arc<SessionHub>, services: &Services, validate: bool) -> Vec<Value> {
    let (mut rows, to_probe) = provider_rows(hub, &services.models);
    if to_probe.is_empty() {
        return rows;
    }
    if validate {
        crate::models_readiness::probe_all(hub.readiness(), &services.models, &to_probe).await;
        let (fresh, _) = provider_rows(hub, &services.models);
        rows = fresh;
        return rows;
    }
    // Lazy: fill the cache in the background; the next read carries the answer.
    let hub = Arc::clone(hub);
    let models = services.models.clone();
    tokio::spawn(async move {
        crate::models_readiness::probe_all(hub.readiness(), &models, &to_probe).await;
    });
    rows
}

/// The rows plus which configured providers still need a probe (cache miss).
fn provider_rows(
    hub: &Arc<SessionHub>,
    models: &oino_models::ModelRegistry,
) -> (Vec<Value>, Vec<String>) {
    let mut rows: Vec<Value> = Vec::new();
    let mut to_probe: Vec<String> = Vec::new();
    for id in models.registered_providers() {
        let Some(registration) = models.registration(&id) else {
            continue;
        };
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
            id: id.clone(),
        };
        let mut row = serde_json::to_value(&info).unwrap_or(Value::Null);
        if let Some(object) = row.as_object_mut() {
            let configured = object
                .get("configured")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match hub.readiness().get(&id) {
                Some(readiness) => {
                    object.insert("state".into(), json!(readiness.state.as_str()));
                    object.insert(
                        "ready".into(),
                        json!(readiness.state == crate::models_readiness::ReadinessState::Ready),
                    );
                    object.insert(
                        "reason".into(),
                        json!(readiness_reason(
                            readiness.state,
                            readiness.detail.as_deref()
                        )),
                    );
                    if let Some(detail) = &readiness.detail {
                        object.insert("detail".into(), json!(detail));
                    }
                    object.insert(
                        "checkedAt".into(),
                        json!(crate::models_readiness::epoch_ms(readiness.checked_at)),
                    );
                }
                None => {
                    let state = if configured {
                        "unknown"
                    } else {
                        "not_configured"
                    };
                    object.insert("state".into(), json!(state));
                    if configured && crate::models_readiness::probeable(&registration) {
                        to_probe.push(id);
                    }
                }
            }
        }
        rows.push(row);
    }
    // Display order, like the session command.
    rows.sort_by(|a, b| {
        let name = |value: &Value| value["name"].as_str().unwrap_or_default().to_lowercase();
        name(a).cmp(&name(b))
    });
    (rows, to_probe)
}

/// One line of "why not ready" for a provider row (D-37: no more `Ready` on a dead key).
fn readiness_reason(
    state: crate::models_readiness::ReadinessState,
    detail: Option<&str>,
) -> String {
    let detail = detail.unwrap_or_default();
    match state {
        crate::models_readiness::ReadinessState::Ready => "the key works".to_owned(),
        crate::models_readiness::ReadinessState::InvalidKey => {
            if detail.is_empty() {
                "the provider rejected the key".to_owned()
            } else {
                format!("the provider rejected the key ({detail})")
            }
        }
        crate::models_readiness::ReadinessState::Unreachable => {
            if detail.is_empty() {
                "could not reach the provider".to_owned()
            } else {
                format!("could not reach the provider ({detail})")
            }
        }
    }
}

/// `default_model` (D-01): the model `session_new` will start on, the source of the choice
/// and why the configured default (when there is one) was not used — so the draft chip shows
/// what the first send will actually use, or says why it cannot.
async fn default_model_reply(
    hub: &Arc<SessionHub>,
    services: &Arc<Services>,
) -> Result<Value, AppErrorDto> {
    // A snapshot, not the lock: the chooser awaits probes and the guard must not cross one.
    let settings = services
        .settings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot();
    let choice = crate::models_readiness::choose_default_model(
        &services.models,
        &settings,
        hub.readiness(),
        false,
    )
    .await;
    let default_configured = match (settings.default_provider(), settings.default_model()) {
        (Some(provider), Some(model)) => json!({ "provider": provider, "model": model }),
        _ => Value::Null,
    };
    // The chosen model's provider, else the configured default's (when nothing was usable).
    let kick = choice
        .model
        .as_ref()
        .map(|model| model.provider.clone())
        .or_else(|| {
            default_configured
                .get("provider")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let provider_state = kick.as_deref().and_then(|provider| {
        hub.readiness()
            .get(provider)
            .map(|readiness| readiness.state.as_str())
    });
    // Refresh that provider's validation in the background so the next read — and
    // `session_new`'s fallback — is informed.
    if let Some(provider) = kick.filter(|provider| {
        services
            .models
            .registration(provider)
            .is_some_and(|registration| crate::models_readiness::probeable(&registration))
    }) {
        let hub = Arc::clone(hub);
        let models = services.models.clone();
        tokio::spawn(async move {
            crate::models_readiness::probe_and_cache(hub.readiness(), &models, &provider).await;
        });
    }
    Ok(json!({
        "model": choice.model,
        "source": choice.source,
        "reason": choice.reason,
        "defaultConfigured": default_configured,
        "providerState": provider_state,
    }))
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

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod tests {
    use super::*;
    use unipi_proto::settings::HostCmd;

    #[test]
    fn permission_mode_writes_into_a_project_are_refused() {
        // E-18/F-09: neither the raw path nor the extension form may write the mode into
        // <repo>/.oino/settings.json; the global default and unset stay allowed.
        let banned = HostCmd::SetSetting {
            key: "extensionSettings.permission.mode".into(),
            value: Some(json!("ask")),
            scope: "project".into(),
        };
        assert!(refused_into_repo(&banned).is_err());
        let banned = HostCmd::SetExtensionSetting {
            namespace: "permission".into(),
            key: "mode".into(),
            value: Some(json!("ask")),
            scope: "project".into(),
            cwd: None,
        };
        assert!(refused_into_repo(&banned).is_err());
        // The global default (the "make default" write path) is fine.
        let ok = HostCmd::SetExtensionSetting {
            namespace: "permission".into(),
            key: "defaultMode".into(),
            value: Some(json!("ask")),
            scope: "global".into(),
            cwd: None,
        };
        assert!(refused_into_repo(&ok).is_ok());
        let ok = HostCmd::SetSetting {
            key: "extensionSettings.permission.rules".into(),
            value: Some(json!([])),
            scope: "project".into(),
        };
        assert!(refused_into_repo(&ok).is_ok());
        let ok = HostCmd::UnsetExtensionSetting {
            namespace: "permission".into(),
            key: "mode".into(),
            scope: "project".into(),
            cwd: None,
        };
        assert!(refused_into_repo(&ok).is_ok());
    }
}

#[cfg(test)]
mod search_capability_tests {
    #[test]
    fn reports_compiled_engine_not_user_toggle() {
        assert_eq!(
            super::search_capabilities(std::path::Path::new("/nonexistent"))["installed"],
            serde_json::json!(cfg!(feature = "zvec"))
        );
    }
}
