//! Tokens-per-second tracker (unipi `src/tps-tracker.ts`), with the clock passed in.
//!
//! Token sources, best first: the provider's `usage.output` at stream end (anchored), else a
//! chars/4 density estimate over streamed text/thinking/tool arguments. Durations measure only
//! output generation (first non-empty delta → stream end); messages never measured by live
//! hooks are excluded from the session average. TTFT = turn start → first delta, sampled only
//! when both bounds exist. Turns/steps/wall/tool time follow the deepseek-harness stats.

use std::collections::HashMap;

use serde_json::Value;

const CHARS_PER_TOKEN: usize = 4;

fn estimate_tokens(text: &str) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "token estimates")]
    let n = text.encode_utf16().count().div_ceil(CHARS_PER_TOKEN) as f64;
    n
}

fn message_timestamp(m: &Value) -> i64 {
    m.get("timestamp").and_then(Value::as_i64).unwrap_or(0)
}

fn provider_output(m: &Value) -> f64 {
    m.pointer("/usage/output")
        .and_then(Value::as_f64)
        .filter(|o| *o > 0.0)
        .unwrap_or(0.0)
}

/// chars/4 over text, thinking and tool-call arguments of an assistant message.
#[must_use]
pub fn estimate_output_tokens(m: &Value) -> f64 {
    if m.get("role").and_then(Value::as_str) != Some("assistant") {
        return 0.0;
    }
    let mut chars = 0usize;
    for b in m
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let len = |k: &str| {
            b.get(k)
                .and_then(Value::as_str)
                .map_or(0, |s| s.encode_utf16().count())
        };
        match b.get("type").and_then(Value::as_str) {
            Some("text") => chars += len("text"),
            Some("thinking") => chars += len("thinking"),
            Some("tool_use" | "toolcall" | "toolCall") => {
                let input = b.get("input").or_else(|| b.get("arguments"));
                chars += match input {
                    Some(Value::String(s)) => s.encode_utf16().count(),
                    Some(v) => v.to_string().encode_utf16().count(),
                    None => 2,
                };
            }
            _ => {}
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "token estimates")]
    let n = chars.div_ceil(CHARS_PER_TOKEN) as f64;
    n
}

#[derive(Clone, Debug, Default)]
struct Record {
    tokens: f64,
    estimated: f64,
    request_at: i64,
    started_at: i64,
    saw_first_delta: bool,
    completed_at: i64,
    decode_ms: i64,
    tps: f64,
    ttft_seeded: bool,
}

/// The tracker (one per session).
#[derive(Clone, Debug, Default)]
pub struct TpsTracker {
    records: Vec<Record>,
    total_output: f64,
    ttft_ms: i64,
    ttft_steps: u32,
    ttft_hook_samples: u32,
    turns: u64,
    steps: u64,
    turn_start_at: i64,
    llm_ms: i64,
    pending_tools: HashMap<String, i64>,
    tool_ms: i64,
    last_turn_start: i64,
}

impl TpsTracker {
    fn blank(&self) -> Record {
        Record {
            request_at: self.last_turn_start,
            ..Record::default()
        }
    }

    fn pad_to(&mut self, index: usize) {
        while self.records.len() < index {
            let r = self.blank();
            self.records.push(r);
        }
    }

    /// An assistant message began streaming (record created; clock not started).
    pub fn on_message_start(&mut self, index: usize) {
        if self.records.get(index).is_some() {
            return;
        }
        self.pad_to(index);
        let r = self.blank();
        self.records.push(r);
    }

    /// pi `turn_start`: stamps the TTFT bound and opens a turn.
    pub fn on_turn_start(&mut self, timestamp: Option<i64>, now: i64) {
        let ts = timestamp.filter(|t| *t > 0).unwrap_or(now);
        self.last_turn_start = ts;
        if self.turn_start_at == 0 {
            self.turns += 1;
            self.turn_start_at = ts;
        }
        if let Some(p) = self.records.iter_mut().find(|r| r.completed_at == 0)
            && p.request_at == 0
        {
            p.request_at = ts;
        }
    }

    /// One streamed delta; the clock starts at the first non-empty one.
    pub fn on_streaming_delta(&mut self, index: usize, delta: &str, now: i64) {
        let Some(r) = self.records.get_mut(index) else {
            return;
        };
        if r.completed_at != 0 || delta.is_empty() {
            return;
        }
        if r.started_at == 0 {
            r.started_at = now;
            if r.request_at > 0 && r.request_at <= r.started_at {
                self.ttft_ms += (r.started_at - r.request_at).max(0);
                self.ttft_steps += 1;
                self.ttft_hook_samples += 1;
            }
        }
        r.saw_first_delta = true;
        r.estimated += estimate_tokens(delta);
        r.tokens = r.estimated;
    }

    /// Stream finished; anchors to `usage.output`.
    pub fn on_message_end(&mut self, index: usize, message: Option<&Value>, now: i64) {
        if self.records.get(index).is_none() {
            self.on_message_start(index);
        }
        let Some(r) = self.records.get_mut(index) else {
            return;
        };
        if r.completed_at != 0 {
            return;
        }
        r.completed_at = now;
        let anchored = message.map_or(0.0, provider_output);
        if anchored > 0.0 {
            r.tokens = anchored;
        } else if let Some(m) = message.filter(|_| r.started_at == 0) {
            r.estimated = r.estimated.max(estimate_output_tokens(m));
            r.tokens = r.estimated;
        }
        if r.started_at == 0 {
            let ts = message.map_or(0, message_timestamp);
            r.started_at = if ts != 0 { ts } else { r.completed_at - 500 };
        }
        #[allow(clippy::cast_precision_loss, reason = "durations")]
        let secs = ((r.completed_at - r.started_at) as f64 / 1000.0).max(0.05);
        if r.saw_first_delta && r.started_at > 0 {
            r.decode_ms = (r.completed_at - r.started_at).clamp(1, 3_600_000);
        }
        r.tps = if r.tokens > 0.0 { r.tokens / secs } else { 0.0 };
        self.total_output += r.tokens;
        self.steps += 1;
    }

    /// Tool execution started.
    pub fn on_tool_call_start(&mut self, id: &str, now: i64) {
        if !id.is_empty() {
            self.pending_tools.entry(id.to_owned()).or_insert(now);
        }
    }

    /// Tool execution ended (unmatched ends ignored).
    pub fn on_tool_call_end(&mut self, id: &str, now: i64) {
        if let Some(start) = self.pending_tools.remove(id) {
            self.tool_ms += (now - start).max(0);
        }
    }

    /// Branch-derived tool time (monotonic).
    pub fn sync_tool_ms(&mut self, ms: i64) {
        self.tool_ms = self.tool_ms.max(ms);
    }

    /// Closes the open turn (agent settled).
    pub fn on_turn_end(&mut self, now: i64) {
        if self.turn_start_at > 0 {
            self.llm_ms += (now - self.turn_start_at).max(0);
            self.turn_start_at = 0;
        }
    }

    /// Branch-derived turns/steps (monotonic).
    pub fn sync_branch_stats(&mut self, users: u64, assistants: u64) {
        self.turns = self.turns.max(users);
        self.steps = self.steps.max(assistants);
    }

    /// Branch-derived wall time (monotonic).
    pub fn sync_wall_ms(&mut self, ms: i64) {
        self.llm_ms = self.llm_ms.max(ms);
    }

    /// TTFT fallback from persisted timestamps when hooks never produced samples.
    pub fn seed_ttft_fallback(&mut self, previous_ts: i64, ts: i64, index: usize) {
        if self.ttft_hook_samples > 0 {
            return;
        }
        let Some(r) = self.records.get_mut(index) else {
            return;
        };
        if r.completed_at == 0 || r.request_at > 0 || r.ttft_seeded {
            return;
        }
        if previous_ts <= 0 || ts <= previous_ts {
            return;
        }
        r.ttft_seeded = true;
        self.ttft_ms += (ts - previous_ts).min(30_000);
        self.ttft_steps += 1;
    }

    /// Snapshot update from the scan (density refinement; completes on stop reason).
    pub fn on_message_update(&mut self, index: usize, message: &Value, has_stop: bool, now: i64) {
        match self.records.get_mut(index) {
            None => {
                self.pad_to(index);
                self.on_message_start(index);
                if has_stop {
                    self.on_message_end(index, Some(message), now);
                }
            }
            Some(r) if r.completed_at != 0 => {}
            Some(r) => {
                let est = estimate_output_tokens(message);
                if est > r.estimated {
                    r.estimated = est;
                    r.tokens = est;
                }
                if has_stop {
                    self.on_message_end(index, Some(message), now);
                }
            }
        }
    }

    /// Live TPS of the generating message, else the last completed one.
    #[must_use]
    pub fn live_tps(&self, now: i64) -> f64 {
        if let Some(r) = self
            .records
            .iter()
            .rev()
            .find(|r| r.completed_at == 0 && r.started_at > 0)
        {
            #[allow(clippy::cast_precision_loss, reason = "durations")]
            let secs = (now - r.started_at) as f64 / 1000.0;
            return if secs <= 0.0 { 0.0 } else { r.tokens / secs };
        }
        self.records.last().map_or(0.0, |r| r.tps)
    }

    /// Σ tokens ÷ Σ measured decode windows (+ the open window).
    #[must_use]
    pub fn session_avg_tps(&self, now: i64) -> f64 {
        let (mut tokens, mut secs) = (0.0, 0.0);
        for r in &self.records {
            #[allow(clippy::cast_precision_loss, reason = "durations")]
            if r.completed_at > 0 && r.started_at > 0 && r.decode_ms > 0 {
                tokens += r.tokens;
                secs += r.decode_ms as f64 / 1000.0;
            } else if r.completed_at == 0 && r.started_at > 0 {
                tokens += r.tokens;
                secs += ((now - r.started_at) as f64 / 1000.0).max(0.05);
            }
        }
        if secs <= 0.0 { 0.0 } else { tokens / secs }
    }

    /// A message is generating.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        for r in self.records.iter().rev() {
            if r.completed_at == 0 && r.started_at > 0 {
                return true;
            }
            if r.completed_at > 0 {
                return false;
            }
        }
        false
    }

    /// Output tokens (completed + generating).
    #[must_use]
    pub fn total_output(&self) -> f64 {
        self.total_output
            + self
                .records
                .iter()
                .filter(|r| r.completed_at == 0 && r.started_at > 0)
                .map(|r| r.tokens)
                .sum::<f64>()
    }

    /// Average TTFT (ms) or `None` without samples.
    #[must_use]
    pub fn avg_ttft_ms(&self) -> Option<i64> {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            reason = "ms"
        )]
        (self.ttft_steps > 0)
            .then(|| (self.ttft_ms as f64 / f64::from(self.ttft_steps)).round() as i64)
    }

    /// TTFT samples.
    #[must_use]
    pub fn ttft_samples(&self) -> u32 {
        self.ttft_steps
    }

    /// Turns.
    #[must_use]
    pub fn turns(&self) -> u64 {
        self.turns
    }

    /// Steps.
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Agent wall time (completed + open turn).
    #[must_use]
    pub fn llm_ms(&self, now: i64) -> i64 {
        self.llm_ms
            + if self.turn_start_at > 0 {
                (now - self.turn_start_at).max(0)
            } else {
                0
            }
    }

    /// Tool wall time.
    #[must_use]
    pub fn tool_ms(&self) -> i64 {
        self.tool_ms
    }

    /// Forget everything.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[cfg(test)]
    fn backdate_start(&mut self, index: usize, ms: i64) {
        if let Some(r) = self.records.get_mut(index) {
            r.started_at -= ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn done(text: &str, out: u64, ts: i64) -> Value {
        json!({"role": "assistant", "content": [{"type": "text", "text": text}], "stopReason": "stop", "timestamp": ts, "usage": {"output": out}})
    }

    #[test]
    fn anchors_and_density() {
        let mut t = TpsTracker::default();
        t.on_message_start(0);
        t.on_streaming_delta(0, "hello world from the model", 10);
        t.on_message_end(0, Some(&done("hello world from the model", 4, 10)), 20);
        assert!((t.total_output() - 4.0).abs() < f64::EPSILON);
        let mut t = TpsTracker::default();
        t.on_message_start(0);
        t.on_streaming_delta(0, "abcdefgh", 10);
        t.on_message_end(0, Some(&done("abcdefgh", 0, 10)), 20);
        assert!((t.total_output() - 2.0).abs() < f64::EPSILON);
        let mut t = TpsTracker::default();
        t.on_message_start(0);
        t.on_streaming_delta(0, &"你好世界".repeat(10), 10);
        assert!((t.total_output() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn scan_snapshots_never_shrink_and_completed_is_immutable() {
        let mut t = TpsTracker::default();
        t.on_message_start(0);
        for _ in 0..100 {
            t.on_streaming_delta(0, "abcd", 10);
        }
        t.on_message_update(
            0,
            &json!({"role": "assistant", "content": [{"type": "text", "text": "ab"}]}),
            false,
            20,
        );
        assert!((t.total_output() - 100.0).abs() < f64::EPSILON);
        t.on_message_end(0, Some(&done("x", 50, 10)), 30);
        t.on_message_update(0, &done("x", 9999, 10), true, 40);
        assert!((t.total_output() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn clock_starts_at_first_delta_and_ttft() {
        let mut t = TpsTracker::default();
        t.on_turn_start(Some(1_000), 1_000);
        t.on_message_start(0);
        t.on_streaming_delta(0, "", 1_200);
        t.on_streaming_delta(0, "word", 1_500);
        assert_eq!(t.avg_ttft_ms(), Some(500));
        t.on_message_end(0, Some(&done("word", 100, 1_000)), 2_500);
        assert!((t.session_avg_tps(9_999) - 100.0).abs() < 1.0);
        // Missing turn start drops the sample.
        let mut t = TpsTracker::default();
        t.on_message_start(0);
        t.on_streaming_delta(0, "x", 100);
        assert_eq!(t.avg_ttft_ms(), None);
    }

    #[test]
    fn scan_only_messages_are_excluded_from_avg() {
        let mut t = TpsTracker::default();
        t.on_message_update(0, &done("aaaa", 500, 1_000), true, 60_000);
        assert!(t.session_avg_tps(60_000).abs() < f64::EPSILON);
        assert_eq!(t.steps(), 1);
        t.on_message_start(1);
        t.on_streaming_delta(1, "hi", 100_000);
        t.backdate_start(1, 10_000);
        t.on_message_end(1, Some(&done("final", 1000, 0)), 100_000);
        assert!((t.session_avg_tps(100_000) - 100.0).abs() < 1.0);
    }

    #[test]
    fn tool_time_between_messages_is_excluded() {
        let mut t = TpsTracker::default();
        t.on_turn_start(None, 500);
        t.on_message_start(0);
        t.on_streaming_delta(0, &"x".repeat(40), 3_000);
        t.backdate_start(0, 2_000);
        t.on_message_end(0, Some(&done("", 200, 0)), 3_000);
        t.on_tool_call_start("call-1", 2_000);
        t.on_tool_call_end("call-1", 32_000);
        assert!((t.session_avg_tps(32_000) - 100.0).abs() <= 5.0);
        assert_eq!(t.tool_ms(), 30_000);
        t.on_tool_call_end("never-started", 40_000);
        assert_eq!(t.tool_ms(), 30_000);
    }

    #[test]
    fn live_tps_then_last_completed() {
        let mut t = TpsTracker::default();
        t.on_message_start(0);
        t.on_streaming_delta(0, &"x".repeat(400), 1_000);
        assert!((t.live_tps(2_000) - 100.0).abs() < f64::EPSILON);
        assert!(t.is_streaming());
        t.on_message_end(0, Some(&done("", 100, 0)), 2_000);
        assert!((t.live_tps(50_000) - 100.0).abs() < f64::EPSILON);
        assert!(!t.is_streaming());
    }

    #[test]
    fn turns_steps_wall_and_sync() {
        let mut t = TpsTracker::default();
        t.on_turn_start(Some(1_000), 1_000);
        t.on_turn_start(Some(1_500), 1_500);
        assert_eq!(t.turns(), 1, "turn_start is idempotent within an open turn");
        t.on_message_start(0);
        t.on_streaming_delta(0, "a", 1_600);
        t.on_message_end(0, Some(&done("a", 1, 0)), 1_700);
        t.on_turn_end(3_000);
        assert_eq!(t.llm_ms(9_000), 2_000);
        t.on_turn_start(Some(10_000), 10_000);
        assert_eq!(t.turns(), 2);
        assert_eq!(t.llm_ms(11_000), 3_000);
        t.sync_branch_stats(5, 4);
        assert_eq!((t.turns(), t.steps()), (5, 4));
        t.sync_branch_stats(1, 1);
        assert_eq!((t.turns(), t.steps()), (5, 4));
        t.sync_wall_ms(1);
        t.on_turn_end(10_000);
        assert_eq!(t.llm_ms(10_000), 2_000);
        t.sync_wall_ms(50_000);
        assert_eq!(t.llm_ms(10_000), 50_000);
        t.sync_tool_ms(7);
        t.sync_tool_ms(3);
        assert_eq!(t.tool_ms(), 7);
    }

    #[test]
    fn ttft_seed_fallback_then_hooks_win() {
        let mut t = TpsTracker::default();
        t.on_message_update(0, &done("a", 1, 0), true, 100);
        t.seed_ttft_fallback(1_000, 4_000, 0);
        t.seed_ttft_fallback(1_000, 4_000, 0);
        assert_eq!((t.avg_ttft_ms(), t.ttft_samples()), (Some(3_000), 1));
        t.on_message_update(1, &done("a", 1, 0), true, 100);
        t.seed_ttft_fallback(1_000, 100_000, 1);
        assert_eq!(t.ttft_samples(), 2, "clamped to 30 s");
        t.reset();
        assert_eq!(t.avg_ttft_ms(), None);
        assert!(t.total_output().abs() < f64::EPSILON);
    }

    #[test]
    fn density_divides_char_count() {
        let m =
            json!({"role": "assistant", "content": [{"type": "text", "text": "x".repeat(8000)}]});
        assert!((estimate_output_tokens(&m) - 2000.0).abs() < f64::EPSILON);
        let tool = json!({"role": "assistant", "content": [{"type": "toolCall", "arguments": {"a": "bcd"}}]});
        assert!((estimate_output_tokens(&tool) - 3.0).abs() < f64::EPSILON);
    }
}
