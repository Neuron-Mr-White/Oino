//! Syntax highlight theme (design.md §2.3): Zai-like colors from the palette — keywords
//! `ansi[5]`, strings `ansi[2]`, numbers `ansi[3]`, functions `ansi[4]`, types `ansi[6]`,
//! comments `fg_subtlest`.

use std::sync::Arc;

use gpui_kit::component::highlighter::HighlightTheme;
use serde_json::json;

use crate::design::theme::OinoTheme;

fn hex(c: u32) -> String {
    format!("#{c:08x}")
}

/// The highlight theme for `t`, or `None` when gpui-kit rejects it (keeps its default).
#[must_use]
pub fn highlight_theme(t: &OinoTheme) -> Option<Arc<HighlightTheme>> {
    let p = t.p;
    let style = |c: u32| json!({ "color": hex(c) });
    let value = json!({
        "name": if t.dark { "Oino Dark" } else { "Oino Light" },
        "appearance": if t.dark { "dark" } else { "light" },
        "style": {
            "editor.background": hex(p.frame),
            "editor.foreground": hex(p.fg),
            "editor.line_number": hex(p.fg_subtlest),
            "editor.active_line_number": hex(p.fg_subtle),
            "syntax": {
                "keyword": style(p.ansi[5]),
                "string": style(p.ansi[2]),
                "string.escape": style(p.ansi[6]),
                "number": style(p.ansi[3]),
                "boolean": style(p.ansi[3]),
                "constant": style(p.ansi[3]),
                "function": style(p.ansi[4]),
                "type": style(p.ansi[6]),
                "constructor": style(p.ansi[6]),
                "comment": style(p.fg_subtlest),
                "comment.doc": style(p.fg_subtlest),
                "property": style(p.fg),
                "variable": style(p.fg),
                "operator": style(p.fg_subtle),
                "punctuation": style(p.fg_subtle),
                "attribute": style(p.ansi[5]),
                "tag": style(p.ansi[4]),
                "title": style(p.fg),
                "link_uri": style(p.ansi[12]),
                "text.literal": style(p.ansi[2]),
            }
        }
    });
    match serde_json::from_value::<HighlightTheme>(value) {
        Ok(theme) => Some(Arc::new(theme)),
        Err(error) => {
            tracing::warn!(%error, "highlight theme");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::tokens::LIGHT;

    #[test]
    fn highlight_theme_parses_for_both_modes() {
        let dark = OinoTheme::default();
        assert!(highlight_theme(&dark).is_some());
        let light = OinoTheme {
            dark: false,
            p: &LIGHT,
            ..OinoTheme::default()
        };
        assert!(highlight_theme(&light).is_some());
    }
}
