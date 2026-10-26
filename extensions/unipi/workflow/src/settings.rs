//! `permission` settings namespace (unipi `src/permission/settings.ts`) and the guest-side
//! store for values the gate writes itself (mode, saved rules).
//!
//! unipi wrote `mode` and `rules` into its settings files. Oino has no guest settings write
//! path (deviations R1), so they live in `/ext-workspace/<wsid>/permission.json` (project)
//! and `/ext-global/permission.json` (global) and are layered over the host settings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unipi_kit::jev;
use unipi_kit::settings::{Field, Kind, Namespace, Scope, Section, Values, choice};

use crate::decide::Mode;
use crate::rules::{PermissionRule, RuleScope, normalize_rules};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.workflow";
/// Namespace.
pub const NAMESPACE: &str = "permission";
/// Rule clear runner.
pub const CLEAR_RULES: &str = "unipi:permission-clear-rules";

/// unipi `DEFAULT_SETTINGS` (+ `decisionModel`). `mode` is not defaulted so an unset project
/// mode falls through to `defaultMode`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "defaultMode": "auto",
        "jevConfidence": 0.7,
        "rules": [],
        "debug": false,
        "decisionModel": jev::consumer_defaults(),
    })
}

fn mode_options() -> Vec<unipi_kit::settings::Choice> {
    vec![
        choice("ask", "ask", "prompt for everything not explicitly allowed"),
        choice(
            "auto",
            "auto",
            "allow safe calls; jev judges ambiguous bash",
        ),
        choice("full", "full", "allow everything except saved deny rules"),
    ]
}

/// The hub schema; the clear action names the rule count.
#[must_use]
pub fn namespace(rule_count: usize) -> Namespace {
    let label = if rule_count == 0 {
        "Clear saved rules…".to_owned()
    } else {
        format!(
            "Clear {rule_count} saved rule{}…",
            if rule_count == 1 { "" } else { "s" }
        )
    };
    Namespace::new(NAMESPACE, "Permissions", EXTENSION_ID)
        .defaults(defaults())
        .section(
            Section::new("permissions", "Permissions")
                .description("Tool-call gate: ask, auto (jev judges ambiguous bash), or full.")
                .field(
                    Field::enumeration("mode", "Mode here", mode_options())
                        .description("Tool-call gate for this project. Alt+M cycles it.")
                        .scope(Scope::Project)
                        .with_kind(|k| {
                            if let Kind::Enum { clearable, .. } = k {
                                *clearable = true;
                            }
                        }),
                )
                .field(
                    Field::enumeration("defaultMode", "Default mode", mode_options())
                        .description("Mode for projects that set none of their own.")
                        .scope(Scope::Global),
                )
                .field(
                    Field::number("jevConfidence", "Minimum jev confidence")
                        .description("Auto mode accepts a 'safe' jev verdict at or above this confidence; below it, the call prompts.")
                        .with_kind(|k| {
                            if let Kind::Number { min, max, .. } = k {
                                *min = Some(0.0);
                                *max = Some(1.0);
                            }
                        }),
                )
                .field(
                    Field::action("rulesCount", &label, CLEAR_RULES)
                        .description("Forget every allow/deny rule saved for this project."),
                )
                .field(
                    Field::boolean("debug", "Debug log")
                        .description("Log every decision to logs/permission.log.")
                        .advanced(),
                ),
        )
        .section(jev::consumer_section(
            Some("Auto mode — Decision model"),
            false,
        ))
}

/// What the gate stored itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// Project mode set by `/unipi:permission` / Alt+M.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Saved rules.
    #[serde(default)]
    pub rules: Vec<PermissionRule>,
}

fn file(scope: RuleScope) -> PathBuf {
    match scope {
        RuleScope::Project => unipi_kit::paths::workspace("permission.json"),
        RuleScope::Global => unipi_kit::paths::global("permission.json"),
    }
}

/// Reads one stored layer.
#[must_use]
pub fn load(scope: RuleScope) -> Stored {
    unipi_kit::paths::read_json(&file(scope)).unwrap_or_default()
}

/// Writes one stored layer.
pub fn save(scope: RuleScope, s: &Stored) {
    let _ = unipi_kit::paths::write_json(&file(scope), s);
}

/// Resolved permission settings (unipi `PermissionSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// Effective mode.
    pub mode: Mode,
    /// Project mode, if any (`-` in status).
    pub project_mode: Option<String>,
    /// Default mode.
    pub default_mode: Mode,
    /// Minimum jev confidence.
    pub jev_confidence: f64,
    /// All rules (settings + stored).
    pub rules: Vec<PermissionRule>,
    /// Debug logging.
    pub debug: bool,
}

/// Layers stored values over the host settings: stored project `mode` → settings `mode` →
/// `defaultMode` → `auto`.
#[must_use]
pub fn resolve(v: &Values, project: &Stored, global: &Stored) -> Resolved {
    let project_mode = project
        .mode
        .clone()
        .or_else(|| v.get("mode").and_then(Value::as_str).map(ToOwned::to_owned));
    let default_mode = Mode::coerce(v.get("defaultMode").and_then(Value::as_str));
    let mode = project_mode
        .as_deref()
        .map_or(default_mode, |m| Mode::coerce(Some(m)));
    let conf = v
        .get("jevConfidence")
        .and_then(Value::as_f64)
        .filter(|c| (0.0..=1.0).contains(c))
        .unwrap_or(0.7);
    let mut rules = normalize_rules(v.get("rules").unwrap_or(&Value::Null));
    rules.extend(project.rules.iter().cloned());
    rules.extend(global.rules.iter().cloned());
    Resolved {
        mode,
        project_mode,
        default_mode,
        jev_confidence: conf,
        rules,
        debug: v.bool("debug", false),
    }
}

/// Adds a rule to its scope's store (replacing a rule with the same tool and pattern).
pub fn add_rule(rule: PermissionRule) {
    let mut s = load(rule.scope);
    s.rules
        .retain(|r| !(r.tool == rule.tool && r.pattern == rule.pattern));
    let scope = rule.scope;
    s.rules.push(rule);
    save(scope, &s);
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
    use crate::rules::RuleDecision;

    #[test]
    fn layering() {
        let v = Values(unipi_kit::settings::deep_merge(
            &defaults(),
            &json!({"jevConfidence": 2}),
        ));
        let r = resolve(&v, &Stored::default(), &Stored::default());
        assert_eq!(r.mode, Mode::Auto);
        assert_eq!(r.project_mode, None);
        assert!((r.jev_confidence - 0.7).abs() < f64::EPSILON);
        let v = Values(json!({"mode": "ask", "defaultMode": "full"}));
        assert_eq!(
            resolve(&v, &Stored::default(), &Stored::default()).mode,
            Mode::Ask
        );
        let stored = Stored {
            mode: Some("full".into()),
            rules: vec![PermissionRule {
                tool: "bash".into(),
                pattern: "x".into(),
                decision: RuleDecision::Allow,
                scope: RuleScope::Project,
            }],
        };
        let r = resolve(&v, &stored, &Stored::default());
        assert_eq!(r.mode, Mode::Full);
        assert_eq!(r.rules.len(), 1);
        let v = Values(json!({"defaultMode": "full"}));
        assert_eq!(
            resolve(&v, &Stored::default(), &Stored::default()).mode,
            Mode::Full
        );
    }

    #[test]
    fn schema() {
        let ns = namespace(2).to_json();
        let s = ns.to_string();
        assert!(s.contains("Clear 2 saved rules…"));
        assert!(s.contains(CLEAR_RULES));
        assert!(
            namespace(1)
                .to_json()
                .to_string()
                .contains("Clear 1 saved rule…")
        );
        assert_eq!(ns["namespace"], "permission");
    }
}
