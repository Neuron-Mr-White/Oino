//! Selectors, extension dialogs and panels.

use oino_view::composer::Composer;
use oino_view::format::{format_age, shorten_path, single_line, width as text_width};
use oino_view::tree::DisplayKind;
use oino_view::ui_state::{DialogKind, Panel, PanelPlacement};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::text::{OLine, boxed, spread, truncate_line, wrap_plain};
use super::{Block, ous};
use crate::app::overlay::{SettingKind, TreePrompt};
use crate::app::{App, DialogUi, Overlay};

const MAX_ROWS: usize = 10;

fn border_line(color: Color, width: usize) -> OLine {
    Line::from(Span::styled("─".repeat(width), Style::default().fg(color)))
}

fn hint_line(app: &App, hints: &[(&str, &str)], width: usize) -> OLine {
    let dim = Style::default().fg(app.theme.dim);
    let muted = Style::default().fg(app.theme.muted);
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", muted));
        }
        spans.push(Span::styled((*key).to_owned(), dim));
        spans.push(Span::styled(format!(" {label}"), muted));
    }
    truncate_line(Line::from(spans), width)
}

fn key(app: &App, action: &str) -> String {
    app.keys
        .key_text(action)
        .split('/')
        .next()
        .unwrap_or("")
        .to_owned()
}

/// Scrolling window around `selected` with at most `rows` items.
fn window(selected: usize, len: usize, rows: usize) -> (usize, usize) {
    let start = selected
        .saturating_sub(rows / 2)
        .min(len.saturating_sub(rows));
    (start, (start + rows).min(len))
}

fn row_line(
    app: &App,
    selected: bool,
    left: Vec<Span<'static>>,
    right: Option<String>,
    width: usize,
) -> OLine {
    let theme = &app.theme;
    let mut spans = vec![Span::styled(
        if selected { "› " } else { "  " },
        Style::default().fg(theme.accent),
    )];
    spans.extend(left);
    let mut line = match right {
        Some(right) => spread(
            spans,
            vec![Span::styled(right, Style::default().fg(theme.dim))],
            width,
        ),
        None => truncate_line(Line::from(spans), width),
    };
    if selected {
        let w = super::text::line_width(&line);
        line.spans
            .push(Span::raw(" ".repeat(width.saturating_sub(w))));
        for span in &mut line.spans {
            span.style = span.style.bg(theme.selected_bg);
        }
    }
    line
}

fn input_line(prefix: &str, input: &str, app: &App, width: usize) -> OLine {
    truncate_line(
        Line::from(vec![
            Span::styled(prefix.to_owned(), Style::default().fg(app.theme.accent)),
            Span::styled(input.to_owned(), Style::default().fg(app.theme.text)),
            Span::styled("▏", Style::default().fg(app.theme.accent)),
        ]),
        width,
    )
}

/// Selector shown in place of the editor.
#[allow(
    clippy::too_many_lines,
    reason = "one layout per selector, mirroring pi components"
)]
pub(super) fn overlay_block(
    app: &App,
    overlay: &Overlay,
    width: usize,
    max_height: usize,
) -> Block {
    let theme = &app.theme;
    let rows = MAX_ROWS.min(max_height.saturating_sub(5)).max(3);
    let muted = Style::default().fg(theme.muted);
    let text = Style::default().fg(theme.text);
    let mut lines = vec![border_line(theme.border_accent, width)];
    match overlay {
        Overlay::Models(picker) => {
            lines.push(Line::from(Span::styled(
                " Select model",
                text.add_modifier(Modifier::BOLD),
            )));
            lines.push(input_line(" Search: ", &picker.query, app, width));
            let (start, end) = window(picker.selected, picker.filtered.len(), rows);
            for (pos, &i) in picker.filtered.iter().enumerate().take(end).skip(start) {
                let model = &app.models[i];
                let current = app
                    .session
                    .model
                    .as_ref()
                    .is_some_and(|m| m.id == model.id && m.provider == model.provider);
                let mut left = vec![Span::styled(model.id.clone(), text)];
                left.push(Span::styled(format!(" [{}]", model.provider), muted));
                if current {
                    left.push(Span::styled(" ✓", Style::default().fg(theme.success)));
                }
                lines.push(row_line(app, pos == picker.selected, left, None, width));
            }
            if picker.filtered.is_empty() {
                lines.push(Line::from(Span::styled("  No matching models", muted)));
            }
            lines.push(Line::from(Span::styled(
                format!(
                    "  ({}/{})",
                    (picker.selected + 1).min(picker.filtered.len()),
                    picker.filtered.len()
                ),
                muted,
            )));
            lines.push(hint_line(
                app,
                &[
                    ("enter", "select"),
                    ("esc", "cancel"),
                    ("type", "to search"),
                ],
                width,
            ));
        }
        Overlay::Scoped(picker) => {
            lines.push(Line::from(Span::styled(
                " Scoped models (cycle order)",
                text.add_modifier(Modifier::BOLD),
            )));
            lines.push(input_line(" Search: ", &picker.query, app, width));
            let (start, end) = window(picker.selected, picker.filtered.len(), rows);
            for (pos, &i) in picker.filtered.iter().enumerate().take(end).skip(start) {
                let label = crate::app::overlay::model_label(&app.models[i]);
                let order = picker.enabled.iter().position(|e| *e == label);
                let mark = order.map_or_else(|| "[ ] ".to_owned(), |n| format!("[{}] ", n + 1));
                let left = vec![
                    Span::styled(
                        mark,
                        Style::default().fg(if order.is_some() {
                            theme.success
                        } else {
                            theme.dim
                        }),
                    ),
                    Span::styled(label, text),
                ];
                lines.push(row_line(app, pos == picker.selected, left, None, width));
            }
            lines.push(hint_line(
                app,
                &[
                    ("enter", "toggle"),
                    (&key(app, "app.models.enableAll"), "all"),
                    (&key(app, "app.models.clearAll"), "clear"),
                    (&key(app, "app.models.toggleProvider"), "provider"),
                    (&key(app, "app.models.reorderUp"), "move"),
                    (&key(app, "app.models.save"), "save"),
                    ("esc", "close"),
                ],
                width,
            ));
        }
        Overlay::Thinking { selected } => {
            lines.push(Line::from(Span::styled(
                " Thinking level",
                text.add_modifier(Modifier::BOLD),
            )));
            for (i, level) in app.thinking_levels.iter().enumerate() {
                let name = App::thinking_label(*level);
                let mut left = vec![Span::styled(
                    name.to_owned(),
                    Style::default().fg(theme.thinking_border(*level)),
                )];
                if *level == app.session.thinking {
                    left.push(Span::styled(" ✓", Style::default().fg(theme.success)));
                }
                lines.push(row_line(app, i == *selected, left, None, width));
            }
            lines.push(hint_line(
                app,
                &[("enter", "select"), ("esc", "cancel")],
                width,
            ));
        }
        Overlay::Sessions(picker) => {
            let model = &picker.model;
            let scope = match model.scope {
                oino_view::sessions::Scope::Current => "◉ Current Folder | ○ All",
                oino_view::sessions::Scope::All => "○ Current Folder | ◉ All",
            };
            let right = format!(
                "{scope}  Name: {}  Sort: {}",
                if model.named_only { "Named" } else { "All" },
                model.sort.label()
            );
            lines.push(spread(
                vec![Span::styled(
                    format!(" {}", model.title()),
                    text.add_modifier(Modifier::BOLD),
                )],
                vec![Span::styled(right, Style::default().fg(theme.accent))],
                width,
            ));
            if let Some(input) = &picker.rename {
                lines.push(input_line(" Rename: ", &input.text(), app, width));
            } else {
                lines.push(input_line(" Search: ", &model.query, app, width));
            }
            if model.loading {
                lines.push(Line::from(Span::styled("  Loading sessions...", muted)));
            } else if model.rows().is_empty() {
                lines.push(Line::from(Span::styled("  No sessions found", muted)));
            }
            let now = now_ms();
            let home = app.host.home().map(|h| h.display().to_string());
            let (start, end) = window(model.selected(), model.rows().len(), rows);
            for (pos, row) in model.rows().iter().enumerate().take(end).skip(start) {
                let Some(item) = model.item(row.index) else {
                    continue;
                };
                let is_current = model.current_path.as_deref() == Some(item.path.as_str());
                let deleting = model.confirm_delete.as_deref() == Some(item.path.as_str());
                let label = single_line(item.name.as_deref().unwrap_or(&item.first_message));
                let color = if deleting {
                    theme.error
                } else if is_current {
                    theme.accent
                } else if item.name.is_some() {
                    theme.warning
                } else {
                    theme.text
                };
                let mut right = format!(
                    "{} {}",
                    item.message_count,
                    format_age(item.modified_ms, now)
                );
                if model.scope == oino_view::sessions::Scope::All && !item.cwd.is_empty() {
                    right = format!("{} {right}", shorten_path(&item.cwd, home.as_deref()));
                }
                if model.show_path {
                    right = format!("{} {right}", shorten_path(&item.path, home.as_deref()));
                }
                let left = vec![
                    Span::styled(row.prefix.clone(), Style::default().fg(theme.dim)),
                    Span::styled(label, Style::default().fg(color)),
                ];
                lines.push(row_line(
                    app,
                    pos == model.selected(),
                    left,
                    Some(right),
                    width,
                ));
            }
            if model.confirm_delete.is_some() {
                lines.push(Line::from(Span::styled(
                    " Delete session? enter confirm · esc cancel",
                    Style::default().fg(theme.error),
                )));
            } else if let Some(status) = &picker.status {
                lines.push(Line::from(Span::styled(
                    format!(" {status}"),
                    Style::default().fg(theme.error),
                )));
            }
            lines.push(hint_line(
                app,
                &[
                    ("enter", "resume"),
                    ("tab", "scope"),
                    (&key(app, "app.session.togglePath"), "path"),
                    (&key(app, "app.session.toggleSort"), "sort"),
                    (&key(app, "app.session.toggleNamedFilter"), "named"),
                    (&key(app, "app.session.rename"), "rename"),
                    (&key(app, "app.session.delete"), "delete"),
                    ("esc", "close"),
                ],
                width,
            ));
        }
        Overlay::Tree(tree) => {
            lines.push(Line::from(Span::styled(
                " Session tree",
                text.add_modifier(Modifier::BOLD),
            )));
            if !tree.model.query.is_empty() {
                lines.push(input_line(" Search: ", &tree.model.query, app, width));
            }
            let tree_rows = (max_height.saturating_sub(6)).clamp(3, 20);
            let (start, end) = tree.model.window(tree_rows);
            for (offset, row) in tree.model.rows(start, end).into_iter().enumerate() {
                let selected = start + offset == tree.model.selected();
                let mut left = vec![Span::styled(
                    row.prefix.clone(),
                    Style::default().fg(theme.dim),
                )];
                if row.folded {
                    left.push(Span::styled("⊞ ", Style::default().fg(theme.accent)));
                }
                if row.on_active_path {
                    left.push(Span::styled("• ", Style::default().fg(theme.accent)));
                }
                if let Some(label) = &row.label {
                    left.push(Span::styled(
                        format!("[{label}] "),
                        Style::default().fg(theme.warning),
                    ));
                }
                if let Some(ts) = &row.label_timestamp {
                    left.push(Span::styled(
                        format!("{} ", ts.get(11..16).unwrap_or(ts)),
                        muted,
                    ));
                }
                let (prefix_color, body_color) = match row.text.kind {
                    DisplayKind::User => (theme.accent, theme.text),
                    DisplayKind::Assistant => (theme.success, theme.text),
                    DisplayKind::AssistantError => (theme.success, theme.error),
                    DisplayKind::AssistantMuted => (theme.success, theme.muted),
                    DisplayKind::Tool => (theme.muted, theme.muted),
                    DisplayKind::Dim => (theme.dim, theme.dim),
                    DisplayKind::Custom => (theme.custom_message_label, theme.text),
                    DisplayKind::Compaction => (theme.border_accent, theme.border_accent),
                    DisplayKind::BranchSummary => (theme.warning, theme.text),
                };
                let mut body = Style::default().fg(body_color);
                if selected {
                    body = body.add_modifier(Modifier::BOLD);
                }
                left.push(Span::styled(
                    row.text.prefix.clone(),
                    Style::default().fg(prefix_color),
                ));
                left.push(Span::styled(row.text.body.clone(), body));
                lines.push(row_line(app, selected, left, None, width));
            }
            if tree.model.is_empty() {
                lines.push(Line::from(Span::styled("  No entries found", muted)));
            }
            lines.push(Line::from(Span::styled(
                format!("  {}", tree.model.status()),
                muted,
            )));
            match &tree.prompt {
                Some(TreePrompt::Label(_, input)) => {
                    lines.push(input_line(" Label: ", &input.text(), app, width));
                }
                Some(TreePrompt::Instructions(_, input)) => {
                    lines.push(input_line(
                        " Summary instructions: ",
                        &input.text(),
                        app,
                        width,
                    ));
                }
                Some(TreePrompt::Summary { selected, .. }) => {
                    lines.push(Line::from(Span::styled(
                        " Summarize the branch you are leaving?",
                        text,
                    )));
                    for (i, option) in ["No summary", "Summarize", "Summarize with custom prompt"]
                        .iter()
                        .enumerate()
                    {
                        lines.push(row_line(
                            app,
                            i == *selected,
                            vec![Span::styled((*option).to_owned(), text)],
                            None,
                            width,
                        ));
                    }
                }
                None => lines.push(hint_line(
                    app,
                    &[
                        ("enter", "go"),
                        (&key(app, "app.tree.foldOrUp"), "fold"),
                        (&key(app, "app.tree.filter.cycleForward"), "filter"),
                        (&key(app, "app.tree.editLabel"), "label"),
                        (&key(app, "app.message.copy"), "copy"),
                        ("type", "search"),
                        ("esc", "close"),
                    ],
                    width,
                )),
            }
        }
        Overlay::Fork(picker) => {
            lines.push(Line::from(Span::styled(
                " Fork from message",
                text.add_modifier(Modifier::BOLD),
            )));
            if !picker.query.is_empty() {
                lines.push(input_line(" Search: ", &picker.query, app, width));
            }
            let (start, end) = window(picker.selected, picker.filtered.len(), rows);
            for (pos, &i) in picker.filtered.iter().enumerate().take(end).skip(start) {
                let left = vec![Span::styled(single_line(&picker.items[i].1), text)];
                lines.push(row_line(app, pos == picker.selected, left, None, width));
            }
            lines.push(hint_line(
                app,
                &[("enter", "fork"), ("esc", "cancel"), ("type", "to search")],
                width,
            ));
        }
        Overlay::Settings(settings) => {
            lines.push(Line::from(Span::styled(
                " Settings",
                text.add_modifier(Modifier::BOLD),
            )));
            let label_width = settings
                .items
                .iter()
                .map(|i| text_width(i.label))
                .max()
                .unwrap_or(0);
            let (start, end) = window(settings.selected, settings.items.len(), rows);
            for (i, item) in settings.items.iter().enumerate().take(end).skip(start) {
                let value = match &item.kind {
                    SettingKind::Bool(b) => if *b { "on" } else { "off" }.to_owned(),
                    SettingKind::Choice(_, v) => v.clone(),
                };
                let pad = label_width.saturating_sub(text_width(item.label));
                let left = vec![
                    Span::styled(format!("{}{}  ", item.label, " ".repeat(pad)), text),
                    Span::styled(value, Style::default().fg(theme.accent)),
                ];
                lines.push(row_line(app, i == settings.selected, left, None, width));
            }
            if let Some(item) = settings.items.get(settings.selected) {
                lines.push(truncate_line(
                    Line::from(Span::styled(format!("  {}", item.description), muted)),
                    width,
                ));
            }
            if let Some(status) = &settings.status {
                lines.extend(wrap_plain(
                    &format!(" {status}"),
                    Style::default().fg(theme.warning),
                    width,
                ));
            }
            lines.push(hint_line(
                app,
                &[
                    ("enter/space", "change"),
                    ("←/→", "cycle"),
                    ("esc", "close"),
                ],
                width,
            ));
        }
    }
    Block::new(lines)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Whether the active dialog renders as a modal over the transcript (large custom dialogs).
pub(super) fn dialog_is_modal(app: &App) -> bool {
    app.ui.active_dialog().is_some_and(|d| {
        matches!(&d.kind, DialogKind::Custom { size, .. } if matches!(size.as_deref(), Some("large" | "full")))
    })
}

fn countdown(
    app: &App,
    timeout: Option<u64>,
    opened: Option<std::time::Instant>,
) -> Option<String> {
    let _ = app;
    let (timeout, opened) = (timeout?, opened?);
    let left = timeout
        .saturating_sub(u64::try_from(opened.elapsed().as_millis()).unwrap_or(u64::MAX))
        / 1000;
    Some(format!(" ({left}s)"))
}

fn composer_lines(
    input: &Composer,
    app: &App,
    width: usize,
    mask: bool,
) -> (Vec<OLine>, (u16, u16)) {
    let text = Style::default().fg(app.theme.text);
    let mut lines = Vec::new();
    for line in input.lines() {
        let shown = if mask {
            "•".repeat(line.chars().count())
        } else {
            line.clone()
        };
        lines.extend(
            wrap_plain(&shown, text, width.saturating_sub(2))
                .into_iter()
                .map(|l| {
                    let mut spans = vec![Span::raw(" ")];
                    spans.extend(l.spans);
                    Line::from(spans)
                }),
        );
    }
    let (line, col) = input.cursor();
    let x = u16::try_from(col + 1).unwrap_or(0);
    let y = u16::try_from(line).unwrap_or(0);
    (
        lines,
        (
            x.min(u16::try_from(width.saturating_sub(1)).unwrap_or(0)),
            y,
        ),
    )
}

/// Dialog in place of the editor (pi `ExtensionSelector` / `ExtensionInput` / `ExtensionEditor`).
#[allow(
    clippy::too_many_lines,
    reason = "one layout per dialog kind (ui-protocol §4)"
)]
pub(super) fn dialog_block(app: &App, width: usize, max_height: usize) -> Block {
    let theme = &app.theme;
    let Some(dialog) = app.ui.active_dialog() else {
        return Block::new(Vec::new());
    };
    let text = Style::default().fg(theme.text);
    let muted = Style::default().fg(theme.muted);
    let bold = text.add_modifier(Modifier::BOLD);
    let mut lines = vec![border_line(theme.border_accent, width)];
    let mut cursor = None;
    let opened = match &app.dialog_ui {
        Some(DialogUi::List { opened, .. } | DialogUi::Input { opened, .. }) => Some(*opened),
        _ => None,
    };
    let timer = countdown(app, dialog.timeout_ms, opened).unwrap_or_default();
    let title = |t: &str| {
        Line::from(vec![
            Span::styled(format!(" {t}"), bold),
            Span::styled(timer.clone(), muted),
        ])
    };
    let queued = app.ui.dialogs.len().saturating_sub(1);
    match (&dialog.kind, &app.dialog_ui) {
        (DialogKind::Select { title: t, options }, Some(DialogUi::List { selected, .. })) => {
            lines.push(title(t));
            let rows = MAX_ROWS.min(max_height.saturating_sub(4)).max(3);
            let (start, end) = window(*selected, options.len(), rows);
            for (i, option) in options.iter().enumerate().take(end).skip(start) {
                let mut left = vec![Span::styled(option.label.clone(), text)];
                if let Some(d) = &option.description {
                    left.push(Span::styled(format!("  {d}"), muted));
                }
                lines.push(row_line(app, i == *selected, left, None, width));
            }
            lines.push(hint_line(
                app,
                &[("↑↓", "navigate"), ("enter", "select"), ("esc", "cancel")],
                width,
            ));
        }
        (DialogKind::Confirm { title: t, message }, Some(DialogUi::List { selected, .. })) => {
            lines.push(title(t));
            if !message.is_empty() {
                lines.extend(
                    wrap_plain(message, text, width.saturating_sub(2))
                        .into_iter()
                        .map(|l| indent1(l)),
                );
            }
            lines.push(row_line(
                app,
                *selected == 0,
                vec![Span::styled("Yes", text)],
                None,
                width,
            ));
            lines.push(row_line(
                app,
                *selected == 1,
                vec![Span::styled("No", text)],
                None,
                width,
            ));
            lines.push(hint_line(
                app,
                &[("enter", "confirm"), ("y/n", "answer"), ("esc", "cancel")],
                width,
            ));
        }
        (
            DialogKind::Approval {
                title: t,
                tool_name,
                summary,
                detail,
                options,
                ..
            },
            Some(DialogUi::List {
                selected,
                feedback,
                feedback_focus,
                ..
            }),
        ) => {
            lines.push(title(t));
            if let Some(tool) = tool_name {
                lines.push(Line::from(Span::styled(format!(" tool: {tool}"), muted)));
            }
            lines.extend(
                wrap_plain(summary, text, width.saturating_sub(2))
                    .into_iter()
                    .map(indent1),
            );
            if let Some(detail) = detail {
                let body = ous::render(
                    detail,
                    width.saturating_sub(4),
                    theme,
                    ous::Focus::default(),
                    app.tick,
                );
                let max = max_height.saturating_sub(options.len() + 6).max(3);
                lines.extend(boxed(body.into_iter().take(max).collect(), width, 2, None));
            }
            for (i, option) in options.iter().enumerate() {
                let color = if option.kind.starts_with("allow") {
                    theme.success
                } else {
                    theme.error
                };
                let left = vec![Span::styled(
                    option.label.clone(),
                    Style::default().fg(color),
                )];
                lines.push(row_line(
                    app,
                    i == *selected && !feedback_focus,
                    left,
                    None,
                    width,
                ));
            }
            if let Some(input) = feedback {
                let placeholder = match &dialog.kind {
                    DialogKind::Approval {
                        feedback: Some(p), ..
                    } => p.clone(),
                    _ => String::new(),
                };
                let shown = if input.is_empty() && !feedback_focus {
                    placeholder
                } else {
                    input.text()
                };
                let style = if *feedback_focus { text } else { muted };
                lines.push(Line::from(vec![
                    Span::styled(" feedback: ", muted),
                    Span::styled(shown, style),
                ]));
                if *feedback_focus {
                    let x = 11 + input.cursor().1;
                    cursor = Some((
                        u16::try_from(x).unwrap_or(0),
                        u16::try_from(lines.len() - 1).unwrap_or(0),
                    ));
                }
            }
            lines.push(hint_line(
                app,
                &[("enter", "choose"), ("tab", "feedback"), ("esc", "cancel")],
                width,
            ));
        }
        (
            DialogKind::Input {
                title: t,
                placeholder,
                secret,
                ..
            },
            Some(DialogUi::Input { input, .. }),
        ) => {
            lines.push(title(t));
            if input.is_empty()
                && let Some(p) = placeholder
            {
                lines.push(Line::from(Span::styled(format!(" {p}"), muted)));
                cursor = Some((1, u16::try_from(lines.len() - 1).unwrap_or(0)));
            } else {
                let top = lines.len();
                let (body, (x, y)) = composer_lines(input, app, width, *secret);
                lines.extend(body);
                cursor = Some((x, u16::try_from(top).unwrap_or(0) + y));
            }
            lines.push(hint_line(
                app,
                &[("enter", "submit"), ("esc", "cancel")],
                width,
            ));
        }
        (DialogKind::Editor { title: t, .. }, Some(DialogUi::Editor { input, .. })) => {
            lines.push(title(t));
            let top = lines.len();
            let (body, (x, y)) = composer_lines(input, app, width, false);
            let max = max_height.saturating_sub(4).max(3);
            let skip = usize::from(y).saturating_sub(max - 1);
            lines.extend(body.into_iter().skip(skip).take(max));
            cursor = Some((x, u16::try_from(top + usize::from(y) - skip).unwrap_or(0)));
            lines.push(hint_line(
                app,
                &[
                    ("enter", "submit"),
                    (&key(app, "tui.input.newLine"), "newline"),
                    (&key(app, "app.editor.external"), "external editor"),
                    ("esc", "cancel"),
                ],
                width,
            ));
        }
        (
            DialogKind::MultiSelect {
                title: t, options, ..
            },
            Some(DialogUi::Multi {
                selected,
                checked,
                error,
                ..
            }),
        ) => {
            lines.push(title(t));
            let rows = MAX_ROWS.min(max_height.saturating_sub(4)).max(3);
            let (start, end) = window(*selected, options.len(), rows);
            for (i, option) in options.iter().enumerate().take(end).skip(start) {
                let on = checked.get(i).copied().unwrap_or(false);
                let mut left = vec![
                    Span::styled(
                        if on { "[x] " } else { "[ ] " },
                        Style::default().fg(if on { theme.success } else { theme.dim }),
                    ),
                    Span::styled(option.label.clone(), text),
                ];
                if let Some(d) = &option.description {
                    left.push(Span::styled(format!("  {d}"), muted));
                }
                lines.push(row_line(app, i == *selected, left, None, width));
            }
            if let Some(error) = error {
                lines.push(Line::from(Span::styled(
                    format!(" {error}"),
                    Style::default().fg(theme.error),
                )));
            }
            lines.push(hint_line(
                app,
                &[("space", "toggle"), ("enter", "submit"), ("esc", "cancel")],
                width,
            ));
        }
        (
            DialogKind::Form {
                title: t,
                questions,
                ..
            },
            Some(DialogUi::Form {
                question,
                selected,
                answers,
                editing_other,
                ..
            }),
        ) => {
            lines.push(title(t));
            // Question tabs.
            let mut tabs = vec![Span::raw(" ")];
            for (i, q) in questions.iter().enumerate() {
                let label = q.header.clone().unwrap_or_else(|| format!("Q{}", i + 1));
                let done = answers.get(i).is_some_and(|a| a.answered);
                let style = if i == *question {
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else if done {
                    Style::default().fg(theme.success)
                } else {
                    muted
                };
                tabs.push(Span::styled(
                    format!("{}{label} ", if done { "✓" } else { "" }),
                    style,
                ));
            }
            lines.push(truncate_line(Line::from(tabs), width));
            if let (Some(q), Some(answer)) = (questions.get(*question), answers.get(*question)) {
                lines.extend(
                    wrap_plain(&q.question, bold, width.saturating_sub(2))
                        .into_iter()
                        .map(indent1),
                );
                for (i, option) in q.options.iter().enumerate() {
                    let on = answer.checked.get(i).copied().unwrap_or(false);
                    let mark = if q.multi_select {
                        if on { "[x] " } else { "[ ] " }
                    } else if on {
                        "● "
                    } else {
                        "○ "
                    };
                    let mut left = vec![
                        Span::styled(format!("{}. ", i + 1), muted),
                        Span::styled(
                            mark,
                            Style::default().fg(if on { theme.success } else { theme.dim }),
                        ),
                        Span::styled(option.label.clone(), text),
                    ];
                    if let Some(d) = &option.description {
                        left.push(Span::styled(format!("  {d}"), muted));
                    }
                    lines.push(row_line(app, i == *selected, left, None, width));
                }
                if q.other {
                    let on_other = *selected == q.options.len();
                    let other = answer.other.text();
                    let shown = if other.is_empty() && !*editing_other {
                        "Other…".to_owned()
                    } else {
                        other
                    };
                    lines.push(row_line(
                        app,
                        on_other,
                        vec![Span::styled(shown, text)],
                        None,
                        width,
                    ));
                    if *editing_other {
                        cursor = Some((
                            u16::try_from(2 + answer.other.cursor().1).unwrap_or(0),
                            u16::try_from(lines.len() - 1).unwrap_or(0),
                        ));
                    }
                }
            }
            if queued > 0 {
                lines.push(Line::from(Span::styled(
                    format!(" +{queued} more dialogs queued"),
                    muted,
                )));
            }
            lines.push(hint_line(
                app,
                &[
                    ("1-9/enter", "choose"),
                    ("space", "toggle"),
                    ("←/→", "questions"),
                    ("esc", "cancel"),
                ],
                width,
            ));
        }
        (
            DialogKind::Custom { title: t, spec, .. },
            Some(DialogUi::Custom {
                focus,
                values,
                input,
                choice,
                ..
            }),
        ) => {
            if let Some(t) = t {
                lines.push(title(t));
            }
            let focus = ous::Focus {
                index: Some(*focus),
                choice: *choice,
                values: Some(values),
                editing: input.as_ref().map(leak_text),
            };
            let body = ous::render(spec, width.saturating_sub(2), theme, focus, app.tick);
            let max = max_height.saturating_sub(3).max(3);
            lines.extend(boxed(body.into_iter().take(max).collect(), width, 1, None));
            lines.push(hint_line(
                app,
                &[("tab", "focus"), ("enter", "activate"), ("esc", "cancel")],
                width,
            ));
        }
        (DialogKind::ClipboardReadImage, _) => {
            lines.push(Line::from(Span::styled(" Reading clipboard…", muted)));
        }
        (DialogKind::EditExternally { .. }, _) => {
            lines.push(Line::from(Span::styled(
                " Waiting for external editor…",
                muted,
            )));
        }
        _ => {}
    }
    if queued > 0 && !matches!(dialog.kind, DialogKind::Form { .. }) {
        lines.push(Line::from(Span::styled(
            format!(" +{queued} more dialogs queued"),
            muted,
        )));
    }
    Block { lines, cursor }
}

/// Text of a composer for display while focused inside OUS (single line).
fn leak_text(input: &Composer) -> &str {
    input.lines().first().map_or("", String::as_str)
}

fn indent1(line: OLine) -> OLine {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(line.spans);
    Line::from(spans)
}

/// Panel lines (title, spec, key hints) for its placement.
pub(super) fn panel_block(app: &App, panel: &Panel, width: usize, max_height: usize) -> Block {
    let theme = &app.theme;
    let mut lines = vec![border_line(theme.border, width)];
    if let Some(title) = &panel.title {
        lines.push(Line::from(Span::styled(
            format!(" {title}"),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )));
    }
    let focus = ous::Focus {
        index: Some(app.panel_focus),
        choice: app.panel_choice,
        values: None,
        editing: app.panel_input.as_ref().map(leak_text),
    };
    let body = ous::render(&panel.spec, width.saturating_sub(2), theme, focus, app.tick);
    let max = max_height.saturating_sub(lines.len() + 2).max(1);
    lines.extend(boxed(body.into_iter().take(max).collect(), width, 1, None));
    let mut hints: Vec<(String, String)> = panel
        .keys
        .iter()
        .map(|k| (k.key.clone(), k.label.clone()))
        .collect();
    if panel.close_on_escape {
        hints.push(("esc".into(), "close".into()));
    }
    let hint_refs: Vec<(&str, &str)> = hints
        .iter()
        .map(|(k, l)| (k.as_str(), l.as_str()))
        .collect();
    lines.push(hint_line(app, &hint_refs, width));
    Block::new(lines)
}

/// Modal content over the transcript: overlay/side-pane panels and large custom dialogs.
pub(super) fn modal(app: &App, width: usize, height: usize) -> Option<Block> {
    let theme = &app.theme;
    let box_width = (width * 4 / 5).max(30).min(width);
    let inner_height = height.saturating_sub(2);
    let content = if dialog_is_modal(app) {
        let mut block = dialog_block(app, box_width - 2, inner_height);
        if !block.lines.is_empty() {
            block.lines.remove(0);
        }
        block
    } else if app.overlay.is_none() && app.ui.active_dialog().is_none() {
        let panel = app.ui.panels.iter().rev().find(|p| {
            matches!(
                p.placement,
                PanelPlacement::Overlay | PanelPlacement::SidePane
            )
        })?;
        let mut block = panel_block(app, panel, box_width - 2, inner_height);
        block.lines.remove(0);
        block
    } else {
        return None;
    };
    let border = Style::default().fg(theme.border_accent);
    let inner = box_width - 2;
    let mut lines = vec![Line::from(Span::styled(
        format!("╭{}╮", "─".repeat(inner)),
        border,
    ))];
    for line in content.lines.into_iter().take(inner_height) {
        let line = truncate_line(line, inner);
        let w = super::text::line_width(&line);
        let mut spans = vec![Span::styled("│", border)];
        spans.extend(line.spans);
        spans.push(Span::raw(" ".repeat(inner.saturating_sub(w))));
        spans.push(Span::styled("│", border));
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(inner)),
        border,
    )));
    let cursor = content.cursor.map(|(x, y)| (x + 1, y + 1));
    Some(Block { lines, cursor })
}
