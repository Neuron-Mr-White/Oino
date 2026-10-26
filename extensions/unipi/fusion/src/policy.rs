//! Lead policy, sidekick prompt, nudges and savings (unipi `prompts.ts`, `nudge.ts`,
//! `savings.ts`).

use std::collections::HashMap;

use crate::child::Usage;
use crate::preset::Price;

/// Non-trivial bash calls between nudges.
pub const BASH_NUDGE_EVERY: u32 = 4;
/// Reminder after a direct edit/write.
pub const EDIT_NUDGE: &str = include_str!("../assets/edit-nudge.txt");
const LEAD_POLICY: &str = include_str!("../assets/lead-policy.md");
const SIDEKICK: &str = include_str!("../assets/sidekick.md");
const BASH_NUDGE: &str = include_str!("../assets/bash-nudge.txt");

/// unipi `FusionIdentity`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Lead display name.
    pub lead_name: String,
    /// Lead effort label.
    pub lead_effort: String,
    /// Sidekick display name.
    pub sidekick_name: String,
    /// Sidekick effort label.
    pub sidekick_effort: String,
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// unipi `fusionIdentityLine`.
#[must_use]
pub fn identity_line(id: &Identity) -> String {
    let join = |n: &str, e: &str| {
        if e.is_empty() {
            n.to_owned()
        } else {
            format!("{n} {}", cap(e))
        }
    };
    format!(
        "You are powered by Fusion ({} + {}).",
        join(&id.lead_name, &id.lead_effort),
        join(&id.sidekick_name, &id.sidekick_effort)
    )
}

/// unipi `leadPolicy`.
#[must_use]
pub fn lead_policy(id: &Identity) -> String {
    LEAD_POLICY.replace("{identity}", &identity_line(id))
}

/// unipi `sidekickSystemPrompt`.
#[must_use]
pub fn sidekick_prompt(id: &Identity) -> String {
    SIDEKICK.replace("{lead}", &id.lead_name)
}

/// unipi `bashNudge`.
#[must_use]
pub fn bash_nudge(count: u32) -> String {
    BASH_NUDGE.replace("{count}", &count.to_string())
}

const TRIVIAL_BINS: &[&str] = &[
    "cd", "ls", "pwd", "cat", "head", "tail", "wc", "echo", "printf", "which", "type", "rg",
    "grep", "stat", "file", "du", "df", "env", "printenv", "date", "whoami", "sort", "uniq", "cut",
    "tr", "jq", "column", "nl", "tree", "fd", "basename", "dirname", "realpath", "readlink",
    "diff", "true",
];
const GIT_READONLY: &[&str] = &[
    "status",
    "log",
    "diff",
    "branch",
    "show",
    "remote",
    "rev-parse",
    "ls-files",
    "blame",
    "grep",
    "describe",
    "shortlog",
];

fn strip_quotes(w: &str) -> &str {
    if w.len() >= 2
        && ((w.starts_with('"') && w.ends_with('"')) || (w.starts_with('\'') && w.ends_with('\'')))
    {
        &w[1..w.len() - 1]
    } else {
        w
    }
}

fn ident_len(s: &str) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_alphabetic() || c == '_' || (i > 0 && c.is_ascii_digit()) {
            n = i + 1;
        } else {
            break;
        }
    }
    n
}

/// One `VAR=value` prefix → `(name, value, consumed)`.
fn assignment(s: &str) -> Option<(String, String, usize)> {
    let n = ident_len(s);
    if n == 0 || !s[n..].starts_with('=') {
        return None;
    }
    let rest = &s[n + 1..];
    let vlen = match rest.chars().next() {
        Some(q @ ('"' | '\'')) => rest[1..].find(q).map(|e| e + 2)?,
        _ => rest.find(char::is_whitespace).unwrap_or(rest.len()),
    };
    let value = strip_quotes(&rest[..vlen]).to_owned();
    let after = &rest[vlen..];
    let ws = after.len() - after.trim_start().len();
    Some((s[..n].to_owned(), value, n + 1 + vlen + ws))
}

fn trivial_segment(segment: &str, vars: &mut HashMap<String, String>) -> bool {
    let mut rest = segment.trim().to_owned();
    let mut assigned = Vec::new();
    while let Some((k, v, used)) = assignment(&rest) {
        assigned.push((k, v));
        rest = rest[used..].trim().to_owned();
    }
    if rest.is_empty() {
        vars.extend(assigned);
        return true;
    }
    for prefix in ["\"${", "${", "\"$", "$"] {
        if let Some(after) = rest.strip_prefix(prefix) {
            let n = ident_len(after);
            let close = match prefix {
                "\"${" => "}\"",
                "${" => "}",
                "\"$" => "\"",
                _ => "",
            };
            if n > 0 && after[n..].starts_with(close) {
                let tail = &after[n + close.len()..];
                if tail.is_empty() || tail.starts_with(char::is_whitespace) {
                    let Some(v) = vars.get(&after[..n]) else {
                        return false;
                    };
                    rest = format!("{v}{tail}");
                }
            }
            break;
        }
    }
    if let Some(after) = rest.strip_prefix("timeout ") {
        let t = after.trim_start();
        let end = t.find(char::is_whitespace).unwrap_or(t.len());
        if t[..end].chars().next().is_some_and(|c| c.is_ascii_digit()) && end < t.len() {
            rest = t[end..].trim().to_owned();
        }
    }
    if rest.is_empty() {
        return true;
    }
    let head_end = match rest.chars().next() {
        Some(q @ ('"' | '\'')) => rest[1..].find(q).map_or(rest.len(), |e| e + 2),
        _ => rest.find(char::is_whitespace).unwrap_or(rest.len()),
    };
    let head = strip_quotes(&rest[..head_end]);
    let bin = head.rsplit('/').next().unwrap_or(head);
    let args: Vec<&str> = rest[head_end..].split_whitespace().collect();
    if bin == "unipi-kanboard" || bin == "oino-kanboard" || TRIVIAL_BINS.contains(&bin) {
        return true;
    }
    match bin {
        "find" => !args
            .iter()
            .any(|a| matches!(*a, "-exec" | "-execdir" | "-delete" | "-ok" | "-okdir")),
        "sed" => !args
            .iter()
            .any(|a| a.starts_with("-i") || a.starts_with("--in-place")),
        "awk" => !args.join(" ").contains("system("),
        "git" => args.first().is_some_and(|a| GIT_READONLY.contains(a)),
        "tmux" => args.first() == Some(&"capture-pane"),
        "npm" => args
            .first()
            .is_some_and(|a| matches!(*a, "view" | "whoami" | "ls")),
        _ => false,
    }
}

/// unipi `isTrivialShell`: only reads/prints — no substitution, heredoc or redirect, and every
/// segment starts with a read-only tool.
#[must_use]
pub fn is_trivial_shell(command: &str) -> bool {
    let rest = command
        .trim()
        .replace("2>&1", " ")
        .replace("&>/dev/null", " ")
        .replace("&> /dev/null", " ")
        .replace("2>/dev/null", " ")
        .replace("1>/dev/null", " ")
        .replace(">/dev/null", " ")
        .replace("2> /dev/null", " ")
        .replace("> /dev/null", " ");
    let b = rest.as_bytes();
    let mut segments = Vec::new();
    let (mut start, mut i, mut quote) = (0usize, 0usize, 0u8);
    while i < b.len() {
        let c = b[i];
        if c == b'\\' && quote != b'\'' {
            i += 2;
            continue;
        }
        if quote != b'\'' && (rest[i..].starts_with("$(") || c == b'`') {
            return false;
        }
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quote = c;
            i += 1;
            continue;
        }
        if c == b'>' || rest[i..].starts_with("<<") {
            return false;
        }
        if c == b';' || c == b'|' || c == b'\n' || rest[i..].starts_with("&&") {
            segments.push(&rest[start..i]);
            if rest[i..].starts_with("&&") || rest[i..].starts_with("||") {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    segments.push(&rest[start.min(rest.len())..]);
    let mut vars = HashMap::new();
    segments.into_iter().all(|s| trivial_segment(s, &mut vars))
}

/// Savings estimate `(sidekickUsd, atLeadUsd, savedUsd)` (unipi `estimateSavings`).
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "token counts")]
pub fn estimate_savings(
    usage: &Usage,
    lead: Option<&Price>,
    side: Option<&Price>,
) -> (f64, f64, f64) {
    let price = |p: Option<&Price>| {
        p.map_or(usage.cost, |c| {
            (usage.input as f64 * c.input
                + usage.cache_read as f64 * c.cached_input
                + usage.cache_write as f64 * c.input
                + usage.output as f64 * c.output)
                / 1_000_000.0
        })
    };
    let (s, l) = (price(side), price(lead));
    (s, l, l - s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_shell_table() {
        for c in [
            "ls -la",
            "git status && git diff",
            "cat a | grep b",
            "sed -n 1,5p f",
            "find . -name x",
            "FOO=bar echo $FOO",
            "X=ls; $X",
            "timeout 5 cat f",
            "rg foo 2>/dev/null",
            "oino-kanboard add x",
        ] {
            assert!(is_trivial_shell(c), "{c}");
        }
        for c in [
            "cargo test",
            "sed -i s/a/b/ f",
            "echo $(rm -rf x)",
            "cat > f",
            "find . -delete",
            "git commit -m x",
            "cat <<EOF",
            "$UNSET",
            "npm install",
        ] {
            assert!(!is_trivial_shell(c), "{c}");
        }
    }

    #[test]
    fn prompts_and_savings() {
        let id = Identity {
            lead_name: "Opus".into(),
            lead_effort: "medium".into(),
            sidekick_name: "GLM".into(),
            sidekick_effort: String::new(),
        };
        assert_eq!(
            identity_line(&id),
            "You are powered by Fusion (Opus Medium + GLM)."
        );
        let lp = lead_policy(&id);
        assert!(lp.starts_with("You are powered by Fusion (Opus Medium + GLM).\n\n## Sidekick\nYou have a `sidekick` tool"));
        assert!(sidekick_prompt(&id).contains("A lead agent (Opus) plans"));
        assert!(bash_nudge(4).contains("You have run 4 non-trivial shell commands"));
        assert!(EDIT_NUDGE.starts_with("<system_guidance>You made a direct edit"));
        let u = Usage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 0,
            cache_write: 0,
            cost: 0.3,
        };
        let lead = Price {
            input: 10.0,
            cached_input: 1.0,
            output: 50.0,
        };
        let (s, l, saved) = estimate_savings(&u, Some(&lead), None);
        assert!((s - 0.3).abs() < 1e-9 && (l - 60.0).abs() < 1e-9 && (saved - 59.7).abs() < 1e-9);
    }
}
