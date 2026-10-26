//! Incremental session-branch scan (unipi `src/session-scan.ts`, `segments/compactor.ts`).
//!
//! Remembers how many branch entries were processed and feeds only new ones to the
//! [`TpsTracker`]; a shrunken branch or a new compaction entry triggers a full rescan. The
//! trailing in-flight assistant message is re-fed until it has a stop reason. The same pass
//! accumulates the strip snapshot (usage sums, child-agent cost, compactions, turn count).

use std::collections::HashMap;

use serde_json::Value;

use crate::time::parse_iso_ms;
use crate::tps::TpsTracker;

/// unipi child-usage custom entry type.
pub const CHILD_USAGE_ENTRY: &str = "unipi-child-usage";

/// Data the stats strip reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Branch entries seen.
    pub branch_length: usize,
    /// User messages.
    pub user_count: u64,
    /// Assistant messages fed (error/aborted skipped).
    pub assistant_count: u64,
    /// Input tokens.
    pub input: f64,
    /// Output tokens.
    pub output: f64,
    /// Cache-read tokens.
    pub cache_read: f64,
    /// Cache-write tokens.
    pub cache_write: f64,
    /// Cost (incl. child agents).
    pub cost: f64,
    /// Part of `cost` spent by child agents.
    pub child_cost: f64,
    /// Compactions on the branch.
    pub compaction_count: u64,
    /// Tokens before, summed.
    pub compaction_before: f64,
    /// Tokens after, summed.
    pub compaction_after: f64,
    /// Last compaction (epoch ms).
    pub compaction_last_at: Option<i64>,
    /// Last kept assistant timestamp.
    pub last_assistant_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Usage {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
    cost: f64,
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(0.0)
}

fn usage_of(u: Option<&Value>) -> Usage {
    let Some(u) = u.filter(|u| u.is_object()) else {
        return Usage::default();
    };
    Usage {
        input: num(u.get("input")),
        output: num(u.get("output")),
        cache_read: num(u.get("cacheRead")),
        cache_write: num(u.get("cacheWrite")),
        cost: num(u.pointer("/cost/total")),
    }
}

/// Child-agent cost of an entry: unipi `unipi-child-usage` custom entries and Oino host
/// `usage` entries (runtime §12, `kind: "child:<label>"` / `"ext:<id>"`).
#[must_use]
pub fn child_cost_of(e: &Value) -> Option<f64> {
    match e.get("type").and_then(Value::as_str)? {
        "custom" if e.get("customType").and_then(Value::as_str) == Some(CHILD_USAGE_ENTRY) => {
            Some(num(e.pointer("/data/cost")))
        }
        "usage" => Some(num(e.pointer("/usage/cost/total"))),
        _ => None,
    }
}

/// Compactions on the branch: count, tokens before → after, last time (unipi
/// `compactionSummary`).
#[must_use]
pub fn compaction_summary(branch: &[Value], now: i64) -> (u64, f64, f64, Option<i64>) {
    let (mut count, mut before, mut after, mut last) = (0, 0.0, 0.0, None);
    for e in branch {
        if e.get("type").and_then(Value::as_str) != Some("compaction") {
            continue;
        }
        count += 1;
        let b = num(e.get("tokensBefore"));
        before += b;
        let recorded = num(e.pointer("/details/tokensAfter"));
        after += if recorded > 0.0 {
            recorded.min(b)
        } else {
            #[allow(clippy::cast_precision_loss, reason = "token estimate")]
            let chars = e
                .get("summary")
                .and_then(Value::as_str)
                .map_or(0, |s| s.encode_utf16().count()) as f64;
            b.min((chars / 4.0).ceil())
        };
        if let Some(at) = e
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
            && at <= now
        {
            last = Some(at);
        }
    }
    (count, before, after, last)
}

/// The scanner (one per session).
#[derive(Clone, Debug, Default)]
pub struct Scanner {
    processed: usize,
    pending_tools: HashMap<String, i64>,
    branch_tool_ms: i64,
    first_ts: i64,
    last_ts: i64,
    prev_ts: i64,
    inflight: Option<(usize, usize, Usage)>,
    /// Strip snapshot.
    pub snapshot: Snapshot,
}

impl Scanner {
    /// The branch shrank, or a compaction entry appeared in the unprocessed tail.
    #[must_use]
    pub fn needs_full_rescan(&self, events: &[Value]) -> bool {
        self.processed == 0
            || events.len() < self.processed
            || events
                .get(self.processed..)
                .into_iter()
                .flatten()
                .any(|e| e.get("type").and_then(Value::as_str) == Some("compaction"))
    }

    /// Forget all scan state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn add(&mut self, u: Usage, sign: f64) {
        let s = &mut self.snapshot;
        s.input += sign * u.input;
        s.output += sign * u.output;
        s.cache_read += sign * u.cache_read;
        s.cache_write += sign * u.cache_write;
        s.cost += sign * u.cost;
    }

    /// Feeds unprocessed entries (reconciling the in-flight tail first).
    pub fn scan(&mut self, events: &[Value], tracker: &mut TpsTracker, now: i64) {
        self.reconcile_inflight(events, tracker, now);
        for i in self.processed..events.len() {
            if let Some(e) = events.get(i) {
                self.process(e, i, tracker, now);
            }
        }
        self.processed = events.len();
        let (count, before, after, last) = compaction_summary(events, now);
        self.snapshot.compaction_count = count;
        self.snapshot.compaction_before = before;
        self.snapshot.compaction_after = after;
        self.snapshot.compaction_last_at = last;
        self.snapshot.branch_length = events.len();
        tracker.sync_branch_stats(self.snapshot.user_count, self.snapshot.assistant_count);
        if self.last_ts > self.first_ts {
            tracker.sync_wall_ms(self.last_ts - self.first_ts);
        }
        tracker.sync_tool_ms(self.branch_tool_ms);
    }

    /// Resets and rescans everything.
    pub fn full_rescan(&mut self, events: &[Value], tracker: &mut TpsTracker, now: i64) {
        self.reset();
        tracker.reset();
        self.scan(events, tracker, now);
    }

    fn reconcile_inflight(&mut self, events: &[Value], tracker: &mut TpsTracker, now: i64) {
        let Some((entry_idx, msg_idx, old)) = self.inflight else {
            return;
        };
        let Some(m) = events.get(entry_idx).and_then(|e| e.get("message")) else {
            self.inflight = None;
            return;
        };
        let stopped = m.get("stopReason").is_some_and(|s| !s.is_null());
        tracker.on_message_update(msg_idx, m, stopped, now);
        self.add(old, -1.0);
        let fresh = usage_of(m.get("usage"));
        self.add(fresh, 1.0);
        self.inflight = (!stopped).then_some((entry_idx, msg_idx, fresh));
    }

    fn process(&mut self, e: &Value, entry_idx: usize, tracker: &mut TpsTracker, now: i64) {
        if let Some(cost) = child_cost_of(e) {
            self.snapshot.cost += cost;
            self.snapshot.child_cost += cost;
            return;
        }
        if e.get("type").and_then(Value::as_str) != Some("message") {
            return;
        }
        let Some(m) = e.get("message") else { return };
        let ts = e
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
            .unwrap_or(0);
        match m.get("role").and_then(Value::as_str) {
            Some("user") => self.snapshot.user_count += 1,
            Some("toolResult") => {
                if let Some(id) = m.get("toolCallId").and_then(Value::as_str)
                    && let Some(issued) = self.pending_tools.remove(id)
                    && ts > issued
                {
                    self.branch_tool_ms += (ts - issued).min(600_000);
                }
            }
            Some("assistant") => self.process_assistant(m, entry_idx, ts, tracker, now),
            _ => {}
        }
    }

    fn process_assistant(
        &mut self,
        m: &Value,
        entry_idx: usize,
        ts: i64,
        tracker: &mut TpsTracker,
        now: i64,
    ) {
        if matches!(
            m.get("stopReason").and_then(Value::as_str),
            Some("error" | "aborted")
        ) {
            return;
        }
        if ts > 0 {
            if self.first_ts == 0 || ts < self.first_ts {
                self.first_ts = ts;
            }
            self.last_ts = self.last_ts.max(ts);
        }
        let msg_idx = usize::try_from(self.snapshot.assistant_count).unwrap_or(usize::MAX);
        let usage = usage_of(m.get("usage"));
        self.add(usage, 1.0);
        let stopped = m.get("stopReason").is_some_and(|s| !s.is_null());
        tracker.on_message_update(msg_idx, m, stopped, now);
        for b in m
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let kind = b
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            if (kind == "toolcall" || kind == "tool_use")
                && ts > 0
                && let Some(id) = b.get("id").and_then(Value::as_str)
            {
                self.pending_tools.insert(id.to_owned(), ts);
            }
        }
        if ts > 0 {
            tracker.seed_ttft_fallback(self.prev_ts, ts, msg_idx);
            self.prev_ts = ts;
        }
        self.snapshot.assistant_count += 1;
        if ts > 0 {
            self.snapshot.last_assistant_at = Some(ts);
        }
        self.inflight = (!stopped).then_some((entry_idx, msg_idx, usage));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn iso(min: u32, sec: u32) -> String {
        format!("2026-01-01T10:{min:02}:{sec:02}.000Z")
    }

    fn user(text: &str) -> Value {
        json!({"type": "message", "timestamp": iso(0, 0), "message": {"role": "user", "content": [{"type": "text", "text": text}]}})
    }

    fn assistant(text: &str, out: u64, sec: u32) -> Value {
        json!({"type": "message", "timestamp": iso(0, sec), "message": {"role": "assistant", "content": [{"type": "text", "text": text}], "stopReason": "stop",
            "usage": {"input": 100, "output": out, "cacheRead": 10, "cacheWrite": 0, "cost": {"total": 0.01}}}})
    }

    fn branch(step: u32) -> Vec<Value> {
        (1..=step)
            .flat_map(|i| {
                [
                    user(&format!("u{i}")),
                    assistant(&format!("a{i}"), 10 + u64::from(i), i),
                ]
            })
            .collect()
    }

    #[test]
    fn two_step_equals_full_scan() {
        let (mut a, mut ta) = (Scanner::default(), TpsTracker::default());
        a.scan(&branch(4), &mut ta, 0);
        let (mut b, mut tb) = (Scanner::default(), TpsTracker::default());
        b.scan(&branch(2), &mut tb, 0);
        b.scan(&branch(4), &mut tb, 0);
        assert_eq!(a.snapshot, b.snapshot);
        assert_eq!((ta.turns(), ta.steps()), (tb.turns(), tb.steps()));
        assert_eq!(a.snapshot.assistant_count, 4);
        assert!((a.snapshot.input - 400.0).abs() < 1e-9);
        b.scan(&branch(4), &mut tb, 0);
        assert_eq!(a.snapshot, b.snapshot, "idempotent");
    }

    #[test]
    fn rescan_triggers() {
        let (mut s, mut t) = (Scanner::default(), TpsTracker::default());
        assert!(s.needs_full_rescan(&branch(1)));
        s.scan(&branch(3), &mut t, 0);
        assert!(!s.needs_full_rescan(&branch(3)));
        assert!(s.needs_full_rescan(&branch(2)), "shrunk");
        let mut grown = branch(3);
        grown.push(json!({"type": "compaction", "tokensBefore": 9000, "summary": "x".repeat(400), "timestamp": iso(5, 0)}));
        assert!(s.needs_full_rescan(&grown));
        s.full_rescan(&grown, &mut t, i64::MAX);
        assert_eq!(s.snapshot.compaction_count, 1);
        assert!((s.snapshot.compaction_after - 100.0).abs() < 1e-9);
        assert!(s.snapshot.compaction_last_at.is_some());
    }

    #[test]
    fn inflight_tail_is_refed_until_stopped() {
        let (mut s, mut t) = (Scanner::default(), TpsTracker::default());
        let mut b = vec![user("go")];
        b.push(json!({"type": "message", "timestamp": iso(0, 1), "message": {"role": "assistant", "content": [{"type": "text", "text": "par"}], "usage": {"input": 5, "output": 1}}}));
        s.scan(&b, &mut t, 0);
        assert!((s.snapshot.output - 1.0).abs() < 1e-9);
        b[1] = assistant("partial done", 40, 1);
        s.scan(&b, &mut t, 0);
        assert!((s.snapshot.output - 40.0).abs() < 1e-9);
        assert!((s.snapshot.input - 100.0).abs() < 1e-9);
        assert_eq!(s.snapshot.assistant_count, 1);
    }

    #[test]
    fn tool_pairing_across_scans() {
        let (mut s, mut t) = (Scanner::default(), TpsTracker::default());
        let mut b = vec![user("go")];
        b.push(json!({"type": "message", "timestamp": iso(0, 1), "message": {"role": "assistant", "stopReason": "toolUse", "content": [{"type": "toolCall", "id": "c1", "name": "bash"}], "usage": {}}}));
        s.scan(&b, &mut t, 0);
        b.push(json!({"type": "message", "timestamp": iso(0, 11), "message": {"role": "toolResult", "toolCallId": "c1", "content": []}}));
        s.scan(&b, &mut t, 0);
        assert_eq!(t.tool_ms(), 10_000);
    }

    #[test]
    fn child_cost_not_tokens() {
        let child = |cost: f64| json!({"type": "custom", "customType": "unipi-child-usage", "data": {"v": 1, "input": 5000, "output": 200, "cost": cost}});
        let host = json!({"type": "usage", "kind": "child:explore", "provider": "p", "model": "m", "usage": {"input": 9, "output": 9, "cost": {"total": 1.0}}});
        let (mut s, mut t) = (Scanner::default(), TpsTracker::default());
        let mut b = vec![
            user("go"),
            assistant("dispatch", 10, 1),
            child(0.25),
            child(0.5),
        ];
        s.scan(&b, &mut t, 0);
        assert!((s.snapshot.cost - 0.76).abs() < 1e-9);
        assert!((s.snapshot.child_cost - 0.75).abs() < 1e-9);
        assert!((s.snapshot.input - 100.0).abs() < 1e-9);
        b.push(host);
        s.scan(&b, &mut t, 0);
        assert!((s.snapshot.cost - 1.76).abs() < 1e-9);
    }

    #[test]
    fn errors_and_aborted_are_skipped() {
        let (mut s, mut t) = (Scanner::default(), TpsTracker::default());
        let err = json!({"type": "message", "timestamp": iso(0, 2), "message": {"role": "assistant", "stopReason": "error", "content": [], "usage": {"input": 999}}});
        s.scan(&[user("u"), assistant("a", 1, 1), err], &mut t, 0);
        assert_eq!(s.snapshot.assistant_count, 1);
        assert!((s.snapshot.input - 100.0).abs() < 1e-9);
    }
}
