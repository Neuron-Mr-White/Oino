//! The session stats strip (unipi `src/strip.ts`) as `setFooter` segments.
//!
//! ```text
//! ↑12.3k in · ↓4.1k out | $1.23 | 350ms avg ttft · 84.0 tok/s | 3 turn · 12 steps |
//! 00:12 · tool 00:04 | 73% cache hit | 2 compactions · 39k→13k · 3m ago
//! ```
//!
//! Parts carry unipi's drop priorities; frontends drop the lowest whole part when the line
//! does not fit `width - 1` and hide the strip below [`MIN_STRIP_ROWS`] rows.

use serde_json::{Value, json};

use crate::scan::Snapshot;
use crate::settings::Strip;
use crate::tps::TpsTracker;

/// Footer key.
pub const FOOTER_KEY: &str = "strip";
/// Below this many rows the strip and process line hide.
pub const MIN_STRIP_ROWS: u32 = 20;

/// Part ids with their drop priority (higher survives longer).
pub const PRIORITIES: &[(&str, i32)] = &[
    ("tokens", 7),
    ("cost", 6),
    ("speed", 5),
    ("turns", 4),
    ("time", 3),
    ("cache", 2),
    ("compactions", 1),
];

fn priority_of(id: &str) -> i32 {
    PRIORITIES
        .iter()
        .find(|(k, _)| *k == id)
        .map_or(0, |(_, p)| *p)
}

/// One strip part.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// Part id.
    pub id: &'static str,
    /// Drop priority.
    pub priority: i32,
    /// Spans (`{text, tone?}`).
    pub spans: Vec<Value>,
}

impl Part {
    /// Plain text.
    #[must_use]
    pub fn text(&self) -> String {
        self.spans
            .iter()
            .filter_map(|s| s.get("text").and_then(Value::as_str))
            .collect()
    }
}

/// Stopwatch duration: `00:12`, `1:00:14`.
#[must_use]
pub fn fmt_wall(ms: i64) -> String {
    if ms < 1000 {
        return "00:00".to_owned();
    }
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `999`, `1.2k`, `12k`, `1.5M`.
#[must_use]
pub fn fmt_tokens_short(n: f64) -> String {
    if n < 1000.0 {
        format!("{}", n.round())
    } else if n < 10_000.0 {
        format!("{:.1}k", n / 1000.0)
    } else if n < 1_000_000.0 {
        format!("{}k", (n / 1000.0).round())
    } else {
        format!("{:.1}M", n / 1_000_000.0)
    }
}

/// `just now`, `3m ago`, `2h ago`, `4d ago`.
#[must_use]
pub fn ago_short(ms: i64) -> String {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "display"
    )]
    let sec = (ms.max(0) as f64 / 1000.0).round() as i64;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "display"
    )]
    let r = |d: i64| (sec as f64 / d as f64).round() as i64;
    if sec < 60 {
        "just now".to_owned()
    } else if sec < 3600 {
        format!("{}m ago", r(60))
    } else if sec < 86_400 {
        format!("{}h ago", r(3600))
    } else {
        format!("{}d ago", r(86_400))
    }
}

/// Cache hit % (`None` without data).
#[must_use]
pub fn cache_hit_pct(s: &Snapshot) -> Option<i64> {
    let denom = s.input + s.cache_read + s.cache_write;
    #[allow(clippy::cast_possible_truncation, reason = "percent")]
    (denom > 0.0).then(|| (s.cache_read / denom * 100.0).round() as i64)
}

/// Inputs beyond the snapshot.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// TPS tracker.
    pub tracker: &'a TpsTracker,
    /// Model billed by subscription (shows `sub`).
    pub subscription: bool,
    /// Text icon style (no arrows).
    pub text_icons: bool,
    /// Emit tones.
    pub color: bool,
    /// Wall clock.
    pub now: i64,
}

/// Builds the visible parts (order: tokens, cost, speed, turns, time, cache, compactions).
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout mirroring the unipi renderer"
)]
pub fn build_parts(toggles: &Strip, s: &Snapshot, cx: &Context<'_>) -> Vec<Part> {
    let span = |text: String, tone: &str| {
        if cx.color {
            json!({"text": text, "tone": tone})
        } else {
            json!({"text": text})
        }
    };
    let plain = |text: &str| json!({"text": text});
    let mut parts = Vec::new();
    let mut push = |id: &'static str, on: bool, spans: Vec<Value>| {
        if on && !spans.is_empty() {
            parts.push(Part {
                id,
                priority: priority_of(id),
                spans,
            });
        }
    };
    if s.input + s.output > 0.0 {
        let (i, o) = (
            format!("{} in", fmt_tokens_short(s.input)),
            format!("{} out", fmt_tokens_short(s.output)),
        );
        let (i, o) = if cx.text_icons {
            (i, o)
        } else {
            (format!("↑{i}"), format!("↓{o}"))
        };
        push(
            "tokens",
            toggles.tokens,
            vec![span(i, "c7"), plain(" · "), span(o, "c7")],
        );
    }
    if s.cost > 0.0 || cx.subscription {
        let t = if cx.subscription {
            "sub".to_owned()
        } else {
            format!("${:.2}", s.cost)
        };
        push("cost", toggles.cost, vec![span(t, "success")]);
    }
    let ttft = cx.tracker.avg_ttft_ms();
    let steps = cx.tracker.steps();
    if toggles.speed && (ttft.is_some() || steps > 0) {
        let avg = cx.tracker.session_avg_tps(cx.now);
        let tone = if avg >= 30.0 {
            "success"
        } else if avg < 10.0 {
            "error"
        } else {
            "default"
        };
        let label = if avg >= 100.0 {
            format!("{}", avg.round())
        } else if avg > 0.0 {
            format!("{avg:.1}")
        } else {
            "0".to_owned()
        };
        let mut spans = Vec::new();
        if let Some(t) = ttft {
            #[allow(clippy::cast_precision_loss, reason = "display")]
            let v = if t >= 10_000 {
                format!("{}s", (t as f64 / 1000.0).round())
            } else {
                format!("{t}ms")
            };
            spans.push(span(v, "c4"));
            spans.push(plain(" avg ttft"));
        }
        if steps > 0 {
            if !spans.is_empty() {
                spans.push(plain(" · "));
            }
            spans.push(span(format!("{label} tok/s"), tone));
        }
        push("speed", true, spans);
    }
    let turns = cx.tracker.turns();
    if turns > 0 || steps > 0 {
        let st = if steps == 1 { "" } else { "s" };
        push(
            "turns",
            toggles.turns,
            vec![
                span(turns.to_string(), "info"),
                plain(" turn · "),
                span(steps.to_string(), "info"),
                plain(&format!(" step{st}")),
            ],
        );
        push(
            "time",
            toggles.time,
            vec![
                span(fmt_wall(cx.tracker.llm_ms(cx.now)), "warning"),
                plain(" · tool "),
                span(fmt_wall(cx.tracker.tool_ms()), "warning"),
            ],
        );
    }
    if let Some(hit) = cache_hit_pct(s) {
        push(
            "cache",
            toggles.cache,
            vec![span(
                format!("{hit}% cache hit"),
                if hit >= 70 { "success" } else { "warning" },
            )],
        );
    }
    if s.compaction_count > 0 {
        let n = s.compaction_count;
        let mut spans = vec![
            span(n.to_string(), "c7"),
            plain(&format!(" compaction{}", if n == 1 { "" } else { "s" })),
        ];
        if s.compaction_before > 0.0 {
            spans.push(plain(" · "));
            spans.push(span(
                format!(
                    "{}→{}",
                    fmt_tokens_short(s.compaction_before),
                    fmt_tokens_short(s.compaction_after)
                ),
                "c7",
            ));
        }
        if let Some(at) = s.compaction_last_at {
            spans.push(plain(&format!(" · {}", ago_short(cx.now - at))));
        }
        push("compactions", toggles.compactions, spans);
    }
    parts
}

fn width(s: &str) -> usize {
    unipi_kit::fmt::visible_width(s)
}

/// Drops the lowest priority (ties: the later part) until the joined line fits `width - 1`;
/// one over-wide part is kept (the caller truncates).
#[must_use]
pub fn fit_parts(parts: &[Part], cols: usize) -> Vec<Part> {
    let cap = cols.saturating_sub(1).max(1);
    let mut kept = parts.to_vec();
    let line = |ps: &[Part]| ps.iter().map(Part::text).collect::<Vec<_>>().join(" | ");
    while kept.len() > 1 && width(&line(&kept)) > cap {
        let mut drop = 0;
        for (i, p) in kept.iter().enumerate().skip(1) {
            if p.priority <= kept.get(drop).map_or(i32::MAX, |d| d.priority) {
                drop = i;
            }
        }
        kept.remove(drop);
    }
    kept
}

/// One centered plain line for `cols` (fallback rendering; `[]` when nothing shows).
#[must_use]
pub fn render_line(parts: &[Part], cols: usize) -> Option<String> {
    if cols <= 1 {
        return None;
    }
    let kept = fit_parts(parts, cols);
    if kept.is_empty() {
        return None;
    }
    let line = kept.iter().map(Part::text).collect::<Vec<_>>().join(" | ");
    let cap = cols - 1;
    let line = if width(&line) > cap {
        unipi_kit::fmt::truncate_to_width(&line, cap, "…")
    } else {
        line
    };
    let pad = (cols - width(&line)) / 2;
    Some(format!("{}{line}", " ".repeat(pad)))
}

/// `setFooter` request: priority segments (Oino) plus a plain OUS fallback spec.
#[must_use]
pub fn footer_request(parts: &[Part]) -> Value {
    if parts.is_empty() {
        return clear_request();
    }
    let mut spans = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(json!({"text": " | ", "dim": true}));
        }
        spans.extend(p.spans.iter().cloned());
    }
    json!({
        "method": "setFooter",
        "key": FOOTER_KEY,
        "spec": {"t": "text", "value": spans, "wrap": false},
        "segments": parts.iter().map(|p| json!({"id": p.id, "text": p.text(), "spans": p.spans, "priority": p.priority, "align": "center"})).collect::<Vec<_>>(),
        "separator": " | ",
        "minRows": MIN_STRIP_ROWS,
    })
}

/// `setFooter` that clears the strip.
#[must_use]
pub fn clear_request() -> Value {
    json!({"method": "setFooter", "key": FOOTER_KEY})
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Strip = Strip {
        turns: true,
        time: true,
        speed: true,
        tokens: true,
        cost: true,
        compactions: true,
        cache: true,
    };

    fn snap() -> Snapshot {
        Snapshot {
            input: 12_300.0,
            output: 4_100.0,
            cache_read: 30_000.0,
            cost: 1.234,
            compaction_count: 2,
            compaction_before: 39_000.0,
            compaction_after: 13_000.0,
            compaction_last_at: Some(0),
            ..Snapshot::default()
        }
    }

    fn tracker() -> TpsTracker {
        let mut t = TpsTracker::default();
        t.on_turn_start(Some(1), 1);
        t.on_message_start(0);
        t.on_streaming_delta(0, &"x".repeat(400), 351);
        t.on_message_end(
            0,
            Some(&json!({"role": "assistant", "usage": {"output": 84}})),
            1_351,
        );
        t.on_turn_end(12_001);
        t
    }

    fn cx(t: &TpsTracker) -> Context<'_> {
        Context {
            tracker: t,
            subscription: false,
            text_icons: false,
            color: true,
            now: 180_000,
        }
    }

    #[test]
    fn every_part_with_all_toggles() {
        let t = tracker();
        let parts = build_parts(&ALL, &snap(), &cx(&t));
        let ids: Vec<&str> = parts.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "tokens",
                "cost",
                "speed",
                "turns",
                "time",
                "cache",
                "compactions"
            ]
        );
        let texts: Vec<String> = parts.iter().map(Part::text).collect();
        assert_eq!(texts[0], "↑12k in · ↓4.1k out");
        assert_eq!(texts[1], "$1.23");
        assert_eq!(texts[2], "350ms avg ttft · 84.0 tok/s");
        assert_eq!(texts[3], "1 turn · 1 step");
        assert_eq!(texts[4], "00:12 · tool 00:00");
        assert_eq!(texts[5], "71% cache hit");
        assert_eq!(texts[6], "2 compactions · 39k→13k · 3m ago");
    }

    #[test]
    fn each_toggle_hides_its_part_only() {
        let t = tracker();
        for id in [
            "turns",
            "time",
            "speed",
            "tokens",
            "cost",
            "compactions",
            "cache",
        ] {
            let mut tg = ALL;
            match id {
                "turns" => tg.turns = false,
                "time" => tg.time = false,
                "speed" => tg.speed = false,
                "tokens" => tg.tokens = false,
                "cost" => tg.cost = false,
                "compactions" => tg.compactions = false,
                _ => tg.cache = false,
            }
            let parts = build_parts(&tg, &snap(), &cx(&t));
            assert_eq!(parts.len(), 6, "{id}");
            assert!(parts.iter().all(|p| p.id != id));
        }
    }

    #[test]
    fn cost_sub_and_text_icons() {
        let t = TpsTracker::default();
        let mut c = cx(&t);
        let mut s = Snapshot {
            input: 10.0,
            output: 5.0,
            ..Snapshot::default()
        };
        assert!(
            build_parts(&ALL, &s, &c).iter().all(|p| p.id != "cost"),
            "zero cost hidden"
        );
        c.subscription = true;
        assert_eq!(build_parts(&ALL, &s, &c)[1].text(), "sub");
        c.text_icons = true;
        s.cost = 2.0;
        assert_eq!(build_parts(&ALL, &s, &c)[0].text(), "10 in · 5 out");
    }

    #[test]
    fn responsive_drop_order() {
        let t = tracker();
        let parts = build_parts(&ALL, &snap(), &cx(&t));
        let full: usize = parts
            .iter()
            .map(|p| p.text().chars().count())
            .sum::<usize>()
            + 3 * (parts.len() - 1);
        assert_eq!(fit_parts(&parts, full + 1).len(), 7);
        let dropped: Vec<&str> = fit_parts(&parts, full).iter().map(|p| p.id).collect();
        assert!(!dropped.contains(&"compactions"));
        let mut order = Vec::new();
        let mut cols = full + 1;
        let mut prev: Vec<&str> = parts.iter().map(|p| p.id).collect();
        while cols > 2 {
            cols -= 1;
            let now: Vec<&str> = fit_parts(&parts, cols).iter().map(|p| p.id).collect();
            for id in &prev {
                if !now.contains(id) {
                    order.push(*id);
                }
            }
            prev = now;
        }
        assert_eq!(
            order,
            ["compactions", "cache", "time", "turns", "speed", "cost"]
        );
        let line = render_line(&parts, 30).unwrap_or_default();
        assert!(unipi_kit::fmt::visible_width(&line) < 30);
        assert_eq!(render_line(&parts, 1), None);
        assert_eq!(render_line(&[], 80), None);
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_wall(999), "00:00");
        assert_eq!(fmt_wall(12_000), "00:12");
        assert_eq!(fmt_wall(3_614_000), "1:00:14");
        assert_eq!(fmt_tokens_short(999.0), "999");
        assert_eq!(fmt_tokens_short(1_234.0), "1.2k");
        assert_eq!(fmt_tokens_short(1_500_000.0), "1.5M");
        assert_eq!(ago_short(10_000), "just now");
        assert_eq!(ago_short(7_200_000), "2h ago");
        assert_eq!(ago_short(3 * 86_400_000), "3d ago");
        let f = footer_request(&build_parts(&ALL, &snap(), &cx(&tracker())));
        assert_eq!(f["segments"][0]["priority"], 7);
        assert_eq!(f["minRows"], 20);
        assert_eq!(footer_request(&[]), clear_request());
    }
}
