//! Summary source (unipi `source.ts`): the branch rebuilt from the full raw history; user words
//! told apart from extension-injected text by `compactor-origin` marks and known shapes.

use std::collections::BTreeSet;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::blocks::{Block, filter_noise, normalize_agent_message, one_line, text_of};
use crate::re::{self, regexes};

/// Origin mark entry type.
pub const ORIGIN_ENTRY_TYPE: &str = "compactor-origin";

regexes! {
    INJ_RALPH_BAR = r"^[─━=\-]{8,}\s*🔄 RALPH LOOP";
    INJ_RALPH = r"🔄 RALPH LOOP:";
    INJ_GUARD = r"^No-progress guard:";
    INJ_GOAL = r"^Continue working toward the active thread goal";
    INJ_GOAL2 = r"^Goal (?:recovery|status audit)\b";
    INJ_BUDGET = r"^The active goal reached a budget limit";
    INJ_KANBOARD = r"^\[kanboard [\w.-]+\]";
    INJ_TAG = r"^<[a-zA-Z][\w-]*(?:\s[^>]*)?>";
    PROGRESS_TOOLS = r"^(?:ralph_done|update_goal)$";
    ASK_TOOLS = r"(?i)^(?:ask_user|ask_user_question|askuserquestion|ask)$";
    ASK_TEXT = r"(?i)^User (?:wrote|selected|answered|chose)[^:]*:\s*([\s\S]+)$";
}

/// Stable key of a user message text.
#[must_use]
pub fn origin_key(text: &str) -> String {
    let n: String = one_line(text).chars().take(400).collect();
    Sha256::digest(n.as_bytes())
        .iter()
        .take(8)
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Text that extensions send with the user role.
#[must_use]
pub fn is_injected_user_text(text: &str, origins: &BTreeSet<String>) -> bool {
    let t = text.trim();
    if t.is_empty() || origins.contains(&origin_key(t)) {
        return true;
    }
    [
        &INJ_RALPH_BAR,
        &INJ_RALPH,
        &INJ_GUARD,
        &INJ_GOAL,
        &INJ_GOAL2,
        &INJ_BUDGET,
        &INJ_KANBOARD,
        &INJ_TAG,
    ]
    .iter()
    .any(|r| re::test(r, t))
}

fn short_question(q: &str) -> String {
    let one = one_line(q);
    if one.chars().count() > 140 {
        format!("{}…", one.chars().take(139).collect::<String>().trim_end())
    } else {
        one
    }
}

/// "Q → A" lines from an ask tool result.
#[must_use]
pub fn ask_answers(message: &Value) -> Vec<String> {
    let d = message.get("details");
    if let (Some(qs), Some(ans)) = (
        d.and_then(|d| d.get("questions")).and_then(Value::as_array),
        d.and_then(|d| d.get("answers")).and_then(Value::as_array),
    ) {
        let mut out = Vec::new();
        for (i, q) in qs.iter().enumerate() {
            let Some(a) = ans.get(i) else { continue };
            if a.get("skipped") == Some(&Value::Bool(true)) || a.is_null() {
                continue;
            }
            let mut parts: Vec<String> = a
                .get("selected")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|v| {
                    q.get("options")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .find(|o| {
                            o.get("value")
                                .or_else(|| o.get("label"))
                                .and_then(Value::as_str)
                                == Some(v)
                        })
                        .and_then(|o| o.get("label").and_then(Value::as_str))
                        .unwrap_or(v)
                        .to_owned()
                })
                .collect();
            if let Some(c) = a
                .get("custom_text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|c| !c.is_empty())
            {
                parts.push(c.to_owned());
            }
            let answer = parts.join(", ");
            if !answer.is_empty() {
                out.push(format!(
                    "{} → {answer}",
                    short_question(
                        q.get("question")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    )
                ));
            }
        }
        return out;
    }
    let text = text_of(message.get("content"));
    let Some(answer) = re::cap(&ASK_TEXT, text.trim(), 1) else {
        return Vec::new();
    };
    let answer = answer.trim().to_owned();
    match d
        .and_then(|d| d.get("question"))
        .and_then(Value::as_str)
        .map(short_question)
        .filter(|q| !q.is_empty())
    {
        Some(q) => vec![format!("{q} → {answer}")],
        None => vec![answer],
    }
}

/// What the lossless summary is built from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    /// Blocks (noise filtered).
    pub blocks: Vec<Block>,
    /// The user's own messages, oldest first.
    pub requests: Vec<String>,
    /// Assistant prose, oldest first.
    pub reports: Vec<String>,
    /// Blocks after the cut.
    pub tail: Vec<Block>,
}

/// Origin marks on the branch.
#[must_use]
pub fn collect_origins(branch: &[Value]) -> BTreeSet<String> {
    branch
        .iter()
        .filter(|e| {
            e.get("type").and_then(Value::as_str) == Some("custom")
                && e.get("customType").and_then(Value::as_str) == Some(ORIGIN_ENTRY_TYPE)
        })
        .filter_map(|e| {
            e.pointer("/data/key")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}

fn assistant_prose(m: &Value) -> String {
    match m.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned(),
        _ => String::new(),
    }
}

/// unipi `collectSummarySource` over branch entries `[0, end)`.
#[must_use]
pub fn collect_source(branch: &[Value], end: usize) -> Source {
    let origins = collect_origins(branch);
    let mut blocks = Vec::new();
    let mut requests = Vec::new();
    let mut reports = Vec::new();
    let end = end.min(branch.len());
    for (i, e) in branch.iter().enumerate().take(end) {
        let ty = e.get("type").and_then(Value::as_str);
        if ty == Some("branch_summary")
            && let Some(s) = e.get("summary").and_then(Value::as_str)
        {
            let flat: String = one_line(s).chars().take(600).collect();
            blocks.push(Block::Assistant {
                text: format!("[branch summary] {flat}"),
                source: Some(i),
            });
            continue;
        }
        if ty != Some("message") {
            continue;
        }
        let Some(m) = e.get("message") else { continue };
        let tool = m
            .get("toolName")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match m.get("role").and_then(Value::as_str) {
            Some("user") => {
                let raw = text_of(m.get("content")).trim().to_owned();
                if is_injected_user_text(&raw, &origins) {
                    continue;
                }
                requests.push(raw);
            }
            Some("assistant") => {
                let p = assistant_prose(m);
                if !p.is_empty() {
                    reports.push(p);
                }
            }
            Some("toolResult") if re::test(&ASK_TOOLS, tool) => requests.extend(ask_answers(m)),
            Some("toolResult") if re::test(&PROGRESS_TOOLS, tool) => {
                let t = text_of(m.get("content")).trim().to_owned();
                if !t.is_empty() {
                    reports.push(t);
                }
            }
            Some("custom") => continue,
            _ => {}
        }
        blocks.extend(normalize_agent_message(m, i));
    }
    let mut tail = Vec::new();
    for (i, e) in branch.iter().enumerate().skip(end) {
        if e.get("type").and_then(Value::as_str) == Some("message")
            && let Some(m) = e.get("message")
            && m.get("role").and_then(Value::as_str) != Some("custom")
        {
            tail.extend(normalize_agent_message(m, i));
        }
    }
    Source {
        blocks: filter_noise(blocks),
        requests,
        reports,
        tail: filter_noise(tail),
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
    use crate::cut::fixtures::*;

    #[test]
    fn all_compile() {
        for (n, r) in all_patterns() {
            assert!(r.is_some(), "{n}");
        }
    }

    #[test]
    fn injected() {
        let none = BTreeSet::new();
        for t in [
            "─────────────────────────────── 🔄 RALPH LOOP: site | Iteration 5/20",
            "No-progress guard: The same action has now run 3 times",
            "Continue working toward the active thread goal from the current conversation state.",
            "[kanboard PIT-3] Fix the header",
            "<background-task-notification> <task-id>b1</task-id> </background-task-notification>",
            "<session_resume events=\"1000\"> … </session_resume>",
        ] {
            assert!(is_injected_user_text(t, &none), "{t}");
        }
        assert!(!is_injected_user_text(
            "Please keep the orange theme",
            &none
        ));
        let text = "Some brand-new extension prompt with no known shape";
        let branch = vec![
            origin_mark(&origin_key(text)),
            user(text),
            user("my real request"),
        ];
        assert!(collect_origins(&branch).contains(&origin_key(text)));
        assert_eq!(
            collect_source(&branch, branch.len()).requests,
            ["my real request"]
        );
        let branch = vec![
            custom("unipi-compactor-resume", &"x".repeat(2000)),
            user("real"),
        ];
        let s = collect_source(&branch, branch.len());
        assert_eq!(s.requests, ["real"]);
        assert!(
            s.blocks
                .iter()
                .all(|b| b.kind() != "user" || b.text() == Some("real"))
        );
    }

    #[test]
    fn answers() {
        let m = json!({"role": "toolResult", "toolName": "ask_user", "content": [{"type": "text", "text": "User selected: soft-delete"}], "details": {"question": "How should user deletion work?"}});
        assert_eq!(
            ask_answers(&m),
            ["How should user deletion work? → soft-delete"]
        );
        let m = json!({"role": "toolResult", "toolName": "ask_user", "details": {
            "questions": [{"question": "Pick", "options": [{"label": "Alpha", "value": "a"}]}, {"question": "Skip me"}],
            "answers": [{"selected": ["a"], "custom_text": "also b"}, {"skipped": true}]}});
        assert_eq!(ask_answers(&m), ["Pick → Alpha, also b"]);
    }
}
