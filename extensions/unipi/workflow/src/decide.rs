//! The permission decision matrix (unipi `src/permission/decide.ts`), pure. The jev step is
//! split out: [`decide`] returns [`Step::NeedsJev`] and [`after_jev`] finishes the decision.

use crate::bash::{BashKind, classify_bash};
use crate::pathx::{basename, is_inside, resolve};
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
    let is_write = tool == "write" || tool == "edit";
    let subject = if is_write {
        resolve(d.cwd, raw_subject)
    } else {
        raw_subject.to_owned()
    };
    if let Some(rule) = match_rule(d.rules, tool, &subject) {
        return match rule.decision {
            RuleDecision::Deny => Step::Done(Decision::Block(format!(
                "Blocked by permission (saved deny rule: {})",
                rule.pattern
            ))),
            RuleDecision::Allow => allow(format!("saved allow rule: {}", rule.pattern)),
        };
    }
    if READ_ONLY_TOOLS.contains(&tool) {
        return allow("read-only tool");
    }
    if is_write {
        let inside = is_inside(&subject, d.cwd) || is_inside(&subject, d.tmpdir);
        if inside && d.mode != Mode::Ask {
            return allow("inside the workspace");
        }
        if d.mode == Mode::Full {
            return allow("full mode · outside the workspace");
        }
        if !d.has_ui {
            return allow("no UI — full behaviour");
        }
        return ask(
            if inside {
                "ask mode"
            } else {
                "writes outside the workspace"
            },
            subject,
        );
    }
    if tool == "bash" {
        let v = classify_bash(&subject);
        return match v.kind {
            BashKind::Dangerous => {
                if !d.has_ui {
                    Step::Done(Decision::Block(format!(
                        "Blocked by permission (dangerous, no UI to confirm): {}",
                        v.reason
                    )))
                } else if d.mode == Mode::Full {
                    allow(format!("full mode · {}", v.reason))
                } else {
                    ask(format!("dangerous: {}", v.reason), subject)
                }
            }
            BashKind::Kanboard | BashKind::ReadOnly => {
                if d.mode == Mode::Ask && d.has_ui {
                    ask("ask mode", subject)
                } else {
                    allow(v.reason)
                }
            }
            BashKind::Unknown => match d.mode {
                Mode::Full => allow("full mode"),
                Mode::Auto => Step::NeedsJev {
                    state: jev_risk_state(d.cwd, &subject),
                    subject,
                },
                Mode::Ask if !d.has_ui => allow("no UI — full behaviour"),
                Mode::Ask => ask("ask mode", subject),
            },
        };
    }
    if d.mode == Mode::Ask && d.has_ui {
        let clipped: String = raw_subject.chars().take(120).collect();
        return ask("ask mode", clipped);
    }
    allow(format!("{} mode", d.mode.as_str()))
}

/// Finishes an auto-mode bash decision with jev's verdict (`None` = unavailable).
#[must_use]
pub fn after_jev(risk: Option<&JevRisk>, subject: &str, d: &Deps<'_>) -> Decision {
    if let Some(r) = risk
        && r.choice == "safe"
        && r.confidence >= d.jev_confidence
    {
        return Decision::Allow(format!("jev: safe {:.2}", r.confidence));
    }
    let label = risk.map_or_else(
        || "jev: unavailable".to_owned(),
        |r| format!("jev: {} {:.2}", r.choice, r.confidence),
    );
    if !d.has_ui {
        return Decision::Allow(format!("no UI — full behaviour ({label})"));
    }
    Decision::Ask(label, subject.to_owned())
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
    fn is_ask(d: &Decision) -> bool {
        matches!(d, Decision::Ask(..))
    }

    fn safe() -> Option<JevRisk> {
        Some(JevRisk {
            choice: "safe".into(),
            confidence: 0.95,
        })
    }
    fn risky() -> Option<JevRisk> {
        Some(JevRisk {
            choice: "needs_approval".into(),
            confidence: 0.82,
        })
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
    fn writes() {
        for tool in ["write", "edit"] {
            for mode in [Mode::Auto, Mode::Full] {
                assert!(is_allow(&run(
                    tool,
                    "src/a.ts",
                    &deps(mode, &[], true),
                    None
                )));
            }
            assert!(is_ask(&run(
                tool,
                "src/a.ts",
                &deps(Mode::Ask, &[], true),
                None
            )));
            for mode in [Mode::Auto, Mode::Ask] {
                let d = run(tool, "/etc/hosts", &deps(mode, &[], true), None);
                assert!(matches!(&d, Decision::Ask(r, _) if r.contains("outside the workspace")));
            }
            assert!(is_allow(&run(
                tool,
                "/etc/hosts",
                &deps(Mode::Full, &[], true),
                None
            )));
            assert!(is_allow(&run(
                tool,
                "/tmp/scratch.ts",
                &deps(Mode::Auto, &[], true),
                None
            )));
            assert!(is_allow(&run(
                tool,
                "/etc/hosts",
                &deps(Mode::Auto, &[], false),
                None
            )));
        }
    }

    #[test]
    fn bash_matrix() {
        for mode in [Mode::Auto, Mode::Full] {
            assert!(is_allow(&run(
                "bash",
                "git status && ls",
                &deps(mode, &[], true),
                None
            )));
        }
        assert!(is_ask(&run(
            "bash",
            "ls",
            &deps(Mode::Ask, &[], true),
            None
        )));
        assert_eq!(
            run(
                "bash",
                "rm -rf /tmp/wd-test",
                &deps(Mode::Auto, &[], true),
                None
            ),
            Decision::Ask("dangerous: rm -rf".into(), "rm -rf /tmp/wd-test".into())
        );
        assert!(is_allow(&run(
            "bash",
            "rm -rf /tmp/wd-test",
            &deps(Mode::Full, &[], true),
            None
        )));
        assert_eq!(
            run(
                "bash",
                "npm install left-pad",
                &deps(Mode::Auto, &[], true),
                safe()
            ),
            Decision::Allow("jev: safe 0.95".into())
        );
        assert!(
            matches!(run("bash", "npm publish", &deps(Mode::Auto, &[], true), risky()), Decision::Ask(r, _) if r == "jev: needs_approval 0.82")
        );
        let mut strict = deps(Mode::Auto, &[], true);
        strict.jev_confidence = 0.99;
        assert!(is_ask(&run(
            "bash",
            "npm install left-pad",
            &strict,
            safe()
        )));
        assert!(
            matches!(run("bash", "npm install x", &deps(Mode::Auto, &[], true), None), Decision::Ask(r, _) if r == "jev: unavailable")
        );
        let board =
            "unipi-kanboard --actor agent --project p-1 move UNI-3 blocked --comment 'need format'";
        for mode in [Mode::Auto, Mode::Full] {
            assert!(
                matches!(run("bash", board, &deps(mode, &[], true), None), Decision::Allow(r) if r.contains("kanboard CLI"))
            );
        }
        assert!(is_ask(&run(
            "bash",
            board,
            &deps(Mode::Ask, &[], true),
            None
        )));
        assert!(is_allow(&run(
            "bash",
            "npm publish",
            &deps(Mode::Full, &[], true),
            None
        )));
    }

    #[test]
    fn other_tools_and_rules() {
        for mode in [Mode::Auto, Mode::Full] {
            assert!(is_allow(&run(
                "spawn_helper",
                "{}",
                &deps(mode, &[], true),
                None
            )));
        }
        assert!(is_ask(&run(
            "spawn_helper",
            "{}",
            &deps(Mode::Ask, &[], true),
            None
        )));
        let allow_rule = [PermissionRule {
            tool: "bash".into(),
            pattern: "npm install *".into(),
            decision: RuleDecision::Allow,
            scope: RuleScope::Project,
        }];
        assert!(is_allow(&run(
            "bash",
            "npm install left-pad",
            &deps(Mode::Ask, &allow_rule, true),
            None
        )));
        let deny = [PermissionRule {
            tool: "*".into(),
            pattern: "*publish*".into(),
            decision: RuleDecision::Deny,
            scope: RuleScope::Project,
        }];
        assert!(
            matches!(run("bash", "npm publish --access public", &deps(Mode::Full, &deny, true), None), Decision::Block(r) if r.contains("saved deny rule"))
        );
        let read_deny = [PermissionRule {
            tool: "read".into(),
            pattern: "secret*".into(),
            decision: RuleDecision::Deny,
            scope: RuleScope::Project,
        }];
        assert!(matches!(
            run(
                "read",
                "secret.txt",
                &deps(Mode::Auto, &read_deny, true),
                None
            ),
            Decision::Block(_)
        ));
        let path_rule = [PermissionRule {
            tool: "write".into(),
            pattern: format!("{CWD}/docs/plans/*"),
            decision: RuleDecision::Allow,
            scope: RuleScope::Project,
        }];
        assert!(is_allow(&run(
            "write",
            "docs/plans/p.md",
            &deps(Mode::Ask, &path_rule, true),
            None
        )));
    }

    #[test]
    fn no_ui() {
        assert!(is_allow(&run(
            "bash",
            "npm install left-pad",
            &deps(Mode::Auto, &[], false),
            risky()
        )));
        assert!(
            matches!(run("bash", "rm -rf /", &deps(Mode::Auto, &[], false), None), Decision::Block(r) if r.contains("no UI to confirm"))
        );
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
