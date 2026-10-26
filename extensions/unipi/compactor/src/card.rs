//! The compaction card (unipi `card.ts`) and branch stats (unipi `stats.ts`, `info-screen.ts`,
//! `tools/context-budget.ts`, `tools/ctx-doctor.ts`).

use serde_json::{Value, json};
use unipi_kit::fmt::format_tokens;

use crate::config::{Config, Method};
use crate::cut::content_chars;

/// Card entry type.
pub const CARD_TYPE: &str = "unipi-compaction";
/// Compactor id in details.
pub const COMPACTOR_ID: &str = "@pi-unipi/compactor";

/// What triggered a compaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// `/compact` or a command.
    Manual,
    /// The context limit.
    Threshold,
    /// Overflow recovery.
    Overflow,
    /// The percentage trigger.
    Percent,
}

impl Trigger {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Threshold => "threshold",
            Self::Overflow => "overflow",
            Self::Percent => "percent",
        }
    }
}

fn fmt(n: f64) -> String {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "display"
    )]
    let u = n.max(0.0) as u64;
    format_tokens(u)
}

fn num(d: &Value, k: &str) -> Option<f64> {
    d.get(k).and_then(Value::as_f64)
}

fn trigger_short(d: &Value) -> String {
    match d.get("trigger").and_then(Value::as_str) {
        Some("percent") => num(d, "percent")
            .map_or_else(|| "at threshold".into(), |p| format!("at {}%", p.round())),
        Some("threshold") => "context limit".into(),
        Some("overflow") => "overflow".into(),
        _ => d
            .get("command")
            .and_then(Value::as_str)
            .map_or_else(|| "manual".into(), |c| format!("/{c}")),
    }
}

fn trigger_long(d: &Value) -> String {
    match d.get("trigger").and_then(Value::as_str) {
        Some("percent") => format!(
            "{} of context{}",
            num(d, "percent").map_or_else(|| "threshold".into(), |p| format!("{}%", p.round())),
            num(d, "threshold")
                .map(|t| format!(" (setting: {t}%)"))
                .unwrap_or_default()
        ),
        Some("threshold") => "Oino's context limit".into(),
        Some("overflow") => "context overflow — the request did not fit".into(),
        _ => d
            .get("command")
            .and_then(Value::as_str)
            .map_or_else(|| "manual /compact".into(), |c| format!("/{c}")),
    }
}

fn method_name(d: &Value) -> &'static str {
    if d.get("method").and_then(Value::as_str) == Some("llm") {
        "model summary"
    } else {
        "lossless"
    }
}

/// The collapsed line.
#[must_use]
pub fn headline(d: &Value) -> String {
    let before = num(d, "tokensBefore").unwrap_or(0.0);
    let size = num(d, "tokensAfter").map_or_else(
        || format!("from {}", fmt(before)),
        |a| format!("{} → {}", fmt(before), fmt(a)),
    );
    format!(
        "Compacted {size} tokens · {} · {}",
        method_name(d),
        trigger_short(d)
    )
}

/// Expanded rows.
#[must_use]
pub fn details(d: &Value) -> Vec<(String, String)> {
    let mut rows = vec![
        ("Method".to_owned(), method_name(d).to_owned()),
        ("Trigger".to_owned(), trigger_long(d)),
    ];
    if let (Some(k), Some(t)) = (num(d, "keptTurns"), num(d, "totalTurns")) {
        rows.push((
            "Kept".into(),
            format!(
                "last {k} of {t} turns{}",
                num(d, "keptTokens")
                    .map(|x| format!(" · ~{} tokens verbatim", fmt(x)))
                    .unwrap_or_default()
            ),
        ));
    }
    if let Some(s) = num(d, "summaryTokens") {
        let secs: Vec<&str> = d
            .get("sections")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        rows.push((
            "Summary".into(),
            format!(
                "~{} tokens{}",
                fmt(s),
                if secs.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", secs.join(", "))
                }
            ),
        ));
    }
    rows.push((
        "Recall".into(),
        "everything before stays searchable: /unipi:session-recall".into(),
    ));
    rows
}

/// OUS rendering of the card.
#[must_use]
pub fn render(d: &Value, expanded: bool) -> Value {
    let mut children = vec![
        json!({"t": "text", "value": [{"text": headline(d)}, {"text": if expanded { "" } else { "  ctrl+o" }, "dim": true}]}),
    ];
    if expanded {
        children.push(json!({"t": "kv", "items": details(d).into_iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>()}));
    }
    json!({"t": "card", "rail": "harness", "source": "Compactor", "body": {"t": "stack", "direction": "vertical", "children": children}})
}

/// Inputs of the card data.
#[derive(Clone, Debug, Default)]
pub struct CardInput {
    /// Method.
    pub method: &'static str,
    /// Trigger.
    pub trigger: &'static str,
    /// Command.
    pub command: Option<String>,
    /// Tokens before.
    pub tokens_before: u64,
    /// Tokens after estimate.
    pub tokens_after: Option<u64>,
    /// Summary text length.
    pub summary_chars: usize,
    /// `(kept, total, kept tokens)`.
    pub kept: Option<(usize, usize, u64)>,
    /// Section names.
    pub sections: Option<Vec<String>>,
    /// Percent.
    pub percent: Option<f64>,
    /// Threshold.
    pub threshold: Option<f64>,
}

/// unipi `buildCardData`.
#[must_use]
pub fn card_data(i: &CardInput) -> Value {
    let mut v = json!({"method": i.method, "trigger": i.trigger, "tokensBefore": i.tokens_before});
    if let Some(c) = &i.command {
        v["command"] = json!(c);
    }
    if let Some(a) = i.tokens_after {
        v["tokensAfter"] = json!(a);
    }
    if i.summary_chars > 0 {
        v["summaryTokens"] = json!(i.summary_chars.div_ceil(4));
    }
    if let Some((k, t, kt)) = i.kept {
        v["keptTurns"] = json!(k);
        v["totalTurns"] = json!(t);
        v["keptTokens"] = json!(kt);
    }
    if let Some(s) = &i.sections {
        v["sections"] = json!(s);
    }
    if let Some(p) = i.percent {
        v["percent"] = json!(p);
    }
    if let Some(t) = i.threshold {
        v["threshold"] = json!(t);
    }
    v
}

/// One compaction record.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Before.
    pub before: f64,
    /// After.
    pub after: f64,
    /// Method.
    pub method: Method,
    /// Epoch ms.
    pub at: Option<i64>,
}

#[allow(clippy::many_single_char_names, reason = "civil-date arithmetic")]
fn parse_iso_ms(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, dd) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':').map(|x| x.parse::<i64>().ok());
    let (h, mi, se) = (t.next()??, t.next()??, t.next()??);
    let ms: i64 = format!("{frac:0<3}")[..3].parse().ok()?;
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + dd - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + se) * 1000) + ms)
}

/// unipi `sessionCompactionStats` → `(records, before, after, saved)`.
#[must_use]
pub fn session_stats(branch: &[Value]) -> (Vec<Record>, f64, f64, f64) {
    let mut out = Vec::new();
    for (i, e) in branch.iter().enumerate() {
        if e.get("type").and_then(Value::as_str) != Some("compaction") {
            continue;
        }
        let before = num(e, "tokensBefore").unwrap_or(0.0);
        let d = e.get("details").cloned().unwrap_or(Value::Null);
        let mut after = num(&d, "tokensAfter").unwrap_or(0.0);
        if after == 0.0 {
            let kept = e
                .get("firstKeptEntryId")
                .and_then(Value::as_str)
                .and_then(|k| {
                    branch
                        .iter()
                        .position(|x| x.get("id").and_then(Value::as_str) == Some(k))
                });
            let mut chars = e
                .get("summary")
                .and_then(Value::as_str)
                .map_or(0, |s| s.chars().count());
            if let Some(k) = kept.filter(|k| *k < i) {
                for x in &branch[k..i] {
                    if x.get("type").and_then(Value::as_str) == Some("message") {
                        chars += content_chars(x.pointer("/message/content"));
                    }
                }
            }
            #[allow(clippy::cast_precision_loss, reason = "char counts fit f64")]
            let a = (chars as f64 / 4.0).ceil();
            after = a;
        }
        let ours = d.get("compactor").and_then(Value::as_str) == Some(COMPACTOR_ID);
        let method = if ours && d.get("method").and_then(Value::as_str) != Some("llm") {
            Method::Vcc
        } else {
            Method::Llm
        };
        out.push(Record {
            before,
            after,
            method,
            at: e
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_iso_ms),
        });
    }
    let before: f64 = out.iter().map(|r| r.before).sum();
    let after: f64 = out
        .iter()
        .map(|r| r.after.min(if r.before > 0.0 { r.before } else { r.after }))
        .sum();
    (out, before, after, (before - after).max(0.0))
}

/// "3 lossless, 1 model".
#[must_use]
pub fn method_breakdown(records: &[Record]) -> String {
    let mut counts: Vec<(Method, usize)> = Vec::new();
    for r in records {
        match counts.iter_mut().find(|(m, _)| *m == r.method) {
            Some(c) => c.1 += 1,
            None => counts.push((r.method, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(m, n)| {
            format!(
                "{n} {}",
                if m == Method::Vcc {
                    "lossless"
                } else {
                    "model"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `context_budget` text.
#[must_use]
pub fn context_budget(tokens: Option<f64>, window: Option<f64>, cfg: &Config) -> String {
    let Some(used) = tokens else {
        return "Context budget: unknown (no token data available yet).".into();
    };
    let window = window.unwrap_or(200_000.0);
    let used = used.max(0.0);
    let remaining = (window - used).max(0.0);
    let pct = if window > 0.0 {
        (used / window * 100.0).round()
    } else {
        0.0
    };
    let advice = if cfg.percent {
        if pct >= cfg.threshold {
            format!(
                "Context will be compacted automatically at the next turn boundary (threshold {}%). Keep working.",
                cfg.threshold
            )
        } else {
            format!(
                "Context compacts automatically at {}%. Keep working.",
                cfg.threshold
            )
        }
    } else if pct >= 85.0 {
        "Context is nearly full; Oino compacts automatically near the limit and the work continues. Keep working.".into()
    } else {
        "Plenty of room. Oino compacts automatically near the limit.".into()
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "display"
    )]
    let rem = remaining as u64;
    format!(
        "Context: ~{pct}% full (estimated {} tokens remaining)\nAdvice: {advice}",
        group_thousands(rem)
    )
}

/// `toLocaleString()` for en-US.
#[must_use]
pub fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `/unipi:compact-doctor` lines → `(healthy, text)`.
#[must_use]
pub fn doctor(cfg: &Config, auto_enabled: Option<bool>, has_model: bool) -> (bool, String) {
    let mut checks: Vec<(&str, &str, String)> = Vec::new();
    let when = if cfg.percent {
        format!("{}% of context", cfg.threshold)
    } else {
        "Oino's context limit".into()
    };
    checks.push((
        "pass",
        "Settings",
        format!(
            "method: {} · when: {when} · /compact: {}",
            cfg.method.name(),
            cfg.pi_compact.map_or("follow", Method::as_str)
        ),
    ));
    if !cfg.percent && auto_enabled == Some(false) {
        checks.push(("fail", "Auto-compaction", "compaction.enabled is false in ~/.oino/settings.json, so nothing compacts automatically. Enable it or set When = at a percentage.".into()));
    } else {
        checks.push((
            "pass",
            "Auto-compaction",
            if auto_enabled.is_none() {
                "settings unreadable (defaults apply)".into()
            } else {
                "enabled".into()
            },
        ));
    }
    if cfg.method == Method::Llm && !has_model {
        checks.push((
            "warn",
            "Model",
            "model summaries need a selected model".into(),
        ));
    }
    let healthy = checks.iter().all(|c| c.0 != "fail");
    let icon = |s: &str| match s {
        "pass" => "✓",
        "warn" => "!",
        _ => "✗",
    };
    let mut lines = vec![if healthy {
        "Compactor: all checks passed".to_owned()
    } else {
        "Compactor: issues found".to_owned()
    }];
    lines.extend(
        checks
            .iter()
            .map(|(s, n, m)| format!("{} {n}: {m}", icon(s))),
    );
    (healthy, lines.join("\n"))
}

/// The compactor help text.
pub const HELP: &str = "Compactor\n  /unipi:compact-vcc [keep:N]    lossless compaction now (no model call)\n  /unipi:compact-by-llm [focus]  compact now with a model-written summary\n  /unipi:session-recall <query>  search everything in this session, including compacted parts\n  /unipi:compact-stats           this session's savings\n  /unipi:compact-doctor          check settings\n\nSettings: /unipi:settings → Compactor (method, what /compact does, when to compact).";

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;
    use crate::config::{defaults, load};
    use unipi_kit::settings::Values;

    #[test]
    fn card() {
        let d = card_data(&CardInput {
            method: "vcc",
            trigger: "manual",
            command: Some("unipi:compact-vcc".into()),
            tokens_before: 7900,
            tokens_after: Some(2200),
            summary_chars: 4000,
            kept: Some((1, 3, 500)),
            sections: Some(vec!["Your Requests".into()]),
            ..CardInput::default()
        });
        assert_eq!(
            headline(&d),
            "Compacted 7.9k → 2.2k tokens · lossless · /unipi:compact-vcc"
        );
        let rows = details(&d);
        assert_eq!(rows[2].1, "last 1 of 3 turns · ~500 tokens verbatim");
        assert_eq!(rows[3].1, "~1.0k tokens · Your Requests");
        let p = json!({"method": "llm", "trigger": "percent", "percent": 81.4, "threshold": 80, "tokensBefore": 100});
        assert_eq!(
            headline(&p),
            "Compacted from 100 tokens · model summary · at 81%"
        );
        assert_eq!(details(&p)[1].1, "81% of context (setting: 80%)");
        assert_eq!(render(&d, true)["t"], "card");
    }

    #[test]
    fn stats_and_budget() {
        let branch = vec![
            json!({"id": "a", "type": "message", "message": {"role": "user", "content": "x".repeat(400)}}),
            json!({"id": "c", "type": "compaction", "tokensBefore": 1000, "summary": "s".repeat(40), "firstKeptEntryId": "a", "timestamp": "2026-01-01T00:00:00.000Z"}),
            json!({"id": "d", "type": "compaction", "tokensBefore": 2000, "details": {"compactor": COMPACTOR_ID, "method": "vcc", "tokensAfter": 500}}),
        ];
        let (rec, before, after, saved) = session_stats(&branch);
        assert_eq!(rec.len(), 2);
        assert!((rec[0].after - 110.0).abs() < f64::EPSILON);
        assert_eq!(rec[0].at, Some(1_767_225_600_000));
        assert!(
            (before - 3000.0).abs() < f64::EPSILON
                && (after - 610.0).abs() < f64::EPSILON
                && (saved - 2390.0).abs() < f64::EPSILON
        );
        assert_eq!(method_breakdown(&rec), "1 model, 1 lossless");
        let cfg = load(&Values(defaults()));
        assert_eq!(
            context_budget(None, None, &cfg),
            "Context budget: unknown (no token data available yet)."
        );
        assert_eq!(
            context_budget(Some(50_000.0), Some(200_000.0), &cfg),
            "Context: ~25% full (estimated 150,000 tokens remaining)\nAdvice: Plenty of room. Oino compacts automatically near the limit."
        );
        let (ok, text) = doctor(&cfg, Some(false), true);
        assert!(!ok && text.contains("✗ Auto-compaction"));
        assert!(doctor(&cfg, Some(true), true).0);
    }
}
