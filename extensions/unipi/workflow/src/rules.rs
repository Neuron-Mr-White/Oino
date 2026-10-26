//! Saved permission rules (unipi `src/permission/rules.ts`): glob-matched allow/deny decisions.
//! Deny always wins.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `allow | deny`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleDecision {
    /// Run the call.
    Allow,
    /// Block the call.
    Deny,
}

/// `project | global`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleScope {
    /// This project.
    #[default]
    Project,
    /// Every project.
    Global,
}

/// One rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Tool name or `*`.
    pub tool: String,
    /// Glob over the subject.
    pub pattern: String,
    /// Decision.
    pub decision: RuleDecision,
    /// Scope.
    pub scope: RuleScope,
}

/// `*` → any run, `?` → one char, everything else literal (case-insensitive, `.` spans
/// newlines).
#[must_use]
pub fn glob_matches(pattern: &str, subject: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let s: Vec<char> = subject.to_lowercase().chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || (p[pi] != '*' && p[pi] == s[si])) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, si));
            pi += 1;
        } else if let Some((sp, ss)) = star {
            pi = sp + 1;
            si = ss + 1;
            star = Some((sp, ss + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// Rule applies to `tool`/`subject`.
#[must_use]
pub fn rule_matches(rule: &PermissionRule, tool: &str, subject: &str) -> bool {
    (rule.tool == "*" || rule.tool == tool) && glob_matches(&rule.pattern, subject)
}

/// The applicable rule (a deny shadows an allow).
#[must_use]
pub fn match_rule<'a>(
    rules: &'a [PermissionRule],
    tool: &str,
    subject: &str,
) -> Option<&'a PermissionRule> {
    let applicable: Vec<&PermissionRule> = rules
        .iter()
        .filter(|r| rule_matches(r, tool, subject))
        .collect();
    applicable
        .iter()
        .find(|r| r.decision == RuleDecision::Deny)
        .or_else(|| {
            applicable
                .iter()
                .find(|r| r.decision == RuleDecision::Allow)
        })
        .copied()
}

/// Drops malformed entries (settings files are user-editable).
#[must_use]
pub fn normalize_rules(value: &Value) -> Vec<PermissionRule> {
    let Some(list) = value.as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            let pattern = e.get("pattern")?.as_str().filter(|p| !p.is_empty())?;
            let decision = match e.get("decision")?.as_str()? {
                "allow" => RuleDecision::Allow,
                "deny" => RuleDecision::Deny,
                _ => return None,
            };
            Some(PermissionRule {
                tool: e
                    .get("tool")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .unwrap_or("*")
                    .to_owned(),
                pattern: pattern.to_owned(),
                decision,
                scope: if e.get("scope").and_then(Value::as_str) == Some("global") {
                    RuleScope::Global
                } else {
                    RuleScope::Project
                },
            })
        })
        .collect()
}

const EXACT_ONLY_BINS: &[&str] = &[
    "rm", "rmdir", "mv", "cp", "dd", "mkfs", "chmod", "chown", "chgrp", "chattr", "sudo", "su",
    "doas", "kill", "pkill", "killall", "tee", "truncate", "ln", "install", "shred",
];

/// The pattern offered as "Always allow …".
#[must_use]
pub fn suggest_pattern(tool: &str, subject: &str) -> String {
    if tool != "bash" {
        let dir = subject.rfind('/').map_or(subject, |i| &subject[..i]);
        return if !dir.is_empty() && dir != subject {
            format!("{dir}/*")
        } else {
            subject.to_owned()
        };
    }
    let trimmed = subject.trim();
    if trimmed.contains(['&', '|', ';', '\n']) {
        return trimmed.to_owned();
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let first = words.first().copied().unwrap_or("");
    let bin = first.rsplit('/').next().unwrap_or("");
    if EXACT_ONLY_BINS.contains(&bin) {
        return trimmed.to_owned();
    }
    let mut head: Vec<&str> = Vec::new();
    for w in &words {
        if !head.is_empty() && (w.starts_with('-') || w.contains('/') || w.contains('=')) {
            break;
        }
        head.push(w);
        if head.len() == 2 {
            break;
        }
    }
    let base = if head.is_empty() {
        if first.is_empty() {
            subject.to_owned()
        } else {
            first.to_owned()
        }
    } else {
        head.join(" ")
    };
    if trimmed == base {
        base
    } else {
        format!("{base} *")
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
    use serde_json::json;

    use super::*;

    fn rule(pattern: &str, decision: RuleDecision, tool: &str) -> PermissionRule {
        PermissionRule {
            tool: tool.into(),
            pattern: pattern.into(),
            decision,
            scope: RuleScope::Project,
        }
    }

    #[test]
    fn globs() {
        assert!(glob_matches("npm run *", "npm run build --watch"));
        assert!(glob_matches("/tmp/*", "/tmp/a/b"));
        assert!(!glob_matches("npm run *", "yarn run build"));
        assert!(glob_matches("rm -rf /tmp/wd-test", "rm -rf /tmp/wd-test"));
        assert!(!glob_matches("a.b", "axb"));
        assert!(glob_matches("a+b", "a+b"));
        assert!(glob_matches("v?", "v1"));
        assert!(!glob_matches("v?", "v12"));
        assert!(glob_matches("NPM *", "npm x\ny"));
        assert!(glob_matches("*publish*", "npm publish --access public"));
    }

    #[test]
    fn matching() {
        let rules = [
            rule("git push *", RuleDecision::Allow, "bash"),
            rule("git push *", RuleDecision::Deny, "bash"),
        ];
        assert_eq!(
            match_rule(&rules, "bash", "git push origin main").map(|r| r.decision),
            Some(RuleDecision::Deny)
        );
        let rules = [
            rule("/tmp/*", RuleDecision::Allow, "write"),
            rule("*all*", RuleDecision::Deny, "*"),
        ];
        assert_eq!(
            match_rule(&rules, "write", "/tmp/x").map(|r| r.decision),
            Some(RuleDecision::Allow)
        );
        assert_eq!(
            match_rule(&rules, "bash", "install-all").map(|r| r.decision),
            Some(RuleDecision::Deny)
        );
        assert!(match_rule(&[rule("npm *", RuleDecision::Allow, "bash")], "bash", "ls").is_none());
    }

    #[test]
    fn normalize() {
        let rules = normalize_rules(&json!([
            {"tool": "bash", "pattern": "npm *", "decision": "allow"},
            {"pattern": "", "decision": "allow"},
            {"pattern": "x", "decision": "maybe"},
            null,
            "nope"
        ]));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].scope, RuleScope::Project);
        assert!(normalize_rules(&Value::Null).is_empty());
    }

    #[test]
    fn suggestions() {
        assert_eq!(
            suggest_pattern("bash", "npm run build --watch"),
            "npm run *"
        );
        assert_eq!(
            suggest_pattern("bash", "npm install left-pad"),
            "npm install *"
        );
        assert_eq!(
            suggest_pattern("bash", "rm -rf /tmp/wd-test"),
            "rm -rf /tmp/wd-test"
        );
        assert_eq!(suggest_pattern("bash", "chmod -R 777 ."), "chmod -R 777 .");
        assert_eq!(
            suggest_pattern("bash", "cd /tmp && ./loop.sh --verbose"),
            "cd /tmp && ./loop.sh --verbose"
        );
        assert_eq!(suggest_pattern("bash", "ls | wc -l"), "ls | wc -l");
        assert_eq!(suggest_pattern("bash", "a; b"), "a; b");
        assert_eq!(
            suggest_pattern("write", "/home/me/project/src/a.ts"),
            "/home/me/project/src/*"
        );
        assert_eq!(suggest_pattern("bash", "git add src/a.ts"), "git add *");
        assert_eq!(suggest_pattern("bash", "ls"), "ls");
    }
}
