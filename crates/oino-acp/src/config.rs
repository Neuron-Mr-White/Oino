//! Session config options (`configOptions`, `session/set_config_option`, modes §6.2).

use oino_agent::QueueMode;
use oino_ai::{JsonValue, ThinkingLevel};
use oino_core::session::AgentSession;
use serde_json::json;

use crate::error::AcpError;

/// Option ids.
pub const MODEL: &str = "model";
/// Thinking level option id.
pub const THINKING: &str = "thinking";
/// Steering queue mode option id.
pub const STEERING_MODE: &str = "steering_mode";
/// Follow-up queue mode option id.
pub const FOLLOW_UP_MODE: &str = "follow_up_mode";
/// Auto-compaction option id.
pub const AUTO_COMPACTION: &str = "auto_compaction";
/// Auto-retry option id.
pub const AUTO_RETRY: &str = "auto_retry";

/// `provider/id` value of a model.
#[must_use]
pub fn model_value(provider: &str, id: &str) -> String {
    format!("{provider}/{id}")
}

fn wire<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn queue_modes() -> JsonValue {
    json!([
        {"value": "one-at-a-time", "name": "One at a time"},
        {"value": "all", "name": "All at once"},
    ])
}

/// Every config option of `session` with current values (wire shape).
#[must_use]
pub fn config_options(session: &AgentSession) -> Vec<JsonValue> {
    let mut options = Vec::new();
    let current = session.model();
    let mut models: Vec<JsonValue> = session
        .models()
        .available()
        .into_iter()
        .map(|model| {
            json!({
                "value": model_value(&model.provider, &model.id),
                "name": model.name,
                "description": model.provider,
            })
        })
        .collect();
    if let Some(model) = &current {
        let value = model_value(&model.provider, &model.id);
        if !models.iter().any(|m| m["value"] == json!(value)) {
            models.insert(
                0,
                json!({"value": value, "name": model.name, "description": model.provider}),
            );
        }
        options.push(json!({
            "id": MODEL, "name": "Model", "category": "model", "type": "select",
            "currentValue": value, "options": models,
        }));
    }
    let levels: Vec<JsonValue> = session
        .available_thinking_levels()
        .into_iter()
        .map(|level| json!({"value": wire(level), "name": wire(level)}))
        .collect();
    options.push(json!({
        "id": THINKING, "name": "Thinking", "category": "thought_level", "type": "select",
        "currentValue": wire(session.thinking_level()), "options": levels,
    }));
    options.push(json!({
        "id": STEERING_MODE, "name": "Steering queue", "type": "select",
        "description": "How queued steering messages are delivered",
        "currentValue": wire(session.steering_mode()), "options": queue_modes(),
    }));
    options.push(json!({
        "id": FOLLOW_UP_MODE, "name": "Follow-up queue", "type": "select",
        "description": "How queued follow-up messages are delivered",
        "currentValue": wire(session.follow_up_mode()), "options": queue_modes(),
    }));
    let (compaction, retry) = session.with_settings(|s| {
        (
            s.settings().compaction_enabled(),
            s.settings().retry_enabled(),
        )
    });
    options.push(json!({
        "id": AUTO_COMPACTION, "name": "Auto-compaction", "type": "boolean",
        "currentValue": compaction,
    }));
    options.push(json!({
        "id": AUTO_RETRY, "name": "Auto-retry", "type": "boolean", "currentValue": retry,
    }));
    options
}

fn parse<T: serde::de::DeserializeOwned>(id: &str, value: &JsonValue) -> Result<T, AcpError> {
    serde_json::from_value(value.clone())
        .map_err(|_| AcpError::InvalidParams(format!("Invalid value {value} for {id}")))
}

fn boolean(id: &str, value: &JsonValue) -> Result<bool, AcpError> {
    value
        .as_bool()
        .ok_or_else(|| AcpError::InvalidParams(format!("{id} takes a boolean")))
}

/// Applies `session/set_config_option` (`value` is the wire value: string id or boolean).
///
/// # Errors
///
/// [`AcpError::InvalidParams`] for unknown options or values, [`AcpError::Command`] when the
/// session rejects the change.
pub async fn set_config_option(
    session: &AgentSession,
    id: &str,
    value: &JsonValue,
) -> Result<(), AcpError> {
    match id {
        MODEL => {
            let wanted = value.as_str().unwrap_or_default();
            let model = session
                .models()
                .available()
                .into_iter()
                .find(|m| model_value(&m.provider, &m.id) == wanted)
                .ok_or_else(|| AcpError::InvalidParams(format!("Model not found: {wanted}")))?;
            session
                .set_model(model, true)
                .await
                .map_err(|e| AcpError::Command(e.to_string()))?;
        }
        THINKING => {
            let level: ThinkingLevel = parse(id, value)?;
            session.set_thinking_level(level, true).await;
        }
        STEERING_MODE => session.set_steering_mode(parse::<QueueMode>(id, value)?),
        FOLLOW_UP_MODE => session.set_follow_up_mode(parse::<QueueMode>(id, value)?),
        AUTO_COMPACTION => session.set_auto_compaction_enabled(boolean(id, value)?),
        AUTO_RETRY => session.set_auto_retry_enabled(boolean(id, value)?),
        other => {
            return Err(AcpError::InvalidParams(format!(
                "Unknown config option: {other}"
            )));
        }
    }
    Ok(())
}

/// `session/set_mode`: pi-acp compatibility, the mode id is a thinking level.
///
/// # Errors
///
/// [`AcpError::InvalidParams`] for an unknown level.
pub async fn set_mode(session: &AgentSession, mode_id: &str) -> Result<(), AcpError> {
    let level: ThinkingLevel = parse("mode", &json!(mode_id))?;
    session.set_thinking_level(level, true).await;
    Ok(())
}
