//! Model and thinking-level control (pi `setModel`, `cycleModel`, `setThinkingLevel`,
//! `cycleThinkingLevel`, `routedModel`, `_recordSelection`, virtual routing).

use oino_ai::{JsonValue, Message, Model, ThinkingLevel};
use oino_models::resolver::{
    ThinkingSwitch, next_available_model, next_scoped_model, thinking_level_for_model_switch,
};
use oino_models::virtual_models::{
    BranchEntryRef, ModelRoute, RouteReason, RouteResult, VIRTUAL_MODEL_STATE_ENTRY,
    VirtualModelStateData, branch_selection, find_latest_response, is_virtual_api,
    should_store_route_state, virtual_model_state,
};
use oino_session::{EntryKind, SessionEntry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, SessionOpError, lock};
use crate::events::{AgentSessionEvent, ModelChangeSource};
use crate::extensions::api::CallKind;

pub use oino_models::resolver::CycleDirection;

/// pi `ModelCycleResult`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleModelResult {
    /// New model.
    pub model: Model,
    /// New thinking level.
    pub thinking_level: ThinkingLevel,
    /// Cycled through the `--models` scope.
    pub is_scoped: bool,
}

/// Branch entries as the virtual-model helpers see them.
pub(crate) fn branch_refs(branch: &[SessionEntry]) -> Vec<BranchEntryRef<'_>> {
    branch
        .iter()
        .map(|entry| match &entry.kind {
            EntryKind::ModelChange { provider, model_id } => {
                BranchEntryRef::ModelChange { provider, model_id }
            }
            EntryKind::Message {
                message: oino_ai::AgentMessage::Assistant(assistant),
            } => BranchEntryRef::Assistant {
                provider: &assistant.provider,
                model: &assistant.model,
                api: &assistant.api,
            },
            EntryKind::Custom { custom_type, data } => BranchEntryRef::Custom {
                custom_type,
                data: data.as_ref(),
            },
            _ => BranchEntryRef::Other,
        })
        .collect()
}

impl AgentSession {
    /// Under a virtual selection: the physical model of the latest successful response.
    #[must_use]
    pub fn routed_model(&self) -> Option<Model> {
        let model = self.model()?;
        if !is_virtual_api(&model.api) {
            return None;
        }
        let llm = oino_session::convert_to_llm(&self.messages());
        let latest = find_latest_response(&llm)?;
        self.inner
            .models
            .find_physical(&latest.provider, &latest.model)
    }

    /// Thinking levels the current model supports (all pi levels without a model).
    #[must_use]
    pub fn available_thinking_levels(&self) -> Vec<ThinkingLevel> {
        self.model().map_or_else(
            || ThinkingLevel::ALL.to_vec(),
            |model| model.available_thinking_levels(),
        )
    }

    fn thinking_for_switch(
        &self,
        target: Option<&Model>,
        explicit: Option<ThinkingLevel>,
    ) -> ThinkingLevel {
        let settings = lock(&self.inner.settings).snapshot();
        let levels = settings.model_thinking_levels();
        thinking_level_for_model_switch(
            target,
            &ThinkingSwitch {
                explicit,
                model_thinking_levels: Some(&levels),
                default_thinking_level: settings.default_thinking_level(),
                current: Some(self.thinking_level()),
            },
        )
    }

    async fn emit_model_select(&self, next: &Model, previous: Option<&Model>, source: &str) {
        if previous.is_some_and(|p| p.provider == next.provider && p.id == next.id) {
            return;
        }
        let runner = self.runner();
        if runner.has_handlers("model_select") {
            let mut payload = json!({"type": "model_select", "model": next, "source": source});
            if let Some(previous) = previous {
                payload["previousModel"] = json!(previous);
            }
            runner.emit(payload).await;
        }
    }

    fn apply_model(&self, model: &Model, persist: bool) -> Result<(), SessionOpError> {
        self.inner.agent.set_model(model.clone());
        lock(&self.inner.manager)
            .append_model_change(&model.provider, &model.id)
            .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        if persist {
            let result = self
                .with_settings(|s| s.set_default_model_and_provider(&model.provider, &model.id));
            if let Err(error) = result {
                tracing::warn!("saving default model failed: {error}");
            }
            self.add_persisted_default_to_scope(model);
        }
        Ok(())
    }

    fn add_persisted_default_to_scope(&self, model: &Model) {
        let mut scope = lock(&self.inner.scoped_models);
        if scope.is_empty()
            || scope
                .iter()
                .any(|s| s.model.provider == model.provider && s.model.id == model.id)
        {
            return;
        }
        scope.push(oino_models::resolver::ScopedModel {
            model: model.clone(),
            thinking_level: None,
        });
        drop(scope);
        let enabled = lock(&self.inner.settings).settings().enabled_models();
        let Some(mut enabled) = enabled.filter(|e| !e.is_empty()) else {
            return;
        };
        let reference = format!("{}/{}", model.provider, model.id);
        if enabled.iter().any(|p| p.eq_ignore_ascii_case(&reference)) {
            return;
        }
        enabled.push(reference);
        if let Err(error) = self.with_settings(|s| s.set_enabled_models(Some(&enabled))) {
            tracing::warn!("saving enabledModels failed: {error}");
        }
    }

    /// pi `setModel`: validates auth, records `model_change`, applies the thinking level.
    ///
    /// # Errors
    ///
    /// `No API key for {provider}/{id}` when the provider has no credentials.
    pub async fn set_model(&self, model: Model, persist: bool) -> Result<Model, SessionOpError> {
        if self.inner.models.check_auth(&model.provider).is_none() {
            return Err(SessionOpError::Failed(format!(
                "No API key for {}/{}",
                model.provider, model.id
            )));
        }
        let previous = self.model();
        let thinking = self.thinking_for_switch(Some(&model), None);
        self.apply_model(&model, persist)?;
        self.set_thinking_level(thinking, false).await;
        self.emit_model_select(&model, previous.as_ref(), "set")
            .await;
        self.emit_model_changed(ModelChangeSource::Set);
        Ok(model)
    }

    /// pi `cycleModel`.
    ///
    /// # Errors
    ///
    /// Persisting the selection failed.
    pub async fn cycle_model(
        &self,
        direction: CycleDirection,
        persist: bool,
    ) -> Result<Option<CycleModelResult>, SessionOpError> {
        let available = self.inner.models.available();
        let current = self.model();
        let scope = self.scoped_models();
        let (next, explicit, is_scoped) = if scope.is_empty() {
            match next_available_model(&available, current.as_ref(), direction) {
                Some(model) => (model, None, false),
                None => return Ok(None),
            }
        } else {
            match next_scoped_model(&scope, &available, current.as_ref(), direction) {
                Some(scoped) => (scoped.model, scoped.thinking_level, true),
                None => return Ok(None),
            }
        };
        let thinking = self.thinking_for_switch(Some(&next), explicit);
        self.apply_model(&next, persist)?;
        self.set_thinking_level(thinking, false).await;
        self.emit_model_select(&next, current.as_ref(), "cycle")
            .await;
        self.emit_model_changed(ModelChangeSource::Cycle);
        Ok(Some(CycleModelResult {
            model: next,
            thinking_level: self.thinking_level(),
            is_scoped,
        }))
    }

    /// pi `setThinkingLevel`: clamps, records only changes; returns the effective level.
    pub async fn set_thinking_level(&self, level: ThinkingLevel, persist: bool) -> ThinkingLevel {
        let available = self.available_thinking_levels();
        let effective = if available.contains(&level) {
            level
        } else if self.model().is_some() {
            level.clamp(&available)
        } else {
            ThinkingLevel::Off
        };
        let previous = self.thinking_level();
        self.inner.agent.set_thinking_level(effective);
        if persist && let Err(error) = self.with_settings(|s| s.set_default_thinking_level(level)) {
            tracing::warn!("saving defaultThinkingLevel failed: {error}");
        }
        if effective != previous {
            if let Err(error) =
                lock(&self.inner.manager).append_thinking_level_change(&effective.to_string())
            {
                tracing::error!("persisting thinking level failed: {error}");
            }
            self.emit(AgentSessionEvent::ThinkingLevelChanged { level: effective });
            let runner = self.runner();
            if runner.has_handlers("thinking_level_select") {
                runner
                    .emit(json!({"type": "thinking_level_select", "level": effective, "previousLevel": previous}))
                    .await;
            }
        }
        effective
    }

    /// pi `cycleThinkingLevel`; `None` when the model has no reasoning.
    pub async fn cycle_thinking_level(&self, persist: bool) -> Option<ThinkingLevel> {
        if !self.model().is_some_and(|m| m.reasoning) {
            return None;
        }
        let levels = self.available_thinking_levels();
        let index = levels.iter().position(|l| *l == self.thinking_level());
        let next = levels[index.map_or(0, |i| (i + 1) % levels.len())];
        self.set_thinking_level(next, persist).await;
        Some(next)
    }

    /// pi `_recordSelection`: appends `model_change` when the branch implies another model.
    pub(crate) fn record_selection(&self) {
        let Some(model) = self.model() else {
            return;
        };
        let models = &self.inner.models;
        let recorded = {
            let manager = lock(&self.inner.manager);
            let branch: Vec<SessionEntry> = manager.branch(None).into_iter().cloned().collect();
            branch_selection(&branch_refs(&branch), |provider, id| {
                models.virtual_model(provider, id).is_some()
            })
        };
        let Some(recorded) = recorded else {
            return;
        };
        if recorded.provider == model.provider && recorded.model_id == model.id {
            return;
        }
        let recorded_virtual = models
            .find(&recorded.provider, &recorded.model_id)
            .is_some_and(|m| is_virtual_api(&m.api));
        if !is_virtual_api(&model.api) && !recorded_virtual {
            return;
        }
        if let Err(error) =
            lock(&self.inner.manager).append_model_change(&model.provider, &model.id)
        {
            tracing::error!("recording model selection failed: {error}");
        }
    }

    /// Routes a virtual model through its extension (pi `ModelRuntime.resolveModel`), storing
    /// router state on the branch.
    pub(crate) async fn route_virtual(
        &self,
        model: &Model,
        messages: Vec<Message>,
        reason: RouteReason,
        thinking: ThinkingLevel,
        failed: Option<oino_ai::AssistantMessage>,
        _cancel: &CancellationToken,
    ) -> Result<ModelRoute, String> {
        let state: Option<JsonValue> = {
            let manager = lock(&self.inner.manager);
            let branch: Vec<SessionEntry> = manager.branch(None).into_iter().cloned().collect();
            virtual_model_state(&branch_refs(&branch), &model.provider, &model.id).cloned()
        };
        let request = self
            .inner
            .models
            .route_request(model, messages, reason, thinking, failed, state.clone())
            .map_err(|e| e.to_string())?;
        let registered = self
            .inner
            .models
            .virtual_model(&model.provider, &model.id)
            .ok_or_else(|| {
                format!(
                    "Virtual model {}/{} is not registered.",
                    model.provider, model.id
                )
            })?;
        let runner = self.runner();
        let extension = runner
            .set()
            .get(&registered.extension_id)
            .cloned()
            .ok_or_else(|| format!("Extension {} is not loaded", registered.extension_id))?;
        let cx = runner.call_context().with_kind(CallKind::Service);
        let response = extension
            .instance
            .route_model(json!(request), cx)
            .await
            .map_err(|e| e.message)?;
        let result: RouteResult = serde_json::from_value(response).map_err(|e| e.to_string())?;
        let route = self
            .inner
            .models
            .validate_route(model, result)
            .map_err(|e| e.to_string())?;
        if should_store_route_state(reason, state.as_ref(), route.state.as_ref()) {
            let data = VirtualModelStateData {
                provider: model.provider.clone(),
                model_id: model.id.clone(),
                state: route.state.clone().unwrap_or(JsonValue::Null),
            };
            let appended = lock(&self.inner.manager)
                .append_custom_entry(VIRTUAL_MODEL_STATE_ENTRY, serde_json::to_value(data).ok());
            if let Ok(id) = appended {
                self.publish_entry(&id);
            }
        }
        Ok(route)
    }
}
