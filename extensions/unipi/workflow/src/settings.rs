//! `permission` settings namespace (unipi `src/permission/settings.ts`) and the values the gate
//! writes itself (saved `rules` and the global `defaultMode`).
//!
//! E-18/F-09: the permission **mode is never written into a project** — changing it in a
//! chat is session state in the extension ([`crate::Workflow`]); the only persisted mode is
//! the global default (`defaultMode` in the agent dir), set by the explicit "make default"
//! action. Saved rules stay per-scope (`extensionSettings.permission.rules`), read per file
//! and concatenated (global ++ project) so a project list does not shadow global rules. The
//! pre-write-path stores `/ext-workspace/<wsid>/permission.json` (project) and
//! `/ext-global/permission.json` (global) are imported once ([`import_legacy`]) and remain
//! the fallback when a write is refused (untrusted project, host without writes).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unipi_kit::settings::{Field, Namespace, Section, Store, Values, WriteScope};

use crate::decide::Mode;
use crate::rules::{PermissionRule, RuleScope, normalize_rules};

/// Extension id.
pub const EXTENSION_ID: &str = "unipi.workflow";
/// Namespace.
pub const NAMESPACE: &str = "permission";
/// Rule clear runner.
pub const CLEAR_RULES: &str = "permission-clear-rules";

/// unipi `DEFAULT_SETTINGS` (+ `decisionModel`). `mode` is not defaulted so an unset project
/// mode falls through to `defaultMode`.
#[must_use]
pub fn defaults() -> Value {
    json!({
        "defaultMode": "auto",
        "rules": [],
        "debug": false,
    })
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
                .description("Autonomy and approvals are enforced by the host. Historical denies remain in effect.")
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
}

/// What the gate stored itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// Legacy project mode (read-only compatibility; never written again, E-18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Legacy global default mode (the store fallback for refused writes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<String>,
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

/// The settings file of a rule scope.
#[must_use]
pub fn write_scope(scope: RuleScope) -> WriteScope {
    match scope {
        RuleScope::Project => WriteScope::Project,
        RuleScope::Global => WriteScope::Global,
    }
}

/// Saved rules of one settings file (`None` when the host cannot read files separately).
#[must_use]
pub fn file_rules(store: &Store, scope: RuleScope) -> Option<Vec<PermissionRule>> {
    store.layer(NAMESPACE, write_scope(scope)).map(|layer| {
        scoped(
            normalize_rules(layer.get("rules").unwrap_or(&Value::Null)),
            scope,
        )
    })
}

fn scoped(mut rules: Vec<PermissionRule>, scope: RuleScope) -> Vec<PermissionRule> {
    for r in &mut rules {
        r.scope = scope;
    }
    rules
}

/// Saved rules from the settings files: `(global, project)`.
pub type FileRules = Option<(Vec<PermissionRule>, Vec<PermissionRule>)>;

/// Reads both files' rules.
#[must_use]
pub fn read_file_rules(store: &Store) -> FileRules {
    Some((
        file_rules(store, RuleScope::Global)?,
        file_rules(store, RuleScope::Project)?,
    ))
}

/// Writes `rules` as the saved rules of `scope`'s settings file (`[]` unsets the key).
fn write_rules(store: &Store, scope: RuleScope, rules: &[PermissionRule]) -> bool {
    let target = write_scope(scope);
    if rules.is_empty() {
        return store.unset(NAMESPACE, "rules", target).is_ok();
    }
    let value = serde_json::to_value(rules).unwrap_or_else(|_| json!([]));
    store.write(NAMESPACE, "rules", &value, target).is_ok()
}

fn with_rule(mut rules: Vec<PermissionRule>, rule: PermissionRule) -> Vec<PermissionRule> {
    rules.retain(|r| !(r.tool == rule.tool && r.pattern == rule.pattern));
    rules.push(rule);
    rules
}

/// Saves a rule in its scope's settings file (replacing one with the same tool and pattern);
/// falls back to the local store when the write is refused.
pub fn persist_rule(store: &Store, rule: PermissionRule) {
    let scope = rule.scope;
    if let Some(current) = file_rules(store, scope)
        && write_rules(store, scope, &with_rule(current, rule.clone()))
    {
        return;
    }
    add_rule(rule);
}

/// Sets the **global** default mode in the agent dir (E-18: the only persisted mode — an
/// explicit "make default" action, never a per-chat change); falls back to the global
/// store. `WriteScope::Project` is refused outright: the repo's settings.json must never
/// learn a permission mode.
pub fn persist_default_mode(store: &Store, mode: &str) {
    if store
        .write(NAMESPACE, "defaultMode", &json!(mode), WriteScope::Global)
        .is_ok()
    {
        return;
    }
    let mut s = load(RuleScope::Global);
    s.default_mode = Some(mode.to_owned());
    save(RuleScope::Global, &s);
}

/// Forgets every saved rule (both files and both local stores).
pub fn clear_rules(store: &Store) {
    for scope in [RuleScope::Project, RuleScope::Global] {
        if file_rules(store, scope).is_some_and(|r| !r.is_empty()) {
            let _ = write_rules(store, scope, &[]);
        }
        let mut s = load(scope);
        if !s.rules.is_empty() {
            s.rules.clear();
            save(scope, &s);
        }
    }
}

/// One-time import of the local stores into the settings files: rules are appended to the
/// file's rules (deduplicated by tool + pattern). A **mode is never imported into a
/// project** (E-18); the global default mode moves into the global settings file when that
/// still holds a legacy value. A store is removed once imported; refused writes keep it as
/// the fallback layer. Returns the number of rules imported.
pub fn import_legacy(store: &Store) -> usize {
    let mut imported = 0;
    for scope in [RuleScope::Global, RuleScope::Project] {
        let path = file(scope);
        if !path.exists() {
            continue;
        }
        let stored = load(scope);
        let Some(current) = file_rules(store, scope) else {
            continue;
        };
        let mut ok = true;
        if !stored.rules.is_empty() {
            let merged = stored.rules.iter().cloned().fold(current, |acc, r| {
                with_rule(acc, PermissionRule { scope, ..r })
            });
            ok = write_rules(store, scope, &merged);
            if ok {
                imported += stored.rules.len();
            }
        }
        // A stored global default mode becomes the global `defaultMode`; a stored project
        // mode is dropped — per-project modes are no longer a thing (E-18).
        if ok
            && scope == RuleScope::Global
            && let Some(mode) = &stored.default_mode
        {
            ok = store
                .write(NAMESPACE, "defaultMode", &json!(mode), WriteScope::Global)
                .is_ok();
        }
        if ok {
            let _ = std::fs::remove_file(&path);
        }
    }
    imported
}

/// Resolved permission settings (unipi `PermissionSettings`).
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// Effective mode.
    pub mode: Mode,
    /// Session mode, if this chat set one (`-` in status means unset).
    pub session_mode: Option<String>,
    /// Legacy project mode, if the project still carries one (read-only, E-18).
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

/// Layers: the session's own mode → a legacy project mode (read-only; repos that carry one
/// from before E-18) → settings `mode` (same legacy channel) → `defaultMode` → `auto`.
/// Rules: the settings files' rules (`files`, else the effective `rules`) ++ the local
/// stores.
#[must_use]
pub fn resolve(
    v: &Values,
    files: FileRules,
    project: &Stored,
    global: &Stored,
    session_mode: Option<&str>,
) -> Resolved {
    let project_mode = project
        .mode
        .clone()
        .or_else(|| v.get("mode").and_then(Value::as_str).map(ToOwned::to_owned));
    let default_mode = Mode::coerce(
        global
            .default_mode
            .as_deref()
            .or_else(|| v.get("defaultMode").and_then(Value::as_str)),
    );
    let session = session_mode.map(ToOwned::to_owned);
    let mode = session
        .as_deref()
        .or(project_mode.as_deref())
        .map_or(default_mode, |m| Mode::coerce(Some(m)));
    let conf = v
        .get("jevConfidence")
        .and_then(Value::as_f64)
        .filter(|c| (0.0..=1.0).contains(c))
        .unwrap_or(0.7);
    let mut rules = match files {
        Some((global_rules, project_rules)) => {
            let mut all = global_rules;
            all.extend(project_rules);
            all
        }
        None => normalize_rules(v.get("rules").unwrap_or(&Value::Null)),
    };
    rules.extend(project.rules.iter().cloned());
    rules.extend(global.rules.iter().cloned());
    Resolved {
        mode,
        session_mode: session,
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
        let r = resolve(&v, None, &Stored::default(), &Stored::default(), None);
        assert_eq!(r.mode, Mode::Auto);
        assert_eq!(r.project_mode, None);
        assert_eq!(r.session_mode, None);
        assert!((r.jev_confidence - 0.7).abs() < f64::EPSILON);
        // A legacy project `mode` (a settings.json written before E-18) still resolves.
        let v = Values(json!({"mode": "ask", "defaultMode": "full"}));
        assert_eq!(
            resolve(&v, None, &Stored::default(), &Stored::default(), None).mode,
            Mode::Ask
        );
        // … but the session's own mode wins over it (E-18).
        assert_eq!(
            resolve(
                &v,
                None,
                &Stored::default(),
                &Stored::default(),
                Some("full")
            )
            .mode,
            Mode::Full
        );
        let stored = Stored {
            mode: Some("full".into()),
            rules: vec![PermissionRule {
                tool: "bash".into(),
                pattern: "x".into(),
                decision: RuleDecision::Allow,
                scope: RuleScope::Project,
            }],
            ..Stored::default()
        };
        let r = resolve(&v, None, &stored, &Stored::default(), None);
        assert_eq!(r.mode, Mode::Full);
        assert_eq!(r.rules.len(), 1);
        let rule = |p: &str, scope| PermissionRule {
            tool: "bash".into(),
            pattern: p.into(),
            decision: RuleDecision::Deny,
            scope,
        };
        let files = Some((
            vec![rule("g", RuleScope::Global)],
            vec![rule("p", RuleScope::Project)],
        ));
        let r = resolve(
            &Values(json!({"rules": [{"pattern": "ignored", "decision": "allow"}]})),
            files,
            &stored,
            &Stored::default(),
            None,
        );
        let patterns: Vec<&str> = r.rules.iter().map(|r| r.pattern.as_str()).collect();
        assert_eq!(patterns, ["g", "p", "x"], "files then local stores");
        assert_eq!(
            with_rule(
                vec![rule("a", RuleScope::Global)],
                rule("a", RuleScope::Global)
            )
            .len(),
            1
        );
        let v = Values(json!({"defaultMode": "full"}));
        assert_eq!(
            resolve(&v, None, &Stored::default(), &Stored::default(), None).mode,
            Mode::Full
        );
        // The store's global default wins over the settings file (it was written by
        // "make default" through the fallback path).
        let global = Stored {
            default_mode: Some("ask".into()),
            ..Stored::default()
        };
        assert_eq!(
            resolve(&v, None, &Stored::default(), &global, None).mode,
            Mode::Ask
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

    #[test]
    fn the_schema_never_offers_a_project_mode() {
        // E-18/F-09: the Settings page writes what the schema offers — `mode` with a
        // project scope would land in <repo>/.oino/settings.json.
        let ns = namespace(0).to_json();
        let fields: Vec<&Value> = ns["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|sec| sec["fields"].as_array().unwrap())
            .collect();
        assert!(
            fields.iter().all(|f| f["key"] != "mode"),
            "no `mode` field in the schema: {fields:?}"
        );
        assert!(
            fields
                .iter()
                .all(|f| f["key"] != "defaultMode" && f["key"] != "jevConfidence")
        );
    }
}
