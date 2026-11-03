//! Work tray rendering (unipi `src/work/tray.ts`): one `editorReplace` panel with a tab per
//! producer, and the dim closed-tray hint line under the editor.

use oino_extension::Result;
use oino_extension::ui::{self, widgets::prelude::*};
use serde_json::{Value, json};
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

/// A producer's pane body (`TrayTab.body`); `None` renders the empty state.
fn body(tab: &TrayTab) -> Node {
    tab.body
        .clone()
        .unwrap_or_else(|| Empty::new().message("Nothing to show.").into())
}

/// Panel spec: tab strip (`tabs` node) + key hints.
#[must_use]
pub fn spec(tabs: &[TrayTab], active: usize) -> Node {
    let tabs_node = Tabs::new(
        tabs.iter()
            .map(|t| TabTitle::new(t.id.clone(), tab_text(t, 0))),
    )
    .panes(tabs.iter().map(|t| (t.id.clone(), body(t))))
    .on_select(TAB_ACTION);
    let tabs_node = match tabs.get(active) {
        Some(t) => tabs_node.selected(t.id.clone()),
        None => tabs_node,
    };
    Layout::default()
        .children([
            tabs_node.into(),
            KeyHints::new([("←→", "tabs"), ("esc", "close")]).into(),
        ])
        .into()
}

/// `/tray` (and a tray refresh): opens the `editorReplace` panel.
///
/// # Errors
///
/// Propagates the host call's error.
pub fn open(tabs: &[TrayTab], active: usize) -> Result<String> {
    ui::Panel::new(PANEL)
        .placement(ui::Placement::EditorReplace)
        .title(TRAY_HINT)
        .spec(spec(tabs, active))
        .focus(true)
        .close_on_escape(true)
        .open()
}

/// Re-renders an open tray panel.
///
/// # Errors
///
/// Propagates the host call's error.
pub fn update(id: &str, tabs: &[TrayTab], active: usize) -> Result<()> {
    ui::Panel::update(id).spec(spec(tabs, active)).send()
}

/// `setWidget` request for the closed-tray hint (clears when nothing has items or it is open).
#[must_use]
pub fn strip_request(tabs: &[TrayTab], open: bool) -> Value {
    match (open, tray_hint_text(tabs, HINT_WIDTH)) {
        (false, Some(text)) => {
            let spec = Paragraph::new(Line::from(Span::from(text.as_str()).dim()));
            json!({
                "method": "setWidget",
                "widgetKey": STRIP_KEY,
                "widgetLines": [format!(" {text}")],
                "widgetPlacement": "belowEditor",
                "widgetSpec": serde_json::to_value(Node::from(spec)).unwrap_or(Value::Null),
            })
        }
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
            body: Some(
                List::new(Vec::<ListItem>::new())
                    .on_activate(format!("{id}:open"))
                    .into(),
            ),
            owner: Some("unipi.background-tasks".into()),
            surfaces: None,
        }
    }

    fn json_of(node: &Node) -> Value {
        serde_json::to_value(node).unwrap_or(Value::Null)
    }

    #[test]
    fn panel_and_strip() {
        let tabs = [tab("bg", 0, 0), tab("sa", 2, 1)];
        let i = active_index(&tabs, None);
        assert_eq!(i, 1);
        let spec_json = json_of(&spec(&tabs, i));
        let t = &spec_json["children"][0];
        assert_eq!(t["t"], "Tabs");
        assert_eq!(t["selected"], "sa");
        assert_eq!(t["onSelect"], TAB_ACTION);
        assert_eq!(t["titles"][1]["title"], "sa label (2 · 1 running)");
        assert_eq!(t["panes"]["sa"]["onActivate"], "sa:open");
        // The panel builder adds the slot fields on top of the spec.
        let strip = strip_request(&tabs, false);
        assert_eq!(strip["widgetLines"][0], " sa label 2 (1 running) · ↓ open");
        assert_eq!(strip["widgetPlacement"], "belowEditor");
        assert_eq!(strip["widgetSpec"]["t"], "Paragraph");
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
            json_of(&spec(&[no_body], 0))["children"][0]["panes"]["x"]["t"],
            "Empty"
        );
    }
}
