//! Frontend command surface (modes.md §4 `_oino/*`, ui-protocol §5–§8): completions, panel
//! actions, shortcuts, editor intents, service calls and provider sign-in.
//!
//! Every frontend (in-process TUI, desktop, RPC clients) reaches these through
//! `oino-client`; core only routes to the owning extension and never renders anything.

use std::path::Path;
use std::time::Duration;

use oino_ai::JsonValue;
use oino_models::auth::credential::{AuthCredential, OAuthCredential};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{AgentSession, lock};
use crate::extensions::api::CallKind;

/// Budget of each completion contributor (ui-protocol §7.3).
const COMPLETION_BUDGET: Duration = Duration::from_millis(100);
/// Most `@` path candidates returned.
const MAX_PATH_ITEMS: usize = 50;
/// Most filesystem entries visited per `@` completion.
const MAX_PATH_SCAN: usize = 20_000;
/// Default deadline of a frontend service call.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
/// Caller id frontends use for service calls.
pub const FRONTEND_CALLER: &str = "oino.frontend";

/// Failure of a frontend command.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FrontendError {
    /// Unknown panel, shortcut, service, command or provider.
    #[error("{0}")]
    NotFound(String),
    /// The target exists but cannot do this.
    #[error("{0}")]
    Unsupported(String),
    /// The extension or the credential store failed.
    #[error("{0}")]
    Failed(String),
}

/// One completion candidate (ui-protocol §7.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionEntry {
    /// Text that replaces `text[from..cursor]`.
    pub value: String,
    /// Label shown (defaults to `value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Extra description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Short tag (`dir`, `skill`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Tone (ui-protocol §6.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Lower tiers sort first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_tier: Option<i64>,
}

/// Completions from one source.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionGroup {
    /// `commands`, `command:<name>`, `files`, or the contributing extension id.
    pub source: String,
    /// Character offset where an accepted value starts replacing (up to the cursor); the
    /// start of the token under the cursor when the contributor omits it.
    #[serde(default)]
    pub from: Option<usize>,
    /// Candidates.
    pub items: Vec<CompletionEntry>,
}

/// Editor intents of `editor_event` (ui-protocol §7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorIntent {
    /// Text changed (debounced by the frontend).
    Changed,
    /// Text became empty.
    Cleared,
    /// ↓ pressed in an empty composer (consumable).
    EmptyArrowDown,
    /// About to send.
    Submit,
    /// Backspace next to an attachment token (consumable).
    TokenBackspace,
}

impl EditorIntent {
    /// Whether an extension may consume the intent (the frontend waits for the reply).
    #[must_use]
    pub const fn consumable(self) -> bool {
        matches!(self, Self::EmptyArrowDown | Self::TokenBackspace)
    }
}

/// One `editor_event`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorEvent {
    /// Frontend event id (echoed to extensions).
    pub event_id: String,
    /// Intent.
    pub intent: EditorIntent,
    /// Composer text after the change.
    #[serde(default)]
    pub text: String,
    /// Attachment token next to the cursor (`tokenBackspace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
}

/// A provider as `/login` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProviderInfo {
    /// Provider id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Supports OAuth sign-in (`/login`).
    pub oauth: bool,
    /// Credentials are configured (any source).
    pub configured: bool,
    /// Stored credential kind (`api_key` | `oauth`), when `auth.json` has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored: Option<String>,
    /// Environment variables that configure the provider.
    pub env: Vec<String>,
}

/// Character index → byte index of `text` (clamped to the end).
fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

/// Case-insensitive subsequence match (pi-tui `fuzzyFilter` approximation).
fn fuzzy(candidate: &str, query: &str) -> bool {
    let mut chars = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|q| chars.any(|c| c == q))
}

/// `@` candidates under `cwd` whose relative path fuzzy-matches `query` (respects
/// `.gitignore`; directories end with `/`). Prefix matches sort first.
fn path_candidates(cwd: &Path, query: &str) -> Vec<CompletionEntry> {
    let (base, needle) = match query.rfind('/') {
        Some(i) if query.starts_with("~/") || query.starts_with('/') => {
            (Path::new(&query[..=i]).to_path_buf(), &query[i + 1..])
        }
        _ => (cwd.to_path_buf(), query),
    };
    let base = match base.strip_prefix("~/") {
        Ok(rest) => oino_settings::paths::home_dir().map_or(base.clone(), |h| h.join(rest)),
        Err(_) => base,
    };
    let absolute = base != cwd;
    let mut found: Vec<(bool, String, bool)> = Vec::new();
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(!needle.starts_with('.'))
        .max_depth(Some(if absolute { 1 } else { 8 }))
        .build();
    for entry in walker.filter_map(Result::ok).take(MAX_PATH_SCAN) {
        let Ok(relative) = entry.path().strip_prefix(&base) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.is_empty() || !fuzzy(&relative, needle) {
            continue;
        }
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let shown = if absolute {
            format!("{}{relative}", &query[..query.len() - needle.len()])
        } else {
            relative.clone()
        };
        let prefix = relative.to_lowercase().starts_with(&needle.to_lowercase());
        found.push((prefix, shown, is_dir));
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.matches('/').count().cmp(&b.1.matches('/').count()))
            .then(a.1.cmp(&b.1))
    });
    found
        .into_iter()
        .take(MAX_PATH_ITEMS)
        .map(|(_, path, is_dir)| {
            let path = if is_dir { format!("{path}/") } else { path };
            CompletionEntry {
                value: format!("@{path}"),
                label: Some(path),
                tag: is_dir.then(|| "dir".to_owned()),
                ..CompletionEntry::default()
            }
        })
        .collect()
}

impl AgentSession {
    /// The composer text the frontend reported last (`ui.editor-text`).
    #[must_use]
    pub fn editor_text(&self) -> String {
        lock(&self.inner.editor_text).clone()
    }

    /// Merged completions for `text` with the cursor at character `cursor` (ui-protocol
    /// §7.3): slash commands, command arguments (`complete-command`), `@` paths and every
    /// extension `complete-input` contributor, each within 100 ms.
    pub async fn completions(&self, text: &str, cursor: usize) -> Vec<CompletionGroup> {
        let before = &text[..byte_index(text, cursor)];
        let cursor = before.chars().count();
        let token_start = before
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| before[..i + c.len_utf8()].chars().count());
        let token = &before[byte_index(before, token_start)..];
        let mut groups = Vec::new();
        if let Some(line) = before.strip_prefix('/') {
            groups.extend(self.command_completions(line).await);
        }
        if let Some(query) = token.strip_prefix('@') {
            let cwd = self.inner.cwd.clone();
            let query = query.to_owned();
            let items = tokio::task::spawn_blocking(move || path_candidates(&cwd, &query))
                .await
                .unwrap_or_default();
            if !items.is_empty() {
                groups.push(CompletionGroup {
                    source: "files".to_owned(),
                    from: Some(token_start),
                    items,
                });
            }
        }
        groups.extend(
            self.contributed_completions(text, cursor, token_start)
                .await,
        );
        groups
    }

    /// `/name` completion, or `complete-command` arguments after `/name `.
    async fn command_completions(&self, line: &str) -> Option<CompletionGroup> {
        let runner = self.runner();
        let cx = runner.call_context().with_kind(CallKind::Command);
        match line.split_once(' ') {
            None => {
                let items: Vec<CompletionEntry> = self
                    .commands()
                    .into_iter()
                    .filter(|c| fuzzy(&c.name, line))
                    .map(|c| CompletionEntry {
                        value: format!("/{}", c.name),
                        label: Some(format!("/{}", c.name)),
                        description: c.description,
                        tag: Some(c.source),
                        ..CompletionEntry::default()
                    })
                    .collect();
                (!items.is_empty()).then(|| CompletionGroup {
                    source: "commands".to_owned(),
                    from: Some(0),
                    items,
                })
            }
            Some((name, argument)) => {
                let command = runner.command(name).filter(|c| c.spec.completions)?;
                let call =
                    command
                        .extension
                        .instance
                        .complete_command(&command.spec.name, argument, cx);
                let items = tokio::time::timeout(COMPLETION_BUDGET, call)
                    .await
                    .unwrap_or_default();
                (!items.is_empty()).then(|| CompletionGroup {
                    source: format!("command:{name}"),
                    from: Some(name.chars().count() + 2),
                    items: items
                        .into_iter()
                        .map(|item| CompletionEntry {
                            label: Some(item.label),
                            value: item.value,
                            description: item.description,
                            ..CompletionEntry::default()
                        })
                        .collect(),
                })
            }
        }
    }

    /// Groups of every extension `complete-input` contributor (100 ms each, in parallel);
    /// groups without `from` replace the token under the cursor.
    async fn contributed_completions(
        &self,
        text: &str,
        cursor: usize,
        token_start: usize,
    ) -> Vec<CompletionGroup> {
        let runner = self.runner();
        let cx = runner.call_context().with_kind(CallKind::Command);
        let request = json!({"text": text, "cursor": cursor});
        let contributors = runner.set().extensions().iter().map(|extension| {
            let call = extension
                .instance
                .complete_input(request.clone(), cx.clone());
            let id = extension.instance.id().to_owned();
            async move {
                let value = tokio::time::timeout(COMPLETION_BUDGET, call)
                    .await
                    .ok()
                    .flatten()?;
                let groups: Vec<CompletionGroup> = serde_json::from_value(value).ok()?;
                Some((id, groups))
            }
        });
        futures::future::join_all(contributors)
            .await
            .into_iter()
            .flatten()
            .flat_map(|(id, contributed)| {
                contributed
                    .into_iter()
                    .filter(|g| !g.items.is_empty())
                    .map(move |mut group| {
                        group.from.get_or_insert(token_start);
                        if group.source.is_empty() {
                            group.source.clone_from(&id);
                        }
                        group
                    })
            })
            .collect()
    }

    /// The extension that opened panel `panel_id` (retained `openPanel`, else the
    /// `<ext>/panel-<n>` id prefix).
    fn panel_owner(&self, panel_id: &str) -> Option<String> {
        self.inner
            .ui
            .snapshot()
            .into_iter()
            .filter(|r| r.method == "openPanel" || r.method == "updatePanel")
            .find(|r| r.params.get("panelId").and_then(JsonValue::as_str) == Some(panel_id))
            .and_then(|r| r.extension_id)
            .or_else(|| {
                panel_id
                    .rsplit_once("/panel-")
                    .map(|(owner, _)| owner.to_owned())
            })
    }

    /// Delivers a panel action (`oino_ui_action{panelId, actionId, value?}`) to the panel's
    /// owner in a command-capable context. Returns once dispatched; handler failures become
    /// `extension_error`.
    ///
    /// # Errors
    ///
    /// [`FrontendError::NotFound`] for an unknown panel.
    pub fn ui_action(
        &self,
        panel_id: &str,
        action_id: &str,
        value: Option<JsonValue>,
    ) -> Result<(), FrontendError> {
        let runner = self.runner();
        let extension = self
            .panel_owner(panel_id)
            .and_then(|owner| runner.set().get(&owner).cloned())
            .ok_or_else(|| FrontendError::NotFound(format!("Unknown panel: {panel_id}")))?;
        let mut payload =
            json!({"type": "oino_ui_action", "panelId": panel_id, "actionId": action_id});
        if let Some(value) = value {
            payload["value"] = value;
        }
        let cx = runner.call_context().with_kind(CallKind::Command);
        self.spawn_task(async move {
            if let Err(error) = extension
                .instance
                .on_event("oino_ui_action", payload, cx)
                .await
            {
                runner.emit_error(&extension.instance.path(), "oino_ui_action", error.message);
            }
        });
        Ok(())
    }

    /// Runs an extension shortcut (`run-shortcut`) in a shortcut context. Returns once
    /// dispatched; failures become `extension_error`.
    ///
    /// # Errors
    ///
    /// [`FrontendError::NotFound`] for an unknown shortcut id.
    pub fn invoke_shortcut(&self, id: &str) -> Result<(), FrontendError> {
        let runner = self.runner();
        let extension = runner
            .shortcuts()
            .into_iter()
            .find(|(_, spec)| spec.id == id)
            .and_then(|(owner, _)| runner.set().get(&owner).cloned())
            .ok_or_else(|| FrontendError::NotFound(format!("Unknown shortcut: {id}")))?;
        let cx = runner.call_context().with_kind(CallKind::Shortcut);
        let id = id.to_owned();
        self.spawn_task(async move {
            if let Err(error) = extension.instance.run_shortcut(&id, cx).await {
                runner.emit_error(&extension.instance.path(), "shortcut", error.message);
            }
        });
        Ok(())
    }

    /// Fires `setShortcuts` (ui-protocol §4) with every extension shortcut, so frontends bind
    /// the keys (GA-43). Retained for late frontends.
    pub(crate) fn publish_shortcuts(&self) {
        let shortcuts: Vec<JsonValue> = self
            .runner()
            .shortcuts()
            .into_iter()
            .map(|(extension, spec)| {
                json!({
                    "id": spec.id,
                    "keys": spec.default_keys,
                    "description": spec.description,
                    "extensionId": extension,
                })
            })
            .collect();
        let mut params = oino_ai::JsonObject::new();
        params.insert("shortcuts".into(), JsonValue::Array(shortcuts));
        self.inner.ui.fire(None, "setShortcuts", params);
    }

    /// Records the composer text and delivers `oino_editor` to subscribed extensions.
    /// Consumable intents wait for the handlers and report whether one returned
    /// `{consume: true}` (first wins); the others are dispatched in the background and
    /// return `false`.
    pub async fn editor_event(&self, event: EditorEvent) -> bool {
        lock(&self.inner.editor_text).clone_from(&event.text);
        let runner = self.runner();
        let mut payload = json!({
            "type": "oino_editor",
            "eventId": event.event_id,
            "intent": event.intent,
            "text": event.text,
        });
        if let Some(token) = &event.token_id {
            payload["tokenId"] = json!(token);
        }
        if !runner.has_handlers("oino_editor") {
            return false;
        }
        if !event.intent.consumable() {
            self.spawn_task(async move {
                runner.emit(payload).await;
            });
            return false;
        }
        runner.emit_consumable(payload).await
    }

    /// Calls service `name` (of `target`, or of the first extension serving it) as the
    /// frontend (`caller = "oino.frontend"`).
    ///
    /// # Errors
    ///
    /// [`FrontendError::NotFound`] when nobody serves it, [`FrontendError::Failed`] for
    /// handler errors and the 30 s timeout.
    pub async fn service_call(
        &self,
        target: Option<&str>,
        name: &str,
        payload: JsonValue,
    ) -> Result<JsonValue, FrontendError> {
        let runner = self.runner();
        let extension = runner
            .set()
            .extensions()
            .iter()
            .filter(|e| target.is_none_or(|t| e.instance.id() == t))
            .find(|e| e.instance.services().iter().any(|s| s == name))
            .cloned()
            .ok_or_else(|| {
                FrontendError::NotFound(match target {
                    Some(target) => format!("Unknown service: {target}/{name}"),
                    None => format!("Unknown service: {name}"),
                })
            })?;
        let cx = runner.call_context().with_kind(CallKind::Service);
        let call = extension
            .instance
            .handle_service(name, payload, FRONTEND_CALLER, cx);
        match tokio::time::timeout(SERVICE_TIMEOUT, call).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(FrontendError::Failed(error.message)),
            Err(_) => Err(FrontendError::Failed(format!(
                "Service {name} timed out after {}s",
                SERVICE_TIMEOUT.as_secs()
            ))),
        }
    }

    /// Registered providers with their sign-in support and credential state (`/login`).
    #[must_use]
    pub fn auth_providers(&self) -> Vec<AuthProviderInfo> {
        let models = &self.inner.models;
        let mut providers: Vec<AuthProviderInfo> = models
            .registered_providers()
            .into_iter()
            .filter_map(|id| {
                let registration = models.registration(&id)?;
                let stored = models
                    .auth_storage()
                    .stored(&id)
                    .ok()
                    .flatten()
                    .map(|c| match c {
                        AuthCredential::ApiKey(_) => "api_key".to_owned(),
                        AuthCredential::Oauth(_) => "oauth".to_owned(),
                    });
                Some(AuthProviderInfo {
                    name: registration.display_name().to_owned(),
                    oauth: registration.oauth,
                    configured: models.has_configured_auth(&id),
                    stored,
                    env: registration.auth_env.clone(),
                    id,
                })
            })
            .collect();
        providers.sort_by_key(|p| p.name.to_lowercase());
        providers
    }

    /// Signs in to `provider` with its component's OAuth flow (dialogs go to this session's
    /// frontend), stores the credential in `auth.json` and refreshes the provider's models.
    ///
    /// # Errors
    ///
    /// Unknown provider, no OAuth support, flow failure or credential store failure.
    pub async fn login(&self, provider: &str) -> Result<(), FrontendError> {
        let models = &self.inner.models;
        let registration = models
            .registration(provider)
            .ok_or_else(|| FrontendError::NotFound(format!("Unknown provider: {provider}")))?;
        if !registration.oauth {
            return Err(FrontendError::Unsupported(format!(
                "{} does not support OAuth sign-in; set an API key instead",
                registration.display_name()
            )));
        }
        let runner = self.runner();
        let extension = runner.provider_for_api(&registration.api).ok_or_else(|| {
            FrontendError::NotFound(format!("No provider component for {}", registration.api))
        })?;
        let component = extension
            .instance
            .provider()
            .ok_or_else(|| FrontendError::Unsupported(format!("{provider}: not a provider")))?;
        let cx = runner.call_context().with_kind(CallKind::Command);
        let mut value = component
            .oauth_login(provider, cx)
            .await
            .map_err(|e| FrontendError::Failed(e.message))?;
        if let Some(object) = value.as_object_mut() {
            object.remove("type");
        }
        let credential: OAuthCredential = serde_json::from_value(value.clone())
            .map_err(|e| FrontendError::Failed(format!("invalid OAuth credential: {e}")))?;
        models
            .auth_storage()
            .set(provider, &AuthCredential::Oauth(credential))
            .map_err(|e| FrontendError::Failed(e.to_string()))?;
        value["type"] = json!("oauth");
        crate::services::refresh_provider_models(runner.set(), models, provider, Some(value)).await;
        self.refresh_current_model();
        Ok(())
    }

    /// Removes the stored credential of `provider` (`/logout`).
    ///
    /// # Errors
    ///
    /// Unknown provider or credential store failure.
    pub fn logout(&self, provider: &str) -> Result<(), FrontendError> {
        let models = &self.inner.models;
        if models.registration(provider).is_none() && !models.auth_storage().has_stored(provider) {
            return Err(FrontendError::NotFound(format!(
                "Unknown provider: {provider}"
            )));
        }
        models
            .auth_storage()
            .remove(provider)
            .map_err(|e| FrontendError::Failed(e.to_string()))
    }

    /// Stores an API key for `provider` in `auth.json`.
    ///
    /// # Errors
    ///
    /// Empty key or credential store failure.
    pub async fn set_api_key(&self, provider: &str, key: &str) -> Result<(), FrontendError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(FrontendError::Failed("API key cannot be empty".to_owned()));
        }
        let models = &self.inner.models;
        let credential = AuthCredential::ApiKey(oino_models::auth::credential::ApiKeyCredential {
            key: Some(key.to_owned()),
            env: None,
            extra: oino_ai::JsonObject::new(),
        });
        models
            .auth_storage()
            .set(provider, &credential)
            .map_err(|e| FrontendError::Failed(e.to_string()))?;
        crate::services::refresh_provider_models(
            self.runner().set(),
            models,
            provider,
            Some(json!({"type": "api_key", "key": key})),
        )
        .await;
        self.refresh_current_model();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_and_indices() {
        assert!(fuzzy("kanboard-add", "kbad"));
        assert!(!fuzzy("model", "x"));
        assert_eq!(byte_index("añb", 2), 3);
        assert_eq!(byte_index("ab", 9), 2);
        assert!(EditorIntent::EmptyArrowDown.consumable());
        assert!(!EditorIntent::Changed.consumable());
    }

    #[test]
    fn path_candidates_prefer_prefix_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();
        let items = path_candidates(dir.path(), "src");
        assert_eq!(items[0].value, "@src/");
        assert_eq!(items[0].tag.as_deref(), Some("dir"));
        assert!(items.iter().any(|i| i.value == "@src/lib.rs"));
        let readme = path_candidates(dir.path(), "rdm");
        assert_eq!(readme[0].value, "@README.md");
    }
}
