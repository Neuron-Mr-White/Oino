//! Work tray rendering (unipi `src/work/tray.ts`): one `editorReplace` panel with a tab per
//! producer, and the dim closed-tray hint line under the editor.

use serde_json::{Value, json};
use unipi_kit::ous::{dim, key_hints, text_nowrap};
use unipi_kit::tray::{TRAY_HINT, TrayTab, pick_initial_tab, tab_text, tray_hint_text};

/// Panel id.
pub const PANEL: &str = "unipi.core/tray";
/// `setWidget` key of the closed-tray hint line.
pub const STRIP_KEY: &str = "work-tray-strip";
/// Action id of tab switches.
pub const TAB_ACTION: &str = "$tab";
/// Width the hint line is laid out for (frontends truncate further).
pub const HINT_WIDTH: usize = 200;

/// Index of the active tab: the remembered/requested tab, else unipi's initial-tab rule.
#[must_use]
pub fn active_index(tabs: &[TrayTab], requested: Option<&str>) -> usize {
    pick_initial_tab(tabs, requested)
}

/// Panel spec: tab strip (`tabs` node) + key hints.
#[must_use]
pub fn spec(tabs: &[TrayTab], active: usize) -> Value {
    let tab_nodes: Vec<Value> = tabs
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "title": tab_text(t, 0),
                "body": t.body.clone().unwrap_or_else(|| json!({"t": "empty", "message": "Nothing to show."})),
            })
        })
        .collect();
    json!({
        "t": "stack",
        "direction": "vertical",
        "gap": 0,
        "children": [
            {"t": "tabs", "tabs": tab_nodes, "active": tabs.get(active).map(|t| t.id.clone()), "onChange": TAB_ACTION},
            key_hints(&[("←→", "tabs"), ("esc", "close")]),
        ],
    })
}

/// `openPanel` request.
#[must_use]
pub fn open_request(tabs: &[TrayTab], active: usize) -> Value {
    json!({
        "panelId": PANEL,
        "placement": "editorReplace",
        "title": TRAY_HINT,
        "spec": spec(tabs, active),
        "focus": true,
        "closeOnEscape": true,
    })
}

/// `setWidget` request for the closed-tray hint (clears when nothing has items or it is open).
#[must_use]
pub fn strip_request(tabs: &[TrayTab], open: bool) -> Value {
    match (open, tray_hint_text(tabs, HINT_WIDTH)) {
        (false, Some(text)) => json!({
            "method": "setWidget",
            "widgetKey": STRIP_KEY,
            "widgetLines": [format!(" {text}")],
            "widgetPlacement": "belowEditor",
            "widgetSpec": text_nowrap(json!([dim(&text)])),
        }),
        _ => json!({"method": "setWidget", "widgetKey": STRIP_KEY}),
    }
}

#[cfg(test)]
mod tests {
    use unipi_kit::tray::Counts;

    use super::*;

    fn tab(id: &str, total: u32, running: u32) -> TrayTab {
        TrayTab {
            id: id.into(),
            label: format!("{id} label"),
            short_label: None,
            order: 0,
            counts: Counts { total, running },
            visible: true,
            body: Some(json!({"t": "list", "items": [], "selectable": format!("{id}:open")})),
            owner: Some("unipi.background-tasks".into()),
        }
    }

    #[test]
    fn panel_and_strip() {
        let tabs = [tab("bg", 0, 0), tab("sa", 2, 1)];
        let i = active_index(&tabs, None);
        assert_eq!(i, 1);
        let open = open_request(&tabs, i);
        assert_eq!(open["placement"], "editorReplace");
        assert_eq!(open["panelId"], PANEL);
        let t = &open["spec"]["children"][0];
        assert_eq!(t["t"], "tabs");
        assert_eq!(t["active"], "sa");
        assert_eq!(t["onChange"], TAB_ACTION);
        assert_eq!(t["tabs"][1]["title"], "sa label (2 · 1 running)");
        assert_eq!(t["tabs"][1]["body"]["selectable"], "sa:open");
        let strip = strip_request(&tabs, false);
        assert_eq!(strip["widgetLines"][0], " sa label 2 (1 running) · ↓ open");
        assert_eq!(strip["widgetPlacement"], "belowEditor");
        assert_eq!(
            strip_request(&tabs, true),
            json!({"method": "setWidget", "widgetKey": STRIP_KEY})
        );
        assert_eq!(
            strip_request(&[tab("bg", 0, 0)], false).get("widgetLines"),
            None
        );
        let mut no_body = tab("x", 1, 0);
        no_body.body = None;
        assert_eq!(
            spec(&[no_body], 0)["children"][0]["tabs"][0]["body"]["t"],
            "empty"
        );
    }
}
