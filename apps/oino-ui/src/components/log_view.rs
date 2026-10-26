//! `LogView` (design.md §4.22): monospace lines with ANSI SGR colors from the `ansi` tokens.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::design::theme::{OinoTheme, TextExt as _};
use crate::design::{radius, space};

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

/// Renders log lines (callers cap the count; the side pane scrolls).
#[must_use]
pub fn log_lines(t: &OinoTheme, lines: &[String], key: &str) -> AnyElement {
    div()
        .id(ElementId::Name(format!("{key}/log").into()))
        .flex()
        .flex_col()
        .w_full()
        .p(px(space::S2))
        .rounded(px(radius::R_LG))
        .bg(t.surface())
        .code_text(t)
        .text_color(t.fg())
        .overflow_x_scroll()
        .children(lines.iter().map(|l| line(t, l)))
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
}
