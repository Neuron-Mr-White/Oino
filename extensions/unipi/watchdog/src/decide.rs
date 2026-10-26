//! Decision logic (unipi `src/decide.ts`, pure).
//!
//! Background tasks and other tools: `act = status ∈ {stuck, looping} && confidence ≥ threshold
//! && !(persistent ≥ 0.5 && status ≠ looping) && streak ≥ agreeChecks`. Bash calls: a process
//! sample (idle twice in a row, no sleeper), a jev stop score streak, or a "should take
//! seconds" expectation trigger the action, unless the command declares a longer bound.

use unipi_kit::jev::Answers;

use crate::sample::ProcSample;

/// unipi `WatchdogDecision.status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Making progress.
    Progressing,
    /// Legitimately waiting.
    Waiting,
    /// Hung.
    Stuck,
    /// Repeating without progress.
    Looping,
    /// No (valid) answer.
    Unknown,
}

impl Status {
    /// jev choice label → status.
    #[must_use]
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("progressing") => Self::Progressing,
            Some("waiting") => Self::Waiting,
            Some("stuck") => Self::Stuck,
            Some("looping") => Self::Looping,
            _ => Self::Unknown,
        }
    }

    /// Label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Progressing => "progressing",
            Self::Waiting => "waiting",
            Self::Stuck => "stuck",
            Self::Looping => "looping",
            Self::Unknown => "unknown",
        }
    }

    fn signal(self) -> &'static str {
        match self {
            Self::Stuck => "judged stuck",
            Self::Looping => "repeating output without progress",
            other => other.as_str(),
        }
    }
}

/// unipi `WatchdogDecision`.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// All conditions met.
    pub act: bool,
    /// Status.
    pub status: Status,
    /// Status confidence.
    pub confidence: f64,
    /// Consecutive agreeing checks including this one.
    pub streak: u32,
    /// Signal text for the reason.
    pub signal: String,
    /// jev judged it long-lived.
    pub persistent: bool,
}

/// unipi `evaluateTick`: `None` answers keep the streak and never act.
#[must_use]
pub fn evaluate_tick(
    answers: Option<&Answers>,
    previous_streak: u32,
    threshold: f64,
    agree_checks: u32,
) -> Decision {
    let Some(answers) = answers else {
        return Decision {
            act: false,
            status: Status::Unknown,
            confidence: 0.0,
            streak: previous_streak,
            signal: String::new(),
            persistent: false,
        };
    };
    let status_answer = answers.get("status");
    let status = Status::parse(status_answer.and_then(|a| a.choice.as_deref()));
    let confidence = status_answer.and_then(|a| a.confidence).unwrap_or(0.0);
    let persistent = answers
        .get("persistent")
        .and_then(|a| a.noul)
        .is_some_and(|n| n >= 0.5);
    let veto = persistent && status != Status::Looping;
    let agrees =
        matches!(status, Status::Stuck | Status::Looping) && confidence >= threshold && !veto;
    let streak = if agrees { previous_streak + 1 } else { 0 };
    Decision {
        act: agrees && streak >= agree_checks,
        status,
        confidence,
        streak,
        signal: status.signal().to_owned(),
        persistent,
    }
}

/// unipi `expect` answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// ≤ 30 s.
    Seconds,
    /// A few minutes.
    Minutes,
    /// Many minutes to hours.
    Long,
    /// Never exits.
    Never,
    /// Not answered.
    Unknown,
}

impl Expect {
    /// jev choice → expectation.
    #[must_use]
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("seconds") => Self::Seconds,
            Some("minutes") => Self::Minutes,
            Some("long") => Self::Long,
            Some("never") => Self::Never,
            _ => Self::Unknown,
        }
    }
}

/// unipi `BashCheckInput`.
#[derive(Clone, Debug)]
pub struct BashCheck<'a> {
    /// Age (s).
    pub age_sec: f64,
    /// Since last output (s).
    pub since_output_sec: f64,
    /// Command.
    pub command: &'a str,
    /// Process sample.
    pub sample: Option<&'a ProcSample>,
    /// Previous check found it idle.
    pub prev_idle: bool,
    /// Previous cumulative I/O.
    pub prev_cumulative_io: Option<u64>,
    /// jev stop score.
    pub stop: Option<f64>,
    /// Stop streak so far.
    pub stop_streak: u32,
    /// Expected duration.
    pub expect: Expect,
    /// Threshold.
    pub threshold: f64,
    /// Agreeing checks.
    pub agree_checks: u32,
}

/// What triggered a bash action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// No CPU/IO/output twice in a row.
    Idle,
    /// jev stop score streak.
    Stop,
    /// Expected seconds, ran longer.
    Expect,
}

/// unipi `BashDecision`.
#[derive(Clone, Debug, PartialEq)]
pub struct BashDecision {
    /// Act.
    pub act: bool,
    /// Idle now.
    pub idle: bool,
    /// Stop streak.
    pub stop_streak: u32,
    /// A declared bound vetoed the action.
    pub vetoed: bool,
    /// Trigger.
    pub trigger: Option<Trigger>,
    /// Reason text.
    pub reason: String,
    /// Cumulative I/O of this sample.
    pub cumulative_io: Option<u64>,
}

fn scan_digits(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    (end > 0).then(|| (&s[..end], &s[end..]))
}

fn skip_ws1(s: &str) -> Option<&str> {
    let t = s.trim_start();
    (t.len() < s.len()).then_some(t)
}

fn unit_factor(rest: &str) -> (f64, &str) {
    match rest.chars().next() {
        Some('h') => (3600.0, &rest[1..]),
        Some('m') => (60.0, &rest[1..]),
        Some('s') => (1.0, &rest[1..]),
        _ => (1.0, rest),
    }
}

fn word_starts(command: &str, word: &str) -> Vec<usize> {
    command
        .match_indices(word)
        .filter(|(i, _)| {
            command[..*i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
        .map(|(i, _)| i + word.len())
        .collect()
}

/// unipi `declaredBoundSec`: `timeout N[smh]`, `sleep N[smh] &&|;`, `seq A B … sleep S`.
#[must_use]
pub fn declared_bound_sec(command: &str) -> Option<f64> {
    let mut bounds: Vec<f64> = Vec::new();
    for after in word_starts(command, "timeout") {
        if let Some((n, rest)) = skip_ws1(&command[after..]).and_then(scan_digits) {
            let (f, _) = unit_factor(rest);
            bounds.extend(n.parse::<f64>().ok().map(|n| n * f));
        }
    }
    for after in word_starts(command, "sleep") {
        if let Some((n, rest)) = skip_ws1(&command[after..]).and_then(scan_digits) {
            let (f, rest) = unit_factor(rest);
            let rest = rest.trim_start();
            if rest.starts_with("&&") || rest.starts_with(';') {
                bounds.extend(n.parse::<f64>().ok().map(|n| n * f));
            }
        }
    }
    for (i, _) in command.match_indices("seq") {
        let rest = &command[i + 3..];
        let Some((_, rest)) = skip_ws1(rest).and_then(scan_digits) else {
            continue;
        };
        let Some((count, rest)) = skip_ws1(rest).and_then(scan_digits) else {
            continue;
        };
        let line = rest.split('\n').next().unwrap_or_default();
        let Some(pos) = line.find("sleep") else {
            continue;
        };
        let Some(after) = skip_ws1(&line[pos + 5..]) else {
            continue;
        };
        let end = after
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(after.len());
        if let (Ok(c), Ok(s)) = (count.parse::<f64>(), after[..end].parse::<f64>()) {
            bounds.push(c * s);
        }
    }
    bounds.into_iter().reduce(f64::max)
}

fn duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{}s", js_round(seconds))
    } else {
        format!("{} min", js_round(seconds / 60.0))
    }
}

/// JS `Math.round`.
#[must_use]
#[allow(clippy::cast_possible_truncation, reason = "display seconds")]
pub fn js_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// unipi `decideBash`.
#[must_use]
pub fn decide_bash(input: &BashCheck<'_>) -> BashDecision {
    let idle = input.sample.is_some_and(|s| {
        let g = &s.group_totals;
        g.cpu_pct < 2.0
            && g.io_total() < 4096
            && input.since_output_sec >= 60.0
            && input
                .prev_cumulative_io
                .is_none_or(|p| s.cumulative_io.saturating_sub(p) < 4096)
    });
    let sleeper = input.sample.is_some_and(ProcSample::has_sleeper);
    let stop_streak = match input.stop {
        Some(s) if s >= input.threshold => input.stop_streak + 1,
        _ => 0,
    };
    let vetoed = declared_bound_sec(input.command).is_some_and(|b| input.age_sec < b);
    let trigger = if idle && input.prev_idle && !sleeper {
        Some(Trigger::Idle)
    } else if stop_streak >= input.agree_checks {
        Some(Trigger::Stop)
    } else if input.expect == Expect::Seconds {
        Some(Trigger::Expect)
    } else {
        None
    };
    let reason = match trigger {
        Some(Trigger::Idle) => format!(
            "no CPU, disk or output activity for {}",
            duration(input.since_output_sec)
        ),
        Some(Trigger::Stop) => format!(
            "jev judged it unlikely to finish on its own ({:.2}, {stop_streak} checks in a row)",
            input.stop.unwrap_or(0.0)
        ),
        Some(Trigger::Expect) => format!(
            "jev expects this command to finish within seconds, but it has run for {}",
            duration(input.age_sec)
        ),
        None => String::new(),
    };
    BashDecision {
        act: !vetoed && trigger.is_some(),
        idle,
        stop_streak,
        vetoed,
        trigger,
        reason,
        cumulative_io: input.sample.map(|s| s.cumulative_io),
    }
}

#[cfg(test)]
mod tests {
    use unipi_kit::jev::Answer;

    use super::*;
    use crate::sample::{GroupTotals, ProcRow};

    fn answers(status: &str, confidence: f64, persistent: f64) -> Answers {
        let mut a = Answers::new();
        a.insert(
            "status".into(),
            Answer {
                choice: Some(status.into()),
                confidence: Some(confidence),
                ..Answer::default()
            },
        );
        a.insert(
            "persistent".into(),
            Answer {
                noul: Some(persistent),
                ..Answer::default()
            },
        );
        a
    }

    #[test]
    fn evaluate_tick_table() {
        let d = evaluate_tick(None, 1, 0.8, 2);
        assert!(!d.act);
        assert_eq!((d.streak, d.status), (1, Status::Unknown));
        let stuck = answers("stuck", 0.9, 0.1);
        let first = evaluate_tick(Some(&stuck), 0, 0.8, 2);
        assert_eq!((first.streak, first.act), (1, false));
        let second = evaluate_tick(Some(&stuck), first.streak, 0.8, 2);
        assert_eq!((second.streak, second.act), (2, true));
        assert!(evaluate_tick(Some(&stuck), 0, 0.8, 1).act);
        let low = answers("stuck", 0.5, 0.1);
        assert_eq!(evaluate_tick(Some(&low), 1, 0.8, 2).streak, 0);
        let ps = answers("stuck", 0.99, 0.9);
        let d = evaluate_tick(Some(&ps), 3, 0.8, 1);
        assert!(!d.act && d.persistent && d.streak == 0);
        let pl = answers("looping", 0.95, 0.9);
        assert!(!evaluate_tick(Some(&pl), 0, 0.8, 2).act);
        assert!(evaluate_tick(Some(&pl), 1, 0.8, 2).act);
        let waiting = answers("waiting", 0.9, 0.2);
        assert_eq!(evaluate_tick(Some(&waiting), 2, 0.8, 2).streak, 0);
        let looping = answers("looping", 0.85, 0.0);
        let d = evaluate_tick(Some(&looping), 1, 0.8, 2);
        assert!(d.act);
        assert_eq!(d.signal, "repeating output without progress");
        let broken = answers("something-else", 1.0, 0.0);
        let d = evaluate_tick(Some(&broken), 1, 0.8, 2);
        assert_eq!((d.act, d.status, d.streak), (false, Status::Unknown, 0));
        let d = evaluate_tick(None, 2, 0.8, 1);
        assert_eq!((d.streak, d.act), (2, false));
    }

    #[test]
    fn declared_bounds() {
        for (command, bound) in [
            ("timeout 600 ./x", Some(600.0)),
            ("timeout 5m x", Some(300.0)),
            ("sleep 330 && k", Some(330.0)),
            ("sleep 3", None),
            ("for i in $(seq 1 150); do x; sleep 3; done", Some(450.0)),
            ("x", None),
            ("mytimeout 5 x", None),
        ] {
            assert_eq!(declared_bound_sec(command), bound, "{command}");
        }
    }

    fn sample(cumulative: u64, sleeper: bool) -> ProcSample {
        ProcSample {
            processes: if sleeper {
                vec![ProcRow {
                    comm: "sleep".into(),
                    ..ProcRow::default()
                }]
            } else {
                Vec::new()
            },
            group_totals: GroupTotals::default(),
            cumulative_io: cumulative,
        }
    }

    #[test]
    fn bash_trigger_and_veto_table() {
        let s = sample(100, false);
        let base = BashCheck {
            age_sec: 300.0,
            since_output_sec: 180.0,
            command: "work",
            sample: Some(&s),
            prev_idle: false,
            prev_cumulative_io: None,
            stop: Some(0.1),
            stop_streak: 0,
            expect: Expect::Minutes,
            threshold: 0.5,
            agree_checks: 2,
        };
        assert!(!decide_bash(&base).act);
        let idle = BashCheck {
            prev_idle: true,
            ..base.clone()
        };
        assert_eq!(decide_bash(&idle).trigger, Some(Trigger::Idle));
        assert_eq!(
            decide_bash(&idle).reason,
            "no CPU, disk or output activity for 3 min"
        );
        let sl = sample(100, true);
        assert!(
            !decide_bash(&BashCheck {
                sample: Some(&sl),
                ..idle.clone()
            })
            .act
        );
        let busy = sample(5000, false);
        assert!(
            !decide_bash(&BashCheck {
                sample: Some(&busy),
                prev_cumulative_io: Some(0),
                ..idle.clone()
            })
            .idle
        );
        let first = decide_bash(&BashCheck {
            stop: Some(0.6),
            ..base.clone()
        });
        assert!(!first.act);
        let second = decide_bash(&BashCheck {
            stop: Some(0.55),
            stop_streak: first.stop_streak,
            ..base.clone()
        });
        assert_eq!(second.trigger, Some(Trigger::Stop));
        assert_eq!(
            decide_bash(&BashCheck {
                stop: Some(0.4),
                stop_streak: 1,
                ..base.clone()
            })
            .stop_streak,
            0
        );
        assert_eq!(
            decide_bash(&BashCheck {
                expect: Expect::Seconds,
                ..base.clone()
            })
            .trigger,
            Some(Trigger::Expect)
        );
        for o in [
            BashCheck {
                prev_idle: true,
                command: "timeout 600 ./x",
                ..base.clone()
            },
            BashCheck {
                stop: Some(0.9),
                stop_streak: 1,
                command: "timeout 600 ./x",
                ..base.clone()
            },
            BashCheck {
                expect: Expect::Seconds,
                command: "timeout 600 ./x",
                ..base.clone()
            },
        ] {
            assert!(!decide_bash(&o).act);
        }
        assert!(
            !decide_bash(&BashCheck {
                sample: None,
                prev_idle: true,
                ..base
            })
            .idle
        );
    }
}
