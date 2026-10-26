//! Kanban recogniser (design.md §4.30): maps the `unipi.kanboard/board` panel's OUS (a
//! horizontal `split` of lane `section`s holding `list`s, plus a detail `section`) to lanes
//! and cards for the native board. Actions go back to the panel unchanged
//! (`select` with the task id, `move:<status>` buttons from the detail).

use crate::model::ous::{OusNode, Rich, Tone, plain};

/// Panel id of the board.
pub const BOARD_PANEL: &str = "unipi.kanboard/board";

/// One card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// Task id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Detail line (claim, blocked reason, deps).
    pub detail: Option<String>,
    /// Priority badge.
    pub priority: Option<String>,
    /// Tone of the item.
    pub tone: Tone,
    /// Selected.
    pub selected: bool,
}

/// One lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane {
    /// Lane id (`todo`, `in_progress`, …).
    pub id: String,
    /// Title without the count.
    pub title: String,
    /// Cards.
    pub cards: Vec<Card>,
}

/// The board.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    /// Header line (project slug, task count).
    pub header: String,
    /// Lanes in board order.
    pub lanes: Vec<Lane>,
    /// Action id for selecting a card.
    pub select_action: Option<String>,
    /// Detail section of the selected task (rendered as OUS).
    pub detail: Option<OusNode>,
    /// Move buttons of the selected task `(label, actionId)`.
    pub moves: Vec<(String, String)>,
}

fn lane_id(title: &str) -> String {
    title
        .split(" (")
        .next()
        .unwrap_or(title)
        .trim()
        .to_lowercase()
        .replace(' ', "_")
}

fn card_title(label: &Rich, id: &str) -> String {
    plain(label).trim_start_matches(id).trim().to_owned()
}

fn moves_of(node: &OusNode, out: &mut Vec<(String, String)>) {
    match node {
        OusNode::Button {
            label, action_id, ..
        } if action_id.starts_with("move:") => {
            out.push((label.trim_start_matches("→ ").to_owned(), action_id.clone()));
        }
        OusNode::Stack { children, .. } | OusNode::Split { children, .. } => {
            for c in children {
                moves_of(c, out);
            }
        }
        OusNode::Section { body, .. } | OusNode::Card { body, .. } => moves_of(body, out),
        _ => {}
    }
}

/// Recognises the board layout; `None` when the spec does not look like a kanboard board.
#[must_use]
pub fn recognise(spec: &OusNode) -> Option<Board> {
    let OusNode::Stack { children, .. } = spec else {
        return None;
    };
    let mut board = Board::default();
    let mut found = false;
    for child in children {
        match child {
            OusNode::Text { value, .. } if board.header.is_empty() => board.header = plain(value),
            OusNode::Split {
                horizontal: true,
                children: lanes,
            } => {
                for lane in lanes {
                    let OusNode::Section { title, body, .. } = lane else {
                        continue;
                    };
                    let title_text = plain(title);
                    let mut cards = Vec::new();
                    if let OusNode::List {
                        items,
                        selectable,
                        selected,
                    } = body.as_ref()
                    {
                        board.select_action.clone_from(selectable);
                        cards = items
                            .iter()
                            .map(|i| Card {
                                id: i.id.clone(),
                                title: card_title(&i.label, &i.id),
                                detail: i.detail.as_ref().map(plain),
                                priority: i.badge.as_ref().map(plain),
                                tone: i.tone,
                                selected: selected.as_deref() == Some(i.id.as_str()),
                            })
                            .collect();
                    }
                    board.lanes.push(Lane {
                        id: lane_id(&title_text),
                        title: title_text.split(" (").next().unwrap_or_default().to_owned(),
                        cards,
                    });
                }
                found = !board.lanes.is_empty();
            }
            OusNode::Section { .. } if found => {
                moves_of(child, &mut board.moves);
                board.detail = Some(child.clone());
            }
            _ => {}
        }
    }
    found.then_some(board)
}

/// Lanes shown by default (Backlog hidden, design.md §4.30).
#[must_use]
pub fn visible(lane: &Lane, show_backlog: bool) -> bool {
    show_backlog || lane.id != "backlog"
}

/// Cards ready to work (Todo).
#[must_use]
pub fn ready_count(board: &Board) -> usize {
    board
        .lanes
        .iter()
        .find(|l| l.id == "todo")
        .map_or(0, |l| l.cards.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognises_the_kanboard_panel() {
        let spec = OusNode::parse(&json!({"t":"stack","direction":"vertical","children":[
            {"t":"text","value":[{"text":"oino","bold":true},{"text":" · 3 tasks","dim":true}]},
            {"t":"split","direction":"horizontal","children":[
                {"t":"section","title":"Backlog (0)","body":{"t":"empty","message":"empty"}},
                {"t":"section","title":"Todo (2)","body":{"t":"list","selectable":"select","selected":"T-2","items":[
                    {"id":"T-1","label":[{"text":"T-1","tone":"muted"},{"text":" Port watchdog"}],"badge":"high","tone":"warning"},
                    {"id":"T-2","label":[{"text":"T-2","tone":"muted"},{"text":" Settings hub"}],"detail":"waits for T-1"}]}},
                {"t":"section","title":"In Progress (1)","body":{"t":"list","selectable":"select","items":[
                    {"id":"T-3","label":"T-3 Release"}]}}]},
            {"t":"section","title":"T-2 Settings hub","body":{"t":"stack","children":[
                {"t":"kv","items":[{"key":"Status","value":"Todo"}]},
                {"t":"stack","direction":"horizontal","children":[
                    {"t":"button","label":"→ In Progress","actionId":"move:in_progress"}]}]}}]}));
        let board = recognise(&spec).unwrap();
        assert_eq!(board.header, "oino · 3 tasks");
        assert_eq!(board.lanes.len(), 3);
        assert_eq!(board.lanes[1].id, "todo");
        assert_eq!(board.lanes[2].id, "in_progress");
        assert_eq!(board.lanes[1].cards[0].title, "Port watchdog");
        assert_eq!(board.lanes[1].cards[0].priority.as_deref(), Some("high"));
        assert!(board.lanes[1].cards[1].selected);
        assert_eq!(board.select_action.as_deref(), Some("select"));
        assert_eq!(
            board.moves,
            vec![("In Progress".into(), "move:in_progress".into())]
        );
        assert!(!visible(&board.lanes[0], false));
        assert_eq!(ready_count(&board), 2);
        assert!(recognise(&OusNode::parse(&json!({"t":"text","value":"x"}))).is_none());
    }
}
