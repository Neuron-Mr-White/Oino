//! Work-tray contract (unipi `src/work/{index,tray}.ts`, unipi-port §2.1).
//!
//! `unipi.core` owns the tray: one `editorReplace` panel with a tab per producer. Producers
//! (background-tasks, subagents, dream, …) serve [`crate::services::fanout::WORK_TRAY_TAB`]
//! returning `Vec<TrayTab>` (or one `TrayTab`) for a [`TabRequest`]; tab bodies are OUS nodes
//! whose interactive `actionId`s are prefixed `<tabId>:`. Panel actions are routed back to the
//! tab owner through [`crate::services::point::WORK_TRAY_ACTION`] with an [`Action`]; the reply
//! ([`ActionReply`]) may close the tray or ask for a refresh. Producers publish
//! [`crate::topics::WORK_TRAY_CHANGED`] when their counts change.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fmt::{truncate_to_width, visible_width};

/// Tray hint shown in the tab strip.
pub const TRAY_HINT: &str = "←→ tabs · esc close";
/// Suffix of the closed-tray hint line.
pub const TRAY_OPEN_HINT: &str = "↓ open";

/// Item counts of a tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Items.
    pub total: u32,
    /// Running items.
    pub running: u32,
}

/// `work-tray.tab` request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabRequest {
    /// The tray is (about to be) open: include `body`.
    #[serde(default)]
    pub open: bool,
    /// Item to open directly (only for the tab it belongs to).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_id: Option<String>,
}

/// One tab (unipi `WorkTrayTab`; `createPane` → `body`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayTab {
    /// Tab id (`bg`, `subagents`, `dream`).
    pub id: String,
    /// Label.
    pub label: String,
    /// Label when narrow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_label: Option<String>,
    /// Sort order.
    pub order: i32,
    /// Counts.
    pub counts: Counts,
    /// `false` hides the tab (Dream with no history and dreaming off).
    #[serde(default = "yes")]
    pub visible: bool,
    /// Pane content (OUS node); `None` when not requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// Owning extension (filled by the tray).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

fn yes() -> bool {
    true
}

/// A panel action forwarded to the tab owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// Tab id.
    pub tab: String,
    /// Action id without the `<tab>:` prefix.
    pub action_id: String,
    /// Action value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

/// Owner reply to an [`Action`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReply {
    /// Close the tray.
    #[serde(default)]
    pub close: bool,
    /// Re-fetch the tabs.
    #[serde(default)]
    pub refresh: bool,
}

/// Splits a tray action id `<tab>:<action>`.
#[must_use]
pub fn split_action(action_id: &str) -> Option<(&str, &str)> {
    action_id
        .split_once(':')
        .filter(|(t, a)| !t.is_empty() && !a.is_empty())
}

/// Decodes one provider's reply (`TrayTab` or `[TrayTab]`), tagging the owner.
#[must_use]
pub fn decode_tabs(owner: &str, reply: &Value) -> Vec<TrayTab> {
    let list: Vec<Value> = match reply {
        Value::Array(a) => a.clone(),
        Value::Object(_) => vec![reply.clone()],
        _ => Vec::new(),
    };
    list.into_iter()
        .filter_map(|v| serde_json::from_value::<TrayTab>(v).ok())
        .map(|mut t| {
            t.owner = Some(owner.to_owned());
            t
        })
        .collect()
}

/// Visible tabs sorted by `order` (stable).
#[must_use]
pub fn sorted_visible(mut tabs: Vec<TrayTab>) -> Vec<TrayTab> {
    tabs.retain(|t| t.visible);
    tabs.sort_by_key(|t| t.order);
    tabs
}

/// Items across visible tabs (↓ opens only when > 0).
#[must_use]
pub fn item_count(tabs: &[TrayTab]) -> u32 {
    tabs.iter()
        .filter(|t| t.visible)
        .map(|t| t.counts.total)
        .sum()
}

/// unipi `pickInitialTab`: the requested tab, else the first unless it is empty and a later one
/// has items.
#[must_use]
pub fn pick_initial_tab(tabs: &[TrayTab], requested: Option<&str>) -> usize {
    if tabs.is_empty() {
        return 0;
    }
    if let Some(i) = requested.and_then(|r| tabs.iter().position(|t| t.id == r)) {
        return i;
    }
    if tabs[0].counts.total > 0 {
        return 0;
    }
    tabs.iter().position(|t| t.counts.total > 0).unwrap_or(0)
}

/// Tab title: `Label (N · k running)` (level 0), short label (1), `Short N/k` (2).
#[must_use]
pub fn tab_text(tab: &TrayTab, level: u8) -> String {
    let c = tab.counts;
    let label = if level > 0 {
        tab.short_label.as_deref().unwrap_or(&tab.label)
    } else {
        &tab.label
    };
    if level >= 2 {
        return if c.running > 0 {
            format!("{label} {}/{}", c.total, c.running)
        } else {
            format!("{label} {}", c.total)
        };
    }
    if c.running > 0 {
        format!("{label} ({} · {} running)", c.total, c.running)
    } else {
        format!("{label} ({})", c.total)
    }
}

fn hint_part(tab: &TrayTab, level: u8) -> String {
    let c = tab.counts;
    let label = if level > 0 {
        tab.short_label.as_deref().unwrap_or(&tab.label)
    } else {
        &tab.label
    };
    if c.running == 0 {
        format!("{label} {}", c.total)
    } else if level >= 2 {
        format!("{label} {}/{}", c.total, c.running)
    } else if c.running >= c.total {
        format!("{label} {} running", c.total)
    } else {
        format!("{label} {} ({} running)", c.total, c.running)
    }
}

/// unipi `trayHintText`: the closed-tray hint for `width` columns, `None` when nothing has items.
/// Full labels, then short labels, then compact counts; past that the counts are cut so
/// `↓ open` stays visible.
#[must_use]
pub fn tray_hint_text(tabs: &[TrayTab], width: usize) -> Option<String> {
    let items: Vec<&TrayTab> = tabs
        .iter()
        .filter(|t| t.visible && t.counts.total > 0)
        .collect();
    if items.is_empty() || width == 0 {
        return None;
    }
    let suffix = format!(" · {TRAY_OPEN_HINT}");
    let mut line = String::new();
    for level in 0..=2 {
        line = items
            .iter()
            .map(|t| hint_part(t, level))
            .collect::<Vec<_>>()
            .join(" · ");
        if visible_width(&line) + visible_width(&suffix) <= width {
            return Some(format!("{line}{suffix}"));
        }
    }
    let room = width.saturating_sub(visible_width(&suffix));
    if room < 4 {
        return Some(truncate_to_width(TRAY_OPEN_HINT, width, ""));
    }
    Some(format!("{}{suffix}", truncate_to_width(&line, room, "…")))
}

/// Footer-style dot of a work item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dot {
    /// Green.
    Running,
    /// Yellow.
    Stopped,
    /// Red.
    Failed,
    /// Gray.
    Done,
}

impl Dot {
    /// OUS tone for the dot.
    #[must_use]
    pub fn tone(self) -> &'static str {
        match self {
            Self::Running => "success",
            Self::Stopped => "warning",
            Self::Failed => "error",
            Self::Done => "muted",
        }
    }
}

/// One work item (unipi `WorkItem`), the shared row model producers render in their tabs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    /// `bg-<id>`, `agent-<id>`, `sidekick`.
    pub id: String,
    /// `bg | subagent | sidekick`.
    pub kind: String,
    /// Title.
    pub title: String,
    /// Dot.
    pub dot: Dot,
    /// Detail line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Start (epoch ms).
    pub started_at: i64,
    /// End (epoch ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// Can be stopped.
    pub can_stop: bool,
    /// Can be re-run.
    pub can_rerun: bool,
    /// Can be sent to the background.
    pub can_background: bool,
}

/// unipi order: running first, then newest-started first.
pub fn sort_work_items(items: &mut [WorkItem]) {
    items.sort_by(|a, b| {
        (b.dot == Dot::Running)
            .cmp(&(a.dot == Dot::Running))
            .then(b.started_at.cmp(&a.started_at))
    });
}

/// Counts of a list of items.
#[must_use]
pub fn counts_of(items: &[WorkItem]) -> Counts {
    let n = |f: &dyn Fn(&WorkItem) -> bool| {
        u32::try_from(items.iter().filter(|i| f(i)).count()).unwrap_or(u32::MAX)
    };
    Counts {
        total: n(&|_| true),
        running: n(&|i| i.dot == Dot::Running),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tab(id: &str, label: &str, short: &str, order: i32, total: u32, running: u32) -> TrayTab {
        TrayTab {
            id: id.into(),
            label: label.into(),
            short_label: Some(short.into()),
            order,
            counts: Counts { total, running },
            visible: true,
            body: None,
            owner: None,
        }
    }

    #[test]
    fn initial_tab() {
        let tabs = [
            tab("bg", "Background tasks", "Bg", 0, 0, 0),
            tab("sa", "Subagents", "Agents", 1, 0, 0),
        ];
        assert_eq!(
            pick_initial_tab(&tabs, None),
            0,
            "first even when both empty"
        );
        let tabs = [
            tab("bg", "Background tasks", "Bg", 0, 0, 0),
            tab("sa", "Subagents", "Agents", 1, 2, 0),
        ];
        assert_eq!(pick_initial_tab(&tabs, None), 1, "skip an empty first tab");
        assert_eq!(pick_initial_tab(&tabs, Some("bg")), 0, "explicit wins");
        assert_eq!(
            pick_initial_tab(&tabs, Some("nope")),
            1,
            "unknown falls back"
        );
        assert_eq!(pick_initial_tab(&[], Some("bg")), 0);
    }

    #[test]
    fn tab_titles() {
        let t = tab("bg", "Background tasks", "Bg", 0, 2, 1);
        assert_eq!(tab_text(&t, 0), "Background tasks (2 · 1 running)");
        assert_eq!(tab_text(&t, 1), "Bg (2 · 1 running)");
        assert_eq!(tab_text(&t, 2), "Bg 2/1");
        assert_eq!(
            tab_text(&tab("s", "Subagents", "A", 0, 3, 0), 0),
            "Subagents (3)"
        );
    }

    #[test]
    fn closed_tray_hint() {
        let tabs = [
            tab("bg", "Background tasks", "Bg", 0, 1, 1),
            tab("sa", "Subagents", "Agents", 1, 3, 1),
            tab("dr", "Dream", "Dream", 2, 2, 0),
            tab("e", "Empty", "E", 3, 0, 0),
        ];
        assert_eq!(
            tray_hint_text(&tabs, 200).as_deref(),
            Some("Background tasks 1 running · Subagents 3 (1 running) · Dream 2 · ↓ open")
        );
        assert_eq!(
            tray_hint_text(&tabs, 56).as_deref(),
            Some("Bg 1 running · Agents 3 (1 running) · Dream 2 · ↓ open")
        );
        assert_eq!(
            tray_hint_text(&tabs, 40).as_deref(),
            Some("Bg 1/1 · Agents 3/1 · Dream 2 · ↓ open")
        );
        let cut = tray_hint_text(&tabs, 20).unwrap_or_default();
        assert!(cut.ends_with(" · ↓ open"));
        assert!(visible_width(&cut) <= 20);
        assert_eq!(tray_hint_text(&tabs, 5).as_deref(), Some("↓ ope"));
        assert_eq!(tray_hint_text(&tabs[3..], 80), None);
        assert_eq!(tray_hint_text(&[], 80), None);
    }

    #[test]
    fn hidden_tabs_and_decoding() {
        let mut hidden = tab("dr", "Dream", "Dream", 2, 4, 0);
        hidden.visible = false;
        let tabs = sorted_visible(vec![
            tab("sa", "Subagents", "A", 1, 1, 0),
            hidden,
            tab("bg", "Bg", "Bg", 0, 1, 0),
        ]);
        assert_eq!(
            tabs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["bg", "sa"]
        );
        assert_eq!(item_count(&tabs), 2);
        let decoded = decode_tabs(
            "unipi.background-tasks",
            &json!({"id": "bg", "label": "Background tasks", "order": 0, "counts": {"total": 1, "running": 0}}),
        );
        assert_eq!(decoded.len(), 1);
        assert!(decoded[0].visible, "visible defaults to true");
        assert_eq!(decoded[0].owner.as_deref(), Some("unipi.background-tasks"));
        assert_eq!(
            decode_tabs("x", &json!([{"id": 1}, null])),
            Vec::<TrayTab>::new()
        );
        assert_eq!(split_action("bg:stop"), Some(("bg", "stop")));
        assert_eq!(split_action("$close"), None);
    }

    #[test]
    fn work_items_sort_running_first_then_newest() {
        let item = |id: &str, dot: Dot, at: i64| WorkItem {
            id: id.into(),
            kind: "bg".into(),
            title: id.into(),
            dot,
            detail: None,
            started_at: at,
            ended_at: None,
            can_stop: false,
            can_rerun: false,
            can_background: false,
        };
        let mut items = vec![
            item("a", Dot::Done, 3),
            item("b", Dot::Running, 1),
            item("c", Dot::Failed, 5),
            item("d", Dot::Running, 2),
        ];
        sort_work_items(&mut items);
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["d", "b", "c", "a"]
        );
        assert_eq!(
            counts_of(&items),
            Counts {
                total: 4,
                running: 2
            }
        );
        assert_eq!(
            serde_json::to_value(Dot::Running).unwrap_or_default(),
            json!("running")
        );
    }
}
