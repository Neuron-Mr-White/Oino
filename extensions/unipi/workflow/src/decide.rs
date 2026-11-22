//! The permission decision matrix (unipi `src/permission/decide.ts`), pure. The jev step is
//! split out: [`decide`] returns [`Step::NeedsJev`] and [`after_jev`] finishes the decision.

use crate::pathx::{basename, resolve};
use crate::rules::{PermissionRule, RuleDecision, match_rule};

/// Tools that cannot mutate anything — always allowed (unipi `READ_ONLY_TOOLS`).
pub const READ_ONLY_TOOLS: &[&str] = &[
    "read",
    "grep",
    "find",
    "ls",
    "ffgrep",
    "fffind",
    "session_recall",
    "ask_user",
    "memory_search",
    "memory_list",
    "global_memory_search",
    "global_memory_list",
    "web_search",
    "multi_web_content_read",
    "web_llm_summarize",
    "bg_status",
    "bg_logs",
    "read_subagent",
    "compactor_stats",
    "compactor_doctor",
    "context_budget",
    "ctx_budget",
    "loop_status",
    "agent_swarm_status",
    "view_agent_graph",
    "agent_output",
    "get_goal",
];

/// `ask | auto | full`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Prompt for everything not explicitly allowed.
    Ask,
    /// jev judges ambiguous bash.
    Auto,
    /// Only deny rules block.
    Full,
}

impl Mode {
    /// All modes in cycle order.
    pub const ALL: [Self; 3] = [Self::Ask, Self::Auto, Self::Full];

    /// unipi `coerceMode`.
    #[must_use]
    pub fn coerce(v: Option<&str>) -> Self {
        match v {
            Some("ask") => Self::Ask,
            Some("full") => Self::Full,
            _ => Self::Auto,
        }
    }

    /// Parses an exact name.
    #[must_use]
    pub fn parse(v: &str) -> Option<Self> {
        match v {
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }

    /// Next in the cycle.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Ask => Self::Auto,
            Self::Auto => Self::Full,
            Self::Full => Self::Ask,
        }
    }
}

/// A gate decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Run.
    Allow(String),
    /// Prompt (`reason`, `subject`).
    Ask(String, String),
    /// Block with a reason.
    Block(String),
}

/// A jev risk verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct JevRisk {
    /// `safe | needs_approval | dangerous`.
    pub choice: String,
    /// Confidence 0–1.
    pub confidence: f64,
}

/// Inputs of one decision.
#[derive(Clone, Debug)]
pub struct Deps<'a> {
    /// Mode.
    pub mode: Mode,
    /// Minimum jev confidence.
    pub jev_confidence: f64,
    /// Saved rules.
    pub rules: &'a [PermissionRule],
    /// Project cwd (host path).
    pub cwd: &'a str,
    /// Temp dir (host path).
    pub tmpdir: &'a str,
    /// Dialogs possible.
    pub has_ui: bool,
}

/// First stage of a decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Final.
    Done(Decision),
    /// Ask jev with this `state`, then call [`after_jev`].
    NeedsJev {
        /// jev state text.
        state: String,
        /// Command.
        subject: String,
    },
}

/// The jev `state` payload for one command.
#[must_use]
pub fn jev_risk_state(cwd: &str, command: &str) -> String {
    let clipped: String = command.chars().take(1500).collect();
    format!("cwd: {}\ncommand: {clipped}", basename(cwd))
}

fn allow(r: impl Into<String>) -> Step {
    Step::Done(Decision::Allow(r.into()))
}
fn ask(r: impl Into<String>, s: impl Into<String>) -> Step {
    Step::Done(Decision::Ask(r.into(), s.into()))
}

/// Classifies one tool call (plan mode is enforced before this, by the caller).
#[must_use]
pub fn decide(tool: &str, raw_subject: &str, d: &Deps<'_>) -> Step {
    let is_write = matches!(tool, "write" | "edit");
    if is_write && raw_subject.trim().is_empty() {
        return Step::Done(Decision::Block("file_path must be nonempty".into()));
    }
    let subject = if is_write {
        resolve(d.cwd, raw_subject)
    } else {
        raw_subject.to_owned()
    };
    if let Some(rule) = match_rule(d.rules, tool, &subject)
        && rule.decision == RuleDecision::Deny
    {
        return Step::Done(Decision::Block(format!(
            "Blocked by permission (saved deny rule: {})",
            rule.pattern
        )));
    }
    if matches!(tool, "bash" | "powershell" | "bg_run") {
        use crate::autonomy::{Level, bash_floor, powershell_floor};
        let floor = if tool == "powershell" {
            powershell_floor(raw_subject)
        } else {
            bash_floor(raw_subject)
        };
        if floor > Level::Medium || tool == "bg_run" {
            return if d.has_ui {
                ask("needs Autonomy High", subject)
            } else {
                Step::Done(Decision::Block("needs Autonomy High".into()))
            };
        }
    }
    allow("host permission gate enforces ceiling and rules")
}

/// Finishes an auto-mode bash decision with jev's verdict (`None` = unavailable).
#[must_use]
pub fn after_jev(risk: Option<&JevRisk>, subject: &str, d: &Deps<'_>) -> Decision {
    let _ = risk;
    if !d.has_ui {
        return Decision::Block("needs Autonomy High".into());
    }
    Decision::Ask("needs Autonomy High".into(), subject.to_owned())
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
    use crate::rules::RuleScope;

    const CWD: &str = "/workspace/project";

    fn deps(mode: Mode, rules: &[PermissionRule], has_ui: bool) -> Deps<'_> {
        Deps {
            mode,
            jev_confidence: 0.7,
            rules,
            cwd: CWD,
            tmpdir: "/tmp",
            has_ui,
        }
    }

    fn run(tool: &str, subject: &str, d: &Deps<'_>, jev: Option<JevRisk>) -> Decision {
        match decide(tool, subject, d) {
            Step::Done(x) => x,
            Step::NeedsJev { subject, .. } => after_jev(jev.as_ref(), &subject, d),
        }
    }

    fn is_allow(d: &Decision) -> bool {
        matches!(d, Decision::Allow(_))
    }

    #[test]
    fn audit_unattended_and_saved_allow_never_bypass_floor() {
        let rules = [PermissionRule {
            tool: "bash".into(),
            pattern: "npm run *".into(),
            decision: RuleDecision::Allow,
            scope: RuleScope::Project,
        }];
        for (tool, command) in [
            ("powershell", "Remove-Item -Recurse -Force x"),
            ("bg_run", "rm -rf x"),
            ("bash", "npm run build; rm -rf x"),
            ("bash", "echo \"$(rm -rf x)\""),
        ] {
            assert_eq!(
                run(tool, command, &deps(Mode::Auto, &rules, false), None),
                Decision::Block("needs Autonomy High".into())
            );
        }
        assert_eq!(
            after_jev(None, "git push", &deps(Mode::Auto, &[], false)),
            Decision::Block("needs Autonomy High".into())
        );
        assert!(matches!(
            run("write", "", &deps(Mode::Auto, &[], false), None),
            Decision::Block(_)
        ));
    }

    #[test]
    fn read_only_tools_always_allowed() {
        for tool in [
            "read",
            "grep",
            "find",
            "ls",
            "memory_search",
            "web_search",
            "ffgrep",
            "session_recall",
            "bg_status",
            "ask_user",
        ] {
            for mode in Mode::ALL {
                assert!(is_allow(&run(tool, "", &deps(mode, &[], true), None)));
            }
        }
    }

    #[test]
    fn jev_state() {
        assert_eq!(
            jev_risk_state("/home/me/project", "npm install left-pad"),
            "cwd: project\ncommand: npm install left-pad"
        );
        assert_eq!(
            jev_risk_state("/x/y", &"a".repeat(2000)).len(),
            "cwd: y\ncommand: ".len() + 1500
        );
        assert_eq!(Mode::Full.next(), Mode::Ask);
        assert_eq!(Mode::coerce(Some("bogus")), Mode::Auto);
    }
}
