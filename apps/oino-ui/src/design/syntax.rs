//! Syntax highlight theme (design.md §2.3, round-5 #17): the unipi token table mapped
//! onto the ANSI palette — keyword `ansi[1]` `#F03C32`, string `ansi[2]`, number
//! `ansi[4]` `#28BEF0`, bool/const `ansi[3]`, function `ansi[6]` `#50BEBE`, type
//! `ansi[14]` `#96DCD2`, property `ansi[15]`, comment `ansi[8]` italic, punctuation
//! `ansi[7]`, attribute `ansi[13]`, operator `fg_strong`; markdown emphasis/strong,
//! list markers and links map onto `emphasis`/`emphasis.strong`/
//! `punctuation.list_marker`/`link_uri`/`title`.

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
                "keyword": style(p.ansi[1]),
                "string": style(p.ansi[2]),
                "string.escape": style(p.ansi[6]),
                "number": style(p.ansi[4]),
                "boolean": style(p.ansi[3]),
                "constant": style(p.ansi[3]),
                "function": style(p.ansi[6]),
                "type": style(p.ansi[14]),
                "constructor": style(p.ansi[14]),
                "comment": { "color": hex(p.ansi[8]), "font_style": "italic" },
                "comment.doc": { "color": hex(p.ansi[8]), "font_style": "italic" },
                "property": style(p.ansi[15]),
                "variable": style(p.fg),
                "operator": style(p.fg_strong),
                "punctuation": style(p.ansi[7]),
                "punctuation.list_marker": style(p.list_marker),
                "attribute": style(p.ansi[13]),
                "tag": style(p.ansi[4]),
                "title": style(p.fg_strong),
                "link_uri": style(p.link),
                "emphasis": { "font_style": "italic" },
                "emphasis.strong": { "color": hex(p.fg_strong), "font_weight": 700 },
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
