//! What compaction saved, from session entries (unipi `src/compaction-savings.ts`).
//!
//! Per compaction: `saved = before − after`, `avoided = saved × replies after it`,
//! `dollars = avoided × (Σ paid for context / Σ context tokens)`. "after" is the compactor's
//! `details.tokensAfter` when present, else the context of the first reply after it. Feed
//! entries in file/branch order.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

/// One compaction.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CompactionSaving {
    /// Epoch ms.
    pub at: i64,
    /// Context before.
    pub before: f64,
    /// Context after.
    pub after: f64,
    /// Replies after it.
    pub replies: u64,
    /// Σ cost paid for context by those replies.
    pub paid: f64,
    /// Σ context tokens of those replies.
    pub context: f64,
}

impl CompactionSaving {
    /// Tokens removed from the context.
    #[must_use]
    pub fn saved(&self) -> f64 {
        let after = if self.after == 0.0 {
            self.before
        } else {
            self.after
        };
        (self.before - after).max(0.0)
    }

    /// Tokens not re-sent.
    #[must_use]
    pub fn avoided(&self) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "reply counts are small")]
        let replies = self.replies as f64;
        self.saved() * replies
    }

    /// Money not spent.
    #[must_use]
    pub fn dollars(&self) -> f64 {
        if self.context > 0.0 {
            self.avoided() * (self.paid / self.context)
        } else {
            0.0
        }
    }
}

/// Totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Totals {
    /// Compactions.
    pub count: usize,
    /// Σ saved.
    pub saved: f64,
    /// Σ avoided.
    pub avoided: f64,
    /// Σ dollars.
    pub dollars: f64,
    /// Σ context sent after compactions.
    pub sent: f64,
}

impl Totals {
    /// Sum of two totals (unipi `addTotals`).
    #[must_use]
    pub fn plus(self, o: Self) -> Self {
        Self {
            count: self.count + o.count,
            saved: self.saved + o.saved,
            avoided: self.avoided + o.avoided,
            dollars: self.dollars + o.dollars,
            sent: self.sent + o.sent,
        }
    }
}

/// Totals of `items`.
#[must_use]
pub fn totals_of(items: &[CompactionSaving]) -> Totals {
    items.iter().fold(
        Totals {
            count: items.len(),
            ..Totals::default()
        },
        |t, c| Totals {
            saved: t.saved + c.saved(),
            avoided: t.avoided + c.avoided(),
            dollars: t.dollars + c.dollars(),
            sent: t.sent + c.context,
            ..t
        },
    )
}

/// Streaming accumulator.
#[derive(Debug, Default)]
pub struct Accumulator {
    /// Compactions seen.
    pub items: Vec<CompactionSaving>,
    seen: BTreeSet<(i64, u64)>,
    open: bool,
    after_known: bool,
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

/// `Date.parse` for the ISO timestamps session files carry (`…Z`); 0 when unparseable.
#[allow(clippy::many_single_char_names, reason = "civil-date arithmetic names")]
fn parse_iso_ms(s: &str) -> i64 {
    let b = s.as_bytes();
    let field = |r: std::ops::Range<usize>| s.get(r).and_then(|t| t.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) = (
        field(0..4),
        field(5..7),
        field(8..10),
        field(11..13),
        field(14..16),
        field(17..19),
    ) else {
        return 0;
    };
    if b.get(4) != Some(&b'-') || b.get(10) != Some(&b'T') {
        return 0;
    }
    let ms = if b.get(19) == Some(&b'.') {
        let digits: String = s[20..]
            .chars()
            .take_while(char::is_ascii_digit)
            .take(3)
            .collect();
        format!("{digits:0<3}").parse().unwrap_or(0)
    } else {
        0
    };
    // days_from_civil (Howard Hinnant).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + h * 3600 + mi * 60 + se) * 1000) + ms
}

impl Accumulator {
    /// Feeds one session entry.
    pub fn feed(&mut self, entry: &Value) {
        match entry.get("type").and_then(Value::as_str) {
            Some("compaction") => {
                let at = entry
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map_or(0, parse_iso_ms);
                let before = num(entry.get("tokensBefore"));
                let key = (at, before.to_bits());
                if !self.seen.insert(key) {
                    // Forked files copy history: count each compaction once.
                    self.open = false;
                    return;
                }
                let after = num(entry.get("details").and_then(|d| d.get("tokensAfter")));
                self.items.push(CompactionSaving {
                    at,
                    before,
                    after,
                    ..CompactionSaving::default()
                });
                self.open = true;
                self.after_known = after > 0.0;
            }
            Some("message") if self.open => {
                let Some(m) = entry.get("message") else {
                    return;
                };
                if m.get("role").and_then(Value::as_str) != Some("assistant") {
                    return;
                }
                if matches!(
                    m.get("stopReason").and_then(Value::as_str),
                    Some("error" | "aborted")
                ) {
                    return;
                }
                let u = m.get("usage").cloned().unwrap_or(Value::Null);
                let ctx = num(u.get("input")) + num(u.get("cacheRead")) + num(u.get("cacheWrite"));
                if ctx <= 0.0 {
                    return;
                }
                let Some(open) = self.items.last_mut() else {
                    return;
                };
                if !self.after_known {
                    open.after = ctx;
                    self.after_known = true;
                }
                open.replies += 1;
                open.context += ctx;
                let c = u.get("cost").cloned().unwrap_or(Value::Null);
                open.paid +=
                    num(c.get("input")) + num(c.get("cacheRead")) + num(c.get("cacheWrite"));
            }
            _ => {}
        }
    }

    /// Totals so far.
    #[must_use]
    pub fn totals(&self) -> Totals {
        totals_of(&self.items)
    }
}

/// Savings for a whole branch.
#[must_use]
pub fn compaction_savings(branch: &[Value]) -> (Vec<CompactionSaving>, Totals) {
    let mut acc = Accumulator::default();
    for e in branch {
        acc.feed(e);
    }
    let totals = acc.totals();
    (acc.items, totals)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn compaction(ts: &str, before: u64, after: Option<u64>) -> Value {
        let mut e = json!({"type": "compaction", "timestamp": ts, "tokensBefore": before});
        if let Some(a) = after {
            e["details"] = json!({"tokensAfter": a});
        }
        e
    }

    fn reply(input: u64, cache_read: u64, cost_in: f64, cost_read: f64) -> Value {
        json!({"type": "message", "message": {"role": "assistant", "stopReason": "stop",
            "usage": {"input": input, "cacheRead": cache_read, "cacheWrite": 0,
                "cost": {"input": cost_in, "cacheRead": cost_read, "cacheWrite": 0}}}})
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn multiplies_saving_by_replies_at_paid_rate() {
        let (items, t) = compaction_savings(&[
            compaction("2026-01-01T00:00:00.000Z", 100_000, None),
            reply(10_000, 10_000, 0.02, 0.002),
            reply(5_000, 15_000, 0.01, 0.003),
            json!({"type": "message", "message": {"role": "user"}}),
        ]);
        assert_eq!(items.len(), 1);
        assert!(close(items[0].after, 20_000.0), "first reply's context");
        assert_eq!(items[0].replies, 2);
        assert!(close(t.saved, 80_000.0));
        assert!(close(t.avoided, 160_000.0));
        assert!(close(t.dollars, 160_000.0 * (0.035 / 40_000.0)));
        assert!(close(t.sent, 40_000.0));
    }

    #[test]
    fn prefers_compactor_tokens_after() {
        let (items, _) = compaction_savings(&[
            compaction("2026-01-01T00:00:00Z", 50_000, Some(8_000)),
            reply(30_000, 0, 0.1, 0.0),
        ]);
        assert!(close(items[0].after, 8_000.0));
    }

    #[test]
    fn forked_copy_counts_once_and_errors_are_skipped() {
        let c = compaction("2026-01-01T00:00:00.000Z", 10_000, Some(1_000));
        let err = json!({"type": "message", "message": {"role": "assistant", "stopReason": "error", "usage": {"input": 99}}});
        let (items, t) = compaction_savings(&[
            c.clone(),
            reply(1_000, 0, 0.0, 0.0),
            c,
            err,
            reply(1_000, 0, 0.0, 0.0),
        ]);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].replies, 1,
            "replies after the duplicate are not attributed"
        );
        assert_eq!(t.count, 1);
    }

    #[test]
    fn free_models_save_tokens_not_money() {
        let (_, t) = compaction_savings(&[
            compaction("2026-01-01T00:00:00.000Z", 10_000, Some(2_000)),
            reply(2_000, 0, 0.0, 0.0),
        ]);
        assert!(close(t.avoided, 8_000.0));
        assert!(close(t.dollars, 0.0));
        let sum = t.plus(t);
        assert_eq!(sum.count, 2);
    }

    #[test]
    fn parses_iso() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:01.5Z"), 1_500);
        assert_eq!(parse_iso_ms("2024-10-27T03:33:20.123Z"), 1_730_000_000_123);
        assert_eq!(parse_iso_ms("garbage"), 0);
    }
}
