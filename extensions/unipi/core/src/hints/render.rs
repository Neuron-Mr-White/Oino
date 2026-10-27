//! Hint widget, Unicrab start header and hint browser as UI requests / OUS nodes.
//!
//! unipi drew ANSI half-block art for a known terminal width. Oino frontends lay out, so the
//! art is OUS text spans (one tone per glyph) and the 72/40-column breakpoints are a
//! `responsive` node whose `fallback` is the large layout (frontends without `responsive`
//! render the fallback; see the unipi-core deviations).

use serde::Deserialize;
use serde_json::{Value, json};
use unipi_kit::ous::{dim, span, stack, text_nowrap};

use super::catalog::{Category, Hint};
use super::store::StoreData;

/// `setWidget` key of the hint line.
pub const WIDGET_KEY: &str = "unipi-hints";
/// `setHeader` key.
pub const HEADER_KEY: &str = "unipi.core/header";
/// Hint browser panel name.
pub const BROWSER_PANEL: &str = "unipi.core/hints";
/// Default lore line.
pub const DEFAULT_LORE: &str = "Unicrab walks sideways so it can read your diffs from both ends.";

/// Mascot style setting (`hints.crab`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrabStyle {
    /// Blocks (unipi `auto` resolves to blocks: images are opt-in).
    #[default]
    Auto,
    /// Unicode half-blocks.
    Blocks,
    /// Image node.
    Image,
}

impl CrabStyle {
    /// Parses the setting value (unknown → `auto`).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "blocks" => Self::Blocks,
            "image" => Self::Image,
            _ => Self::Auto,
        }
    }
}

#[derive(Deserialize)]
struct CrabData {
    png: String,
    grid22: Vec<Vec<u8>>,
    grid14: Vec<Vec<u8>>,
}

fn crab() -> CrabData {
    serde_json::from_str(include_str!("../../assets/crab.json")).unwrap_or(CrabData {
        png: String::new(),
        grid22: Vec::new(),
        grid14: Vec::new(),
    })
}

/// Palette index → OUS tone (`None` = transparent: background, outline, shell shadow).
fn tone_of(i: u8) -> Option<&'static str> {
    match i {
        3..=6 => Some("error"),
        7..=11 => Some("warning"),
        12 => Some("default"),
        _ => None,
    }
}

/// Half-block art lines (two grid rows per line) as span lists.
#[must_use]
pub fn art_lines(grid: &[Vec<u8>]) -> Vec<Vec<Value>> {
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    grid.chunks(2)
        .map(|pair| {
            let mut spans: Vec<(String, Option<&str>)> = Vec::new();
            for c in 0..width {
                let top = pair
                    .first()
                    .and_then(|r| r.get(c))
                    .copied()
                    .and_then(tone_of);
                let bottom = pair
                    .get(1)
                    .and_then(|r| r.get(c))
                    .copied()
                    .and_then(tone_of);
                let (glyph, tone) = match (top, bottom) {
                    (None, None) => (' ', None),
                    (None, Some(b)) => ('▄', Some(b)),
                    (Some(t), Some(b)) if t == b => ('█', Some(t)),
                    // Top colour wins when the halves differ (OUS spans carry one tone).
                    (Some(t), _) => ('▀', Some(t)),
                };
                match spans.last_mut() {
                    Some((s, t)) if *t == tone => s.push(glyph),
                    _ => spans.push((glyph.to_string(), tone)),
                }
            }
            spans.into_iter().map(|(s, t)| span(&s, t)).collect()
        })
        .collect()
}

/// 7-column one-row mascot (unipi `renderBlockCrab`).
#[must_use]
pub fn mini_crab() -> Vec<Value> {
    vec![
        span("▀▄", Some("error")),
        span("▀", Some("warning")),
        span("█", Some("error")),
        span("▀", Some("warning")),
        span("▄▀", Some("error")),
    ]
}

/// Plain text of the mini crab.
pub const MINI_CRAB: &str = "▀▄▀█▀▄▀";

/// Hint line suffix.
#[must_use]
pub fn suffix(hint: &Hint) -> String {
    format!("{} · alt+h ›", hint.category.as_str())
}

/// `setWidget` request showing `hint` above the editor.
#[must_use]
pub fn widget_request(hint: &Hint, style: CrabStyle) -> Value {
    let crab = if style == CrabStyle::Image {
        json!({"t": "image", "data": crab().png, "mimeType": "image/png", "alt": "Unicrab", "maxWidth": 2})
    } else {
        text_nowrap(Value::Array(mini_crab()))
    };
    let line = format!("{MINI_CRAB} {}  {}", hint.text, suffix(hint));
    json!({
        "method": "setWidget",
        "widgetKey": WIDGET_KEY,
        "widgetLines": [line],
        "widgetPlacement": "aboveEditor",
        "widgetSpec": stack("horizontal", 1, &[crab, json!({"t": "text", "value": hint.text, "wrap": false}), text_nowrap(json!([dim(&suffix(hint))]))]),
    })
}

/// `setWidget` request clearing the hint line.
#[must_use]
pub fn clear_request() -> Value {
    json!({"method": "setWidget", "widgetKey": WIDGET_KEY})
}

fn wordmark() -> Vec<Value> {
    let letters = [
        ("U", "default"),
        ("N", "info"),
        ("I", "success"),
        ("P", "warning"),
        ("I", "error"),
    ];
    let mut out = Vec::new();
    for (i, (l, tone)) in letters.iter().enumerate() {
        if i > 0 {
            out.push(span(" ", None));
        }
        out.push(json!({"text": l, "tone": tone, "bold": true}));
    }
    out
}

fn column(lines: Vec<Value>) -> Value {
    stack(
        "vertical",
        0,
        &lines.into_iter().map(text_nowrap).collect::<Vec<_>>(),
    )
}

fn art(grid: &[Vec<u8>], style: CrabStyle, png: &str) -> Value {
    if style == CrabStyle::Image && !png.is_empty() {
        return json!({"t": "image", "data": png, "mimeType": "image/png", "alt": "Unicrab"});
    }
    column(art_lines(grid).into_iter().map(Value::Array).collect())
}

/// `setHeader` request with the Unicrab start screen (unipi `renderHeaderLines`): large layout
/// from 72 columns, compact from 40, nothing below.
#[must_use]
pub fn header_request(
    unipi_version: &str,
    oino_version: &str,
    lore: &str,
    style: CrabStyle,
) -> Value {
    let data = crab();
    let mut title = wordmark();
    title.push(span("  ", None));
    title.push(dim(&format!("v{unipi_version} · Oino {oino_version}")));
    let large = stack(
        "horizontal",
        3,
        &[
            art(&data.grid22, style, &data.png),
            column(vec![
                Value::Array(title),
                json!(""),
                json!("Hi, I'm Unicrab."),
                json!(""),
                json!([dim("/settings  every module's options")]),
                json!([dim("Alt+S            shortcut overlay")]),
                json!([dim("/hint      browse every hint")]),
                json!(""),
                json!([span(lore, Some("warning"))]),
            ]),
        ],
    );
    let mut small_title = wordmark();
    small_title.push(span("  ", None));
    small_title.push(dim(&format!("v{unipi_version}")));
    let compact = stack(
        "horizontal",
        2,
        &[
            art(&data.grid14, style, &data.png),
            column(vec![
                json!(""),
                Value::Array(small_title),
                json!(""),
                json!([dim("/settings · Alt+S · /hint")]),
            ]),
        ],
    );
    json!({
        "method": "setHeader",
        "key": HEADER_KEY,
        "spec": {
            "t": "responsive",
            "key": HEADER_KEY,
            "variants": [{"minWidth": 72, "node": large}, {"minWidth": 40, "node": compact}],
            "fallback": large,
        },
    })
}

/// Seen / unseen / learned status of a hint.
#[must_use]
pub fn status(hint: &Hint, store: &StoreData) -> &'static str {
    if store.learned.contains(&hint.id) {
        "learned"
    } else if store.counts.get(&hint.id).is_some_and(|r| r.count > 0) {
        "seen"
    } else {
        "unseen"
    }
}

/// Hints matching a case-insensitive filter over text, id and category.
#[must_use]
pub fn filter<'a>(hints: &'a [Hint], query: &str) -> Vec<&'a Hint> {
    let q = query.trim().to_lowercase();
    hints
        .iter()
        .filter(|h| {
            q.is_empty()
                || h.text.to_lowercase().contains(&q)
                || h.id.to_lowercase().contains(&q)
                || h.category.as_str().contains(&q)
        })
        .collect()
}

/// Hint browser panel (`openPanel` spec): filter input, one section per category, Enter shows.
#[must_use]
pub fn browser_spec(hints: &[Hint], store: &StoreData, query: &str) -> Value {
    let matching = filter(hints, query);
    let mut children = vec![
        json!({"t": "input", "id": "filter", "value": query, "placeholder": "type to filter", "onSubmit": "filter"}),
    ];
    for c in Category::ALL {
        let items: Vec<Value> = matching
            .iter()
            .filter(|h| h.category == c)
            .map(|h| {
                let st = status(h, store);
                let tone = match st {
                    "learned" => "success",
                    "seen" => "muted",
                    _ => "default",
                };
                json!({"id": h.id, "label": h.text, "badge": st, "tone": tone})
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        children.push(json!({
            "t": "section",
            "title": format!("{} ({})", c.as_str(), items.len()),
            "body": {"t": "list", "items": items, "selectable": "show"},
        }));
    }
    if matching.is_empty() {
        children.push(json!({"t": "empty", "message": "No hints match."}));
    }
    children.push(unipi_kit::ous::key_hints(&[
        ("enter", "show above the editor"),
        ("esc", "close"),
    ]));
    stack("vertical", 1, &children)
}

/// `openPanel` request for the browser.
#[must_use]
pub fn browser_panel(hints: &[Hint], store: &StoreData, query: &str) -> Value {
    json!({
        "panelId": BROWSER_PANEL,
        "placement": "overlay",
        "title": "Unicrab hints",
        "spec": browser_spec(hints, store, query),
        "focus": true,
        "closeOnEscape": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hints::catalog::{StartupTag, When, bundled};

    fn hint() -> Hint {
        Hint {
            id: "cmd.hint".into(),
            category: Category::Command,
            text: "Try /hint".into(),
            when: When::Startup(StartupTag::Startup),
            max_shows: None,
            teaches: None,
            since: None,
        }
    }

    fn plain(spans: &[Value]) -> String {
        spans.iter().filter_map(|s| s["text"].as_str()).collect()
    }

    #[test]
    fn mini_crab_is_seven_columns() {
        assert_eq!(plain(&mini_crab()).chars().count(), 7);
        assert_eq!(plain(&mini_crab()), MINI_CRAB);
    }

    #[test]
    fn widget_line_and_spec() {
        let w = widget_request(&hint(), CrabStyle::Blocks);
        assert_eq!(w["method"], "setWidget");
        assert_eq!(w["widgetKey"], WIDGET_KEY);
        assert_eq!(w["widgetPlacement"], "aboveEditor");
        assert_eq!(w["widgetLines"][0], "▀▄▀█▀▄▀ Try /hint  command · alt+h ›");
        assert_eq!(
            w["widgetSpec"]["children"].as_array().map(Vec::len),
            Some(3)
        );
        let img = widget_request(&hint(), CrabStyle::Image);
        assert_eq!(img["widgetSpec"]["children"][0]["t"], "image");
        assert_eq!(
            clear_request(),
            json!({"method": "setWidget", "widgetKey": WIDGET_KEY})
        );
        assert_eq!(CrabStyle::parse("image"), CrabStyle::Image);
        assert_eq!(CrabStyle::parse("weird"), CrabStyle::Auto);
    }

    #[test]
    fn art_sizes_match_unipi_layouts() {
        let d = crab();
        let large = art_lines(&d.grid22);
        assert_eq!(large.len(), 10, "22-col crab is 10 rows");
        assert!(large.iter().all(|l| plain(l).chars().count() == 22));
        let compact = art_lines(&d.grid14);
        assert_eq!(compact.len(), 6, "14-col crab is 6 rows");
        assert!(compact.iter().all(|l| plain(l).chars().count() == 14));
        assert!(large.iter().flatten().any(|s| s["tone"] == "error"));
        assert_ne!(d.png, "");
    }

    #[test]
    fn header_has_breakpoints() {
        let h = header_request("3.0.0", "0.1.0", DEFAULT_LORE, CrabStyle::Auto);
        assert_eq!(h["method"], "setHeader");
        assert_eq!(h["key"], HEADER_KEY);
        let spec = &h["spec"];
        assert_eq!(spec["t"], "responsive");
        assert_eq!(spec["variants"][0]["minWidth"], 72);
        assert_eq!(spec["variants"][1]["minWidth"], 40);
        assert_eq!(spec["fallback"], spec["variants"][0]["node"]);
        let text = spec.to_string();
        assert!(text.contains("v3.0.0 · Oino 0.1.0"));
        assert!(text.contains("Hi, I'm Unicrab."));
        assert!(text.contains(DEFAULT_LORE));
        let img = header_request("3.0.0", "0.1.0", DEFAULT_LORE, CrabStyle::Image);
        assert_eq!(img["spec"]["fallback"]["children"][0]["t"], "image");
    }

    #[test]
    fn browser_groups_and_filters() {
        let hints = bundled().unwrap_or_default();
        let mut store = StoreData::default();
        store.record_show("cmd.settings", 1);
        store.record_learned("cmd.hint");
        let spec = browser_spec(&hints, &store, "");
        let sections: Vec<&Value> = spec["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["t"] == "section")
            .collect();
        assert_eq!(sections.len(), 9);
        assert!(
            sections[0]["title"]
                .as_str()
                .is_some_and(|t| t.starts_with("command ("))
        );
        let items = &sections[0]["body"]["items"];
        assert_eq!(items[0]["badge"], "seen");
        assert_eq!(items[1]["badge"], "learned");
        assert_eq!(items[2]["badge"], "unseen");
        assert_eq!(
            filter(&hints, "KANBOARD").len(),
            filter(&hints, "kanboard").len()
        );
        assert_ne!(filter(&hints, "kanboard").len(), 0);
        let none = browser_spec(&hints, &store, "zzzz-no-match");
        assert!(none.to_string().contains("No hints match."));
        assert_eq!(browser_panel(&hints, &store, "")["placement"], "overlay");
    }
}
