//! Panic-probe fixture: a component that traps on demand (core spec §2.3 X-TRAP).
//!
//! Subscribes to `tool_call`; when the gated tool is `"trap"` the guest panics — a wasm
//! trap that poisons the shared instance. Every other `tool_call` is recorded as a
//! session `custom` entry `panic-probe.seen`, so tests can observe the extension still
//! dispatching for sessions that did not disable it.

// The one deliberate guest trap in the tree lives here; that is the fixture's purpose.
#![allow(clippy::panic, reason = "this fixture exists to trap")]

use oino_extension::prelude::*;

/// Fixture state.
#[derive(Default)]
struct PanicProbe;

impl Extension for PanicProbe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.on(EventKind::ToolCall, |_ext, ev: Event, _cx| async move {
            let tool = ev
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if tool == "trap" {
                panic!("panic-probe: deliberate trap");
            }
            session::append_entry("panic-probe.seen", Some(&json!({"toolName": tool})))?;
            Ok(None::<Value>)
        });
        Ok(())
    }
}

oino_extension::export_extension!(PanicProbe);
