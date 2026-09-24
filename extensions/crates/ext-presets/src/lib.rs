//! `oino.presets` — named work-mode presets switched via `/preset`.
//!
//! - `minimal`: read/edit/write/bash only, plus a short `oino/preset` section.
//! - `standard`: removes the preset section, all tools visible.
//! - `thorough`: planning/verification checklist section, all tools visible.
//!
//! The current preset is persisted per project through `host.kv`.

use oino_extension_guest::{export_extension, host, Extension, Registration};
use serde_json::{json, Value};
use std::sync::Mutex;

const MINIMAL_SECTION: &str = "Be concise; prefer the fewest tool calls that solve the task.";
const THOROUGH_SECTION: &str = "Plan before acting: outline the approach, then verify each change before finishing. Prefer explicit checks over assumptions.";
const SECTION_NAME: &str = "oino/preset";
const KV_KEY: &str = "preset";

static CURRENT: Mutex<Option<&'static str>> = Mutex::new(None);

struct Presets;

impl Extension for Presets {
    fn init(config: Value) -> Result<Registration, String> {
        let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        // CLI `--preset` arrives as init config; else per-project kv, else standard.
        if let Some(name) = config["preset"].as_str() {
            if is_preset(name) {
                *current = Some(preset(name));
            }
        }
        if current.is_none() {
            if let Some(bytes) = host::kv_get(KV_KEY) {
                if let Ok(name) = String::from_utf8(bytes) {
                    if is_preset(&name) {
                        *current = Some(preset(&name));
                    }
                }
            }
        }
        if current.is_none() {
            *current = Some("standard");
        }
        Ok(Registration {
            commands: vec![(
                "preset".into(),
                "Switch work preset (minimal|standard|thorough)".into(),
            )],
            hooks: vec!["before_agent_start".into()],
            ..Registration::default()
        })
    }

    fn run_command(name: &str, args: &str) -> Result<Value, String> {
        if name != "preset" {
            return Err(format!("unknown command `{name}`"));
        }
        let arg = args.trim();
        if arg.is_empty() {
            let current = *CURRENT.lock().unwrap_or_else(|e| e.into_inner());
            return Ok(json!({
                "message": format!(
                    "presets: minimal, standard, thorough\ncurrent: {}",
                    current.unwrap_or("standard")
                )
            }));
        }
        if !is_preset(arg) {
            return Ok(json!({
                "message": format!("unknown preset `{arg}` (use minimal|standard|thorough)")
            }));
        }
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(preset(arg));
        let _ = host::kv_set(KV_KEY, arg.as_bytes());
        Ok(json!({ "message": format!("preset: {arg}") }))
    }

    fn handle_event(kind: &str, _payload: Value) -> Result<Value, String> {
        if kind != "before_agent_start" {
            return Ok(Value::Null);
        }
        let current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = json!({ "section_edits": [], "active_tools": null });
        match current.unwrap_or("standard") {
            "minimal" => {
                result["section_edits"] = json!([{
                    "name": SECTION_NAME,
                    "text": MINIMAL_SECTION
                }]);
                result["active_tools"] = json!(["read", "edit", "write", "bash"]);
            }
            "thorough" => {
                result["section_edits"] = json!([{
                    "name": SECTION_NAME,
                    "text": THOROUGH_SECTION
                }]);
            }
            _ => {
                // `standard`: remove the preset section if present.
                result["section_edits"] = json!([{ "name": SECTION_NAME, "text": null }]);
            }
        }
        Ok(result)
    }
}

fn is_preset(name: &str) -> bool {
    matches!(name, "minimal" | "standard" | "thorough")
}

fn preset(name: &str) -> &'static str {
    match name {
        "minimal" => "minimal",
        "thorough" => "thorough",
        _ => "standard",
    }
}

export_extension!(Presets);
