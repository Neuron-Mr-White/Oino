//! The node-building half of the §11.3 example, as a standalone example: it must
//! compile for `wasm32-wasip2` (acceptance of WP-U1). The open / update / close
//! calls (`ui::Panel`) live in `oino_extension::ui` (WP-U6) and re-export these
//! types, so the guest-facing code shape is identical.

use oino_ui::prelude::*;

/// A task row of the kanboard demo.
struct Task {
    id: String,
    title: String,
    status: String,
    priority: String,
}

/// The board panel of ui-protocol v2 §11.3.
#[must_use]
fn board_panel(tasks: &[Task]) -> Node {
    let header = Paragraph::new(Line::from(vec![
        "Board ".bold(),
        Span::from(format!("{} open", tasks.len())).fg(Color::Muted),
    ]));
    let list = List::new(tasks.iter().map(|t| {
        ListItem::new(t.title.as_str())
            .key(&t.id)
            .detail(Line::from(t.status.as_str()).fg(Color::Dim))
            .badge(Span::from(t.priority.as_str()).bg(Color::WarningBg))
    }))
    .highlight_symbol("› ")
    .on_select("select-task")
    .on_activate("open-task")
    .block(
        Block::bordered()
            .title("Tasks")
            .border_type(BorderType::Rounded),
    );
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .children([
        header.into(),
        list.into(),
        KeyHints::new([("enter", "open"), ("n", "new")])
            .hint_action("n", "new-task")
            .into(),
    ])
    .into()
}

fn main() {
    let tasks = vec![Task {
        id: "T-12".into(),
        title: "Rewire panel".into(),
        status: "open".into(),
        priority: "high".into(),
    }];
    let node = board_panel(&tasks);
    // In the guest this goes out via ui::Panel::new("tasks")
    //     .placement(Placement::SidePane).spec(node).open();
    let json = serde_json::to_string_pretty(&node).unwrap_or_default();
    let _ = json; // in the guest this is the openPanel payload
}
