//! The lossless compaction plan (unipi `planLosslessCompaction` in `hooks.ts`).

use serde_json::{Value, json};

use crate::brief::FileOps;
use crate::card::COMPACTOR_ID;
use crate::config::Config;
use crate::cut::{
    Cut, Refusal, apply_tail_budget, build_own_cut, calibrate, content_chars, resolve_smart_keep,
    tokens_from_chars,
};
use crate::source::collect_source;
use crate::summary::{Input, auto_budget_tokens, build};

const SUMMARY_CPT: f64 = 4.0;
const TAIL_WINDOW_LIMIT: f64 = 0.4;
const TAIL_WINDOW_SHARE: f64 = 0.25;

/// Plan inputs.
pub struct PlanInput<'a> {
    /// Branch entries.
    pub branch: &'a [Value],
    /// Tokens before.
    pub tokens_before: Option<u64>,
    /// Previous summary.
    pub previous_summary: Option<&'a str>,
    /// File ops.
    pub ops: FileOps,
    /// keep:N.
    pub keep: Option<usize>,
    /// keep given explicitly.
    pub explicit: bool,
    /// Model window.
    pub context_window: Option<u64>,
    /// Config.
    pub config: &'a Config,
    /// cwd.
    pub cwd: Option<&'a str>,
    /// Reason.
    pub reason: Option<&'a str>,
    /// Active-work blocks.
    pub active_work: &'a [(String, String)],
}

/// Stats of a plan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Messages summarized.
    pub summarized: usize,
    /// Messages kept.
    pub kept: usize,
    /// Tokens before.
    pub tokens_before: Option<u64>,
    /// Tokens after (estimate).
    pub tokens_after: u64,
    /// Kept user turns.
    pub kept_user_turns: usize,
    /// Total user turns.
    pub total_user_turns: usize,
    /// Kept tokens.
    pub kept_tokens: u64,
    /// Smart keep boosted.
    pub smart_adjusted: bool,
    /// Budget cut.
    pub budget_cut: Option<&'static str>,
}

/// A plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Summary.
    pub summary: String,
    /// First kept entry (`""` = all).
    pub first_kept: String,
    /// Details.
    pub details: Value,
    /// Stats.
    pub stats: Stats,
    /// Sections.
    pub sections: Vec<String>,
}

/// Builds a lossless compaction.
///
/// # Errors
///
/// [`Refusal`] when there is nothing to compact.
#[allow(
    clippy::too_many_lines,
    reason = "one plan, as unipi planLosslessCompaction"
)]
pub fn plan_lossless(input: &PlanInput<'_>) -> Result<Plan, Refusal> {
    let branch = input.branch;
    let cfg = input.config;
    let calib_chars = build_own_cut(branch, 0).map_or(0, |c| {
        c.messages
            .iter()
            .map(|m| content_chars(m.get("content")))
            .sum::<usize>()
    }) + input.previous_summary.map_or(0, |s| s.chars().count());
    let cpt = calibrate(calib_chars, input.tokens_before);
    let smart = resolve_smart_keep(
        branch,
        if input.explicit {
            Some(input.keep.unwrap_or(1))
        } else {
            None
        },
        input.explicit,
        cfg.smart_keep_tail,
        cpt,
    );
    let mut cut: Cut = build_own_cut(branch, smart.keep)?;
    if !input.explicit {
        cut = apply_tail_budget(branch, cut, None, None, cpt);
    }
    if let Some(w) = input.context_window.filter(|w| *w > 0) {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "window sizes fit"
        )]
        let max = (w as f64 * TAIL_WINDOW_SHARE).floor() as u64;
        cut = apply_tail_budget(
            branch,
            cut,
            Some(max),
            Some(TAIL_WINDOW_LIMIT / TAIL_WINDOW_SHARE),
            cpt,
        );
    }
    let kept_idx = (!cut.first_kept.is_empty())
        .then(|| {
            branch
                .iter()
                .position(|e| e.get("id").and_then(Value::as_str) == Some(cut.first_kept.as_str()))
        })
        .flatten();
    let end = kept_idx.unwrap_or(branch.len());
    let kept_chars: usize = kept_idx.map_or(0, |k| {
        branch[k..]
            .iter()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("message"))
            .map(|e| content_chars(e.pointer("/message/content")))
            .sum()
    });
    let kept_tokens = tokens_from_chars(kept_chars, cpt);
    let source = collect_source(branch, end);
    let wanted = if cfg.summary_budget_tokens > 0 {
        cfg.summary_budget_tokens
    } else {
        auto_budget_tokens(source.blocks.len())
    };
    let budget = input
        .context_window
        .filter(|w| *w > 0)
        .map_or(wanted, |w| wanted.min((w * 8 / 100).max(800)));
    let active: &[(String, String)] = if cfg.sections.active_work {
        input.active_work
    } else {
        &[]
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "budgets are small and positive"
    )]
    let budget_chars = (budget as f64 * SUMMARY_CPT) as usize;
    let summary = build(&Input {
        source: &source,
        active_work: active,
        budget_chars,
        cwd: input.cwd,
        ops: &input.ops,
        sections: cfg.sections,
    });
    let tokens_after = kept_tokens + tokens_from_chars(summary.text.chars().count(), SUMMARY_CPT);
    let kept_messages = kept_idx.map_or(0, |k| {
        branch[k..]
            .iter()
            .filter(|e| e.get("type").and_then(Value::as_str) == Some("message"))
            .count()
    });
    let stats = Stats {
        summarized: cut.messages.len(),
        kept: kept_messages,
        tokens_before: input.tokens_before,
        tokens_after,
        kept_user_turns: cut.kept_user_turns,
        total_user_turns: cut.total_user_turns,
        kept_tokens,
        smart_adjusted: smart.adjusted,
        budget_cut: cut.budget_cut.map(crate::cut::BudgetCut::as_str),
    };
    let mut details = json!({
        "compactor": COMPACTOR_ID, "version": 2, "method": "vcc", "sections": summary.sections,
        "sourceMessageCount": cut.messages.len(), "tokensAfter": tokens_after, "budgetTokens": budget,
    });
    if let Some(r) = input.reason {
        details["reason"] = json!(r);
    }
    if let Some(b) = stats.budget_cut {
        details["budgetCut"] = json!(b);
    }
    Ok(Plan {
        summary: summary.text,
        first_kept: cut.first_kept,
        details,
        sections: summary.sections,
        stats,
    })
}

/// unipi `formatCompactionStats`.
#[must_use]
pub fn format_stats(s: &Stats) -> String {
    use unipi_kit::fmt::format_tokens;
    let before = s.tokens_before.map_or_else(String::new, |b| {
        format!(
            "{} → ~{} tok, ",
            format_tokens(b),
            format_tokens(s.tokens_after)
        )
    });
    if let Some(b) = s.budget_cut {
        let reason = if b == "no_anchor" {
            "no user turn to anchor on"
        } else {
            "oversized tail"
        };
        return format!(
            "compactor: {before}kept ~{} tok tail ({reason}), summarized {}.",
            format_tokens(s.kept_tokens),
            s.summarized
        );
    }
    let mut notes = vec![format!("summarized {}", s.summarized)];
    if s.smart_adjusted {
        notes.push("smart-keep".into());
    }
    format!(
        "compactor: {before}kept {}/{} turns ({}).",
        s.kept_user_turns,
        s.total_user_turns,
        notes.join(", ")
    )
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
    use crate::config::{defaults, load};
    use crate::cut::fixtures::*;
    use unipi_kit::settings::Values;

    #[test]
    fn plans() {
        let cfg = load(&Values(defaults()));
        let b = working_session();
        let active = vec![(
            "goal".to_owned(),
            "Goal (active, turn 3/50): \"ship the login page\"".to_owned(),
        )];
        let p = plan_lossless(&PlanInput {
            branch: &b,
            tokens_before: Some(50_000),
            previous_summary: None,
            ops: FileOps::default(),
            keep: None,
            explicit: false,
            context_window: Some(200_000),
            config: &cfg,
            cwd: Some("/repo"),
            reason: Some("threshold"),
            active_work: &active,
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            p.summary
                .starts_with("[Active Work]\nGoal (active, turn 3/50)")
        );
        assert!(p.summary.contains("[Your Requests]"));
        assert_eq!(p.details["method"], "vcc");
        assert_eq!(p.details["compactor"], COMPACTOR_ID);
        assert!(format_stats(&p.stats).starts_with("compactor: 50k → ~"));
        assert_eq!(
            plan_lossless(&PlanInput {
                branch: &[],
                tokens_before: None,
                previous_summary: None,
                ops: FileOps::default(),
                keep: None,
                explicit: false,
                context_window: None,
                config: &cfg,
                cwd: None,
                reason: None,
                active_work: &[],
            }),
            Err(Refusal::NoLiveMessages)
        );
    }
}
