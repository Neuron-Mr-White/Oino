//! UI probe fixture: `/ui-probe` raises every dialog method, fires every fire-and-forget
//! method, drives a panel and the editor, and returns what came back.

#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use oino_extension::command::Command;
use oino_extension::prelude::*;

/// Fixture state.
#[derive(Default)]
struct UiProbe;

const DIALOGS: &[&str] = &[
    "select",
    "confirm",
    "input",
    "editor",
    "multiSelect",
    "form",
    "approval",
    "custom",
];
const FIRES: &[&str] = &[
    "notify",
    "setStatus",
    "setWidget",
    "setTitle",
    "set_editor_text",
    "setFrame",
    "setWorkingMessage",
    "setHeader",
    "setFooter",
];

async fn run_probe() -> Result<Value> {
    let mut answers = Vec::new();
    for method in DIALOGS {
        answers.push(
            ui::dialog(&json!({"method": method, "title": format!("probe {method}")})).await?,
        );
    }
    for method in FIRES {
        ui::fire(&json!({"method": method, "probe": true}))?;
    }
    let panel = ui::panel_open(&json!({"title": "Probe", "body": {"type": "text", "text": "hi"}}))?;
    ui::panel_update(&panel, &json!({"title": "Probe 2"}))?;
    ui::panel_close(&panel);
    ui::editor_set("draft");
    ui::editor_insert("!");
    let editor = ui::editor_text();
    ui::clipboard_write_text("copied")?;
    ui::open_url("https://example.com")?;
    let confirmed = ui::confirm("Sure?", "really").await?;
    Ok(json!({"answers": answers, "panel": panel, "editor": editor, "confirmed": confirmed}))
}

impl Extension for UiProbe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.command(
            Command::new("ui-probe")
                .description("Exercise every UI request")
                .run_value(|_ext, _args, _cx| async { run_probe().await.map(Some) }),
        );
        Ok(())
    }
}

oino_extension::export_extension!(UiProbe);
