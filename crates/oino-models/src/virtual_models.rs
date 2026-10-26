//! pi `virtual-models.ts`: catalog entries that route each request to a physical model.
//!
//! Extensions register definitions (`register-virtual-model`) and implement the guest export
//! `route-model`. This module holds the definitions, the route request/result wire types,
//! validation of route results and the branch helpers for selection and router state. The
//! routing call itself is made by `oino-core` through the extension.

use indexmap::IndexMap;
use oino_ai::{AssistantMessage, InputModality, JsonValue, Message, Model, ModelCost};
use oino_ai::{StopReason, ThinkingLevel, ThinkingLevelMap};
use serde::{Deserialize, Deserializer, Serialize};

/// API id of virtual catalog entries (pi `pi-virtual`, D13). Requests for it fail unless routed.
pub const VIRTUAL_MODEL_API: &str = "oino-virtual";

/// `customType` of the session entry storing router state on a branch.
pub const VIRTUAL_MODEL_STATE_ENTRY: &str = "oino.virtual-model-state";

/// pi `VirtualModelDefinition` without `route` (implemented by the guest export).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualModelDefinition {
    /// Provider the model is listed under (any id, also one with physical models).
    pub provider: String,
    /// Model id; must not be a physical model id of `provider`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Levels offered for selection (default `["off"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_levels: Option<Vec<ThinkingLevel>>,
    /// Limit shown before the first response (unset = unknown, 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Output limit shown before the first response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Input types offered (default text and images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<InputModality>>,
}

/// Whether a model (or message `api`) names a virtual model (pi `isVirtualModel`).
#[must_use]
pub fn is_virtual_api(api: &str) -> bool {
    api == VIRTUAL_MODEL_API
}

/// pi `createVirtualModel`: the catalog entry of a definition.
#[must_use]
pub fn create_virtual_model(definition: &VirtualModelDefinition) -> Model {
    let levels = definition
        .thinking_levels
        .clone()
        .unwrap_or_else(|| vec![ThinkingLevel::Off]);
    let slot =
        |level: ThinkingLevel| Some(levels.contains(&level).then(|| level.as_str().to_owned()));
    Model {
        id: definition.id.clone(),
        name: definition.name.clone(),
        api: VIRTUAL_MODEL_API.to_owned(),
        provider: definition.provider.clone(),
        base_url: String::new(),
        input: definition
            .input
            .clone()
            .unwrap_or_else(|| vec![InputModality::Text, InputModality::Image]),
        input_limits: None,
        cost: ModelCost::default(),
        headers: None,
        kind: None,
        reasoning: levels.iter().any(|level| *level != ThinkingLevel::Off),
        thinking_level_map: Some(ThinkingLevelMap {
            off: slot(ThinkingLevel::Off),
            minimal: slot(ThinkingLevel::Minimal),
            low: slot(ThinkingLevel::Low),
            medium: slot(ThinkingLevel::Medium),
            high: slot(ThinkingLevel::High),
            xhigh: slot(ThinkingLevel::Xhigh),
            max: slot(ThinkingLevel::Max),
        }),
        prompt_cache: None,
        context_window: definition.context_window.unwrap_or(0),
        max_tokens: definition.max_tokens.unwrap_or(0),
        sampling_params: None,
        sampling_params_by_thinking_level: None,
        compat: None,
    }
}

/// A registered virtual model.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisteredVirtualModel {
    /// The definition as registered.
    pub definition: VirtualModelDefinition,
    /// Extension implementing `route-model`.
    pub extension_id: String,
    /// Catalog entry.
    pub model: Model,
}

/// Errors registering a virtual model (pi texts).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VirtualModelError {
    /// Empty provider or id.
    #[error("Virtual model provider and id must not be empty.")]
    EmptyId,
    /// The id is a physical model of the provider.
    #[error("Virtual model {provider}/{id} conflicts with a physical model.")]
    ConflictsWithPhysical {
        /// Provider id.
        provider: String,
        /// Model id.
        id: String,
    },
}

/// Virtual model definitions by provider, then id (pi `ModelRuntime.virtualModels`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VirtualModelRegistry {
    by_provider: IndexMap<String, IndexMap<String, RegisteredVirtualModel>>,
}

impl VirtualModelRegistry {
    /// Registers or replaces a definition. `is_physical` reports whether `provider/id` is a
    /// physical catalog model.
    ///
    /// # Errors
    ///
    /// [`VirtualModelError`] for empty ids or a physical conflict.
    pub fn register(
        &mut self,
        extension_id: &str,
        definition: VirtualModelDefinition,
        is_physical: bool,
    ) -> Result<(), VirtualModelError> {
        if definition.provider.trim().is_empty() || definition.id.trim().is_empty() {
            return Err(VirtualModelError::EmptyId);
        }
        if is_physical {
            return Err(VirtualModelError::ConflictsWithPhysical {
                provider: definition.provider,
                id: definition.id,
            });
        }
        let model = create_virtual_model(&definition);
        self.by_provider
            .entry(definition.provider.clone())
            .or_default()
            .insert(
                definition.id.clone(),
                RegisteredVirtualModel {
                    definition,
                    extension_id: extension_id.to_owned(),
                    model,
                },
            );
        Ok(())
    }

    /// Removes a definition; `false` when it was not registered.
    pub fn unregister(&mut self, provider: &str, id: &str) -> bool {
        let Some(models) = self.by_provider.get_mut(provider) else {
            return false;
        };
        let removed = models.shift_remove(id).is_some();
        if models.is_empty() {
            self.by_provider.shift_remove(provider);
        }
        removed
    }

    /// Removes every definition registered by `extension_id`.
    pub fn remove_extension(&mut self, extension_id: &str) {
        for models in self.by_provider.values_mut() {
            models.retain(|_, entry| entry.extension_id != extension_id);
        }
        self.by_provider.retain(|_, models| !models.is_empty());
    }

    /// A registered definition.
    #[must_use]
    pub fn get(&self, provider: &str, id: &str) -> Option<&RegisteredVirtualModel> {
        self.by_provider.get(provider)?.get(id)
    }

    /// Catalog entries listed under `provider`.
    pub fn models_for(&self, provider: &str) -> impl Iterator<Item = &Model> {
        self.by_provider
            .get(provider)
            .into_iter()
            .flat_map(|models| models.values().map(|entry| &entry.model))
    }

    /// Providers with at least one virtual model, in registration order.
    pub fn provider_ids(&self) -> impl Iterator<Item = &str> {
        self.by_provider.keys().map(String::as_str)
    }

    /// Every registered virtual model.
    pub fn iter(&self) -> impl Iterator<Item = &RegisteredVirtualModel> {
        self.by_provider.values().flat_map(IndexMap::values)
    }
}

/// Why a request is routed (pi `ModelRouteReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteReason {
    /// First request after a user-written message (prompt, steering, follow-up).
    User,
    /// Any other request of the agent loop.
    Continuation,
    /// Automatic retry after a failed request.
    Retry,
    /// Request outside the agent loop (compaction summary, extension call).
    Direct,
}

/// Physical model and level of the latest successful response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePrevious {
    /// Physical model.
    pub model: Model,
    /// Level it ran with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
}

/// The failed request of a retry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteFailed {
    /// Physical model.
    pub model: Model,
    /// Level it ran with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// The failed assistant message (`stopReason`, `errorMessage`).
    pub message: AssistantMessage,
}

/// pi `ModelRouteRequest` (the abort signal is the host call's cancellation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteRequest {
    /// The selected virtual model.
    pub model: Model,
    /// The selected level (meaning is up to the router).
    pub thinking_level: ThinkingLevel,
    /// Why the request is made.
    pub reason: RouteReason,
    /// Latest successful physical response in `messages`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<RoutePrevious>,
    /// For retries: the failed request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<RouteFailed>,
    /// Router state last stored on the branch (absent for `direct`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<JsonValue>,
    /// Conversation for this request, including system messages.
    pub messages: Vec<Message>,
}

/// `provider/id` named by a route result: either a `"provider/id"` string or a model-like
/// object with `provider` and `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RouteTarget {
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub id: String,
}

impl<'de> Deserialize<'de> for RouteTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Key(String),
            Object { provider: String, id: String },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Object { provider, id } => Ok(Self { provider, id }),
            Raw::Key(key) => key
                .split_once('/')
                .map(|(provider, id)| Self {
                    provider: provider.to_owned(),
                    id: id.to_owned(),
                })
                .ok_or_else(|| {
                    serde::de::Error::custom(format!("route model \"{key}\" is not provider/id"))
                }),
        }
    }
}

fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<JsonValue>, D::Error> {
    JsonValue::deserialize(deserializer).map(Some)
}

/// pi `ModelRoute` as returned by the guest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteResult {
    /// Physical target.
    pub model: RouteTarget,
    /// Requested level (clamped to the target).
    pub thinking_level: ThinkingLevel,
    /// New router state; absent keeps the current state, `null` is stored as `null`.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub state: Option<JsonValue>,
}

/// A validated route: physical model with credentials and clamped level.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelRoute {
    /// Physical model.
    pub model: Model,
    /// Level clamped to `model`.
    pub thinking_level: ThinkingLevel,
    /// State returned by the router.
    pub state: Option<JsonValue>,
}

/// Routing failures (pi texts); the request ends with an error response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    /// The selected virtual model is not registered.
    #[error("Virtual model {provider}/{id} is not registered.")]
    NotRegistered {
        /// Provider id.
        provider: String,
        /// Model id.
        id: String,
    },
    /// The target is unknown or virtual.
    #[error("Virtual model {provider}/{id} routed to {target}, which is not a physical model.")]
    NotPhysical {
        /// Virtual provider.
        provider: String,
        /// Virtual id.
        id: String,
        /// `provider/id` of the target.
        target: String,
    },
    /// The target's provider has no credentials.
    #[error("Virtual model {provider}/{id} routed to {target}, which has no credentials.")]
    NoCredentials {
        /// Virtual provider.
        provider: String,
        /// Virtual id.
        id: String,
        /// `provider/id` of the target.
        target: String,
    },
    /// The router itself failed (guest error or invalid result).
    #[error("{message}")]
    Router {
        /// Error text from the guest.
        message: String,
    },
}

/// pi `findLatestResponse`: the latest assistant message that did not fail or abort.
#[must_use]
pub fn find_latest_response(messages: &[Message]) -> Option<&AssistantMessage> {
    messages.iter().rev().find_map(|message| match message {
        Message::Assistant(assistant)
            if !matches!(
                assistant.stop_reason,
                StopReason::Error | StopReason::Aborted
            ) =>
        {
            Some(assistant)
        }
        _ => None,
    })
}

/// Data of a [`VIRTUAL_MODEL_STATE_ENTRY`] custom entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualModelStateData {
    /// Virtual model provider.
    pub provider: String,
    /// Virtual model id.
    pub model_id: String,
    /// Router state.
    pub state: JsonValue,
}

/// Whether core stores the router state returned for a request before sending it.
///
/// pi stores any returned object except `undefined` and `request.state` itself. Object
/// identity does not survive the WASM boundary, so Oino skips a state equal to the current one.
/// `direct` requests never store state.
#[must_use]
pub fn should_store_route_state(
    reason: RouteReason,
    current: Option<&JsonValue>,
    returned: Option<&JsonValue>,
) -> bool {
    match (reason, returned) {
        (RouteReason::Direct, _) | (_, None) => false,
        (_, Some(returned)) => current != Some(returned),
    }
}

/// A session entry as seen by the branch helpers (core maps `oino-session` entries to this).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BranchEntryRef<'a> {
    /// `model_change`.
    ModelChange {
        /// Provider id.
        provider: &'a str,
        /// Model id.
        model_id: &'a str,
    },
    /// `message` with an assistant message.
    Assistant {
        /// Provider of the response.
        provider: &'a str,
        /// Model of the response.
        model: &'a str,
        /// API of the response (virtual after a failed routing).
        api: &'a str,
    },
    /// `custom` entry.
    Custom {
        /// `customType`.
        custom_type: &'a str,
        /// `data`.
        data: Option<&'a JsonValue>,
    },
    /// Anything else.
    Other,
}

/// A recorded model selection (`provider/modelId`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchSelection {
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model_id: String,
}

/// pi `getBranchSelection`: a virtual `model_change` holds until the next `model_change`;
/// otherwise the latest physical response wins. `is_registered_virtual(provider, id)` reports a
/// currently registered virtual model (unregistered ones fall back to the last response).
#[must_use]
pub fn branch_selection(
    branch: &[BranchEntryRef<'_>],
    is_registered_virtual: impl Fn(&str, &str) -> bool,
) -> Option<BranchSelection> {
    for (index, entry) in branch.iter().enumerate().rev() {
        match *entry {
            BranchEntryRef::ModelChange { provider, model_id } => {
                return Some(BranchSelection {
                    provider: provider.to_owned(),
                    model_id: model_id.to_owned(),
                });
            }
            BranchEntryRef::Assistant {
                provider,
                model,
                api,
            } if !is_virtual_api(api) => {
                let change = branch[..index].iter().rev().find_map(|e| match *e {
                    BranchEntryRef::ModelChange { provider, model_id } => {
                        Some((provider, model_id))
                    }
                    _ => None,
                });
                return Some(match change {
                    Some((p, m)) if is_registered_virtual(p, m) => BranchSelection {
                        provider: p.to_owned(),
                        model_id: m.to_owned(),
                    },
                    _ => BranchSelection {
                        provider: provider.to_owned(),
                        model_id: model.to_owned(),
                    },
                });
            }
            _ => {}
        }
    }
    None
}

/// pi `getVirtualModelState`: the latest router state stored on the branch for a model.
#[must_use]
pub fn virtual_model_state<'a>(
    branch: &[BranchEntryRef<'a>],
    provider: &str,
    model_id: &str,
) -> Option<&'a JsonValue> {
    branch.iter().rev().find_map(|entry| match *entry {
        BranchEntryRef::Custom {
            custom_type,
            data: Some(data),
        } if custom_type == VIRTUAL_MODEL_STATE_ENTRY
            && data.get("provider").and_then(JsonValue::as_str) == Some(provider)
            && data.get("modelId").and_then(JsonValue::as_str) == Some(model_id) =>
        {
            data.get("state")
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn definition() -> VirtualModelDefinition {
        serde_json::from_value(json!({"provider": "router", "id": "auto", "name": "Auto", "thinkingLevels": ["low", "high"]}))
            .unwrap()
    }

    #[test]
    fn catalog_entry_matches_pi() {
        let model = create_virtual_model(&definition());
        assert_eq!(
            serde_json::to_value(&model).unwrap(),
            json!({
                "id": "auto", "name": "Auto", "api": "oino-virtual", "provider": "router", "baseUrl": "",
                "input": ["text", "image"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                "reasoning": true,
                "thinkingLevelMap": {"off": null, "minimal": null, "low": "low", "medium": null, "high": "high", "xhigh": null, "max": null},
                "contextWindow": 0, "maxTokens": 0
            })
        );
        assert_eq!(
            model.available_thinking_levels(),
            vec![ThinkingLevel::Low, ThinkingLevel::High]
        );
    }

    #[test]
    fn registry_conflicts_and_removal() {
        let mut registry = VirtualModelRegistry::default();
        assert_eq!(
            registry
                .register("ext", definition(), true)
                .unwrap_err()
                .to_string(),
            "Virtual model router/auto conflicts with a physical model."
        );
        registry.register("ext", definition(), false).unwrap();
        assert_eq!(registry.models_for("router").count(), 1);
        registry.remove_extension("ext");
        assert!(registry.get("router", "auto").is_none());
    }

    #[test]
    fn route_result_accepts_key_or_object() {
        let a: RouteResult =
            serde_json::from_value(json!({"model": "anthropic/claude", "thinkingLevel": "high"}))
                .unwrap();
        let b: RouteResult = serde_json::from_value(json!({"model": {"provider": "anthropic", "id": "claude"}, "thinkingLevel": "high", "state": null})).unwrap();
        assert_eq!(a.model, b.model);
        assert_eq!(a.state, None);
        assert_eq!(b.state, Some(JsonValue::Null));
    }

    #[test]
    fn branch_selection_rules() {
        let branch = [
            BranchEntryRef::ModelChange {
                provider: "router",
                model_id: "auto",
            },
            BranchEntryRef::Assistant {
                provider: "anthropic",
                model: "claude",
                api: "anthropic-messages",
            },
        ];
        let selection = branch_selection(&branch, |p, m| p == "router" && m == "auto").unwrap();
        assert_eq!(selection.model_id, "auto");
        let selection = branch_selection(&branch, |_, _| false).unwrap();
        assert_eq!(selection.model_id, "claude");
        let state = json!({"provider": "router", "modelId": "auto", "state": {"phase": "plan"}});
        let branch = [BranchEntryRef::Custom {
            custom_type: VIRTUAL_MODEL_STATE_ENTRY,
            data: Some(&state),
        }];
        assert_eq!(
            virtual_model_state(&branch, "router", "auto"),
            Some(&json!({"phase": "plan"}))
        );
    }

    #[test]
    fn state_storage_rules() {
        let current = json!({"a": 1});
        assert!(!should_store_route_state(
            RouteReason::User,
            Some(&current),
            None
        ));
        assert!(!should_store_route_state(
            RouteReason::User,
            Some(&current),
            Some(&json!({"a": 1}))
        ));
        assert!(should_store_route_state(
            RouteReason::User,
            Some(&current),
            Some(&json!({"a": 2}))
        ));
        assert!(!should_store_route_state(
            RouteReason::Direct,
            None,
            Some(&json!(1))
        ));
    }
}
