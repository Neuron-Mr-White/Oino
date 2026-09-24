//! `oino.ralph_loop` — Ralph iterative development loops.
//!
//! The state machine and `.oino/ralph` file format are ported verbatim from
//! the previous native module (`crates/oino-app/src/ralph_loop.rs`), so
//! existing loops keep working and the host's floating panel keeps reading
//! the same files. Loop driving uses `turn_end`: after each assistant turn
//! the final message's promise tag is recorded; while the loop stays active
//! and `auto_continue` is set, the next iteration prompt is injected as a
//! user entry with `continue_run: true` (matching the old
//! `continue_ralph_after_prompt_if_needed` behavior).

mod commands;
mod state;

use oino_extension_guest::{export_extension, host, Extension, Registration};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

const KV_ACTIVE_LOOP: &str = "active_loop";
const KV_AUTO_CONTINUE: &str = "auto_continue";

static PROJECT_ROOT: Mutex<String> = Mutex::new(String::new());
/// Loop whose iteration prompt is currently running — set when a `/ralph`
/// command submits a prompt and consumed by `turn_end` (mirrors the old
/// `prompt_loop_in_flight` controller field).
static PROMPT_LOOP_IN_FLIGHT: Mutex<Option<String>> = Mutex::new(None);

struct Ralph;

fn project_root() -> PathBuf {
    PathBuf::from(
        PROJECT_ROOT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    )
}

fn set_active(name: Option<String>, auto_continue: bool) {
    let _ = host::kv_set(KV_ACTIVE_LOOP, name.clone().unwrap_or_default().as_bytes());
    let _ = host::kv_set(KV_AUTO_CONTINUE, if auto_continue { b"1" } else { b"0" });
    *PROMPT_LOOP_IN_FLIGHT
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = name;
}

fn active_loop() -> Option<String> {
    host::kv_get(KV_ACTIVE_LOOP)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|name| !name.is_empty())
}

fn auto_continue() -> bool {
    host::kv_get(KV_AUTO_CONTINUE).is_some_and(|bytes| bytes == b"1")
}

/// Text of the last assistant message in a turn_end payload.
fn message_text(message: &Value) -> String {
    message["content"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|block| block["text"].as_str().map(str::to_string))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Extension for Ralph {
    fn init(config: Value) -> Result<Registration, String> {
        if let Some(root) = config["project_root"].as_str() {
            *PROJECT_ROOT.lock().unwrap_or_else(|e| e.into_inner()) = root.to_string();
        }
        Ok(Registration {
            commands: vec![("ralph".into(), "Ralph loop controls".into())],
            hooks: vec!["turn_end".into()],
            ..Registration::default()
        })
    }

    fn handle_event(kind: &str, payload: Value) -> Result<Value, String> {
        match kind {
            "turn_end" => {
                // Consumed prompt-flight loop name, else the active loop —
                // matches the native controller: record only when an
                // iteration prompt was in flight.
                let in_flight = PROMPT_LOOP_IN_FLIGHT
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                let Some(loop_name) = in_flight.or_else(active_loop) else {
                    return Ok(Value::Null);
                };
                let root = project_root();
                let Ok(state) = state::load_state(&root, &loop_name) else {
                    return Ok(Value::Null);
                };
                if state.status != state::RalphLoopStatus::Active {
                    return Ok(Value::Null);
                }
                let output = message_text(&payload["message"]);
                let state = state::record_iteration_output(&root, &loop_name, &output)
                    .map_err(|err| err.to_string())?;
                let _ = host::session_append(
                    "ralph_iteration",
                    &json!({
                        "loop": state.name,
                        "iteration": state.iteration,
                        "status": format!("{:?}", state.status),
                    }),
                );
                if state.status != state::RalphLoopStatus::Active || !auto_continue() {
                    if state.status != state::RalphLoopStatus::Active {
                        set_active(None, false);
                    }
                    return Ok(Value::Null);
                }
                let prompt =
                    state::build_iteration_prompt(&root, &state).map_err(|err| err.to_string())?;
                *PROMPT_LOOP_IN_FLIGHT
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(state.name.clone());
                Ok(json!({
                    "entries": [{"role": "user", "text": prompt}],
                    "continue_run": true
                }))
            }
            _ => Ok(Value::Null),
        }
    }

    fn run_command(name: &str, args: &str) -> Result<Value, String> {
        if name != "ralph" {
            return Err(format!("unknown command `{name}`"));
        }
        let command = commands::parse_ralph_args(args)
            .ok_or_else(|| "invalid /ralph arguments; run `/ralph help`".to_string())?;
        let root = project_root();
        let output = commands::execute(&command, &root).map_err(|err| err.to_string())?;
        // Driving state lives in kv so hooks see it across calls.
        match command {
            commands::RalphCommand::Start { .. }
            | commands::RalphCommand::Resume { .. }
            | commands::RalphCommand::Continue { .. }
            | commands::RalphCommand::Once { .. } => {
                set_active(output.loop_name.clone(), output.auto_continue);
            }
            commands::RalphCommand::Pause { .. } | commands::RalphCommand::Cancel { .. }
                if active_loop() == output.loop_name =>
            {
                set_active(None, false);
            }
            _ => {}
        }
        let mut result = json!({ "message": output.message });
        if let Some(prompt) = output.prompt {
            result["prompt"] = Value::String(prompt);
        }
        Ok(result)
    }
}

export_extension!(Ralph);
