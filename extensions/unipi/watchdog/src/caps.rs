//! Opt-in session caps (D22: the removed Oino `budget` extension, unipi-port §3.10).
//!
//! Counted per run (reset at `agent_start`, except for the summary turn this module asks for):
//! turns, USD cost (assistant `usage.cost.total`), wall time, consecutive failed tool calls.
//! At `turn_end` the first reached cap aborts the run; with `summaryTurn` one follow-up user
//! message asks for a wrap-up, and that summary run is aborted at its own first `turn_end`
//! if it keeps working.

use serde_json::Value;

use crate::settings::NAMESPACE;

/// Cap limits (`None` = off).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Caps {
    /// Turns per run.
    pub max_turns: Option<u64>,
    /// USD per run.
    pub max_cost_usd: Option<f64>,
    /// Minutes per run.
    pub max_wall_minutes: Option<f64>,
    /// Consecutive tool errors.
    pub max_consecutive_tool_errors: Option<u64>,
    /// Ask for a summary turn.
    pub summary_turn: bool,
}

fn positive(v: Option<&Value>) -> Option<f64> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (n.is_finite() && n > 0.0).then_some(n)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "positive counts"
)]
fn count(v: Option<f64>) -> Option<u64> {
    v.map(|n| n.ceil() as u64)
}

impl Caps {
    /// From `watchdog.sessionCaps` settings, overridden by CLI flags
    /// (`--max-turns`, `--max-cost`, `--max-wall` minutes, `--max-tool-errors`).
    #[must_use]
    pub fn resolve(settings: &Value, flag: impl Fn(&str) -> Option<Value>) -> Self {
        let caps = settings.get("sessionCaps").unwrap_or(&Value::Null);
        let pick = |key: &str, flag_name: &str| {
            positive(flag(flag_name).as_ref()).or_else(|| positive(caps.get(key)))
        };
        Self {
            max_turns: count(pick("maxTurns", "max-turns")),
            max_cost_usd: pick("maxCostUsd", "max-cost"),
            max_wall_minutes: pick("maxWallMinutes", "max-wall"),
            max_consecutive_tool_errors: count(pick("maxConsecutiveToolErrors", "max-tool-errors")),
            summary_turn: caps
                .get("summaryTurn")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    /// Every cap is off.
    #[must_use]
    pub fn inert(&self) -> bool {
        self.max_turns.is_none()
            && self.max_cost_usd.is_none()
            && self.max_wall_minutes.is_none()
            && self.max_consecutive_tool_errors.is_none()
    }
}

/// Run counters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunState {
    /// Turns.
    pub turns: u64,
    /// USD.
    pub cost_usd: f64,
    /// Consecutive failed tool results.
    pub consecutive_tool_errors: u64,
    /// Run start (epoch ms).
    pub started_at: i64,
    /// The summary message was sent; the next run is the summary turn.
    pub summary_pending: bool,
    /// This run is the summary turn.
    pub in_summary: bool,
}

/// What to do at `turn_end`.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Keep going.
    Continue,
    /// Abort (cap reached), with the reason; `summary` = also send the summary message.
    Abort {
        /// `12 turns`, `$1.2000 spent`, …
        reason: String,
        /// Send the summary user message.
        summary: bool,
    },
}

/// The summary request text.
#[must_use]
pub fn summary_message(reason: &str) -> String {
    format!(
        "Session cap reached ({reason}). Stop working now and reply with a concise summary of what is done, what remains, and next steps. Do not call tools."
    )
}

/// Notification text.
#[must_use]
pub fn notice(reason: &str) -> String {
    format!("{NAMESPACE}: session cap reached — {reason}")
}

impl RunState {
    /// `agent_start`: a new run (the summary turn keeps its counters).
    pub fn start(&mut self, now: i64) {
        if self.summary_pending {
            self.summary_pending = false;
            self.in_summary = true;
            return;
        }
        *self = Self {
            started_at: now,
            ..Self::default()
        };
    }

    /// Folds one `turn_end` payload (`message.usage.cost.total`, `toolResults[].isError`) and
    /// decides.
    pub fn turn_end(&mut self, caps: &Caps, payload: &Value, now: i64) -> Verdict {
        if caps.inert() {
            return Verdict::Continue;
        }
        self.turns += 1;
        self.cost_usd += payload
            .pointer("/message/usage/cost/total")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        for r in payload
            .get("toolResults")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if r.get("isError").and_then(Value::as_bool).unwrap_or(false) {
                self.consecutive_tool_errors += 1;
            } else {
                self.consecutive_tool_errors = 0;
            }
        }
        if self.in_summary {
            let working = payload
                .get("toolResults")
                .and_then(Value::as_array)
                .is_some_and(|r| !r.is_empty());
            return if working {
                Verdict::Abort {
                    reason: "summary turn kept working".into(),
                    summary: false,
                }
            } else {
                Verdict::Continue
            };
        }
        #[allow(clippy::cast_precision_loss, reason = "minutes")]
        let minutes = (now - self.started_at).max(0) as f64 / 60_000.0;
        let reason = if caps
            .max_consecutive_tool_errors
            .is_some_and(|m| self.consecutive_tool_errors >= m)
        {
            Some(format!(
                "{} consecutive tool errors",
                self.consecutive_tool_errors
            ))
        } else if caps.max_turns.is_some_and(|m| self.turns >= m) {
            Some(format!("{} turns", self.turns))
        } else if caps.max_cost_usd.is_some_and(|m| self.cost_usd >= m) {
            Some(format!("${:.4} spent", self.cost_usd))
        } else {
            caps.max_wall_minutes
                .filter(|m| minutes >= *m)
                .map(|m| format!("{m} min wall time"))
        };
        match reason {
            None => Verdict::Continue,
            Some(reason) => {
                let summary = caps.summary_turn;
                self.summary_pending = summary;
                Verdict::Abort { reason, summary }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn turn(cost: f64, errors: &[bool]) -> Value {
        json!({
            "message": {"usage": {"cost": {"total": cost}}},
            "toolResults": errors.iter().map(|e| json!({"isError": e})).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn resolves_settings_and_flags() {
        let s = json!({"sessionCaps": {"maxTurns": 0, "maxCostUsd": 2.5, "summaryTurn": false}});
        let caps = Caps::resolve(&s, |n| (n == "max-turns").then(|| json!("3")));
        assert_eq!(caps.max_turns, Some(3));
        assert_eq!(caps.max_cost_usd, Some(2.5));
        assert_eq!(caps.max_wall_minutes, None);
        assert!(!caps.summary_turn);
        assert!(Caps::resolve(&json!({}), |_| None).inert());
    }

    #[test]
    fn caps_trigger_and_summary_turn() {
        let caps = Caps {
            max_turns: Some(2),
            summary_turn: true,
            ..Caps::default()
        };
        let mut r = RunState::default();
        r.start(0);
        assert_eq!(r.turn_end(&caps, &turn(0.0, &[]), 0), Verdict::Continue);
        assert_eq!(
            r.turn_end(&caps, &turn(0.0, &[]), 0),
            Verdict::Abort {
                reason: "2 turns".into(),
                summary: true
            }
        );
        r.start(10);
        assert!(r.in_summary);
        assert_eq!(r.turn_end(&caps, &turn(0.0, &[]), 0), Verdict::Continue);
        r.start(20);
        assert!(!r.in_summary);
        assert_eq!(r.turns, 0);

        let errs = Caps {
            max_consecutive_tool_errors: Some(2),
            ..Caps::default()
        };
        let mut r = RunState::default();
        assert_eq!(
            r.turn_end(&errs, &turn(0.0, &[true, false, true]), 0),
            Verdict::Continue
        );
        assert!(
            matches!(r.turn_end(&errs, &turn(0.0, &[true]), 0), Verdict::Abort { ref reason, summary: false } if reason == "2 consecutive tool errors")
        );

        let cost = Caps {
            max_cost_usd: Some(1.0),
            ..Caps::default()
        };
        let mut r = RunState::default();
        assert!(
            matches!(r.turn_end(&cost, &turn(1.5, &[]), 0), Verdict::Abort { ref reason, .. } if reason == "$1.5000 spent")
        );
        let wall = Caps {
            max_wall_minutes: Some(1.0),
            ..Caps::default()
        };
        let mut r = RunState::default();
        r.start(0);
        assert!(matches!(
            r.turn_end(&wall, &turn(0.0, &[]), 61_000),
            Verdict::Abort { .. }
        ));
        assert!(summary_message("2 turns").starts_with("Session cap reached (2 turns)."));
    }
}
