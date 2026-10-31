//! Theme tokens → ratatui styles, and the §4.2 text model → ratatui text.
//!
//! Colours are role tokens only (§5): a token resolves through the active theme
//! ([`Theme`]); unknown tokens are dropped (they render with the inherited colour).

use oino_ui::style::{Color as Token, Modifier as TokenModifier, Style as TokenStyle};
use oino_ui::text::{Alignment, Line as TokenLine, Span as TokenSpan, Text as TokenText};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};

use crate::theme::Theme;

/// Categorical palette for `c1`..`c12` (§5, same values as [`Theme`]'s OUS tones).
const CATEGORY: [Color; 12] = [
    Color::Rgb(0x69, 0xad, 0xd0),
    Color::Rgb(0x68, 0xb7, 0x8d),
    Color::Rgb(0xcd, 0x9a, 0x22),
    Color::Rgb(0xa7, 0x98, 0xd7),
    Color::Rgb(0xea, 0x7f, 0x81),
    Color::Rgb(0x5d, 0xb3, 0xba),
    Color::Rgb(0xde, 0x8d, 0x5a),
    Color::Rgb(0x97, 0x76, 0xe5),
    Color::Rgb(0xde, 0x54, 0xc1),
    Color::Rgb(0x61, 0x85, 0xcc),
    Color::Rgb(0x5e, 0xb2, 0x86),
    Color::Rgb(0xfe, 0x54, 0x62),
];

/// Resolves a theme token (§5); `None` for an unknown token (dropped, §4.8).
#[must_use]
pub(crate) fn token(token: &Token, theme: &Theme) -> Option<Color> {
    Some(match token {
        Token::Reset => Color::Reset,
        Token::Text | Token::Assistant => theme.text,
        Token::Muted => theme.muted,
        Token::Dim => theme.dim,
        // Both map to `accent` in the TUI (§5: `primary` is shell red in the app only).
        Token::Accent | Token::Primary => theme.accent,
        Token::Info | Token::Link => theme.md_link,
        Token::Success => theme.success,
        Token::Warning => theme.warning,
        Token::Error => theme.error,
        Token::Harness | Token::BorderAccent => theme.border_accent,
        Token::Thinking => theme.thinking_text,
        Token::Tool => theme.tool_title,
        Token::User => theme.user_message_text,
        Token::Code => theme.md_code,
        Token::Border => theme.border,
        Token::BorderMuted => theme.border_muted,
        Token::C1 => CATEGORY[0],
        Token::C2 => CATEGORY[1],
        Token::C3 => CATEGORY[2],
        Token::C4 => CATEGORY[3],
        Token::C5 => CATEGORY[4],
        Token::C6 => CATEGORY[5],
        Token::C7 => CATEGORY[6],
        Token::C8 => CATEGORY[7],
        Token::C9 => CATEGORY[8],
        Token::C10 => CATEGORY[9],
        Token::C11 => CATEGORY[10],
        Token::C12 => CATEGORY[11],
        Token::Surface1 | Token::InfoBg => theme.custom_message_bg,
        Token::Surface2 => theme.user_message_bg,
        Token::Surface3 | Token::SelectedBg | Token::AccentBg => theme.selected_bg,
        Token::SuccessBg => theme.tool_success_bg,
        Token::ErrorBg => theme.tool_error_bg,
        Token::WarningBg => theme.tool_pending_bg,
        Token::DiffAdded => theme.diff_added,
        Token::DiffRemoved => theme.diff_removed,
        Token::DiffContext => theme.diff_context,
        Token::Other(_) => return None,
    })
}

/// ratatui modifier of a token modifier.
#[must_use]
pub(crate) fn modifier(m: TokenModifier) -> Modifier {
    match m {
        TokenModifier::BOLD => Modifier::BOLD,
        TokenModifier::DIM => Modifier::DIM,
        TokenModifier::ITALIC => Modifier::ITALIC,
        TokenModifier::UNDERLINED => Modifier::UNDERLINED,
        TokenModifier::SLOW_BLINK => Modifier::SLOW_BLINK,
        TokenModifier::RAPID_BLINK => Modifier::RAPID_BLINK,
        TokenModifier::REVERSED => Modifier::REVERSED,
        TokenModifier::HIDDEN => Modifier::HIDDEN,
        TokenModifier::CROSSED_OUT => Modifier::CROSSED_OUT,
    }
}

/// Patches `base` with a token style (ratatui `Style::patch` semantics, §4.2).
#[must_use]
pub(crate) fn style(source: &TokenStyle, base: Style, theme: &Theme) -> Style {
    let mut out = base;
    if let Some(fg) = source.fg.as_ref().and_then(|t| token(t, theme)) {
        out = out.fg(fg);
    }
    if let Some(bg) = source.bg.as_ref().and_then(|t| token(t, theme)) {
        out = out.bg(bg);
    }
    if let Some(color) = source
        .underline_color
        .as_ref()
        .and_then(|t| token(t, theme))
    {
        out = out.underline_color(color);
    }
    for m in source.add_modifier.iter() {
        out = out.add_modifier(modifier(m));
    }
    for m in source.sub_modifier.iter() {
        out = out.remove_modifier(modifier(m));
    }
    out
}

/// Applies an optional token style.
#[must_use]
pub(crate) fn patch(style: Option<&TokenStyle>, base: Style, theme: &Theme) -> Style {
    style.map_or(base, |s| self::style(s, base, theme))
}

/// Alignment conversion.
#[must_use]
pub(crate) fn alignment(alignment: Option<Alignment>) -> ratatui::layout::Alignment {
    match alignment {
        Some(Alignment::Center) => ratatui::layout::Alignment::Center,
        Some(Alignment::Right) => ratatui::layout::Alignment::Right,
        Some(Alignment::Left) | None => ratatui::layout::Alignment::Left,
    }
}

/// One span (§4.2). A `link` gets an underline (§4.2: OSC 8 + underline).
#[must_use]
pub(crate) fn span(span: &TokenSpan, base: Style, theme: &Theme) -> Span<'static> {
    let mut style = patch(span.style(), base, theme);
    if span.link().is_some() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    Span::styled(span.content().to_owned(), style)
}

/// One line (§4.2); the line style patches `base`, spans patch the line.
#[must_use]
pub(crate) fn line(line: &TokenLine, base: Style, theme: &Theme) -> Line<'static> {
    let line_style = patch(line.style(), base, theme);
    let spans: Vec<Span<'static>> = line
        .spans()
        .iter()
        .map(|s| span(s, line_style, theme))
        .collect();
    Line::from(spans)
        .style(line_style)
        .alignment(alignment(line.alignment()))
}

/// A whole text block (§4.2); a shorthand string splits on `\n`.
#[must_use]
pub(crate) fn text(text: &TokenText, base: Style, theme: &Theme) -> Text<'static> {
    let style = patch(text.style(), base, theme);
    let lines: Vec<Line<'static>> = text
        .lines()
        .iter()
        .map(|l| {
            let line_style = patch(l.style(), style, theme);
            let spans: Vec<Span<'static>> = l
                .spans()
                .iter()
                .map(|s| span(s, line_style, theme))
                .collect();
            Line::from(spans)
                .style(line_style)
                .alignment(alignment(l.alignment().or(text.alignment())))
        })
        .collect();
    Text::from(lines).style(style)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )]
    use super::*;
    use crate::theme::DARK;
    use oino_ui::text::Span as OSpan;

    #[test]
    fn tokens_resolve_and_unknowns_drop() {
        assert_eq!(token(&Token::Accent, &DARK), Some(DARK.accent));
        assert_eq!(token(&Token::Primary, &DARK), Some(DARK.accent));
        assert_eq!(token(&Token::C1, &DARK), Some(CATEGORY[0]));
        assert_eq!(token(&Token::Other("neon".into()), &DARK), None);
        let patched = style(
            &TokenStyle::new().fg(Token::Other("neon".into())),
            Style::default(),
            &DARK,
        );
        assert_eq!(patched.fg, Style::default().fg);
    }

    #[test]
    fn text_patches_styles_and_links() {
        let source = TokenText::Str("a\nb".into());
        let rendered = text(&source, Style::default().fg(DARK.text), &DARK);
        assert_eq!(rendered.lines.len(), 2);
        let linked = TokenSpan::linked("docs", "https://example.com");
        let rendered_span = span(&linked, Style::default(), &DARK);
        assert!(
            rendered_span
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );

        let source = TokenLine::from_spans(vec![OSpan::Str("x".into())]);
        let rendered = line(&source, Style::default(), &DARK);
        assert_eq!(rendered.spans[0].content, "x");
    }
}

// ── blocks (§4.5.1) ─────────────────────────────────────────────────────────

use oino_ui::widgets::block::{
    Block as TokenBlock, BlockProps, BlockTitle, BorderSide, BorderType, Borders, Padding,
    TitlePosition,
};
use ratatui::widgets::{
    Block, BorderType as RBorderType, Borders as RBorders, Padding as RPadding,
};

/// Borrowed view of the block props of a `Block` node or the common `block` prop.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BlockSpec<'a> {
    pub(crate) title: Option<&'a BlockTitle>,
    pub(crate) title_alignment: Option<Alignment>,
    pub(crate) borders: Option<&'a Borders>,
    pub(crate) border_type: Option<BorderType>,
    pub(crate) border_style: Option<&'a TokenStyle>,
    pub(crate) title_style: Option<&'a TokenStyle>,
    pub(crate) padding: Option<&'a Padding>,
    pub(crate) rail: Option<&'a Token>,
}

impl<'a> From<&'a BlockProps> for BlockSpec<'a> {
    fn from(props: &'a BlockProps) -> Self {
        Self {
            title: props.title.as_ref(),
            title_alignment: props.title_alignment,
            borders: props.borders.as_ref(),
            border_type: props.border_type,
            border_style: props.border_style.as_ref(),
            title_style: props.title_style.as_ref(),
            padding: props.padding.as_ref(),
            rail: props.rail.as_ref(),
        }
    }
}

impl<'a> From<&'a TokenBlock> for BlockSpec<'a> {
    fn from(block: &'a TokenBlock) -> Self {
        Self {
            title: block.title.as_ref(),
            title_alignment: block.title_alignment,
            borders: block.borders.as_ref(),
            border_type: block.border_type,
            border_style: block.border_style.as_ref(),
            title_style: block.title_style.as_ref(),
            padding: block.padding.as_ref(),
            rail: block.rail.as_ref(),
        }
    }
}

/// Border sides → ratatui `Borders`; `rail` adds a left border (§4.5.1).
fn borders(spec: &BlockSpec<'_>) -> RBorders {
    let mut out = match spec.borders {
        None | Some(Borders::None_ | Borders::Other(_)) => RBorders::NONE,
        Some(Borders::All) => RBorders::ALL,
        Some(Borders::Sides(sides)) => {
            let mut b = RBorders::NONE;
            for side in sides {
                b |= match side {
                    BorderSide::TOP => RBorders::TOP,
                    BorderSide::RIGHT => RBorders::RIGHT,
                    BorderSide::BOTTOM => RBorders::BOTTOM,
                    BorderSide::LEFT => RBorders::LEFT,
                };
            }
            b
        }
    };
    if spec.rail.is_some() {
        out |= RBorders::LEFT;
    }
    out
}

/// Border type conversion.
fn border_type(border_type: Option<BorderType>) -> RBorderType {
    match border_type.unwrap_or_default() {
        BorderType::Plain => RBorderType::Plain,
        BorderType::Rounded => RBorderType::Rounded,
        BorderType::Double => RBorderType::Double,
        BorderType::Thick => RBorderType::Thick,
    }
}

/// Padding conversion.
fn padding(padding: Option<&Padding>) -> RPadding {
    match padding {
        None => RPadding::default(),
        Some(Padding::Uniform(n)) => RPadding::uniform(*n),
        Some(Padding::Rect {
            left,
            right,
            top,
            bottom,
        }) => RPadding::new(
            left.unwrap_or(0),
            right.unwrap_or(0),
            top.unwrap_or(0),
            bottom.unwrap_or(0),
        ),
    }
}

/// Builds the ratatui block of `spec`; a `Some(collapsed)` marker prefixes `▸`/`▾` on
/// the title (§4.5.1 collapsible).
#[must_use]
pub(crate) fn block(
    spec: &BlockSpec<'_>,
    marker: Option<bool>,
    base: Style,
    theme: &Theme,
) -> Block<'static> {
    let mut out = Block::default()
        .borders(borders(spec))
        .border_type(border_type(spec.border_type));
    // §5: `border` is the default frame colour; an explicit `borderStyle` patches it.
    let border_style = match spec.border_style {
        Some(style) => patch(Some(style), base, theme),
        None => base.fg(theme.border),
    };
    let border_style = match spec.rail.and_then(|t| token(t, theme)) {
        // ratatui 0.29 has no per-side border style: the rail tints the border set.
        Some(rail) => border_style.fg(rail),
        None => border_style,
    };
    out = out.border_style(border_style);
    out = out.padding(padding(spec.padding));
    let title_style = patch(spec.title_style, base, theme);
    if let Some(title) = spec.title {
        match title {
            BlockTitle::Line(title_line) => {
                let mut line = line(title_line, title_style, theme);
                if let Some(collapsed) = marker {
                    let glyph = if collapsed { "▸ " } else { "▾ " };
                    let mut spans = vec![Span::styled(glyph, title_style)];
                    spans.extend(line.spans);
                    line = Line::from(spans);
                }
                out = out.title(line);
                out = out.title_alignment(alignment(spec.title_alignment));
            }
            BlockTitle::Titles(titles) => {
                for title in titles {
                    let mut line = line(&title.content, title_style, theme);
                    if let Some(collapsed) = marker
                        && title.position != Some(TitlePosition::Bottom)
                    {
                        let glyph = if collapsed { "▸ " } else { "▾ " };
                        let mut spans = vec![Span::styled(glyph, title_style)];
                        spans.extend(line.spans);
                        line = Line::from(spans);
                    }
                    let alignment = alignment(title.alignment.or(spec.title_alignment));
                    out = match title.position {
                        Some(TitlePosition::Bottom) => out.title_bottom(line),
                        _ => out.title_top(line),
                    };
                    out = out.title_alignment(alignment);
                }
            }
        }
    }
    out
}
