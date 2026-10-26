//! Page building blocks (unipi `tui/page-kit.ts`, core `viz.ts` formatters) as OUS nodes.
//!
//! unipi painted tiles, meters, share bars, braille charts and grids itself; here each maps to
//! an OUS node the frontend draws (`card`, `meter`, `barChart`, `area`, `list`, `kv`, …).

use serde_json::{Value, json};

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
pub fn span(text: impl Into<String>, tone: Option<&str>) -> Value {
    let mut s = json!({"text": text.into()});
    if let Some(t) = tone {
        s["tone"] = json!(t);
    }
    s
}

/// A dim span.
#[must_use]
pub fn dim(text: impl Into<String>) -> Value {
    json!({"text": text.into(), "dim": true})
}

/// A bold span.
#[must_use]
pub fn bold(text: impl Into<String>, tone: Option<&str>) -> Value {
    let mut s = span(text, tone);
    s["bold"] = json!(true);
    s
}

/// `text` node from spans.
#[must_use]
pub fn line(spans: Vec<Value>) -> Value {
    json!({"t": "text", "value": Value::Array(spans), "wrap": false})
}

/// Section header: a labelled rule plus an optional right-hand line and scope tag.
#[must_use]
pub fn section(title: &str, right: Vec<Value>, scope: Option<Scope>) -> Value {
    let mut label = title.to_uppercase();
    if let Some(s) = scope {
        label = format!("{label} {}", s.tag());
    }
    if right.is_empty() {
        return json!({"t": "divider", "label": label});
    }
    json!({"t": "stack", "direction": "vertical", "gap": 0, "children": [{"t": "divider", "label": label}, line(right)]})
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
pub fn tiles(items: &[Tile]) -> Value {
    let cards: Vec<Value> = items
        .iter()
        .map(|t| {
            let mut title = vec![span(t.label.to_uppercase(), Some("muted"))];
            if let Some(s) = t.scope {
                title.push(span(" ", None));
                title.push(span(s.tag(), Some(s.tone())));
            }
            json!({"t": "card", "title": title, "rail": t.tone, "body": {"t": "stack", "direction": "vertical", "gap": 0, "children": [
                line(vec![bold(t.value.clone(), Some(t.tone))]),
                line(vec![dim(t.sub.clone())]),
            ]}})
        })
        .collect();
    json!({"t": "stack", "direction": "horizontal", "gap": 2, "children": cards})
}

/// Coloured legend chips (`■ label`).
#[must_use]
pub fn legend(items: &[(String, &str, Option<Scope>)]) -> Value {
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
    line(spans)
}

/// Share bar (`barChart` with categorical tones).
#[must_use]
pub fn share_bar(bars: &[(String, f64, &str)]) -> Value {
    json!({"t": "barChart", "bars": bars.iter().map(|(l, v, t)| json!({"label": l, "value": v, "tone": t})).collect::<Vec<_>>()})
}

/// Key/value block.
#[must_use]
pub fn kv(items: &[(&str, Value)]) -> Value {
    json!({"t": "kv", "items": items.iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>()})
}

/// Empty state.
#[must_use]
pub fn empty(title: &str, hint: &str) -> Value {
    json!({"t": "empty", "message": format!("{title} — {hint}")})
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

/// A vertical page.
#[must_use]
pub fn page(children: Vec<Value>) -> Value {
    json!({"t": "stack", "direction": "vertical", "gap": 1, "children": Value::Array(children)})
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn nodes() {
        let t = tiles(&[Tile {
            scope: Some(Scope::Global),
            label: "cost".into(),
            value: "$1".into(),
            sub: "x".into(),
            tone: "c3",
        }]);
        assert_eq!(t["children"][0]["t"], "card");
        assert_eq!(t["children"][0]["title"][2]["text"], "g");
        assert_eq!(
            section("tools", vec![], Some(Scope::Session))["label"],
            "TOOLS s"
        );
    }
}
