//! `compactor` settings (unipi `config/{schema,manager}.ts`) and the percentage trigger
//! (unipi `auto-trigger.ts`).

use serde_json::{Value, json};
use unipi_kit::settings::{Choice, Field, Kind, Namespace, Section, Unit, Values, choice};

use crate::summary::Sections;

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.compactor";
/// Namespace.
pub const NAMESPACE: &str = "compactor";
/// Cooldown between percentage triggers.
pub const COOLDOWN_MS: i64 = 60_000;
/// Growth required before repeating above the threshold.
pub const REPEAT_MIN_GROWTH_TOKENS: f64 = 4_000.0;

/// Section ids in canonical order.
pub const SECTION_IDS: [&str; 9] = [
    "activeWork",
    "requests",
    "state",
    "decisions",
    "files",
    "commits",
    "errors",
    "lessons",
    "transcript",
];

/// `vcc | llm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Lossless.
    Vcc,
    /// Model summary.
    Llm,
}

impl Method {
    fn parse(v: Option<&str>) -> Option<Self> {
        match v {
            Some("vcc") => Some(Self::Vcc),
            Some("llm") => Some(Self::Llm),
            _ => None,
        }
    }
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vcc => "vcc",
            Self::Llm => "llm",
        }
    }
    /// Display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Vcc => "lossless",
            Self::Llm => "model summary",
        }
    }
}

/// Resolved config.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi CompactorConfig"
)]
pub struct Config {
    /// Method.
    pub method: Method,
    /// What `/compact` does (`None` = follow).
    pub pi_compact: Option<Method>,
    /// Percent trigger on.
    pub percent: bool,
    /// Threshold.
    pub threshold: f64,
    /// Notices.
    pub notify: bool,
    /// Smart keep tail.
    pub smart_keep_tail: bool,
    /// Summary budget (0 = auto).
    pub summary_budget_tokens: u64,
    /// Sections.
    pub sections: Sections,
    /// Extra llm instructions.
    pub llm_instructions: String,
    /// Debug dump.
    pub debug: bool,
}

/// Defaults.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "method": "vcc", "piCompact": "follow", "trigger": "pi", "thresholdPercent": 80, "notify": true,
        "smartKeepTail": true, "summaryBudgetTokens": 0, "summarySections": SECTION_IDS,
        "llmInstructions": "", "debug": false,
    })
}

/// Loads from the namespace values.
#[must_use]
pub fn load(v: &Values) -> Config {
    let raw = &v.0;
    let s = |k: &str| raw.get(k).and_then(Value::as_str);
    let method = Method::parse(s("method")).unwrap_or(Method::Vcc);
    let percent = s("trigger") == Some("percent");
    let num = |k: &str| raw.get(k).and_then(Value::as_f64);
    let list = raw.get("summarySections").and_then(Value::as_array);
    let on = |id: &str| list.is_none_or(|l| l.iter().any(|x| x.as_str() == Some(id)));
    let sections = Sections {
        active_work: on("activeWork"),
        requests: on("requests"),
        state: on("state"),
        decisions: on("decisions"),
        files: on("files"),
        commits: on("commits"),
        errors: on("errors"),
        lessons: on("lessons"),
        transcript: on("transcript"),
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=20000"
    )]
    let budget = num("summaryBudgetTokens")
        .unwrap_or(0.0)
        .clamp(0.0, 20_000.0) as u64;
    Config {
        method,
        pi_compact: match s("piCompact") {
            Some("follow") | None => None,
            other => Method::parse(other),
        },
        percent,
        threshold: num("thresholdPercent").unwrap_or(80.0),
        notify: v.bool("notify", true),
        smart_keep_tail: v.bool("smartKeepTail", true),
        summary_budget_tokens: budget,
        sections,
        llm_instructions: v.str("llmInstructions", "").to_owned(),
        debug: v.bool("debug", false),
    }
}

fn method_options() -> Vec<Choice> {
    vec![
        choice(
            "vcc",
            "lossless",
            "instant structured summary; history stays searchable",
        ),
        choice(
            "llm",
            "model summary",
            "the model writes the summary — costs one model call",
        ),
    ]
}

/// The hub schema.
#[must_use]
pub fn namespace() -> Namespace {
    let mut pi_opts = vec![choice(
        "follow",
        "same as Method",
        "reuse whatever Method is set to",
    )];
    pi_opts.extend(method_options());
    Namespace::new(NAMESPACE, "Compactor", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("compaction", "Compaction")
                .description("How the context is shrunk when it fills up")
                .field(Field::enumeration("method", "Method", method_options()).description("How the context is summarized. Takes effect on the next compaction."))
                .field(Field::enumeration("piCompact", "/compact", pi_opts).description("What the built-in /compact command does: follow Method, or force one."))
                .field(
                    Field::enumeration(
                        "trigger",
                        "When",
                        vec![
                            choice("pi", "Oino's context limit", "compact when Oino itself nears the model's window"),
                            choice("percent", "at a percentage", "compact when usage crosses the threshold below"),
                        ],
                    )
                    .description("What trips a compaction while you work."),
                )
                .field(
                    Field::number("thresholdPercent", "Trigger threshold")
                        .description("Context share that trips a compaction when When = at a percentage.")
                        .with_kind(|k| {
                            if let Kind::Number { min, max, unit, .. } = k {
                                *min = Some(30.0);
                                *max = Some(95.0);
                                *unit = Some(Unit::Percent);
                            }
                        }),
                )
                .field(Field::boolean("notify", "Notifications").description("Notice when a compaction runs or fails.")),
        )
        .section(
            Section::new("advanced", "Advanced compaction")
                .description("Tuned defaults — rarely worth changing")
                .advanced(true)
                .field(Field::boolean("smartKeepTail", "Smart keep tail").description("Keep more recent turns when the kept tail would be tiny (≤5k tokens, up to 25k)."))
                .field(
                    Field::number("summaryBudgetTokens", "Summary budget")
                        .description("Lossless summary size in tokens (auto scales 1.5k–4k with session size).")
                        .with_kind(|k| {
                            if let Kind::Number { min, max, unit, zero_label, .. } = k {
                                *min = Some(0.0);
                                *max = Some(20_000.0);
                                *unit = Some(Unit::Tokens);
                                *zero_label = Some("auto".into());
                            }
                        }),
                )
                .field(
                    Field::multiselect(
                        "summarySections",
                        "Summary sections",
                        vec![
                            choice("activeWork", "Active work", "goal, ralph and kanboard state"),
                            choice("requests", "Your requests", "the user requests this session answered"),
                            choice("state", "Latest state", "the agent's most recent progress reports"),
                            choice("decisions", "Decisions", "decisions and constraints reached so far"),
                            choice("files", "Files", "files touched and what was done to them"),
                            choice("commits", "Commits", "commit history of this session"),
                            choice("errors", "Open errors", "unresolved errors worth remembering"),
                            choice("lessons", "Lessons", "memory notes and diagnoses written down"),
                            choice("transcript", "Recent transcript", "the last turns kept verbatim"),
                        ],
                    )
                    .description("What the summary keeps — deselect to drop it from every new summary.")
                    .with_kind(|k| {
                        if let Kind::Multiselect { empty_label, .. } = k {
                            *empty_label = Some("none".into());
                        }
                    }),
                )
                .field(
                    Field::string("llmInstructions", "Summary instructions")
                        .description("Extra instructions passed to model-written summaries.")
                        .empty_label("none"),
                )
                .field(Field::boolean("debug", "Debug dump").description("Write the last plan to logs/compactor-debug.json.").advanced()),
        )
}

// ── percentage trigger ───────────────────────────────────────────────────

/// Trigger state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AutoState {
    /// Previous percent.
    pub previous_percent: Option<f64>,
    /// Previous tokens.
    pub previous_tokens: Option<f64>,
    /// A compaction is in flight.
    pub in_flight: bool,
    /// Last trigger time.
    pub last_trigger_at: Option<i64>,
    /// Tokens at the last trigger.
    pub last_trigger_tokens: Option<f64>,
    /// Repeat baseline.
    pub repeat_baseline: Option<f64>,
    /// Next sample is a baseline.
    pub awaiting_post_sample: bool,
}

/// A decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    /// Compact now.
    pub trigger: bool,
    /// Reason.
    pub reason: &'static str,
    /// New state.
    pub state: AutoState,
    /// Usage `(tokens, percent)`.
    pub usage: Option<(f64, f64)>,
}

/// unipi `decideAutoCompaction`.
#[must_use]
pub fn decide(
    enabled: bool,
    threshold: f64,
    usage: Option<(f64, f64)>,
    st: AutoState,
    now: i64,
) -> Decision {
    let threshold = if threshold.is_finite() {
        threshold.clamp(1.0, 99.0)
    } else {
        80.0
    };
    let no = |reason, state, usage| Decision {
        trigger: false,
        reason,
        state,
        usage,
    };
    if !enabled {
        return no("disabled", st, None);
    }
    if st.in_flight {
        return no("in_flight", st, None);
    }
    let Some((tokens, percent)) =
        usage.filter(|(t, p)| t.is_finite() && *t >= 0.0 && p.is_finite() && *p >= 0.0)
    else {
        return no("unknown_usage", st, None);
    };
    let u = Some((tokens, percent));
    let mut next = AutoState {
        previous_percent: Some(percent),
        previous_tokens: Some(tokens),
        ..st
    };
    if st.awaiting_post_sample {
        next.awaiting_post_sample = false;
        next.repeat_baseline = (percent >= threshold).then_some(tokens);
        return no(
            if percent >= threshold {
                "post_compaction_baseline"
            } else {
                "below_threshold"
            },
            next,
            u,
        );
    }
    if percent < threshold {
        next.repeat_baseline = None;
        return no("below_threshold", next, u);
    }
    if let Some(at) = st.last_trigger_at
        && (now - at).max(0) < COOLDOWN_MS
    {
        return no("cooldown_active", next, u);
    }
    let fire = |reason| Decision {
        trigger: true,
        reason,
        state: AutoState {
            previous_percent: Some(percent),
            previous_tokens: Some(tokens),
            in_flight: true,
            last_trigger_at: Some(now),
            last_trigger_tokens: Some(tokens),
            repeat_baseline: Some(tokens),
            awaiting_post_sample: false,
        },
        usage: u,
    };
    match st.previous_percent {
        None => return fire("threshold_reached"),
        Some(p) if p < threshold => return fire("threshold_crossed"),
        _ => {}
    }
    if st.last_trigger_at.is_none() {
        return fire("threshold_reached");
    }
    let base = st
        .repeat_baseline
        .or(st.last_trigger_tokens)
        .or(st.previous_tokens)
        .unwrap_or(tokens);
    if (tokens - base).max(0.0) < REPEAT_MIN_GROWTH_TOKENS {
        return no("repeat_growth_needed", next, u);
    }
    fire("repeat_growth_reached")
}

/// After a successful compaction.
#[must_use]
pub fn mark_complete(s: AutoState) -> AutoState {
    AutoState {
        in_flight: false,
        awaiting_post_sample: true,
        ..s
    }
}

/// After a failed compaction.
#[must_use]
pub fn mark_error(s: AutoState, now: i64) -> AutoState {
    AutoState {
        in_flight: false,
        awaiting_post_sample: false,
        last_trigger_at: s.last_trigger_at.or(Some(now)),
        ..s
    }
}

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

    #[test]
    fn loading() {
        let c = load(&Values(defaults()));
        assert_eq!(c.method, Method::Vcc);
        assert!(!c.percent && c.notify && c.pi_compact.is_none());
        let c = load(&Values(
            json!({"method": "llm", "trigger": "percent", "thresholdPercent": 70}),
        ));
        assert_eq!(c.method, Method::Llm);
        assert!(c.percent);
        assert!((c.threshold - 70.0).abs() < f64::EPSILON);
        let c = load(&Values(json!({"method": "vcc", "piCompact": "vcc"})));
        assert_eq!(c.method, Method::Vcc);
        assert_eq!(c.pi_compact, Some(Method::Vcc));
        assert!(c.sections.files && c.sections.requests);
        let c = load(&Values(json!({"summarySections": ["requests"]})));
        assert!(c.sections.requests && !c.sections.transcript);
        assert_eq!(namespace().to_json()["namespace"], "compactor");
    }

    #[test]
    fn auto_trigger() {
        let s = AutoState::default();
        assert_eq!(
            decide(false, 80.0, Some((1.0, 90.0)), s, 0).reason,
            "disabled"
        );
        assert_eq!(decide(true, 80.0, None, s, 0).reason, "unknown_usage");
        assert_eq!(
            decide(true, 80.0, Some((1.0, 50.0)), s, 0).reason,
            "below_threshold"
        );
        let d = decide(true, 80.0, Some((100_000.0, 85.0)), s, 0);
        assert!(d.trigger && d.reason == "threshold_reached");
        assert_eq!(
            decide(true, 80.0, Some((100_000.0, 85.0)), d.state, 1).reason,
            "in_flight"
        );
        let done = mark_complete(d.state);
        let b = decide(true, 80.0, Some((90_000.0, 82.0)), done, 70_000);
        assert_eq!(b.reason, "post_compaction_baseline");
        assert_eq!(
            decide(true, 80.0, Some((91_000.0, 82.0)), b.state, 80_000).reason,
            "repeat_growth_needed"
        );
        assert!(decide(true, 80.0, Some((95_000.0, 84.0)), b.state, 80_000).trigger);
        assert_eq!(
            decide(true, 80.0, Some((95_000.0, 84.0)), b.state, 10_000).reason,
            "cooldown_active"
        );
        let crossed = decide(
            true,
            80.0,
            Some((1.0, 85.0)),
            AutoState {
                previous_percent: Some(50.0),
                last_trigger_at: Some(0),
                ..s
            },
            100_000,
        );
        assert_eq!(crossed.reason, "threshold_crossed");
        assert_eq!(mark_error(s, 5).last_trigger_at, Some(5));
    }
}
