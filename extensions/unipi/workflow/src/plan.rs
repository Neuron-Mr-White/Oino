//! Plan mode, pure parts (unipi `src/plan/{state,enforce,index,review}.ts`): plan file naming,
//! session state restore, enforcement, instructions, the `plan_submit` gates, the review split.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::bash::{BashKind, classify_bash};
use crate::pathx::{is_inside, resolve};

/// Custom entry holding `{active, planFile}`.
pub const PLAN_STATE_ENTRY: &str = "unipi:plan-mode";
/// Custom message type of plan-mode notes and reminders.
pub const PLAN_MESSAGE_TYPE: &str = "unipi:plan-mode-message";
/// The tool.
pub const PLAN_TOOL: &str = "plan_submit";

/// Review choice labels (documented order).
pub const APPROVE: &str = "Approve & implement";
/// Keep planning.
pub const KEEP: &str = "Keep planning…";
/// Discard.
pub const DISCARD: &str = "Discard plan";

/// Per-session plan state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanState {
    /// Plan mode on.
    pub active: bool,
    /// Absolute host path of the plan file.
    pub plan_file: Option<String>,
    /// Hash of the plan when the user last chose "Keep planning" (not persisted).
    pub last_kept_hash: Option<String>,
}

/// `<YYYY-MM-DD>-<short-session-id>.md` (UTC date).
#[must_use]
pub fn plan_file_name(session_id: &str, now_ms: i64) -> String {
    let date = unipi_kit::fmt::iso_from_ms(now_ms);
    let short: String = session_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    let short = if short.is_empty() {
        "session".to_owned()
    } else {
        short
    };
    format!("{}-{short}.md", &date[..10])
}

/// `<cwd>/docs/plans/<name>`.
#[must_use]
pub fn plan_file_path(cwd: &str, session_id: &str, now_ms: i64) -> String {
    resolve(
        cwd,
        &format!("docs/plans/{}", plan_file_name(session_id, now_ms)),
    )
}

/// Rebuilds the state from the session's custom entries (last entry wins).
#[must_use]
pub fn restore(entries: &[Value], cwd: &str, session_id: &str, now_ms: i64) -> PlanState {
    let mut s = PlanState::default();
    for e in entries {
        if e.get("customType").and_then(Value::as_str) != Some(PLAN_STATE_ENTRY) {
            continue;
        }
        let data = e.get("data");
        s.active = data.and_then(|d| d.get("active")) == Some(&Value::Bool(true));
        s.plan_file = data
            .and_then(|d| d.get("planFile"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    if s.active && s.plan_file.is_none() {
        s.plan_file = Some(plan_file_path(cwd, session_id, now_ms));
    }
    s
}

/// Plan path as shown in messages (relative to `cwd` when inside it).
#[must_use]
pub fn display_plan_path(cwd: &str, plan_file: Option<&str>) -> String {
    match plan_file {
        None => format!("{}/docs/plans", cwd.trim_end_matches('/')),
        Some(f) => f
            .strip_prefix(cwd)
            .and_then(|r| r.strip_prefix('/'))
            .map_or_else(|| f.to_owned(), ToOwned::to_owned),
    }
}

/// Enforcement outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enforcement {
    /// Hard block.
    Block(String),
    /// Ask first.
    Ask(String),
}

/// The write block reason.
#[must_use]
pub fn write_block_reason(plan_file: Option<&str>, cwd: &str) -> String {
    format!(
        "Plan mode: only the plan file ({}), docs/plans/ and temp files may be written. Put this change in the plan instead. If a route is denied twice, record the uncertainty in the plan rather than retrying.",
        display_plan_path(cwd, plan_file)
    )
}

/// Plan-mode enforcement (runs before the permission gate).
#[must_use]
pub fn enforce(
    tool: &str,
    subject: &str,
    state: &PlanState,
    cwd: &str,
    tmpdir: &str,
) -> Option<Enforcement> {
    if !state.active {
        return None;
    }
    if tool == "write" || tool == "edit" {
        let target = resolve(cwd, subject);
        if state
            .plan_file
            .as_deref()
            .is_some_and(|f| target == resolve("/", f))
        {
            return None;
        }
        if is_inside(&target, &resolve(cwd, "docs/plans")) || is_inside(&target, tmpdir) {
            return None;
        }
        return Some(Enforcement::Block(write_block_reason(
            state.plan_file.as_deref(),
            cwd,
        )));
    }
    if tool == "bash" {
        let v = classify_bash(subject);
        if matches!(v.kind, BashKind::ReadOnly | BashKind::Kanboard) {
            return None;
        }
        return Some(Enforcement::Ask(format!(
            "plan mode · may change state: {}",
            v.reason
        )));
    }
    None
}

/// Instructions sent when plan mode turns on.
#[must_use]
pub fn instructions(plan_path: &str) -> String {
    [
        "Plan mode is ON — plan with the user, do not implement.".to_owned(),
        "Investigate freely: read, search, research, read-only shell, run existing tests/builds to learn current behaviour, delegate research-only tasks. Do not edit source files, change git state, install packages, or send/publish anything. The harness blocks writes outside the plan file, docs/plans/ and temp dirs, and asks the user before state-changing shell. If something is denied, take another route; after two denials, record the uncertainty in the plan.".to_owned(),
        "Clarifying is part of planning: when a decision that shapes the plan cannot be settled from evidence (a real trade-off or a user preference), ask the user with ask_user instead of guessing. Keep planning across turns until every such question is answered. Do not call plan_submit in the same turn you first draft the plan unless nothing is left to ask.".to_owned(),
        format!("Write the plan to {plan_path} with exactly these two top-level sections:"),
        "## Summary".to_owned(),
        "For the human approving it. One-sentence answer first (no heading above it inside the section), then short numbered points: what changes and why, what the user will notice, decisions or risks that need them. Product level, plain words, at most three code names, no file lists or line numbers. 5–15 lines.".to_owned(),
        "## Implementation".to_owned(),
        "For whoever implements it: ### Steps, ### Files, ### Risks, ### Verification, ### Open questions (write \"None\" when nothing is open).".to_owned(),
        "Call plan_submit only when ### Open questions is \"None\" and the user has had the chance to settle every shaping decision. plan_submit refuses while open questions remain.".to_owned(),
    ]
    .join("\n")
}

/// The per-turn hidden reminder.
#[must_use]
pub fn reminder(plan_path: &str) -> String {
    [
        format!("[plan mode on — plan file {plan_path}]"),
        "Investigate freely; no source edits, git state changes, installs or publishing.".into(),
        "Settle shaping decisions with ask_user; keep planning until they are answered.".into(),
        "Call plan_submit only when ### Open questions says \"None\".".into(),
    ]
    .join("\n")
}

/// Content hash used by the keep-planning gate.
#[must_use]
pub fn content_hash(content: &str) -> String {
    Sha256::digest(content.as_bytes())
        .iter()
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn is_open_questions_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(2..=3).contains(&hashes) {
        return false;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let t = rest.trim().to_lowercase();
    let t = t.strip_suffix(':').unwrap_or(&t).trim_end();
    let mut words = t.split_whitespace();
    words.next() == Some("open") && words.next() == Some("questions") && words.next().is_none()
}

fn strip_list_marker(s: &str) -> &str {
    for m in ['-', '*', '•'] {
        if let Some(r) = s.strip_prefix(m)
            && r.starts_with(char::is_whitespace)
        {
            return r.trim_start();
        }
    }
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let r = &s[digits..];
        if let Some(r) = r.strip_prefix('.').or_else(|| r.strip_prefix(')'))
            && r.starts_with(char::is_whitespace)
        {
            return r.trim_start();
        }
    }
    s
}

fn is_none_item(s: &str) -> bool {
    let l = s.to_lowercase();
    let l = l.strip_suffix('.').unwrap_or(&l);
    matches!(l, "none" | "n/a" | "-")
}

/// Items under `## Open questions` / `### Open questions` (markers stripped, "None" dropped).
#[must_use]
pub fn open_questions(plan: &str) -> Vec<String> {
    let lines: Vec<&str> = plan.split('\n').collect();
    let Some(start) = lines.iter().position(|l| is_open_questions_heading(l)) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for line in &lines[start + 1..] {
        if line.starts_with('#') {
            break;
        }
        let text = strip_list_marker(line.trim()).trim();
        if text.is_empty() || is_none_item(text) {
            continue;
        }
        items.push(text.to_owned());
    }
    items
}

/// The `plan_submit` refusal, or `None` when the plan may go to approval.
#[must_use]
pub fn submit_refusal(content: &str, last_kept_hash: Option<&str>) -> Option<String> {
    let open = open_questions(content);
    if !open.is_empty() {
        let list: Vec<String> = open.iter().map(|q| format!("- {q}")).collect();
        return Some(format!(
            "Open questions remain:\n{}\nSettle them with ask_user first, or move items that can safely wait to ### Risks.",
            list.join("\n")
        ));
    }
    if last_kept_hash.is_some_and(|h| h == content_hash(content)) {
        return Some("The plan has not changed since the user chose \"Keep planning\". Revise the plan (or ask_user about the feedback) before submitting again.".into());
    }
    None
}

/// Summary / implementation halves of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sections {
    /// Everything up to `## Implementation` (title kept, `## Summary` heading dropped).
    pub summary: String,
    /// After `## Implementation`.
    pub implementation: String,
}

fn is_heading(line: &str, word: &str) -> bool {
    line.strip_prefix("##")
        .filter(|r| r.starts_with([' ', '\t']))
        .is_some_and(|r| r.trim_matches([' ', '\t']) == word)
}

/// unipi `splitPlan`.
#[must_use]
pub fn split_plan(plan: &str) -> Option<Sections> {
    let lines: Vec<&str> = plan.split('\n').collect();
    let mut s = None;
    let mut im = None;
    for (i, l) in lines.iter().enumerate() {
        if is_heading(l, "Summary") {
            s = Some(i);
        }
        if is_heading(l, "Implementation") {
            im = Some(i);
        }
    }
    let (s, im) = (s?, im?);
    if im <= s {
        return None;
    }
    let trim = |t: String| t.trim_start_matches('\n').trim_end().to_owned();
    let mut summary: Vec<&str> = lines[..s].to_vec();
    summary.extend_from_slice(&lines[s + 1..im]);
    Some(Sections {
        summary: trim(summary.join("\n")),
        implementation: trim(lines[im + 1..].join("\n")),
    })
}

/// The review dialog (`custom`): the plan as markdown (side by side when both sections exist)
/// and the three decisions.
#[must_use]
pub fn review_dialog(plan: &str, path: &str) -> Value {
    let body = match split_plan(plan) {
        Some(sec) => {
            serde_json::json!({"t": "split", "direction": "horizontal", "ratios": [0.4, 0.6], "fallback": {"t": "markdown", "value": plan}, "children": [
                {"t": "section", "title": "Summary", "body": {"t": "markdown", "value": sec.summary}},
                {"t": "section", "title": "Implementation", "body": {"t": "markdown", "value": sec.implementation}},
            ]})
        }
        None => serde_json::json!({"t": "markdown", "value": plan}),
    };
    serde_json::json!({
        "method": "custom",
        "title": format!("Review plan  {path}"),
        "size": "large",
        "spec": {"t": "stack", "direction": "vertical", "gap": 1, "children": [
            body,
            {"t": "stack", "direction": "horizontal", "gap": 2, "children": [
                {"t": "button", "label": format!("1 {APPROVE}"), "actionId": "approve", "primary": true, "tone": "accent"},
                {"t": "button", "label": format!("2 {KEEP}"), "actionId": "keep"},
                {"t": "button", "label": format!("3 {DISCARD}"), "actionId": "discard", "tone": "error"},
            ]},
            {"t": "keyHints", "hints": [{"keys": "←→", "label": "choose"}, {"keys": "enter", "label": "confirm"}, {"keys": "esc", "label": "keep planning"}]},
        ]},
    })
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

    const DAY: i64 = 1_790_244_000_000; // 2026-09-24T10:00:00Z

    fn active(dir: &str) -> PlanState {
        PlanState {
            active: true,
            plan_file: Some(plan_file_path(dir, "session-1", DAY)),
            last_kept_hash: None,
        }
    }

    #[test]
    fn naming() {
        assert_eq!(
            plan_file_name("2f1a9c34-8b7e-4f0d-9a11-000000000000", DAY),
            "2026-09-24-2f1a9c34.md"
        );
        assert_eq!(
            plan_file_path("/w", "abcdef123456", DAY),
            "/w/docs/plans/2026-09-24-abcdef12.md"
        );
        assert_eq!(plan_file_name("---", DAY), "2026-09-24-session.md");
    }

    #[test]
    fn enforcement() {
        let dir = "/w/p";
        let s = active(dir);
        let file = s.plan_file.clone().unwrap_or_default();
        assert_eq!(enforce("write", &file, &s, dir, "/tmp"), None);
        assert_eq!(enforce("edit", &file, &s, dir, "/tmp"), None);
        assert_eq!(
            enforce("write", "/w/p/docs/plans/other.md", &s, dir, "/tmp"),
            None
        );
        assert_eq!(enforce("write", "/tmp/notes.md", &s, dir, "/tmp"), None);
        let rel = file.strip_prefix("/w/p/").unwrap_or_default();
        assert_eq!(enforce("write", rel, &s, dir, "/tmp"), None);
        for target in ["/w/p/src/foo.ts", "package.json"] {
            match enforce("write", target, &s, dir, "/tmp") {
                Some(Enforcement::Block(r)) => {
                    assert!(r.contains("only the plan file"));
                    assert!(r.contains("docs/plans"));
                    assert!(r.contains("denied twice"));
                    assert!(r.contains("docs/plans/2026-09-24-session1.md"));
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(enforce("bash", "git status && ls", &s, dir, "/tmp"), None);
        assert_eq!(
            enforce("bash", "unipi-kanboard --actor agent list", &s, dir, "/tmp"),
            None
        );
        for cmd in [
            "npm test",
            "npm install",
            "rm -rf x",
            "echo hi > out.txt",
            "cat $(ls)",
        ] {
            assert!(
                matches!(enforce("bash", cmd, &s, dir, "/tmp"), Some(Enforcement::Ask(r)) if r.starts_with("plan mode · may change state")),
                "{cmd}"
            );
        }
        for tool in ["memory_store", "run_subagent", "read", "plan_submit"] {
            assert_eq!(enforce(tool, "{}", &s, dir, "/tmp"), None);
        }
        let off = PlanState::default();
        assert_eq!(enforce("bash", "rm -rf /", &off, dir, "/tmp"), None);
    }

    #[test]
    fn persistence() {
        let file = plan_file_path("/w", "sess-9", DAY);
        let s = restore(
            &[json!({"customType": PLAN_STATE_ENTRY, "data": {"active": true, "planFile": file}})],
            "/w",
            "sess-9",
            DAY,
        );
        assert!(s.active);
        assert_eq!(s.plan_file.as_deref(), Some(file.as_str()));
        let s = restore(
            &[
                json!({"customType": PLAN_STATE_ENTRY, "data": {"active": true, "planFile": file}}),
                json!({"customType": PLAN_STATE_ENTRY, "data": {"active": false, "planFile": file}}),
            ],
            "/w",
            "sess-9",
            DAY,
        );
        assert!(!s.active);
        let s = restore(
            &[json!({"customType": PLAN_STATE_ENTRY, "data": {"active": true}})],
            "/w",
            "sess-9",
            DAY,
        );
        assert_eq!(s.plan_file.as_deref(), Some(file.as_str()));
        let s = restore(
            &[json!({"customType": "other", "data": {"active": true}})],
            "/w",
            "a",
            DAY,
        );
        assert!(!s.active);
    }

    #[test]
    fn submit_gates() {
        assert!(open_questions("## Implementation\n### Open questions\nNone\n").is_empty());
        assert!(open_questions("# Plan\n## Summary\nDo it.\n").is_empty());
        assert_eq!(
            open_questions(
                "## Implementation\n### Open questions\n- Postgres or sqlite?\n1. Which vendor?\n### Risks\n- heat"
            ),
            ["Postgres or sqlite?", "Which vendor?"]
        );
        assert_eq!(
            open_questions("## Open questions\n- one?\n## Verification\n- not a question"),
            ["one?"]
        );
        assert_eq!(
            open_questions("### Open questions\n- one?\n#### Details\n- elsewhere"),
            ["one?"]
        );
        assert!(open_questions("### OPEN QUESTIONS\nNone\n").is_empty());
        assert!(open_questions("### Open Questions:\nN/A\n-\n").is_empty());
        let refusal = submit_refusal(
            "## Implementation\n### Open questions\n- pick a vendor\n",
            None,
        )
        .unwrap_or_default();
        assert!(refusal.contains("Open questions remain"));
        assert!(refusal.contains("- pick a vendor"));
        assert!(refusal.contains("ask_user"));
        let done = "## Summary\nDo it.\n## Implementation\n### Open questions\nNone";
        assert_eq!(submit_refusal(done, None), None);
        let h = content_hash(done);
        assert!(
            submit_refusal(done, Some(&h))
                .unwrap_or_default()
                .contains("Keep planning")
        );
        assert_eq!(
            submit_refusal("## Summary\nDo it differently.", Some(&h)),
            None
        );
        assert_ne!(content_hash("a"), content_hash("b"));
    }

    #[test]
    fn messages_and_split() {
        let t = instructions("docs/plans/2026-09-24-abcd1234.md");
        for needle in [
            "2026-09-24-abcd1234.md",
            "## Summary",
            "## Implementation",
            "### Open questions",
            "ask_user",
            "plan_submit",
            "blocks writes outside the plan file",
            "asks the user before state-changing shell",
        ] {
            assert!(t.contains(needle), "{needle}");
        }
        let r = reminder("docs/plans/x.md");
        assert_eq!(r.lines().count(), 4);
        assert!(r.starts_with("[plan mode"));
        let sec = split_plan("# Title\n## Summary\nshort\n## Implementation\n### Steps\n- a\n");
        assert_eq!(
            sec,
            Some(Sections {
                summary: "# Title\nshort".into(),
                implementation: "### Steps\n- a".into()
            })
        );
        assert_eq!(split_plan("## Implementation\nx\n## Summary\ny"), None);
        let d = review_dialog("plain", "p.md");
        assert_eq!(d["method"], "custom");
        assert_eq!(
            d["spec"]["children"][1]["children"][0]["actionId"],
            "approve"
        );
    }
}
