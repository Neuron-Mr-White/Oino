//! Page building blocks (unipi `tui/page-kit.ts`, core `viz.ts` formatters) as OUS nodes.
//!
//! unipi painted tiles, meters, share bars, braille charts and grids itself; here each maps to
//! a v2 node the frontend draws (`Block`, `LineGauge`, `BarChart`, `Chart`, `List`, `Table`, …).

use oino_extension::ui::widgets::prelude::*;

/// A colour token by name (`Color::parse` keeps unknown names).
#[must_use]
pub fn color(tone: &str) -> Color {
    Color::parse(tone)
}

/// A section header: a top rule with the title (`section` in the §12.1 mapping).
#[must_use]
pub fn rule(label: impl Into<Line>) -> Block {
    Block::default()
        .borders(Borders::Sides(vec![BorderSide::TOP]))
        .border_style(Style::new().fg(Color::BorderMuted))
        .title(BlockTitle::Line(label.into()))
}

/// Number scope: this session, this project, global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// This session.
    Session,
    /// This project.
    Project,
    /// Every project on this machine.
    Global,
}

impl Scope {
    /// One-letter tag.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Session => "s",
            Self::Project => "p",
            Self::Global => "g",
        }
    }

    /// Tag tone.
    #[must_use]
    pub fn tone(self) -> &'static str {
        match self {
            Self::Session => "info",
            Self::Project => "success",
            Self::Global => "c3",
        }
    }
}

/// unipi `compact`: `1.2k`, `340k`, `1.5M`.
#[must_use]
pub fn compact(n: f64) -> String {
    if !n.is_finite() {
        return "0".to_owned();
    }
    let f = |v: f64, u: &str| {
        if v.abs() >= 100.0 {
            format!("{}{u}", v.round())
        } else {
            let r = (v * 10.0).round() / 10.0;
            if r.fract() == 0.0 {
                format!("{r:.0}{u}")
            } else {
                format!("{r:.1}{u}")
            }
        }
    };
    let a = n.abs();
    if a >= 1e9 {
        f(n / 1e9, "B")
    } else if a >= 1e6 {
        f(n / 1e6, "M")
    } else if a >= 1e3 {
        f(n / 1e3, "k")
    } else {
        format!("{}", n.round())
    }
}

/// unipi `money`: `$0.004`, `$3.40`, `$1.2k`.
#[must_use]
pub fn money(n: f64) -> String {
    if !n.is_finite() || n <= 0.0 {
        "$0".to_owned()
    } else if n >= 1000.0 {
        format!("${}", compact(n))
    } else if n >= 0.01 {
        format!("${n:.2}")
    } else {
        format!("${n:.3}")
    }
}

/// unipi `duration`: `35s`, `1m 35s`, `1h 01m`.
#[must_use]
pub fn duration(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m {:02}s", s % 60);
    }
    format!("{}h {:02}m", m / 60, m % 60)
}

/// `12s ago` … `3d ago`, `never`.
#[must_use]
pub fn ago(ms: i64, now: i64) -> String {
    if ms == 0 {
        return "never".to_owned();
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "display"
    )]
    let s = ((now - ms).max(0) as f64 / 1000.0).round() as i64;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "display"
    )]
    let r = |d: i64| (s as f64 / d as f64).round() as i64;
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", r(60))
    } else if s < 86_400 {
        format!("{}h ago", r(3600))
    } else {
        format!("{}d ago", r(86_400))
    }
}

/// A span.
#[must_use]
pub fn span(text: impl Into<String>, tone: Option<&str>) -> Span {
    let s = Span::from_string(text);
    match tone {
        Some(t) => s.fg(color(t)),
        None => s,
    }
}

/// A dim span.
#[must_use]
pub fn dim(text: impl Into<String>) -> Span {
    Span::from_string(text).dim()
}

/// A bold span.
#[must_use]
pub fn bold(text: impl Into<String>, tone: Option<&str>) -> Span {
    let s = Span::from_string(text).bold();
    match tone {
        Some(t) => s.fg(color(t)),
        None => s,
    }
}

/// `Paragraph` node from spans (one line, no wrapping).
#[must_use]
pub fn line(spans: Vec<Span>) -> Node {
    Paragraph::new(Line::from(spans))
        .wrap(Wrap::Disabled(false))
        .into()
}

/// Section header: a labelled rule plus an optional right-hand line and scope tag.
#[must_use]
pub fn section(title: &str, right: Vec<Span>, scope: Option<Scope>) -> Node {
    let mut label = title.to_uppercase();
    if let Some(s) = scope {
        label = format!("{label} {}", s.tag());
    }
    if right.is_empty() {
        return rule(label).into();
    }
    Layout::default()
        .children([rule(label).into(), line(right)])
        .into()
}

/// One tile.
#[derive(Clone, Debug)]
pub struct Tile {
    /// Scope tag.
    pub scope: Option<Scope>,
    /// Label.
    pub label: String,
    /// Big value.
    pub value: String,
    /// Sub line.
    pub sub: String,
    /// Value tone.
    pub tone: &'static str,
}

/// A tile row (big numbers).
#[must_use]
pub fn tiles(items: &[Tile]) -> Node {
    let cards = items.iter().map(|t| {
        let mut title = vec![span(t.label.to_uppercase(), Some("muted"))];
        if let Some(s) = t.scope {
            title.push(span(" ", None));
            title.push(span(s.tag(), Some(s.tone())));
        }
        let body = Layout::default().children([
            line(vec![bold(t.value.clone(), Some(t.tone))]),
            line(vec![dim(t.sub.clone())]),
        ]);
        Block::default()
            .borders(Borders::All)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(Color::Border))
            .rail(color(t.tone))
            .title(BlockTitle::Line(Line::from(title)))
            .child(body)
            .into()
    });
    Layout::horizontal(vec![Constraint::Fill(1); items.len()])
        .spacing(2)
        .children(cards)
        .into()
}

/// Coloured legend chips (`■ label`).
#[must_use]
pub fn legend(items: &[(String, &str, Option<Scope>)]) -> Line {
    let mut spans = Vec::new();
    for (i, (label, tone, scope)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(span("   ", None));
        }
        spans.push(span("■", Some(tone)));
        if let Some(s) = scope {
            spans.push(span(format!(" {}", s.tag()), Some(s.tone())));
        }
        spans.push(span(format!(" {label}"), Some("muted")));
    }
    Line::from(spans)
}

/// Share bar (`BarChart` with categorical tones).
#[must_use]
pub fn share_bar(bars: &[(String, f64, &str)]) -> Node {
    let bar = |label: &str, value: f64, tone: &str| {
        Bar::new(units(value))
            .label(Line::from(label))
            .text_value(compact(value))
            .style(Style::new().fg(color(tone)))
    };
    let group = BarGroup::new(bars.iter().map(|(l, v, t)| bar(l, *v, t)));
    BarChart::new([group]).bar_gap(0).into()
}

/// A share value as a bar chart unit (rounded, never negative).
#[must_use]
pub fn units(value: f64) -> u64 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "display"
    )]
    let v = value.max(0.0).round() as u64;
    v
}

/// Key/value block (§12.1 `kv` → 2-column `Table`, key cells muted).
#[must_use]
pub fn kv(items: &[(&str, Line)]) -> Node {
    let rows = items.iter().map(|(key, value)| {
        Row::new([
            Cell::new(Line::from(span(*key, Some("muted")))),
            Cell::new(value.clone()),
        ])
    });
    Table::new(rows, [Constraint::Max(24), Constraint::Fill(1)]).into()
}

/// A one-row progress bar (`meter` → `LineGauge`, §12.1): `value / max`, tinted by `tone`.
#[must_use]
pub fn meter(value: f64, max: f64, tone: &str, label: impl Into<String>) -> Node {
    let ratio = if max > 0.0 {
        (value / max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    LineGauge::new(ratio)
        .label(Line::from(label.into()))
        .filled_style(Style::new().fg(color(tone)))
        .into()
}

/// A two-column key/value row (see [`kv`]).
#[must_use]
pub fn kv_row(key: Line, value: Line) -> Row {
    Row::new([Cell::new(key), Cell::new(value)])
}

/// A key/value table from prepared rows (see [`kv`]).
#[must_use]
pub fn kv_table(rows: Vec<Row>) -> Node {
    let rows = if rows.is_empty() {
        vec![kv_row(Line::from(""), Line::from(""))]
    } else {
        rows
    };
    Table::new(rows, [Constraint::Max(24), Constraint::Fill(1)]).into()
}

/// A series plot (`area` → single-dataset `Chart`, §12.1).
#[must_use]
pub fn area(label: &str, values: &[f64]) -> Node {
    let points: Vec<[f64; 2]> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            #[allow(clippy::cast_precision_loss, reason = "display")]
            let x = i as f64;
            [x, *v]
        })
        .collect();
    let max = values.iter().copied().fold(0.0, f64::max);
    let last = values.len().saturating_sub(1);
    #[allow(clippy::cast_precision_loss, reason = "display")]
    let axis = Axis::bounds(0.0, last as f64);
    Chart::new([Dataset::new(points)
        .name(label)
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)])
    .x_axis(axis)
    .y_axis(Axis::bounds(0.0, max))
    .hide_legend()
    .into()
}

/// Empty state.
#[must_use]
pub fn empty(title: &str, hint: &str) -> Node {
    Empty::new()
        .title(title)
        .message(format!("{title} — {hint}"))
        .into()
}

/// Health dot tone.
#[must_use]
pub fn health(state: &str) -> &'static str {
    match state {
        "ok" => "success",
        "warn" => "warning",
        "bad" => "error",
        _ => "muted",
    }
}

/// A vertical page (auto-mode stack, one blank row between children).
#[must_use]
pub fn page(children: Vec<Node>) -> Node {
    Layout::default().spacing(1).children(children).into()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn formatters() {
        assert_eq!(compact(950.0), "950");
        assert_eq!(compact(1_234.0), "1.2k");
        assert_eq!(compact(2_000.0), "2k");
        assert_eq!(compact(340_000.0), "340k");
        assert_eq!(compact(12_500_000.0), "12.5M");
        assert_eq!(money(0.0), "$0");
        assert_eq!(money(0.0042), "$0.004");
        assert_eq!(money(3.4), "$3.40");
        assert_eq!(money(1234.0), "$1.2k");
        assert_eq!(duration(35_000), "35s");
        assert_eq!(duration(95_000), "1m 35s");
        assert_eq!(duration(3_700_000), "1h 01m");
        assert_eq!(ago(0, 5), "never");
        assert_eq!(ago(1_000, 121_000), "2m ago");
    }

    fn json_of(node: &Node) -> Value {
        serde_json::to_value(node).unwrap_or(Value::Null)
    }

    #[test]
    fn nodes() {
        let t = json_of(&tiles(&[Tile {
            scope: Some(Scope::Global),
            label: "cost".into(),
            value: "$1".into(),
            sub: "x".into(),
            tone: "c3",
        }]));
        assert_eq!(t["t"], "Layout");
        assert_eq!(t["children"][0]["t"], "Block");
        assert_eq!(t["children"][0]["rail"], "c3");
        assert_eq!(t["children"][0]["title"][2]["content"], "g");
        let section = json_of(&section("tools", vec![], Some(Scope::Session)));
        assert_eq!(section["t"], "Block");
        assert_eq!(section["title"], "TOOLS s");
        assert_eq!(section["borders"], json!(["TOP"]));
        let table = json_of(&kv(&[("branch", Line::from("main"))]));
        assert_eq!(table["t"], "Table");
        assert_eq!(
            table["rows"][0]["cells"][0]["content"],
            json!([[{"content": "branch", "style": {"fg": "muted"}}]])
        );
        assert_eq!(table["widths"], json!([{"Max": 24}, {"Fill": 1}]));
        let bar = json_of(&share_bar(&[("x".into(), 3.0, "info")]));
        assert_eq!(bar["t"], "BarChart");
        assert_eq!(bar["data"][0]["bars"][0]["value"], 3);
        assert_eq!(bar["data"][0]["bars"][0]["textValue"], "3");
    }
}
