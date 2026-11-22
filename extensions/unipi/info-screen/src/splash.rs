//! The Unicrab startup splash (unipi `tui/splash.ts`) as a v2 header node.
//!
//! Big splash (22-column crab beside the facts) from 76 columns; below that the `Layout`
//! collapses to a column (ui-protocol §4.3). The shimmer and the draining countdown bar were
//! v1 `effect` hints with no v2 equivalent — the bar renders full (`§12.1`: effects dropped).

use oino_extension::ui::widgets::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kit::{bold, dim, money, span};

/// Header key.
pub const HEADER_KEY: &str = "splash";

#[derive(Deserialize)]
struct Crab {
    grid22: Vec<Vec<u8>>,
}

fn crab() -> Crab {
    serde_json::from_str(include_str!("../assets/crab.json")).unwrap_or(Crab { grid22: Vec::new() })
}

fn tone_of(i: u8) -> Option<&'static str> {
    match i {
        3..=6 => Some("error"),
        7..=11 => Some("warning"),
        12 => Some("default"),
        _ => None,
    }
}

/// Half-block art (two grid rows per line) as unwrapped `Paragraph` nodes.
#[must_use]
pub fn art(grid: &[Vec<u8>]) -> Vec<Node> {
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    grid.chunks(2)
        .map(|pair| {
            let mut spans: Vec<(String, Option<&str>)> = Vec::new();
            for c in 0..width {
                let cell = |r: usize| {
                    pair.get(r)
                        .and_then(|row| row.get(c))
                        .copied()
                        .and_then(tone_of)
                };
                let (g, t) = match (cell(0), cell(1)) {
                    (None, None) => (' ', None),
                    (None, Some(b)) => ('▄', Some(b)),
                    (Some(t), Some(b)) if t == b => ('█', Some(t)),
                    (Some(t), _) => ('▀', Some(t)),
                };
                match spans.last_mut() {
                    Some((s, tone)) if *tone == t => s.push(g),
                    _ => spans.push((g.to_string(), t)),
                }
            }
            Paragraph::new(Line::from(
                spans
                    .into_iter()
                    .map(|(s, t)| span(s, t))
                    .collect::<Vec<_>>(),
            ))
            .wrap(Wrap::Disabled(false))
            .into()
        })
        .collect()
}

/// unipi block wordmark rows.
#[must_use]
pub fn wordmark_rows() -> [String; 3] {
    let glyph = |c: char| match c {
        'U' => ["█  █", "█  █", "▀▀▀▀"],
        'N' => ["██▄█", "█ ▀█", "▀  ▀"],
        'I' => ["█", "█", "▀"],
        'P' => ["█▀▀█", "█▀▀▀", "▀   "],
        _ => [" ", " ", " "],
    };
    let mut rows = [String::new(), String::new(), String::new()];
    for (i, c) in "UNIPI".chars().enumerate() {
        for (r, g) in rows.iter_mut().zip(glyph(c)) {
            if i > 0 {
                r.push(' ');
            }
            r.push_str(g);
        }
    }
    rows
}

/// `α24` / `β3` / `3.0`.
#[must_use]
pub fn short_version(v: &str) -> String {
    for (tag, sym) in [("alpha.", "α"), ("beta.", "β")] {
        if let Some(i) = v.find(tag) {
            let n: String = v[i + tag.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            return format!("{sym}{n}");
        }
    }
    v.split('.').take(2).collect::<Vec<_>>().join(".")
}

/// Greeting by hour (UTC until the host exposes local time).
#[must_use]
pub fn greeting(hour: i64) -> &'static str {
    match hour {
        0..=4 => "Up late",
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// Facts on the right (unipi `SplashFacts`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// Ready time (ms).
    pub ready_ms: Option<i64>,
    /// Loaded modules.
    pub modules: Option<usize>,
    /// Tools.
    pub tools: Option<usize>,
    /// Today's spend (same-day snapshot only).
    pub today_cost: Option<f64>,
    /// Today's sessions.
    pub today_sessions: Option<u64>,
    /// Resumed session `(replies, tokens, cost)`.
    pub resumed: Option<(u64, String, String)>,
    /// Cwd (home shortened).
    pub cwd: Option<String>,
    /// Git branch.
    pub branch: Option<String>,
    /// Available update.
    pub update: Option<String>,
}

fn txt(spans: Vec<Span>) -> Node {
    Paragraph::new(Line::from(spans))
        .wrap(Wrap::Disabled(false))
        .into()
}

/// The splash spec.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
pub fn spec(
    unipi_version: &str,
    oino_version: &str,
    facts: &Facts,
    hour: i64,
    interactive: bool,
    auto_close_ms: Option<u64>,
) -> Node {
    let word = wordmark_rows();
    let chip = Span::from_string(short_version(unipi_version))
        .bold()
        .fg(Color::Warning);
    let mut right = vec![
        txt(vec![
            bold(word[0].clone(), Some("warning")),
            span("   ", None),
            chip,
        ]),
        txt(vec![
            bold(word[1].clone(), Some("warning")),
            span("   ", None),
            dim(format!("v{unipi_version}")),
        ]),
        txt(vec![
            bold(word[2].clone(), Some("warning")),
            span("   ", None),
            dim(format!("Oino {oino_version}")),
        ]),
        txt(vec![
            bold(greeting(hour), None),
            span(" — I'm ", Some("muted")),
            bold("Unicrab", Some("warning")),
            span(".", Some("muted")),
        ]),
    ];
    let mut ready = Vec::new();
    if let Some(ms) = facts.ready_ms.filter(|m| *m > 0) {
        #[allow(clippy::cast_precision_loss, reason = "display")]
        ready.push(if ms >= 1000 {
            format!("ready in {:.1}s", ms as f64 / 1000.0)
        } else {
            format!("ready in {ms}ms")
        });
    }
    if let Some(m) = facts.modules.filter(|m| *m > 0) {
        ready.push(format!("{m} modules"));
    }
    if let Some(t) = facts.tools.filter(|t| *t > 0) {
        ready.push(format!("{t} tools"));
    }
    if !ready.is_empty() {
        right.push(txt(vec![
            span("● ", Some("success")),
            span(ready.join(" · "), Some("muted")),
        ]));
    }
    if let Some((replies, tokens, cost)) = &facts.resumed {
        right.push(txt(vec![
            span("● ", Some("info")),
            span(
                format!("resumed · {replies} replies · {tokens} tokens · {cost}"),
                Some("muted"),
            ),
        ]));
    } else if let Some(cwd) = &facts.cwd {
        let mut s = vec![
            span("● ", Some("info")),
            span("new session in ", Some("muted")),
            bold(cwd.clone(), None),
        ];
        if let Some(b) = &facts.branch {
            s.push(dim(format!(" on {b}")));
        }
        right.push(txt(s));
    }
    if let Some(c) = facts.today_cost.filter(|c| *c > 0.0) {
        let across = facts
            .today_sessions
            .filter(|n| *n > 0)
            .map(|n| format!(" across {n} session{}", if n == 1 { "" } else { "s" }))
            .unwrap_or_default();
        right.push(txt(vec![
            span("● ", Some("c3")),
            span("today ", Some("muted")),
            bold(money(c), Some("c3")),
            span(across, Some("muted")),
        ]));
    }
    if let Some(u) = &facts.update {
        right.push(txt(vec![
            span(
                format!("▲ update {} available", short_version(u)),
                Some("warning"),
            ),
            dim(" oino update"),
        ]));
    }
    let mut hint = vec![
        span("/info", Some("warning")),
        dim(" dashboard   "),
        span("alt+s", Some("muted")),
        dim(" shortcuts"),
    ];
    if interactive {
        hint.extend([span("   any key", Some("muted")), dim(" close")]);
    }
    right.push(txt(hint));
    let column = |children: Vec<Node>| -> Node { Layout::default().children(children).into() };
    let c = crab();
    // v1 `divider` → a top-rule `Block`; the auto-close countdown is a full gauge (`effect`
    // `drain` and `durationMs` have no v2 node prop, §12.1).
    let bar: Node = if auto_close_ms.is_some() {
        LineGauge::new(1.0)
            .label(Line::from(""))
            .filled_style(Style::new().fg(Color::Warning))
            .into()
    } else {
        Block::default()
            .borders(Borders::Sides(vec![BorderSide::TOP]))
            .border_style(Style::new().fg(Color::BorderMuted))
            .into()
    };
    let splash = Layout::horizontal([Constraint::Length(22), Constraint::Fill(1)])
        .spacing(4)
        .collapse(76)
        .children([column(art(&c.grid22)), column(right)]);
    Block::default()
        .key(HEADER_KEY)
        .borders(Borders::All)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Border))
        .rail(Color::Warning)
        .child(column(vec![splash.into(), bar]))
        .into()
}

/// `setHeader` request.
#[must_use]
pub fn header_request(spec: Node) -> Value {
    // The ASCII splash is the terminal's welcome; the app draws its own empty chat.
    let mut r = json!({"method": "setHeader", "key": HEADER_KEY, "surfaces": ["tui"]});
    r["spec"] = serde_json::to_value(spec).unwrap_or(Value::Null);
    r
}

/// Clears the splash.
#[must_use]
pub fn clear_request() -> Value {
    json!({"method": "setHeader", "key": HEADER_KEY})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordmark_and_versions() {
        let w = wordmark_rows();
        assert_eq!(w[0], "█  █ ██▄█ █ █▀▀█ █");
        assert_eq!(short_version("3.0.0-alpha.24"), "α24");
        assert_eq!(short_version("3.0.0"), "3.0");
        assert_eq!(greeting(3), "Up late");
        assert_eq!(greeting(20), "Good evening");
    }

    #[test]
    fn spec_has_breakpoints_and_facts() {
        let f = Facts {
            ready_ms: Some(412),
            modules: Some(21),
            tools: Some(64),
            today_cost: Some(107.0),
            today_sessions: Some(12),
            resumed: Some((98, "12.5M".into(), "$3".into())),
            update: Some("3.1.0".into()),
            ..Facts::default()
        };
        let s: Value = serde_json::to_value(spec("3.0.0", "0.1.0", &f, 20, false, Some(2500)))
            .unwrap_or_default();
        assert_eq!(s["t"], "Block");
        assert_eq!(s["key"], HEADER_KEY);
        assert_eq!(s["rail"], "warning");
        assert_eq!(s["child"]["children"][0]["collapse"], 76);
        assert_eq!(s["child"]["children"][1]["t"], "LineGauge");
        let t = s.to_string();
        for needle in [
            "ready in 412ms · 21 modules · 64 tools",
            "resumed · 98 replies",
            "today ",
            "$107.00",
            "across 12 sessions",
            "update 3.1 available",
            "Good evening",
        ] {
            assert!(t.contains(needle), "{needle}");
        }
        assert!(!t.contains("any key"));
        assert!(
            serde_json::to_string(&spec("3.0.0", "0.1.0", &Facts::default(), 9, true, None))
                .unwrap_or_default()
                .contains("any key")
        );
        assert_eq!(art(&crab().grid22).len(), 10);
        assert_eq!(header_request(Paragraph::new("x").into())["key"], "splash");
    }
}
