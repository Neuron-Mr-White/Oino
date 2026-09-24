//! `oino.budget` — run caps for turns, cost, wall time and consecutive tool
//! errors. `agent_start` resets run state; `turn_end` evaluates caps. On a
//! cap the extension optionally injects one "wrap up now" user turn, then
//! ends the run via `end_run`.

use oino_extension_guest::{export_extension, host, Extension, Registration};
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Default)]
struct Caps {
    max_turns: Option<u64>,
    max_cost_usd: Option<f64>,
    max_wall_secs: Option<u64>,
    max_consecutive_tool_errors: Option<u64>,
    summary_on_cap: bool,
}

#[derive(Default)]
struct RunState {
    turns: u64,
    cost_usd: f64,
    consecutive_tool_errors: u64,
    summarized: bool,
    start_millis: u64,
}

static CAPS: Mutex<Caps> = Mutex::new(Caps {
    max_turns: None,
    max_cost_usd: None,
    max_wall_secs: None,
    max_consecutive_tool_errors: None,
    summary_on_cap: true,
});
static STATE: Mutex<RunState> = Mutex::new(RunState {
    turns: 0,
    cost_usd: 0.0,
    consecutive_tool_errors: 0,
    summarized: false,
    start_millis: 0,
});

fn now_millis() -> u64 {
    // monotonic-clock counts from an arbitrary epoch — fine for wall-time
    // deltas; the nanosecond count divides to millis.
    oino_extension_guest::ext::wasi::clocks::monotonic_clock::now() / 1_000_000
}

struct Budget;

impl Extension for Budget {
    fn init(config: Value) -> Result<Registration, String> {
        let mut caps = CAPS.lock().unwrap_or_else(|e| e.into_inner());
        *caps = Caps {
            max_turns: config["max_turns"].as_u64(),
            max_cost_usd: config["max_cost_usd"].as_f64(),
            max_wall_secs: config["max_wall_secs"].as_u64(),
            max_consecutive_tool_errors: config["max_consecutive_tool_errors"].as_u64(),
            summary_on_cap: config["summary_on_cap"].as_bool().unwrap_or(true),
        };
        Ok(Registration {
            hooks: vec!["agent_start".into(), "turn_end".into()],
            ..Registration::default()
        })
    }

    fn handle_event(kind: &str, payload: Value) -> Result<Value, String> {
        match kind {
            "agent_start" => {
                *STATE.lock().unwrap_or_else(|e| e.into_inner()) = RunState {
                    start_millis: now_millis(),
                    ..RunState::default()
                };
                Ok(Value::Null)
            }
            "turn_end" => Ok(evaluate_turn_end(&payload)),
            _ => Ok(Value::Null),
        }
    }
}

fn evaluate_turn_end(payload: &Value) -> Value {
    let caps = CAPS.lock().unwrap_or_else(|e| e.into_inner());
    if caps.max_turns.is_none()
        && caps.max_cost_usd.is_none()
        && caps.max_wall_secs.is_none()
        && caps.max_consecutive_tool_errors.is_none()
    {
        return Value::Null; // inert
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.turns += 1;
    // cost: sum assistant usage.cost.amount (USD only, missing ignored)
    if let Some(cost) = payload["message"]["usage"]["cost"].as_object() {
        if cost
            .get("currency")
            .and_then(Value::as_str)
            .is_none_or(|c| c == "USD")
        {
            state.cost_usd += cost.get("amount").and_then(Value::as_f64).unwrap_or(0.0);
        }
    }
    // consecutive tool errors
    if let Some(results) = payload["tool_results"].as_array() {
        for result in results {
            if result["is_error"].as_bool().unwrap_or(false) {
                state.consecutive_tool_errors += 1;
            } else {
                state.consecutive_tool_errors = 0;
            }
        }
    }

    let cap_reason = if caps
        .max_consecutive_tool_errors
        .is_some_and(|max| state.consecutive_tool_errors >= max && max > 0)
    {
        Some(format!(
            "{} consecutive tool errors",
            state.consecutive_tool_errors
        ))
    } else if caps.max_turns.is_some_and(|max| state.turns >= max) {
        Some(format!("{} turns", state.turns))
    } else if caps.max_cost_usd.is_some_and(|max| state.cost_usd >= max) {
        Some(format!("${:.4} spent", state.cost_usd))
    } else {
        caps.max_wall_secs
            .filter(|max| now_millis().saturating_sub(state.start_millis) >= *max * 1000)
            .map(|max| format!("{max}s wall time"))
    };
    let Some(reason) = cap_reason else {
        return Value::Null;
    };

    host::notify(1, &format!("budget cap reached: {reason}"));
    let _ = host::session_append(
        "budget_cap",
        &json!({
            "reason": reason,
            "turns": state.turns,
            "cost_usd": state.cost_usd,
            "consecutive_tool_errors": state.consecutive_tool_errors,
        }),
    );

    if caps.summary_on_cap && !state.summarized {
        state.summarized = true;
        return json!({
            "entries": [{
                "kind": "user",
                "id": uuid::Uuid::new_v4().to_string(),
                "content": [{
                    "kind": "text",
                    "text": format!(
                        "Budget cap reached ({reason}). Stop working now and reply with a concise summary of what is done, what remains, and next steps. Do not call tools."
                    )
                }]
            }],
            "continue_run": true
        });
    }
    json!({ "end_run": true })
}

export_extension!(Budget);
