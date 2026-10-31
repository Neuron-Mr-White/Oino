//! Client events on the wire (pi `json-event.ts` + RPC event filtering, modes.md §2.2/§3.1;
//! ui-protocol v2 §9.2/§10).
//!
//! A connection declares itself in `_oino/hello`: without it only pi records go out; with
//! `ui.protocol: 2` the connection gets registry patches (`oino_ui_patch`) instead of the
//! retained `extension_ui_request` stream, degraded to what it declared.

use oino_ai::{JsonObject, JsonValue};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};
use oino_core::ui::is_dialog;
use oino_ui::plain::to_lines;
use oino_ui::slot::{RegistryOp, Slot, UiCapabilities, UiPatch, UiView};

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

/// What a connection declared (`_oino/hello`, §10).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connection {
    /// `_oino/hello` was received.
    pub oino: bool,
    /// UI capabilities (`ui` field of `_oino/hello`); `None` for pi clients.
    pub ui: Option<UiCapabilities>,
}

impl Connection {
    /// A pi client (no `_oino/hello`).
    #[must_use]
    pub const fn pi() -> Self {
        Self {
            oino: false,
            ui: None,
        }
    }

    /// An Oino client without UI capabilities (v1 `_oino/hello`).
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            oino: true,
            ui: None,
        }
    }

    /// A v2 client with the capabilities it declared.
    #[must_use]
    pub fn v2(ui: UiCapabilities) -> Self {
        Self {
            oino: true,
            ui: Some(ui),
        }
    }

    /// Whether the connection speaks the v2 registry wire (§9.2).
    #[must_use]
    pub fn is_v2(&self) -> bool {
        self.ui.as_ref().is_some_and(|ui| ui.protocol >= 2)
    }
}

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

/// Maps one client event for an RPC connection.
///
/// Pi clients get pi records only (pi UI methods without the Oino `extensionId`/
/// `sessionId` fields, `extension_error` without `stack`, no Oino events; Oino-only
/// dialogs resolve as cancelled). v2 clients get the registry wire: `oino_ui_patch`
/// degraded to their declared capabilities (§10) plus `extension_error`.
#[must_use]
pub fn rpc_event(event: &ClientEvent, connection: &Connection) -> WireEvent {
    let oino = connection.oino;
    let v2 = connection.is_v2();
    match event {
        ClientEvent::Session(event) => WireEvent::Line(session_event_json(event)),
        ClientEvent::Ui(request) => {
            if v2 {
                // Dialogs and effects travel in the patch (§9.2); their answers still
                // arrive as `extension_ui_response`.
                return WireEvent::Drop;
            }
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
        ClientEvent::Oino(event) => {
            if let Some(patch) = event.ui_patch() {
                if !v2 {
                    return WireEvent::Drop;
                }
                return match connection
                    .ui
                    .as_ref()
                    .and_then(|ui| degrade_patch(&patch, ui))
                {
                    Some(degraded) if !degraded.ops.is_empty() => WireEvent::Line(
                        serde_json::to_value(OinoEvent::from(degraded)).unwrap_or(JsonValue::Null),
                    ),
                    _ => WireEvent::Drop,
                };
            }
            if !oino || (v2 && matches!(event, OinoEvent::ExtensionUiDismiss { .. })) {
                return WireEvent::Drop;
            }
            match event {
                OinoEvent::AgentEvent { agent_id, event } => {
                    WireEvent::Line(agent_event_json(agent_id, event))
                }
                other => WireEvent::Line(serde_json::to_value(other).unwrap_or(JsonValue::Null)),
            }
        }
    }
}

/// §10 degradation: drop what the frontend cannot host, render reserved entries as their
/// fallback (or as plain text for a frontend without a renderer).
#[must_use]
pub fn degrade_patch(patch: &UiPatch, caps: &UiCapabilities) -> Option<UiPatch> {
    let mut ops = Vec::with_capacity(patch.ops.len());
    for op in &patch.ops {
        match op {
            RegistryOp::Put { entry } => {
                let Some(entry) = degrade_entry(entry, caps) else {
                    continue;
                };
                ops.push(RegistryOp::Put { entry });
            }
            RegistryOp::Tree { .. } if caps.nodes != "all" => {}
            other => ops.push(other.clone()),
        }
    }
    Some(UiPatch {
        scope: patch.scope,
        session_id: patch.session_id.clone(),
        rev: patch.rev,
        ops,
    })
}

/// One entry under a frontend's capabilities (§10).
fn degrade_entry(
    entry: &oino_ui::slot::UiEntry,
    caps: &UiCapabilities,
) -> Option<oino_ui::slot::UiEntry> {
    if !caps.slots.contains(&entry.slot) {
        return None;
    }
    let mut entry = entry.clone();
    if let UiView::Reserved {
        component,
        fallback,
        ..
    } = &entry.view
        && !caps.reserved.contains(component)
    {
        let fallback = fallback.as_deref()?;
        entry.view = text_fallback(fallback, caps);
        if matches!(entry.view, UiView::Lines { .. })
            && !matches!(
                entry.slot,
                Slot::Widget | Slot::WidgetBelow | Slot::Status | Slot::Header | Slot::Footer
            )
        {
            return None;
        }
        return Some(entry);
    }
    if caps.nodes != "all"
        && let UiView::Tree { root } = &entry.view
    {
        if !matches!(
            entry.slot,
            Slot::Widget | Slot::WidgetBelow | Slot::Status | Slot::Header | Slot::Footer
        ) {
            return None;
        }
        entry.view = UiView::Lines {
            lines: to_lines(root, 80),
        };
    }
    Some(entry)
}

fn text_fallback(node: &oino_ui::node::Node, caps: &UiCapabilities) -> UiView {
    if caps.nodes == "all" {
        UiView::Tree { root: node.clone() }
    } else {
        UiView::Lines {
            lines: to_lines(node, 80),
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
    use oino_core::ui::{UiRequest, UiResponse, UiView};
    use oino_ui::slot::{ReservedComponent, Scope};
    use oino_ui::widgets::paragraph::Paragraph;
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

    fn entry(slot: Slot, view: UiView) -> oino_ui::slot::UiEntry {
        oino_ui::slot::UiEntry {
            id: "ext/panel-1".into(),
            extension_id: "ext".into(),
            scope: Scope::Session,
            slot,
            order: 0,
            seq: 1,
            title: Some("Panel".into()),
            icon: None,
            badge: None,
            tone: None,
            view,
            keys: Vec::new(),
            focus: None,
            close_on_escape: None,
            size: None,
            rev: 1,
        }
    }

    fn patch(ops: Vec<RegistryOp>) -> ClientEvent {
        ClientEvent::Oino(OinoEvent::from(UiPatch {
            scope: Scope::Session,
            session_id: Some("s".into()),
            rev: 1,
            ops,
        }))
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
        let pi = Connection::pi();
        assert_eq!(
            rpc_event(&ui("select"), &pi),
            WireEvent::Line(
                json!({"type":"extension_ui_request","id":"u1","method":"select","title":"t"})
            )
        );
        assert_eq!(
            rpc_event(&ui("form"), &pi),
            WireEvent::CancelDialog("u1".into())
        );
        assert_eq!(rpc_event(&ui("setFrame"), &pi), WireEvent::Drop);
        assert!(matches!(
            rpc_event(&ui("form"), &Connection::v1()),
            WireEvent::Line(_)
        ));
        let dismiss = ClientEvent::Oino(OinoEvent::CommandsChanged);
        assert_eq!(rpc_event(&dismiss, &pi), WireEvent::Drop);
        assert_eq!(
            rpc_event(&dismiss, &Connection::v1()),
            WireEvent::Line(json!({"type":"commands_changed"}))
        );
    }

    #[test]
    fn v2_connections_get_registry_patches() {
        let v2 = Connection::v2(UiCapabilities::full());
        let event = patch(vec![
            RegistryOp::Put {
                entry: entry(
                    Slot::SidePane,
                    UiView::Tree {
                        root: Paragraph::new("hi").into(),
                    },
                ),
            },
            RegistryOp::Dialog {
                request: oino_ui::slot::UiRequest {
                    id: "d1".into(),
                    fields: serde_json::Map::from_iter([(
                        "method".to_owned(),
                        JsonValue::from("select"),
                    )]),
                },
            },
        ]);
        let WireEvent::Line(value) = rpc_event(&event, &v2) else {
            panic!("dropped");
        };
        assert_eq!(value["type"], "oino_ui_patch");
        assert_eq!(value["rev"], 1);
        assert_eq!(value["sessionId"], "s");
        assert_eq!(value["ops"][0]["op"], "put");
        assert_eq!(value["ops"][0]["entry"]["id"], "ext/panel-1");
        assert_eq!(value["ops"][1]["op"], "dialog");
        // A v1 Oino client keeps the pi-shaped records and no patches.
        assert_eq!(rpc_event(&event, &Connection::v1()), WireEvent::Drop);
        // A v2 client gets dialogs through the patch, not as a second record.
        assert_eq!(rpc_event(&ui("select"), &v2), WireEvent::Drop);
        assert_eq!(
            rpc_event(&ui("select"), &Connection::v1()),
            WireEvent::Line(
                json!({"type":"extension_ui_request","id":"u1","method":"select","title":"t",
                       "extensionId":"ext","sessionId":"s"})
            )
        );
    }

    #[test]
    fn degradation_follows_the_declared_capabilities() {
        let base = UiCapabilities::full();
        // A frontend without `sidePane` never sees side-pane entries.
        let mut no_side_pane = base.clone();
        no_side_pane.slots.retain(|slot| *slot != Slot::SidePane);
        let side = patch(vec![RegistryOp::Put {
            entry: entry(
                Slot::SidePane,
                UiView::Tree {
                    root: Paragraph::new("hi").into(),
                },
            ),
        }]);
        assert_eq!(
            rpc_event(&side, &Connection::v2(no_side_pane)),
            WireEvent::Drop
        );

        // A missing reserved component falls back to the owner's tree, else it is omitted.
        let mut no_board = base.clone();
        let board = ReservedComponent::new("board");
        no_board.reserved.retain(|c| *c != board);
        let fallback = oino_ui::node::Node::from(Paragraph::new("tasks"));
        let reserved = patch(vec![RegistryOp::Put {
            entry: entry(
                Slot::Fullscreen,
                UiView::Reserved {
                    component: board.clone(),
                    fallback: Some(Box::new(fallback.clone())),
                    params: None,
                },
            ),
        }]);
        let WireEvent::Line(value) = rpc_event(&reserved, &Connection::v2(no_board.clone())) else {
            panic!("dropped");
        };
        assert_eq!(value["ops"][0]["entry"]["view"]["kind"], "tree");
        let no_fallback = patch(vec![RegistryOp::Put {
            entry: entry(
                Slot::Fullscreen,
                UiView::Reserved {
                    component: board.clone(),
                    fallback: None,
                    params: None,
                },
            ),
        }]);
        assert_eq!(
            rpc_event(&no_fallback, &Connection::v2(no_board)),
            WireEvent::Drop
        );

        // Without a v2 renderer a widget tree becomes `widgetLines` (plain text); panels
        // are omitted.
        let mut text_only = base;
        text_only.nodes = "none".into();
        let widget = patch(vec![RegistryOp::Put {
            entry: entry(
                Slot::Widget,
                UiView::Tree {
                    root: Paragraph::new("hello").into(),
                },
            ),
        }]);
        let WireEvent::Line(value) = rpc_event(&widget, &Connection::v2(text_only)) else {
            panic!("dropped");
        };
        assert_eq!(value["ops"][0]["entry"]["view"]["kind"], "lines");
        assert_eq!(value["ops"][0]["entry"]["view"]["lines"], json!(["hello"]));
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
        let v1 = Connection::v1();
        let WireEvent::Line(wire) = rpc_event(&update, &v1) else {
            panic!("dropped");
        };
        assert_eq!(wire["type"], "agent_event");
        assert_eq!(wire["agentId"], "child");
        assert!(wire["event"].get("message").is_none());
        assert_eq!(rpc_event(&update, &Connection::pi()), WireEvent::Drop);
        let mut decoder = crate::backend::EventDecoder::default();
        let WireEvent::Line(first) = rpc_event(&start, &v1) else {
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
            rpc_event(&shutdown, &v1),
            WireEvent::Line(json!({"type": "shutdown_requested"}))
        );
    }

    #[test]
    fn patch_event_round_trips_through_the_wire() {
        let v2 = Connection::v2(UiCapabilities::full());
        let event = patch(vec![RegistryOp::Put {
            entry: entry(
                Slot::TurnFooter,
                UiView::Renderer {
                    key: "files".into(),
                },
            ),
        }]);
        let WireEvent::Line(value) = rpc_event(&event, &v2) else {
            panic!("dropped");
        };
        let decoded: ClientEvent = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, event);
        // Responses stay pi-shaped for every frontend.
        let response: UiResponse =
            serde_json::from_value(json!({"type":"extension_ui_response","id":"u1","value":"ok"}))
                .unwrap();
        assert_eq!(response.id, "u1");
        assert!(matches!(
            rpc_event(&ui("select"), &Connection::pi()),
            WireEvent::Line(_)
        ));
    }
}
