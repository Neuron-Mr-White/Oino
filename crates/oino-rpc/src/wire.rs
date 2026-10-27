//! Client events on the wire (pi `json-event.ts` + RPC event filtering, modes.md §2.2/§3.1).

use oino_ai::{JsonObject, JsonValue};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};
use oino_core::ui::is_dialog;

/// pi dialog methods (`rpc-extension-ui.md`).
const PI_DIALOGS: [&str; 4] = ["select", "confirm", "input", "editor"];
/// pi fire-and-forget methods.
const PI_FIRE: [&str; 5] = [
    "notify",
    "setStatus",
    "setWidget",
    "setTitle",
    "set_editor_text",
];

/// What to do with one client event for a connection.
#[derive(Clone, Debug, PartialEq)]
pub enum WireEvent {
    /// Write this record.
    Line(JsonValue),
    /// Not for this client.
    Drop,
    /// A dialog the client cannot render: resolve it as cancelled.
    CancelDialog(String),
}

/// pi `toJsonEvent` for a session event: `message_update` loses the cumulative `message`
/// and every `partial`, gains top-level `usage`; `toolcall_start` carries `id`/`toolName`.
#[must_use]
pub fn session_event_json(event: &AgentSessionEvent) -> JsonValue {
    let value = serde_json::to_value(event).unwrap_or(JsonValue::Null);
    match value {
        JsonValue::Object(object)
            if object.get("type").and_then(JsonValue::as_str) == Some("message_update") =>
        {
            JsonValue::Object(message_update(object))
        }
        other => other,
    }
}

fn message_update(mut object: JsonObject) -> JsonObject {
    let usage = object
        .shift_remove("message")
        .and_then(|mut message| message.get_mut("usage").map(JsonValue::take))
        .unwrap_or(JsonValue::Null);
    let mut event = match object.shift_remove("assistantMessageEvent") {
        Some(JsonValue::Object(event)) => event,
        _ => JsonObject::new(),
    };
    let partial = event.shift_remove("partial");
    if event.get("type").and_then(JsonValue::as_str) == Some("toolcall_start") {
        let block = event
            .get("contentIndex")
            .and_then(JsonValue::as_u64)
            .and_then(|index| {
                let index = usize::try_from(index).ok()?;
                partial.as_ref()?.get("content")?.get(index).cloned()
            });
        if let Some(block) = block {
            event.insert(
                "id".to_owned(),
                block.get("id").cloned().unwrap_or(JsonValue::Null),
            );
            event.insert(
                "toolName".to_owned(),
                block.get("name").cloned().unwrap_or(JsonValue::Null),
            );
        }
    }
    let mut out = JsonObject::new();
    out.insert("type".to_owned(), JsonValue::from("message_update"));
    out.insert("usage".to_owned(), usage);
    out.insert("assistantMessageEvent".to_owned(), JsonValue::Object(event));
    out
}

/// Maps one client event for an RPC connection. Without `_oino/hello` (`oino == false`)
/// only pi records go out: pi UI methods without the Oino `extensionId`/`sessionId`
/// fields, `extension_error` without `stack`, no Oino events; Oino-only dialogs resolve as
/// cancelled.
#[must_use]
pub fn rpc_event(event: &ClientEvent, oino: bool) -> WireEvent {
    match event {
        ClientEvent::Session(event) => WireEvent::Line(session_event_json(event)),
        ClientEvent::Ui(request) => {
            let mut value = serde_json::to_value(request).unwrap_or(JsonValue::Null);
            if oino {
                return WireEvent::Line(value);
            }
            let method = request.method.as_str();
            if PI_DIALOGS.contains(&method) || PI_FIRE.contains(&method) {
                if let Some(object) = value.as_object_mut() {
                    object.shift_remove("extensionId");
                    object.shift_remove("sessionId");
                }
                WireEvent::Line(value)
            } else if is_dialog(method) {
                WireEvent::CancelDialog(request.id.clone())
            } else {
                WireEvent::Drop
            }
        }
        ClientEvent::ExtensionError(error) => {
            let mut value = serde_json::to_value(error).unwrap_or(JsonValue::Null);
            if !oino && let Some(object) = value.as_object_mut() {
                object.shift_remove("stack");
            }
            WireEvent::Line(value)
        }
        ClientEvent::Oino(_) if !oino => WireEvent::Drop,
        ClientEvent::Oino(OinoEvent::AgentEvent { agent_id, event }) => {
            WireEvent::Line(agent_event_json(agent_id, event))
        }
        ClientEvent::Oino(event) => {
            WireEvent::Line(serde_json::to_value(event).unwrap_or(JsonValue::Null))
        }
    }
}

/// `agent_event` with the nested session event in wire form (`message_update` deltas only,
/// like top-level events).
#[must_use]
pub fn agent_event_json(agent_id: &str, event: &ClientEvent) -> JsonValue {
    let inner = match event {
        ClientEvent::Session(event) => session_event_json(event),
        other => serde_json::to_value(other).unwrap_or(JsonValue::Null),
    };
    serde_json::json!({"type": "agent_event", "agentId": agent_id, "event": inner})
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_ai::{
        AgentMessage, AssistantContent, AssistantMessage, AssistantMessageEvent, ToolCall,
    };
    use oino_core::events::OinoEvent;
    use oino_core::ui::UiRequest;
    use serde_json::json;

    fn partial() -> AssistantMessage {
        let mut message = AssistantMessage::empty("faux", "faux", "m", 1);
        message.content = vec![AssistantContent::ToolCall(ToolCall {
            id: "c1".into(),
            name: "read".into(),
            ..ToolCall::default()
        })];
        message
    }

    #[test]
    fn message_update_is_delta_only() {
        let event = AgentSessionEvent::MessageUpdate {
            message: AgentMessage::Assistant(partial()),
            assistant_message_event: AssistantMessageEvent::ToolcallStart {
                content_index: 0,
                partial: partial(),
            },
        };
        let value = session_event_json(&event);
        assert_eq!(value["type"], "message_update");
        assert!(value.get("message").is_none());
        assert_eq!(
            value["usage"],
            serde_json::to_value(partial().usage).unwrap()
        );
        assert_eq!(
            value["assistantMessageEvent"],
            json!({"type":"toolcall_start","contentIndex":0,"id":"c1","toolName":"read"})
        );
    }

    fn ui(method: &str) -> ClientEvent {
        ClientEvent::Ui(UiRequest {
            id: "u1".into(),
            method: method.into(),
            params: serde_json::from_value(json!({"title": "t"})).unwrap(),
            extension_id: Some("ext".into()),
            session_id: Some("s".into()),
        })
    }

    #[test]
    fn pi_clients_get_pi_records_only() {
        assert_eq!(
            rpc_event(&ui("select"), false),
            WireEvent::Line(
                json!({"type":"extension_ui_request","id":"u1","method":"select","title":"t"})
            )
        );
        assert_eq!(
            rpc_event(&ui("form"), false),
            WireEvent::CancelDialog("u1".into())
        );
        assert_eq!(rpc_event(&ui("setFrame"), false), WireEvent::Drop);
        assert!(matches!(rpc_event(&ui("form"), true), WireEvent::Line(_)));
        let dismiss = ClientEvent::Oino(OinoEvent::CommandsChanged);
        assert_eq!(rpc_event(&dismiss, false), WireEvent::Drop);
        assert_eq!(
            rpc_event(&dismiss, true),
            WireEvent::Line(json!({"type":"commands_changed"}))
        );
    }

    #[test]
    fn nested_agent_events_travel_as_deltas_and_decode_back() {
        let start = ClientEvent::Oino(OinoEvent::AgentEvent {
            agent_id: "child".into(),
            event: Box::new(ClientEvent::Session(AgentSessionEvent::MessageStart {
                message: AgentMessage::Assistant(AssistantMessage::empty("faux", "faux", "m", 1)),
            })),
        });
        let update = ClientEvent::Oino(OinoEvent::AgentEvent {
            agent_id: "child".into(),
            event: Box::new(ClientEvent::Session(AgentSessionEvent::MessageUpdate {
                message: AgentMessage::Assistant(partial()),
                assistant_message_event: AssistantMessageEvent::ToolcallStart {
                    content_index: 0,
                    partial: partial(),
                },
            })),
        });
        let WireEvent::Line(wire) = rpc_event(&update, true) else {
            panic!("dropped");
        };
        assert_eq!(wire["type"], "agent_event");
        assert_eq!(wire["agentId"], "child");
        assert!(wire["event"].get("message").is_none());
        assert_eq!(rpc_event(&update, false), WireEvent::Drop);
        let mut decoder = crate::backend::EventDecoder::default();
        let WireEvent::Line(first) = rpc_event(&start, true) else {
            panic!("dropped");
        };
        assert!(decoder.decode(first).is_some());
        let Some(ClientEvent::Oino(OinoEvent::AgentEvent { agent_id, event })) =
            decoder.decode(wire)
        else {
            panic!("not decoded");
        };
        assert_eq!(agent_id, "child");
        let ClientEvent::Session(AgentSessionEvent::MessageUpdate {
            message: AgentMessage::Assistant(rebuilt),
            ..
        }) = *event
        else {
            panic!("not a message update");
        };
        assert_eq!(rebuilt.content.len(), 1);
        let shutdown = ClientEvent::Oino(OinoEvent::ShutdownRequested);
        assert_eq!(
            rpc_event(&shutdown, true),
            WireEvent::Line(json!({"type": "shutdown_requested"}))
        );
    }
}
