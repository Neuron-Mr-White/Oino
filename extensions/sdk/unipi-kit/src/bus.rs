//! Typed bus helpers over the host `bus` (unipi `bus.emit/get/on`, audit-unipi §1.3.1).
//!
//! unipi's in-process bus becomes the host bus: [`emit`] publishes sticky topics sticky
//! automatically, [`get`] reads the last sticky value, and deliveries arrive as `oino_bus`
//! events decoded with [`Delivery::from_event`]. Subscribing is per instance
//! ([`subscribe`]); the host drops sticky state and subscriptions with the session, so
//! publishers republish on `session_start` as in unipi. No function here fails: bus errors are
//! cosmetic (unipi "no bus function ever throws").

use oino_extension::events::Event;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::topics::{self, Topic};

/// Publishes a typed payload on its topic (sticky when the topic is sticky).
pub fn emit<T: Topic>(payload: &T) {
    emit_raw(T::NAME, payload);
}

/// Publishes any payload on `topic` (sticky when [`topics::is_sticky`]).
pub fn emit_raw(topic: &str, payload: &impl Serialize) {
    let _ = oino_extension::bus::publish(topic, payload, topics::is_sticky(topic));
}

/// Last sticky value of a typed topic (`None` for one-shot topics or undecodable values).
#[must_use]
pub fn get<T: Topic>() -> Option<T> {
    get_raw(T::NAME).and_then(|v| serde_json::from_value(v).ok())
}

/// Last sticky value of `topic`.
#[must_use]
pub fn get_raw(topic: &str) -> Option<Value> {
    oino_extension::bus::get(topic)
}

/// Subscribes to every topic in `topics` (exact names or `prefix:*`).
pub fn subscribe(patterns: &[&str]) {
    for p in patterns {
        oino_extension::bus::subscribe(p);
    }
}

/// One `oino_bus` delivery (`{type, topic, data, sticky, publisher}`).
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    /// Topic.
    pub topic: String,
    /// Payload.
    pub data: Value,
    /// Published sticky.
    pub sticky: bool,
    /// Publishing extension id.
    pub publisher: Option<String>,
}

impl Delivery {
    /// Decodes an `oino_bus` payload.
    #[must_use]
    pub fn from_payload(payload: &Value) -> Option<Self> {
        Some(Self {
            topic: payload.get("topic")?.as_str()?.to_owned(),
            data: payload.get("data").cloned().unwrap_or(Value::Null),
            sticky: payload
                .get("sticky")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            publisher: payload
                .get("publisher")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }

    /// Decodes an `oino_bus` event.
    #[must_use]
    pub fn from_event(event: &Event) -> Option<Self> {
        Self::from_payload(&event.payload)
    }

    /// The delivery is on `T`'s topic and decodes as `T`.
    #[must_use]
    pub fn decode<T: Topic>(&self) -> Option<T> {
        (self.topic == T::NAME)
            .then(|| serde_json::from_value(self.data.clone()).ok())
            .flatten()
    }

    /// Decodes the payload as any type.
    #[must_use]
    pub fn data_as<T: DeserializeOwned>(&self) -> Option<T> {
        serde_json::from_value(self.data.clone()).ok()
    }
}

/// Where a unipi event name is observed (unipi name routing used by notify and hints).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route<'a> {
    /// A pi lifecycle event (`session_start`, `tool_call`, …), by wire name.
    Pi(&'a str),
    /// A unipi bus topic (`unipi:*`).
    Unipi(&'a str),
    /// Another bus topic (foreign channel or extension-private `prefix:name`).
    Foreign(&'a str),
}

/// pi 1.4.0 extension event wire names (registry `event-kind`, minus `oino_*`).
pub const PI_EVENTS: &[&str] = &[
    "project_trust",
    "resources_discover",
    "mcp_servers_change",
    "session_start",
    "session_info_changed",
    "session_before_switch",
    "session_before_fork",
    "session_before_compact",
    "session_compact",
    "session_compact_failed",
    "session_shutdown",
    "session_before_tree",
    "session_tree",
    "context",
    "context_with_system",
    "cache_warming_decision",
    "before_provider_headers",
    "before_provider_request",
    "after_provider_response",
    "provider_stream_event",
    "before_agent_start",
    "agent_start",
    "agent_end",
    "agent_before_settle",
    "agent_settled",
    "ui_prompt_start",
    "ui_prompt_end",
    "turn_start",
    "turn_end",
    "message_start",
    "message_update",
    "message_end",
    "tool_execution_start",
    "tool_execution_update",
    "tool_execution_end",
    "model_select",
    "thinking_level_select",
    "tool_call",
    "tool_result",
    "user_bash",
    "input",
];

/// Routes an event name: pi lifecycle names → pi events, `unipi:*` → bus, else foreign.
#[must_use]
pub fn route(name: &str) -> Route<'_> {
    if PI_EVENTS.contains(&name) {
        Route::Pi(name)
    } else if name.starts_with("unipi:") {
        Route::Unipi(name)
    } else {
        Route::Foreign(name)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::topics::{FusionStatus, WorkflowStatus};

    #[test]
    fn decodes_deliveries() {
        let d = Delivery::from_payload(&json!({
            "type": "oino_bus", "topic": "unipi:workflow:status",
            "data": {"planMode": true, "permissionMode": "ask"}, "sticky": true, "publisher": "unipi.workflow"
        }));
        let Some(d) = d else { unreachable!("decodes") };
        assert!(d.sticky);
        assert_eq!(d.publisher.as_deref(), Some("unipi.workflow"));
        assert_eq!(
            d.decode::<WorkflowStatus>(),
            Some(WorkflowStatus {
                plan_mode: true,
                permission_mode: Some("ask".into())
            })
        );
        assert_eq!(d.decode::<Option<FusionStatus>>(), None, "other topic");
        assert!(Delivery::from_payload(&json!({"type": "oino_bus"})).is_none());
    }

    #[test]
    fn cleared_fusion_status_decodes_as_none() {
        let d = Delivery::from_payload(&json!({"topic": "unipi:fusion:status", "data": null}));
        assert_eq!(
            d.and_then(|d| d.decode::<Option<FusionStatus>>()),
            Some(None)
        );
    }

    #[test]
    fn routes_names() {
        assert_eq!(route("tool_call"), Route::Pi("tool_call"));
        assert_eq!(
            route("unipi:memory:stored"),
            Route::Unipi("unipi:memory:stored")
        );
        assert_eq!(route("hints:long-bash"), Route::Foreign("hints:long-bash"));
        assert_eq!(PI_EVENTS.len(), 41);
    }
}
