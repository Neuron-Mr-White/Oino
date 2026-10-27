//! Per-frame event coalescing (gpui-app §3.1): within one drained batch, runs of
//! `message_update` events (cumulative partial messages) and `tool_execution_update` events of
//! the same call keep only the latest, so a burst of deltas costs one reduction and one
//! redraw. Every other event is kept in order.

use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};

fn same_run(previous: &ClientEvent, next: &ClientEvent) -> bool {
    match (previous, next) {
        (
            ClientEvent::Session(AgentSessionEvent::MessageUpdate { .. }),
            ClientEvent::Session(AgentSessionEvent::MessageUpdate { .. }),
        ) => true,
        (
            ClientEvent::Session(AgentSessionEvent::ToolExecutionUpdate {
                tool_call_id: a, ..
            }),
            ClientEvent::Session(AgentSessionEvent::ToolExecutionUpdate {
                tool_call_id: b, ..
            }),
        ) => a == b,
        // Nested agent deltas (`agent_event`) coalesce per agent the same way.
        (
            ClientEvent::Oino(OinoEvent::AgentEvent {
                agent_id: a,
                event: e1,
            }),
            ClientEvent::Oino(OinoEvent::AgentEvent {
                agent_id: b,
                event: e2,
            }),
        ) => a == b && same_run(e1, e2),
        _ => false,
    }
}

/// Coalesces `events` in place order; returns the reduced batch.
#[must_use]
pub fn coalesce(events: Vec<ClientEvent>) -> Vec<ClientEvent> {
    let mut out: Vec<ClientEvent> = Vec::with_capacity(events.len());
    for event in events {
        if let Some(last) = out.last_mut()
            && same_run(last, &event)
        {
            *last = event;
            continue;
        }
        out.push(event);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update(text: &str) -> ClientEvent {
        serde_json::from_value(json!({"type":"message_update","message":{
            "role":"assistant","content":[{"type":"text","text":text}],"api":"x","provider":"p","model":"m",
            "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"stop","timestamp":0},
            "assistantMessageEvent":{"type":"start","partial":{
            "role":"assistant","content":[],"api":"x","provider":"p","model":"m",
            "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                     "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason":"stop","timestamp":0}}}))
        .unwrap()
    }

    fn tool_update(id: &str, text: &str) -> ClientEvent {
        serde_json::from_value(
            json!({"type":"tool_execution_update","toolCallId":id,"toolName":"bash",
            "args":{},"partialResult":{"content":[{"type":"text","text":text}]}}),
        )
        .unwrap()
    }

    #[test]
    fn keeps_latest_of_each_run_and_order_of_others() {
        let start: ClientEvent = serde_json::from_value(json!({"type":"agent_start"})).unwrap();
        let batch = vec![
            start.clone(),
            update("a"),
            update("ab"),
            update("abc"),
            tool_update("t1", "1"),
            tool_update("t1", "12"),
            tool_update("t2", "x"),
            update("abcd"),
        ];
        let out = coalesce(batch);
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], start);
        assert_eq!(out[1], update("abc"));
        assert_eq!(out[2], tool_update("t1", "12"));
        assert_eq!(out[3], tool_update("t2", "x"));
        assert_eq!(out[4], update("abcd"));
    }
}
