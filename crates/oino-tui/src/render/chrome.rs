//! Header, status indicator, pending queue, widgets, composer, autocomplete and footer.

use std::fmt::Write as _;

use oino_ai::JsonValue;
use oino_view::format::{format_cwd, format_tokens, single_line, width as text_width};
use oino_view::ui_state::{Placement, Segment, SegmentAlign, Widget};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use super::text::{OLine, boxed, spread, truncate_line};
use super::{Block, RenderCtx, ctx, ous, overlays};
use crate::app::App;

/// pi built-in header (compact unless tools are expanded).
pub(super) fn builtin_header(app: &App, width: usize) -> Vec<OLine> {
    let theme = &app.theme;
    let k = |id: &str| app.keys.key_text(id);
    let dim = Style::default().fg(theme.dim);
    let muted = Style::default().fg(theme.muted);
    let hint = |key: String, text: &str| -> Vec<Span<'static>> {
        vec![
            Span::styled(key, dim),
            Span::styled(format!(" {text}"), muted),
        ]
    };
    let mut out = vec![
        Line::default(),
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "oino",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" v{}", app.options.version), dim),
        ]),
    ];
    if app.tools_expanded {
        let rows: Vec<(String, &str)> = vec![
            (k("app.interrupt"), "to interrupt"),
            (k("app.clear"), "to clear"),
            (format!("{} twice", k("app.clear")), "to exit"),
            (k("app.exit"), "to exit (empty)"),
            (k("app.suspend"), "to suspend"),
            (k("tui.editor.deleteToLineEnd"), "to delete to end"),
            (k("app.thinking.cycle"), "to cycle thinking level"),
            (
                format!(
                    "{}/{}",
                    k("app.model.cycleForward"),
                    k("app.model.cycleBackward")
                ),
                "to cycle models",
            ),
            (k("app.model.select"), "to select model"),
            (k("app.tools.expand"), "to expand tools"),
            (k("app.thinking.toggle"), "to expand thinking"),
            (k("app.editor.external"), "for external editor"),
            ("/".into(), "for commands"),
            ("!".into(), "to run bash"),
            ("!!".into(), "to run bash (no context)"),
            (k("app.message.followUp"), "to queue follow-up"),
            (k("app.message.dequeue"), "to edit all queued messages"),
            (k("app.clipboard.pasteImage"), "to paste images or text"),
            ("drop files".into(), "to attach"),
        ];
        for (key, text) in rows {
            if key.is_empty() {
                continue;
            }
            let mut spans = vec![Span::raw(" ")];
            spans.extend(hint(key, text));
            out.push(truncate_line(Line::from(spans), width));
        }
    } else {
        let sep = Span::styled(" · ", muted);
        let mut spans = vec![Span::raw(" ")];
        spans.extend(hint(k("app.interrupt"), "interrupt"));
        spans.push(sep.clone());
        spans.extend(hint(
            format!("{}/{}", k("app.clear"), k("app.exit")),
            "clear/exit",
        ));
        spans.push(sep.clone());
        spans.extend(hint("/".into(), "commands"));
        spans.push(sep.clone());
        spans.extend(hint("!".into(), "bash"));
        spans.push(sep);
        spans.extend(hint(k("app.tools.expand"), "more"));
        out.push(truncate_line(Line::from(spans), width));
        out.push(truncate_line(
            Line::from(Span::styled(
                format!(
                    " Press {} to show full startup help.",
                    k("app.tools.expand")
                ),
                dim,
            )),
            width,
        ));
    }
    out.push(Line::default());
    out.push(truncate_line(
        Line::from(Span::styled(
            " Oino can explain its own features and look up its docs. Ask it how to use or extend Oino.",
            dim,
        )),
        width,
    ));
    out
}

/// Working / retry / compaction indicator and the pending queue.
pub(super) fn status_lines(app: &App, width: usize) -> Vec<OLine> {
    let theme = &app.theme;
    let muted = Style::default().fg(theme.muted);
    let accent = Style::default().fg(theme.accent);
    let mut out = Vec::new();
    let interrupt = app.keys.key_text("app.interrupt");
    let indicator = if let Some(retry) = &app.retry {
        let secs = retry
            .until
            .saturating_duration_since(std::time::Instant::now())
            .as_secs();
        Some(format!(
            "Retrying ({}/{}) in {secs}s: {} ({interrupt} to cancel)",
            retry.attempt,
            retry.max,
            single_line(&retry.error)
        ))
    } else if app.session.compacting {
        Some(match app.compaction_reason {
            Some("manual") | None => format!("Compacting context... ({interrupt} to cancel)"),
            Some(_) => format!("Auto-compacting... ({interrupt} to cancel)"),
        })
    } else if app.session.streaming && app.ui.working_visible {
        let message = app
            .ui
            .working_message
            .clone()
            .unwrap_or_else(|| "Working...".into());
        Some(format!("{message} ({interrupt} to interrupt)"))
    } else if app.session.run_state == Some(oino_core::events::RunState::Waiting)
        && !app.session.waiting_on.is_empty()
    {
        Some(format!("Waiting on: {}", app.session.waiting_on.join(", ")))
    } else {
        None
    };
    if let Some(text) = indicator {
        out.push(Line::default());
        out.push(truncate_line(
            Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{} ", app.spinner()), accent),
                Span::styled(text, muted),
            ]),
            width,
        ));
    }
    let (steering, follow) = &app.queue;
    if !steering.is_empty() || !follow.is_empty() {
        out.push(Line::default());
        let dim = Style::default().fg(theme.dim);
        for text in steering {
            out.push(truncate_line(
                Line::from(Span::styled(
                    format!(" Steering: {}", single_line(text)),
                    dim,
                )),
                width,
            ));
        }
        for text in follow {
            out.push(truncate_line(
                Line::from(Span::styled(
                    format!(" Follow-up: {}", single_line(text)),
                    dim,
                )),
                width,
            ));
        }
        out.push(truncate_line(
            Line::from(Span::styled(
                format!(
                    " ↳ {} to edit all queued messages",
                    app.keys.key_text("app.message.dequeue")
                ),
                dim,
            )),
            width,
        ));
    }
    out
}

/// Lines of one widget (OUS spec preferred over plain lines).
pub(super) fn widget_body(widget: &Widget, cx: &RenderCtx<'_>) -> Vec<OLine> {
    match &widget.spec {
        Some(spec) => boxed(
            ous::render(
                spec,
                cx.width.saturating_sub(2),
                cx.theme,
                ous::Focus::default(),
                cx.tick,
            ),
            cx.width,
            1,
            None,
        ),
        None => widget
            .lines
            .iter()
            .map(|l| truncate_line(Line::from(format!(" {l}")), cx.width))
            .collect(),
    }
}

/// Widgets at `placement`.
pub(super) fn widget_lines(app: &App, placement: Placement, width: usize) -> Vec<OLine> {
    let cx = ctx(app, width);
    app.ui
        .widgets_at(placement)
        .into_iter()
        .flat_map(|w| widget_body(w, &cx))
        .collect()
}

fn segments_line(segments: &[Segment], width: usize, border: Style, app: &App) -> OLine {
    // Drop lowest priority segments until they fit (ui-protocol §4.4).
    let mut chosen: Vec<&Segment> = segments.iter().collect();
    let fits = |chosen: &[&Segment]| {
        chosen
            .iter()
            .map(|s| {
                text_width(&s.text)
                    + 3
                    + usize::from(s.min_width.unwrap_or(0)).saturating_sub(text_width(&s.text))
            })
            .sum::<usize>()
            + 2
            <= width
    };
    while !chosen.is_empty() && !fits(&chosen) {
        if let Some((i, _)) = chosen.iter().enumerate().min_by_key(|(_, s)| s.priority) {
            chosen.remove(i);
        }
    }
    let piece = |s: &Segment| -> Vec<Span<'static>> {
        let icon = s
            .icon
            .as_deref()
            .map(icon_glyph)
            .map(|g| format!("{g} "))
            .unwrap_or_default();
        vec![
            Span::styled(" ", border),
            Span::styled(
                format!("{icon}{}", s.text),
                Style::default().fg(app.theme.tone(s.tone)),
            ),
            Span::styled(" ", border),
        ]
    };
    let mut left: Vec<Span<'static>> = vec![Span::styled("─", border)];
    let mut right: Vec<Span<'static>> = Vec::new();
    for s in &chosen {
        if s.align == SegmentAlign::Right {
            right.extend(piece(s));
            right.push(Span::styled("─", border));
        } else {
            left.extend(piece(s));
            left.push(Span::styled("─", border));
        }
    }
    let used: usize = left
        .iter()
        .chain(right.iter())
        .map(|s| text_width(&s.content))
        .sum();
    left.push(Span::styled("─".repeat(width.saturating_sub(used)), border));
    left.extend(right);
    truncate_line(Line::from(left), width)
}

/// pi icon names → glyphs.
pub(super) fn icon_glyph(name: &str) -> &'static str {
    match name {
        "check" => "✓",
        "x" => "✗",
        "dot" => "●",
        "warn" => "⚠",
        "info" => "ℹ",
        "clock" => "◷",
        "gear" => "⚙",
        "branch" => "⎇",
        "bolt" => "⚡",
        "bot" => "◉",
        "folder" => "▸",
        "file" => "▫",
        "search" => "⌕",
        "cloud" => "☁",
        "lock" => "🔒",
        _ => "•",
    }
}

/// The editor area: dialog, selector, editor-replacing panel or the composer.
pub(super) fn editor_block(app: &App, width: usize, max_height: usize) -> Block {
    if app.ui.active_dialog().is_some() && !overlays::dialog_is_modal(app) {
        return overlays::dialog_block(app, width, max_height);
    }
    if let Some(overlay) = &app.overlay {
        return overlays::overlay_block(app, overlay, width, max_height);
    }
    if let Some(panel) = app.ui.focused_panel()
        && matches!(
            panel.placement,
            oino_view::ui_state::PanelPlacement::EditorReplace
                | oino_view::ui_state::PanelPlacement::BottomPane
        )
    {
        return overlays::panel_block(app, panel, width, max_height);
    }
    composer_block(app, width, max_height)
}

fn composer_block(app: &App, width: usize, max_height: usize) -> Block {
    let theme = &app.theme;
    let text = app.composer.text();
    let color = if text.trim_start().starts_with('!') {
        theme.bash_mode
    } else {
        theme.thinking_border(app.session.thinking)
    };
    let border = Style::default().fg(color);
    let (tops, bottoms): (Vec<Segment>, Vec<Segment>) =
        app.ui
            .frames
            .values()
            .fold((Vec::new(), Vec::new()), |(mut t, mut b), f| {
                t.extend(f.top.iter().cloned());
                b.extend(f.bottom.iter().cloned());
                (t, b)
            });
    let mut lines = vec![segments_line(&tops, width, border, app)];
    // Character-wrap each logical line; track the cursor.
    let (cursor_line, cursor_col) = app.composer.cursor();
    let text_style = Style::default().fg(theme.text);
    let mut visual: Vec<OLine> = Vec::new();
    let mut cursor = (0u16, 0u16);
    for (index, line) in app.composer.lines().iter().enumerate() {
        let mut row = String::new();
        let mut row_width = 0usize;
        let mut col = 0usize;
        let mut rows: Vec<String> = Vec::new();
        for c in line.chars() {
            let w = c.width().unwrap_or(0);
            if row_width + w > width.saturating_sub(1) {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            if index == cursor_line && col == cursor_col {
                cursor = (
                    u16::try_from(row_width).unwrap_or(0),
                    u16::try_from(visual.len() + rows.len()).unwrap_or(0),
                );
            }
            row.push(c);
            row_width += w;
            col += 1;
        }
        if index == cursor_line && col <= cursor_col {
            if row_width + 1 > width {
                rows.push(std::mem::take(&mut row));
                row_width = 0;
            }
            cursor = (
                u16::try_from(row_width).unwrap_or(0),
                u16::try_from(visual.len() + rows.len()).unwrap_or(0),
            );
        }
        rows.push(row);
        visual.extend(
            rows.into_iter()
                .map(|r| Line::from(Span::styled(r, text_style))),
        );
    }
    let max_rows = max_height.saturating_sub(2).max(1);
    let offset = usize::from(cursor.1).saturating_sub(max_rows - 1);
    let shown: Vec<OLine> = visual.into_iter().skip(offset).take(max_rows).collect();
    cursor.1 = cursor.1.saturating_sub(u16::try_from(offset).unwrap_or(0)) + 1;
    lines.extend(shown);
    lines.push(segments_line(&bottoms, width, border, app));
    Block {
        lines,
        cursor: Some(cursor),
    }
}

/// Autocomplete popup lines (`None` when closed).
pub(super) fn completion_popup(app: &App, width: usize) -> Option<Vec<OLine>> {
    let completion = app.completion.as_ref()?;
    if app.overlay.is_some() || app.ui.active_dialog().is_some() {
        return None;
    }
    let theme = &app.theme;
    let max_visible = app
        .settings
        .get("autocompleteMaxVisible")
        .and_then(JsonValue::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(5)
        .max(1);
    let len = completion.items.len();
    let start = completion
        .selected
        .saturating_sub(max_visible - 1)
        .min(len.saturating_sub(max_visible));
    let name_width = completion
        .items
        .iter()
        .map(|(v, _)| text_width(v))
        .max()
        .unwrap_or(0)
        .min(width / 2);
    let mut out = Vec::new();
    for (i, (value, description)) in completion
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(max_visible)
    {
        let selected = i == completion.selected;
        let mut name_style = Style::default().fg(if selected { theme.accent } else { theme.text });
        let mut desc_style = Style::default().fg(theme.muted);
        if selected {
            name_style = name_style
                .bg(theme.selected_bg)
                .add_modifier(Modifier::BOLD);
            desc_style = desc_style.bg(theme.selected_bg);
        }
        let cursor = if selected { "› " } else { "  " };
        let name = oino_view::format::truncate(value, name_width, "…");
        let pad = name_width.saturating_sub(text_width(&name));
        let line = Line::from(vec![
            Span::styled(cursor.to_owned(), name_style),
            Span::styled(format!("{name}{}  ", " ".repeat(pad)), name_style),
            Span::styled(single_line(description), desc_style),
        ]);
        let mut line = truncate_line(line, width);
        if selected {
            let w = super::text::line_width(&line);
            line.spans.push(Span::styled(
                " ".repeat(width.saturating_sub(w)),
                desc_style,
            ));
        }
        out.push(line);
    }
    if len > max_visible {
        out.push(Line::from(Span::styled(
            format!("  ({}/{len})", completion.selected + 1),
            Style::default().fg(theme.dim),
        )));
    }
    Some(out)
}

/// pi footer: cwd line, stats line, extension statuses; plus extension footers.
#[allow(
    clippy::too_many_lines,
    reason = "mirrors pi FooterComponent.render step by step"
)]
pub(super) fn footer_lines(app: &App, width: usize) -> Vec<OLine> {
    let theme = &app.theme;
    let dim = Style::default().fg(theme.dim);
    let cwd = app.host.cwd();
    let mut pwd = format_cwd(&cwd, app.host.home().as_deref());
    if let Some(branch) = &app.git_branch {
        let _ = write!(pwd, " ({branch})");
    }
    if let Some(name) = &app.session.session_name {
        let _ = write!(pwd, " • {name}");
    }
    let mut out = vec![truncate_line(Line::from(Span::styled(pwd, dim)), width)];
    let u = &app.usage;
    let mut stats: Vec<Span<'static>> = Vec::new();
    let mut push = |text: String| {
        if !stats.is_empty() {
            stats.push(Span::styled(" ", dim));
        }
        stats.push(Span::styled(text, dim));
    };
    if u.input > 0 {
        push(format!("↑{}", format_tokens(u.input)));
    }
    if u.output > 0 {
        push(format!("↓{}", format_tokens(u.output)));
    }
    if u.cache_read > 0 {
        push(format!("R{}", format_tokens(u.cache_read)));
    }
    if u.cache_write > 0 {
        push(format!("W{}", format_tokens(u.cache_write)));
    }
    if u.cost > 0.0 {
        push(format!("${:.3}", u.cost));
    }
    let window = if u.context_window > 0 {
        u.context_window
    } else {
        app.session.model.as_ref().map_or(0, |m| m.context_window)
    };
    let auto = if app.session.auto_compaction {
        " (auto)"
    } else {
        ""
    };
    let context = match u.context_percent {
        Some(p) => format!("{p:.1}%/{}{auto}", format_tokens(window)),
        None => format!("?/{}{auto}", format_tokens(window)),
    };
    let percent = u.context_percent.unwrap_or(0.0);
    let context_style = if percent > 90.0 {
        Style::default().fg(theme.error)
    } else if percent > 70.0 {
        Style::default().fg(theme.warning)
    } else {
        dim
    };
    if !stats.is_empty() {
        stats.push(Span::styled(" ", dim));
    }
    stats.push(Span::styled(context, context_style));
    let model_name = app
        .session
        .model
        .as_ref()
        .map_or("no-model", |m| m.id.as_str())
        .to_owned();
    let mut right = model_name;
    if app.session.model.as_ref().is_some_and(|m| m.reasoning) {
        let level = App::thinking_label(app.session.thinking);
        right = if level == "off" {
            format!("{right} • thinking off")
        } else {
            format!("{right} • {level}")
        };
    }
    if let Some(routed) = &app.session.routed_model {
        let _ = write!(right, " → {}", routed.id);
    }
    let left_width: usize = stats.iter().map(|s| text_width(&s.content)).sum();
    if app.provider_count > 1
        && let Some(model) = &app.session.model
    {
        let with_provider = format!("({}) {right}", model.provider);
        if left_width + 2 + text_width(&with_provider) <= width {
            right = with_provider;
        }
    }
    out.push(spread(stats, vec![Span::styled(right, dim)], width));
    let statuses = app.ui.sorted_statuses();
    if !statuses.is_empty() {
        let mut spans = Vec::new();
        for (i, (_, status)) in statuses.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let style = match status.tone {
                oino_view::ous::Tone::Default => Style::default().fg(theme.text),
                tone => Style::default().fg(theme.tone(tone)),
            };
            spans.push(Span::styled(single_line(&status.text), style));
        }
        out.push(truncate_line(Line::from(spans), width));
    }
    let cx = ctx(app, width);
    for spec in app.ui.footers.values() {
        out.extend(boxed(
            ous::render(
                spec,
                width.saturating_sub(2),
                theme,
                ous::Focus::default(),
                app.tick,
            ),
            width,
            1,
            None,
        ));
    }
    for widget in app.ui.widgets_at(Placement::Footer) {
        out.extend(widget_body(widget, &cx));
    }
    out
}
