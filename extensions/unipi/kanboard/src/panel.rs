//! The board entry (round-5 #7, ui-protocol §8): `/kanboard open` opens the **reserved**
//! `unipi.kanboard/board` entry — the app renders its native board component, other frontends
//! (the TUI) get the task-list `fallback`. There is no OUS spec and no SSE-driven republish.

use oino_extension::ui::{self, widgets::prelude::*};
use oino_extension::{Result, ui::Placement};

use crate::render::claim_order;
use crate::shapes::Task;

/// Reserved entry id (§8), declared in `extension.json` (`ui.reserved`).
pub const PANEL_ID: &str = "unipi.kanboard/board";

/// Opens (or focuses) the board entry and installs the task-list fallback.
///
/// # Errors
///
/// Propagates the host call's error.
pub fn open(tasks: &[Task]) -> Result<String> {
    let id = ui::Panel::new(PANEL_ID)
        .placement(Placement::SidePane)
        .title("Board")
        .icon(Icon::Board)
        .focus(true)
        .close_on_escape(true)
        .open()?;
    // `fallback` is an `updatePanel` field (§9.1), not part of `openPanel`.
    ui::Panel::update(&id).fallback(fallback(tasks)).send()?;
    Ok(id)
}

/// The task list rendered by frontends without the board component (§8 rule 3): one row per
/// task, claim order, `id · title · status`.
#[must_use]
pub fn fallback(tasks: &[Task]) -> Node {
    if tasks.is_empty() {
        return Empty::new()
            .title("Board")
            .message("No tasks yet — /kanboard onboard, then -add.")
            .into();
    }
    let rows = claim_order(tasks).into_iter().map(|t| {
        let mut detail = vec![Span::from(format!(" {}", t.status)).fg(Color::Muted)];
        if let Some(p) = t.priority.as_deref().filter(|p| !p.is_empty()) {
            detail.push(Span::from(format!(" · {p}")).fg(Color::Dim));
        }
        ListItem::new(t.display_title.clone().unwrap_or_else(|| t.title.clone()))
            .key(t.id.clone())
            .detail(Line::from(detail))
            .icon(Icon::Board)
    });
    List::new(rows)
        .highlight_symbol("› ")
        .block(Block::bordered().title("Board").border_type(BorderType::Rounded))
        .into()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests assert by unwrapping")]
    use serde_json::{Value, json};

    use super::*;

    fn task(id: &str, status: &str) -> Task {
        serde_json::from_value(json!({"id": id, "title": format!("{id} title"), "status": status}))
            .unwrap()
    }

    #[test]
    fn fallback_lists_tasks_in_claim_order() {
        let node = fallback(&[
            task("T-2", "todo"),
            Task {
                priority: Some("urgent".into()),
                ..task("T-1", "in_progress")
            },
        ]);
        let json: Value = serde_json::to_value(node).unwrap();
        assert_eq!(json["t"], "List");
        assert_eq!(json["items"][0]["key"], "T-1", "urgent first");
        assert_eq!(json["items"][0]["content"], "T-1 title");
        assert_eq!(json["items"][0]["detail"][0]["content"], " in_progress");
        assert_eq!(json["block"]["title"], "Board");
    }

    #[test]
    fn empty_board_renders_the_empty_state() {
        let json: Value = serde_json::to_value(fallback(&[])).unwrap();
        assert_eq!(json["t"], "Empty");
        assert_eq!(json["title"], "Board");
    }
}
