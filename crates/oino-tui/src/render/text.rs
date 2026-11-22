//! Width-aware line building: styled word wrapping, padding and background fill.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Owned line type used by every renderer.
pub(crate) type OLine = Line<'static>;

/// Display width of a line.
pub(crate) fn line_width(line: &OLine) -> usize {
    line.spans.iter().map(|s| s.content.width()).sum()
}

/// A styled run before wrapping.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub(crate) text: String,
    pub(crate) style: Style,
}

impl Run {
    pub(crate) fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Word-wraps styled runs to `width` cells. `\n` inside runs forces breaks. Continuation
/// lines start with `indent` (a styled prefix such as a list indent).
pub(crate) fn wrap(runs: &[Run], width: usize, first_prefix: &[Run], indent: &[Run]) -> Vec<OLine> {
    let width = width.max(1);
    let prefix_width = |p: &[Run]| p.iter().map(|r| r.text.width()).sum::<usize>();
    let mut lines: Vec<OLine> = Vec::new();
    let mut current: Vec<Span<'static>> = first_prefix
        .iter()
        .map(|r| Span::styled(r.text.clone(), r.style))
        .collect();
    let mut used = prefix_width(first_prefix);
    let start_line = |current: &mut Vec<Span<'static>>, used: &mut usize| {
        current.extend(indent.iter().map(|r| Span::styled(r.text.clone(), r.style)));
        *used = prefix_width(indent);
    };
    let mut line_has_content = false;
    for run in runs {
        // Tokenise into words and whitespace, keeping newlines.
        let mut token = String::new();
        let mut tokens: Vec<String> = Vec::new();
        for c in run.text.chars() {
            if c == '\n' {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
                tokens.push("\n".to_owned());
            } else if c == ' ' {
                if !token.is_empty() && !token.ends_with(' ') {
                    tokens.push(std::mem::take(&mut token));
                }
                token.push(c);
            } else {
                if token.ends_with(' ') {
                    tokens.push(std::mem::take(&mut token));
                }
                token.push(c);
            }
        }
        if !token.is_empty() {
            tokens.push(token);
        }
        for token in tokens {
            if token == "\n" {
                lines.push(finish(std::mem::take(&mut current)));
                start_line(&mut current, &mut used);
                line_has_content = false;
                continue;
            }
            let w = token.width();
            let is_space = token.starts_with(' ');
            if used + w <= width {
                current.push(Span::styled(token, run.style));
                used += w;
                line_has_content = true;
                continue;
            }
            if is_space {
                // Drop spaces at the wrap point.
                lines.push(finish(std::mem::take(&mut current)));
                start_line(&mut current, &mut used);
                line_has_content = false;
                continue;
            }
            if line_has_content && w <= width.saturating_sub(prefix_width(indent)) {
                lines.push(finish(std::mem::take(&mut current)));
                start_line(&mut current, &mut used);
                current.push(Span::styled(token, run.style));
                used += w;
                line_has_content = true;
                continue;
            }
            // Hard-break long words.
            let mut piece = String::new();
            for c in token.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > width && used > prefix_width(indent) {
                    if !piece.is_empty() {
                        current.push(Span::styled(std::mem::take(&mut piece), run.style));
                    }
                    lines.push(finish(std::mem::take(&mut current)));
                    start_line(&mut current, &mut used);
                }
                piece.push(c);
                used += cw;
            }
            if !piece.is_empty() {
                current.push(Span::styled(piece, run.style));
                line_has_content = true;
            }
        }
    }
    lines.push(Line::from(current));
    lines
}

/// Ends a wrapped line: trailing whitespace-only spans are dropped.
fn finish(mut spans: Vec<Span<'static>>) -> OLine {
    while spans.len() > 1 && spans.last().is_some_and(|s| s.content.trim().is_empty()) {
        spans.pop();
    }
    Line::from(spans)
}

/// Wraps plain text in one style.
pub(crate) fn wrap_plain(text: &str, style: Style, width: usize) -> Vec<OLine> {
    wrap(&[Run::new(text, style)], width, &[], &[])
}

/// Truncates a line to `width` cells (appends `…` when cut).
pub(crate) fn truncate_line(line: OLine, width: usize) -> OLine {
    if line_width(&line) <= width {
        return line;
    }
    let mut out = Vec::new();
    let mut used = 0;
    let budget = width.saturating_sub(1);
    'outer: for span in line.spans {
        let mut text = String::new();
        for c in span.content.chars() {
            let cw = c.width().unwrap_or(0);
            if used + cw > budget {
                if !text.is_empty() {
                    out.push(Span::styled(text, span.style));
                }
                break 'outer;
            }
            used += cw;
            text.push(c);
        }
        out.push(Span::styled(text, span.style));
    }
    if width > 0 {
        out.push(Span::raw("…"));
    }
    Line::from(out).style(line.style)
}

/// Adds left/right padding and fills the line to `width` with `bg`.
pub(crate) fn boxed(lines: Vec<OLine>, width: usize, pad: usize, bg: Option<Color>) -> Vec<OLine> {
    lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![Span::raw(" ".repeat(pad))];
            let w = line_width(&line);
            spans.extend(line.spans);
            let fill = width.saturating_sub(w + pad);
            spans.push(Span::raw(" ".repeat(fill)));
            let mut out = Line::from(spans);
            if let Some(bg) = bg {
                out = out.style(Style::default().bg(bg));
                for span in &mut out.spans {
                    if span.style.bg.is_none() {
                        span.style = span.style.bg(bg);
                    }
                }
            }
            out
        })
        .collect()
}

/// An empty line filled with `bg` (vertical padding of a box).
pub(crate) fn blank(width: usize, bg: Option<Color>) -> OLine {
    let span = Span::raw(" ".repeat(width));
    match bg {
        Some(bg) => Line::from(span.style(Style::default().bg(bg))),
        None => Line::from(span),
    }
}

/// Left text and right text on one line, right-aligned within `width`.
pub(crate) fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> OLine {
    let lw: usize = left.iter().map(|s| s.content.width()).sum();
    let rw: usize = right.iter().map(|s| s.content.width()).sum();
    let mut spans = left;
    if lw + rw < width {
        spans.push(Span::raw(" ".repeat(width - lw - rw)));
        spans.extend(right);
    }
    truncate_line(Line::from(spans), width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[OLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wraps_words_and_breaks_long_ones() {
        let lines = wrap_plain("hello brave new world", Style::default(), 11);
        assert_eq!(texts(&lines), vec!["hello brave", "new world"]);
        let lines = wrap_plain("abcdefghij", Style::default(), 4);
        assert_eq!(texts(&lines), vec!["abcd", "efgh", "ij"]);
        let lines = wrap(
            &[Run::new("one two three", Style::default())],
            8,
            &[Run::new("- ", Style::default())],
            &[Run::new("  ", Style::default())],
        );
        assert_eq!(texts(&lines), vec!["- one", "  two", "  three"]);
        let lines = wrap_plain("a\nb", Style::default(), 10);
        assert_eq!(texts(&lines), vec!["a", "b"]);
    }

    #[test]
    fn truncates_and_spreads() {
        let line = truncate_line(Line::from("hello world"), 6);
        assert_eq!(texts(&[line]), vec!["hello…"]);
        let line = spread(vec![Span::raw("ab")], vec![Span::raw("cd")], 8);
        assert_eq!(texts(&[line]), vec!["ab    cd"]);
    }
}
