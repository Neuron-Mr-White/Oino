//! The memory **browse panel** (app spec §5 WP-A25, G-9): list, search, open,
//! forget and consolidate over the project's memories (or every project's),
//! as a UI v2 overlay built with the `oino-ui` builder — the same tree the TUI
//! and the app render.
//!
//! Opened by `/memory` (`open`), driven by `_oino/ui_action` (`action`): the
//! extension keeps one [`View`] per session, so the panel survives a re-render
//! and a second `/memory` focuses it instead of opening a twin (DIRECTION §4.2.2).
//!
//! The data comes from the extension's own files/reader paths (`Memory::list`,
//! `Memory::all_projects`, `Memory::search`) — this module only shapes it:
//!
//! ```text
//! Memory · unipi · 12 memories                    ← header
//! [ Search memories…                          ]   ← Input (on_submit "search")
//! ┌ Project memories ──────────────────────────┐
//! │ › Decisions use the oino host              │   ← List (on_activate "open")
//! │   decision · 2 h ago                       │
//! └────────────────────────────────────────────┘
//! [Forget…] [Consolidate] [All projects] [Close]
//! ```
use oino_extension::prelude::*;
use oino_extension::ui::widgets::prelude::*;
use oino_extension::ui::{KeyBinding, Placement};
use serde_json::Value;

use crate::files::Record;
use crate::search::Hit;

/// The panel id (one per session; `unipi.memory` owns the namespace).
pub const PANEL_ID: &str = "unipi.memory/browse";

/// One row of the list, whatever it was built from (a stored memory or a
/// search hit): key = what `forget` deletes, body = what the detail shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// File/id key.
    pub key: String,
    /// Title.
    pub title: String,
    /// Second line (`decision · 2 h ago`, or the hit's snippet).
    pub detail: String,
    /// The memory's markdown (detail view).
    pub body: String,
    /// Project the memory belongs to (global view groups by it).
    pub project: String,
    /// Type (`preference` / `decision` / `pattern` / `summary`), when known.
    pub kind: String,
}

/// The panel's state for one session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The last search (empty = the stored list).
    pub query: String,
    /// Show every project instead of this one.
    pub global: bool,
    /// The search results replaced the list (`hits` in the extension).
    pub searched: bool,
    /// The opened entry's key (`None` = the list).
    pub open: Option<String>,
    /// A one-line result of the last action ("Forgot …", "No memory matches …").
    pub notice: Option<String>,
}

impl View {
    /// `/memory` opens the list, not a stale detail of another chat.
    #[must_use]
    pub fn fresh() -> Self {
        Self::default()
    }

    /// Back to the list (Esc / `close` on the detail view keep the panel open).
    pub fn to_list(&mut self) {
        self.open = None;
    }
}

/// A stored memory as a row.
#[must_use]
pub fn entry_of_record(record: &Record) -> Entry {
    Entry {
        key: record.id.clone(),
        title: record.title.clone(),
        detail: format!(
            "{} · {}",
            record.kind,
            record.updated.get(..10).unwrap_or(&record.updated)
        ),
        body: record.content.clone(),
        project: record.project.clone(),
        kind: record.kind.clone(),
    }
}

/// Every memory of one project, newest first (the caller sorts).
#[must_use]
pub fn entries_of_project(records: &[Record]) -> Vec<Entry> {
    records.iter().map(entry_of_record).collect()
}

/// Every memory across projects (`global`): the row says which project.
#[must_use]
pub fn entries_of_all(grouped: &[(String, Vec<Record>)]) -> Vec<Entry> {
    grouped
        .iter()
        .flat_map(|(project, records)| {
            records.iter().map(move |record| {
                let mut entry = entry_of_record(record);
                entry.project.clone_from(project);
                entry.detail = format!("{project} · {}", entry.detail);
                entry
            })
        })
        .collect()
}

/// Search hits as rows (`hit` rows forget by title, which `delete` accepts).
#[must_use]
pub fn entries_of_hits(hits: &[Hit]) -> Vec<Entry> {
    hits.iter()
        .map(|hit| Entry {
            key: hit.title.clone(),
            title: hit.title.clone(),
            detail: hit.snippet.clone(),
            body: hit.snippet.clone(),
            project: hit.wing.clone(),
            kind: hit.room.clone(),
        })
        .collect()
}

/// The header line.
#[must_use]
pub fn header(view: &View, project: &str, count: usize) -> String {
    let scope = if view.global {
        "all projects".to_owned()
    } else {
        project.to_owned()
    };
    if view.searched && !view.query.is_empty() {
        return format!(
            "Memory · {scope} · {count} {} for \u{201c}{}\u{201d}",
            if count == 1 { "match" } else { "matches" },
            view.query
        );
    }
    format!(
        "Memory · {scope} · {count} {}",
        if count == 1 { "memory" } else { "memories" }
    )
}

fn buttons(view: &View, has_rows: bool) -> Node {
    let mut children: Vec<Node> = Vec::new();
    if view.open.is_some() {
        children.push(
            Button::new("Forget\u{2026}", "forget")
                .variant(ButtonVariant::Danger)
                .icon(Icon::Trash)
                .confirm("Delete this memory?")
                .into(),
        );
        children.push(Button::new("Back", "back").into());
    } else {
        children.push(
            Button::new("Forget\u{2026}", "forget")
                .variant(ButtonVariant::Danger)
                .icon(Icon::Trash)
                .disabled(!has_rows)
                .confirm("Delete this memory?")
                .into(),
        );
        children.push(Button::new("Consolidate", "consolidate").into());
        children.push(
            Button::new(
                if view.global {
                    "This project"
                } else {
                    "All projects"
                },
                "scope",
            )
            .into(),
        );
    }
    children.push(Button::new("Close", "close").into());
    Layout::horizontal(vec![Constraint::Min(0); children.len()])
        .spacing(1)
        .children(children)
        .into()
}

/// The detail of the opened entry (markdown body + its type/project line).
fn detail(entry: &Entry) -> Vec<Node> {
    vec![
        Paragraph::new(Line::from(vec![
            Span::from_string(entry.title.clone()).bold(),
            Span::from_string(format!("  {} · {}", entry.kind, entry.project)).fg(Color::Muted),
        ]))
        .into(),
        Markdown::new(entry.body.clone()).into(),
    ]
}

/// The panel's node tree. `entries` is already the list the view asks for
/// (stored memories, every project's, or search hits).
#[must_use]
pub fn spec(view: &View, project: &str, entries: &[Entry]) -> Node {
    let mut children: Vec<Node> = vec![
        Paragraph::new(Line::from(vec![
            Span::from_string(header(view, project, entries.len())).fg(Color::Muted),
        ]))
        .into(),
        Input::new("q")
            .value(view.query.clone())
            .placeholder("Search memories\u{2026}")
            .on_submit("search")
            .into(),
    ];
    if let Some(key) = &view.open {
        match entries.iter().find(|entry| &entry.key == key) {
            Some(entry) => children.extend(detail(entry)),
            // The opened memory went away (forgotten in another frontend): the list.
            None => children.push(
                Empty::new()
                    .title("Memory")
                    .message("That memory is gone \u{2014} forget left the list open.")
                    .into(),
            ),
        }
    } else if entries.is_empty() {
        children.push(
            Empty::new()
                .title("Memory")
                .message(if view.query.is_empty() {
                    "Nothing remembered here yet. The agent saves memories as it works, or /memory-consolidate writes what this chat is worth."
                } else {
                    "No memory matches that search."
                })
                .into(),
        );
    } else {
        let rows: Vec<ListItem> = entries
            .iter()
            .map(|entry| {
                ListItem::new(entry.title.clone())
                    .key(entry.key.clone())
                    .detail(Line::from(entry.detail.clone()))
                    .icon(Icon::List)
            })
            .collect();
        children.push(
            List::new(rows)
                .on_activate("open")
                .block(
                    Block::bordered()
                        .title(if view.global {
                            "All memories"
                        } else {
                            "Project memories"
                        })
                        .border_type(BorderType::Rounded),
                )
                .into(),
        );
    }
    if let Some(notice) = &view.notice {
        children.push(
            Paragraph::new(Line::from(
                Span::from_string(notice.clone()).fg(Color::Warning),
            ))
            .into(),
        );
    }
    children.push(buttons(view, !entries.is_empty()));
    children.push(KeyHints::new([("enter", "open"), ("/", "search"), ("esc", "close")]).into());
    Layout::default().spacing(1).children(children).into()
}

/// What one `/memory` command argument (or `ui_action` value) means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `search` (the input's submit, query in `values.q`).
    Search,
    /// `scope`: flip project ↔ all projects.
    Scope,
    /// `open`: show the entry in `value`.
    Open,
    /// `back`: the list.
    Back,
    /// `forget`: delete the opened entry.
    Forget,
    /// `consolidate`: the save pass for this chat.
    Consolidate,
    /// `close`: the panel.
    Close,
    /// An action this panel does not know.
    Unknown,
}

/// Maps an action id.
#[must_use]
pub fn op_of(action: &str) -> Op {
    match action {
        "search" => Op::Search,
        "scope" => Op::Scope,
        "open" | "select" => Op::Open,
        "back" => Op::Back,
        "forget" => Op::Forget,
        "consolidate" => Op::Consolidate,
        "close" | "$close" => Op::Close,
        _ => Op::Unknown,
    }
}

/// The query a `search` action carries (`values.q`, else the `value`).
#[must_use]
pub fn search_query(event: &Value) -> String {
    event
        .pointer("/values/q")
        .or_else(|| event.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Opens (or focuses) the panel for a view.
///
/// # Errors
///
/// Propagates the host call's failure.
pub fn open(view: &View, project: &str, entries: &[Entry]) -> Result<String> {
    ui::Panel::new(PANEL_ID)
        .placement(Placement::Overlay)
        .title("Memory")
        .icon(Icon::Gear)
        .spec(spec(view, project, entries))
        .keys([
            KeyBinding {
                key: "enter".into(),
                action: Some("open".into()),
                label: "open".into(),
            },
            KeyBinding {
                key: "/".into(),
                action: Some("search".into()),
                label: "search".into(),
            },
        ])
        .focus(true)
        .close_on_escape(true)
        .open()
}

/// Re-renders an open panel.
///
/// # Errors
///
/// Propagates the host call's failure.
pub fn update(view: &View, project: &str, entries: &[Entry]) -> Result<()> {
    ui::Panel::update(PANEL_ID)
        .title("Memory")
        .spec(spec(view, project, entries))
        .send()
}
