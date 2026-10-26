//! Test-only work-tray producer: one `bg` tab (2 items, 1 running) and an action handler that
//! records the action as a session entry and asks the tray to close.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK handler signatures are async even when a body does not await"
)]

use oino_extension::prelude::*;
use unipi_kit::services::{fanout, point};

/// The probe.
#[derive(Default)]
pub struct Probe;

impl Extension for Probe {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        cx.service(fanout::WORK_TRAY_TAB, |_ext, req, _caller, _cx| async move {
            let open = req.get("open").and_then(Value::as_bool).unwrap_or(false);
            let mut tab = json!({
                "id": "bg", "label": "Background tasks", "shortLabel": "Bg", "order": 0,
                "counts": {"total": 2, "running": 1},
            });
            if open {
                tab["body"] = json!({"t": "list", "items": [{"id": "t1", "label": "npm test"}], "selectable": "bg:open"});
            }
            Ok(json!([tab]))
        });
        cx.service(
            point::WORK_TRAY_ACTION,
            |_ext, action, _caller, _cx| async move {
                session::append_entry("probe.tray-action", Some(&action))?;
                Ok(json!({"close": true}))
            },
        );
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
oino_extension::export_extension!(Probe);
