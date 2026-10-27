//! `oino.browser` — the agent surface of browser control (browser-control.md §11).
//!
//! The host browser service (`oino-browser`, WIT `browser`) does the work; this component
//! registers the 19 `browser_*` tools, turns `approval_required` answers into user dialogs,
//! adds the static `oino-browser` system prompt section, appends the turn-tail screenshot
//! entry, renders it, and provides `/browser`.
//!
//! | Module | Contents |
//! |---|---|
//! | [`tools`] | tool specs (names, descriptions, schemas) |
//! | [`result`] | `BrowserResult` JSON → `ToolResult`, approval prompts |
//! | [`commands`] | `/browser` |

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

pub mod commands;
pub mod result;
pub mod tools;

use std::cell::RefCell;
use std::collections::HashSet;

use oino_extension::bindings::oino::extension::registry::{RendererTarget, ToolAnnotations};
use oino_extension::bindings::oino::extension::types::LogLevel;
use oino_extension::browser;
use oino_extension::host::log;
use oino_extension::prelude::*;

/// Custom entry type of the turn-tail screenshot.
pub const TURN_SHOT: &str = "oino.browser/turn-shot";

/// Extension state.
#[derive(Default)]
pub struct BrowserExt {
    /// Sessions whose current turn ran a successful page action.
    acted: RefCell<HashSet<String>>,
    /// Tools registered (service available at init).
    registered: RefCell<bool>,
}

impl BrowserExt {
    fn available() -> (bool, Value) {
        let st = browser::status();
        (st["available"] == json!(true), st)
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "specs are built fresh and consumed one by one"
    )]
    fn tool(spec: tools::Spec) -> Tool<Self> {
        let name = spec.name;
        Tool::new(name)
            .label(spec.label)
            .description(spec.description)
            .parameters(&spec.schema)
            .annotations(ToolAnnotations {
                read_only: Some(spec.read_only),
                destructive: Some(false),
                idempotent: Some(spec.read_only),
                open_world: Some(true),
            })
            .render_hints(&json!({"icon": "globe", "group": "browser"}))
            .run(
                move |ext: &'static Self, args: Value, cx: ToolCtx| async move {
                    let r = result::run_tool(name, args, cx.cx.has_ui()).await;
                    if !r.is_error && tools::is_page_action(name) {
                        ext.acted.borrow_mut().insert(cx.session_id().to_owned());
                    }
                    Ok(r)
                },
            )
    }

    fn register_tools(&'static self) {
        if *self.registered.borrow() {
            return;
        }
        for spec in tools::specs() {
            if let Err(e) = Self::tool(spec).register(self) {
                log(LogLevel::Warn, &format!("browser: register tool: {e}"));
                return;
            }
        }
        *self.registered.borrow_mut() = true;
    }

    fn prompt_section(ev: &Event) -> Option<Value> {
        let (available, st) = Self::available();
        let mut opts = ev
            .get("systemPromptOptions")
            .cloned()
            .filter(Value::is_object)?;
        if !opts.get("sections").is_some_and(Value::is_object) {
            opts["sections"] = json!({});
        }
        let sections = opts.get_mut("sections").and_then(Value::as_object_mut)?;
        if available {
            sections.insert(
                "oino-browser".into(),
                json!(prompt_line(
                    st["product"].as_str().unwrap_or("Chrome"),
                    st["sessionProfile"].as_str().unwrap_or("default"),
                )),
            );
        } else {
            sections.remove("oino-browser");
        }
        Some(json!({"systemPromptOptions": opts}))
    }

    async fn turn_shot(&self, ev: &Event, cx: &Ctx) {
        let sid = cx.session_id().to_owned();
        if !self.acted.borrow_mut().remove(&sid) || cx.is_child() {
            return;
        }
        let stop = ev
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|m| m.iter().rev().find(|x| x["role"] == "assistant"))
            .and_then(|m| m["stopReason"].as_str())
            .unwrap_or("stop")
            .to_owned();
        if matches!(stop.as_str(), "aborted" | "error") {
            return;
        }
        let turn = format!("turn-{}", timers::now_ms());
        match browser::execute(&json!({"op": "turn_shot", "turn_id": turn})).await {
            Ok((r, _)) if r["isError"] != json!(true) => {
                let data = &r["details"]["data"];
                if let Err(e) = session::append_entry(TURN_SHOT, Some(data)) {
                    log(LogLevel::Warn, &format!("browser: turn shot entry: {e}"));
                }
            }
            Ok((r, _)) => log(
                LogLevel::Debug,
                &format!(
                    "browser: turn shot skipped: {}",
                    r["text"].as_str().unwrap_or_default()
                ),
            ),
            Err(e) => log(LogLevel::Debug, &format!("browser: turn shot: {e}")),
        }
    }
}

/// The static `oino-browser` system prompt section (§11.1; changes only with engine/profile).
#[must_use]
pub fn prompt_line(product: &str, profile: &str) -> String {
    format!(
        "The in-app browser is available ({product}, profile \"{profile}\"). Use the browser_* tools; the user sees the tabs you open and can take over. See skill:browser."
    )
}

/// OUS node of a turn-tail screenshot entry (§11.3).
#[must_use]
pub fn render_turn_shot(data: &Value) -> Value {
    let title = data["title"]
        .as_str()
        .filter(|t| !t.is_empty())
        .unwrap_or("untitled");
    let caption = format!("Browser at end of turn · {title}");
    match data["path"].as_str() {
        Some(path) if std::path::Path::new(path).is_file() || !cfg!(target_arch = "wasm32") => {
            json!({"t": "stack", "direction": "vertical", "children": [
                {"t": "image", "path": path, "alt": caption, "maxWidth": 640},
                {"t": "text", "value": [{"text": caption, "dim": true}]}
            ]})
        }
        _ => json!({"t": "text", "value": [{"text": caption, "dim": true}]}),
    }
}

impl Extension for BrowserExt {
    #[allow(clippy::unused_async_trait_impl, reason = "trait signature")]
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        let (available, _) = Self::available();
        if available {
            for spec in tools::specs() {
                cx.tool(Self::tool(spec));
            }
            *self.registered.borrow_mut() = true;
        }
        cx.on(EventKind::SessionStart, |ext, _ev, _cx| async move {
            if Self::available().0 {
                ext.register_tools();
            }
            Ok(())
        });
        cx.on(EventKind::BeforeAgentStart, |_ext, ev, _cx| async move {
            Ok(Self::prompt_section(&ev))
        });
        cx.on(EventKind::AgentEnd, |ext, ev, cx| async move {
            ext.turn_shot(&ev, &cx).await;
            let _ = browser::execute(&json!({"op": "turn_ended"})).await;
            Ok(())
        });
        cx.on(EventKind::SessionShutdown, |_ext, _ev, cx| async move {
            if !cx.is_child() {
                let _ = browser::execute(&json!({"op": "session_closed"})).await;
            }
            Ok(())
        });
        cx.renderer(RendererTarget::CustomEntry, TURN_SHOT, |_ext, req, _cx| {
            let entry = req.get("data")?;
            Some(render_turn_shot(entry.get("data").unwrap_or(entry)))
        });
        cx.command(commands::command());
        Ok(())
    }
}

// Native builds (unit tests) skip the component exports.
#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(BrowserExt);
