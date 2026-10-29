//! `LogView` (design.md §4.22, A2.20 P5): monospace lines with ANSI SGR colors from the `ansi`
//! tokens, at the unipi tool-output size (mono 11.5/17 at the default code size 12) on a
//! surface-1 card (radius 12, inset `line`). Callers cap the line count ([`MAX_LINES`]).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, div, px,
};

use gpui_kit::{Pixels, Styled};

use crate::design::theme::OinoTheme;
use crate::design::{radius, space};

/// Lines a log view renders at most (design.md §4.22: 5 000 in OUS and local logs here).
pub const MAX_LINES: usize = 5000;

/// Log text `(size, line height)`: the code size less ½ px, line ≈ 1.42 × (12 → 11.5/17).
#[must_use]
pub fn log_metrics(t: &OinoTheme) -> (Pixels, Pixels) {
    let (code, _) = t.code_text();
    let size = code * (23.0 / 24.0);
    let line = (f32::from(size) * (17.0 / 11.5)).round();
    (size, px(line))
}

/// Applies the log font (mono 11.5/17).
pub fn log_text<E: Styled>(el: E, t: &OinoTheme) -> E {
    let (size, line) = log_metrics(t);
    el.font_family(t.mono_font())
        .text_size(size)
        .line_height(line)
}

/// The last `max` of `lines` (the head is dropped: a log view follows the tail).
#[must_use]
pub fn tail_window(lines: &[String], max: usize) -> &[String] {
    &lines[lines.len().saturating_sub(max)..]
}

/// One colored run of a log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnsiRun {
    /// Text.
    pub text: String,
    /// ANSI color index (0–15) when set.
    pub color: Option<usize>,
    /// Bold.
    pub bold: bool,
}

/// Parses SGR color codes (`ESC[...m`) out of `line`.
#[must_use]
pub fn parse_ansi(line: &str) -> Vec<AnsiRun> {
    let mut out = Vec::new();
    let mut color = None;
    let mut bold = false;
    let mut rest = line;
    while let Some(i) = rest.find('\u{1b}') {
        if i > 0 {
            out.push(AnsiRun {
                text: rest[..i].to_owned(),
                color,
                bold,
            });
        }
        let tail = &rest[i + 1..];
        let Some(end) = tail.find(|c: char| c.is_ascii_alphabetic()) else {
            rest = "";
            break;
        };
        if tail.starts_with('[') && tail.as_bytes().get(end) == Some(&b'm') {
            for code in tail[1..end].split(';') {
                match code.parse::<usize>().unwrap_or(0) {
                    0 => {
                        color = None;
                        bold = false;
                    }
                    1 => bold = true,
                    22 => bold = false,
                    c @ 30..=37 => color = Some(c - 30),
                    39 => color = None,
                    c @ 90..=97 => color = Some(c - 90 + 8),
                    _ => {}
                }
            }
        }
        rest = &tail[end + 1..];
    }
    if !rest.is_empty() {
        out.push(AnsiRun {
            text: rest.to_owned(),
            color,
            bold,
        });
    }
    out
}

/// Renders log lines (at most [`MAX_LINES`], the tail; the side pane scrolls).
#[must_use]
pub fn log_lines(t: &OinoTheme, lines: &[String], key: &str) -> AnyElement {
    log_text(
        div()
            .id(ElementId::Name(format!("{key}/log").into()))
            .flex()
            .flex_col()
            .w_full()
            .py(px(space::S2))
            .px(px(space::S3))
            .rounded(px(radius::R_TOOL))
            .bg(t.card())
            .border_1()
            .border_color(t.border())
            .text_color(t.fg_subtle())
            // Round-5 #21: the log strip never steals vertical gestures — a scroll
            // gesture stays on the axis it started on (Zed editor-hscroll precedent).
            .overflow_x_scroll()
            .restrict_scroll_to_axis(),
        t,
    )
    .children(tail_window(lines, MAX_LINES).iter().map(|l| line(t, l)))
    .into_any_element()
}

/// One ANSI-colored line.
#[must_use]
pub fn line(t: &OinoTheme, text: &str) -> AnyElement {
    if !text.contains('\u{1b}') {
        return div()
            .whitespace_nowrap()
            .child(SharedString::from(text.to_owned()))
            .into_any_element();
    }
    div()
        .flex()
        .flex_row()
        .whitespace_nowrap()
        .children(parse_ansi(text).into_iter().map(|r| {
            div()
                .when_some(r.color, |el, c| el.text_color(t.ansi(c)))
                .when(
                    r.bold,
                    super::super::design::theme::TextExt::weight_semibold,
                )
                .child(SharedString::from(r.text))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr() {
        let runs = parse_ansi("a\u{1b}[1;31merr\u{1b}[0m b");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].color, Some(1));
        assert!(runs[1].bold);
        assert_eq!(runs[2].color, None);
        assert_eq!(parse_ansi("x\u{1b}[92mok")[1].color, Some(10));
    }

    #[test]
    fn tail_window_keeps_the_end() {
        let lines: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        assert_eq!(tail_window(&lines, 3), &lines[7..]);
        assert_eq!(tail_window(&lines, 30).len(), 10);
    }
}
