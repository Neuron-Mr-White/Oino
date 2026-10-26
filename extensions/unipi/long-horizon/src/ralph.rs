//! Ralph loop, todo list and runaway guard (unipi `engine/ralph.ts`, `tools/todo.ts`,
//! `engine/runaway.ts`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Task-file directory (project).
pub const RALPH_DIR: &str = ".oino/ralph";
/// Guest path of the task-file directory.
pub const RALPH_GUEST_DIR: &str = "/project/.oino/ralph";
/// unipi `RALPH_COMPLETE_MARKER` (never emitted manually).
pub const RALPH_COMPLETE_MARKER: &str = "<promise>COMPLETE</promise>";
const REFLECT: &str = include_str!("../assets/reflect.md");

/// unipi `LoopFileState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopState {
    /// Version (1).
    pub version: u32,
    /// Loop id.
    pub loop_id: String,
    /// Sanitised name.
    pub name: String,
    /// `<name>.md`.
    pub task_file: String,
    /// Iteration.
    pub iteration: u32,
    /// Max iterations (0 = none).
    pub max_iterations: u32,
    /// Items per iteration.
    pub items_per_iteration: u32,
    /// Reflection cadence.
    pub reflect_every: u32,
    /// `active | paused | complete`.
    pub status: String,
    /// Goal id.
    pub goal_id: String,
    /// Start (ISO).
    pub started_at: String,
}

/// A checklist item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Text.
    pub text: String,
    /// Checked.
    pub checked: bool,
}

/// unipi `parseChecklist` (`- [ ]` / `* [x]` lines).
#[must_use]
pub fn parse_checklist(markdown: &str) -> Vec<Item> {
    markdown
        .split('\n')
        .filter_map(|line| {
            let t = line.trim_start();
            let rest = t
                .strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))?
                .trim_start();
            let (mark, text) = (rest.get(..3)?, rest.get(3..)?);
            let checked = match mark {
                "[ ]" => false,
                "[x]" | "[X]" => true,
                _ => return None,
            };
            Some(Item {
                text: text.trim().to_owned(),
                checked,
            })
        })
        .collect()
}

/// unipi `sanitize`.
#[must_use]
pub fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t: String = out.trim_matches('-').chars().take(60).collect();
    if t.is_empty() { "loop".into() } else { t }
}

/// unipi `buildIterationPrompt`.
#[must_use]
pub fn iteration_prompt(
    state: &LoopState,
    markdown: &str,
    task_path: &str,
    reflection: bool,
) -> String {
    let items = parse_checklist(markdown);
    let pending: Vec<&Item> = items.iter().filter(|i| !i.checked).collect();
    let selected: Vec<&&Item> = if state.items_per_iteration > 0 {
        pending
            .iter()
            .take(state.items_per_iteration as usize)
            .collect()
    } else {
        pending.iter().collect()
    };
    let max = if state.max_iterations > 0 {
        format!("/{}", state.max_iterations)
    } else {
        String::new()
    };
    let rule = "───────────────────────────────────────────────────────────────────────";
    let mut parts = vec![
        format!(
            "{rule}\n🔄 RALPH LOOP: {} | Iteration {}{max}{}\n{rule}",
            state.name,
            state.iteration,
            if reflection { " | 🪞 REFLECTION" } else { "" }
        ),
        String::new(),
    ];
    if reflection {
        parts.push(REFLECT.to_owned());
        parts.push("\n---\n".into());
    }
    parts.push(format!(
        "## This iteration{}\n",
        if state.items_per_iteration > 0 {
            format!(" (≈{} items)", state.items_per_iteration)
        } else {
            String::new()
        }
    ));
    if selected.is_empty() {
        parts.push("(no unchecked items — verify the file, then claim completion)".into());
    } else {
        parts.extend(selected.iter().map(|i| format!("- [ ] {}", i.text)));
    }
    parts.push(format!(
        "\nTask file: {task_path} ({}/{} done)",
        items.len() - pending.len(),
        items.len()
    ));
    parts.push("\n## Instructions\n".into());
    parts.push("1. Work the items above; update the task file as you go (`- [x]`)".into());
    parts.push(if state.items_per_iteration > 0 {
        format!(
            "2. Then call ralph_done — approximately {} items per iteration",
            state.items_per_iteration
        )
    } else {
        "2. Then call ralph_done to advance the iteration".into()
    });
    parts.push("3. When EVERY item is checked and verified, call ralph_done one final time to claim completion".into());
    parts.join("\n")
}

/// `(checked, total, next)` (unipi `progressSummary`).
#[must_use]
pub fn progress(markdown: &str, per: u32) -> (usize, usize, Vec<String>) {
    let items = parse_checklist(markdown);
    let next = items
        .iter()
        .filter(|i| !i.checked)
        .take(if per > 0 { per as usize } else { usize::MAX })
        .map(|i| i.text.clone())
        .collect();
    (
        items.iter().filter(|i| i.checked).count(),
        items.len(),
        next,
    )
}

/// unipi todo item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    /// Content.
    pub content: String,
    /// `pending | in_progress | completed | cancelled`.
    pub status: String,
    /// `high | medium | low`.
    pub priority: String,
}

/// unipi `MAX_TODO_ITEMS`.
pub const MAX_TODO_ITEMS: usize = 50;

/// unipi `normalizeTodoItem` + `validateTodoList`.
///
/// # Errors
///
/// unipi's rejection reason.
pub fn validate_todos(raw: &Value) -> Result<Vec<Todo>, String> {
    let list = raw.as_array().ok_or("todos must be an array")?;
    let mut out = Vec::new();
    for item in list {
        let content = item
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .ok_or("every todo needs non-empty content")?;
        let status = item
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| matches!(*s, "pending" | "in_progress" | "completed" | "cancelled"))
            .ok_or("status must be one of pending, in_progress, completed, cancelled")?;
        let priority = item
            .get("priority")
            .and_then(Value::as_str)
            .filter(|p| matches!(*p, "high" | "medium" | "low"))
            .unwrap_or("medium");
        out.push(Todo {
            content: content.chars().take(200).collect(),
            status: status.into(),
            priority: priority.into(),
        });
    }
    if out.len() > MAX_TODO_ITEMS {
        return Err(format!("too many items ({} > {MAX_TODO_ITEMS})", out.len()));
    }
    if out.iter().filter(|t| t.status == "in_progress").count() > 1 {
        return Err("at most one item may be in_progress".into());
    }
    Ok(out)
}

/// unipi `renderTodoLine`.
#[must_use]
pub fn todo_line(list: &[Todo]) -> String {
    let total = list.iter().filter(|t| t.status != "cancelled").count();
    if total == 0 {
        return "no plan".into();
    }
    let done = list.iter().filter(|t| t.status == "completed").count();
    match list.iter().find(|t| t.status == "in_progress") {
        Some(c) => format!(
            "{done}/{total} · ▶ {}",
            c.content.chars().take(60).collect::<String>()
        ),
        None => format!("{done}/{total}"),
    }
}

/// One step for the runaway guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Tool.
    pub tool: String,
    /// Stable input fingerprint.
    pub input: String,
    /// Result text (first 400 chars).
    pub result: String,
    /// Error.
    pub is_error: bool,
    /// bash/powershell command.
    pub command: String,
}

/// unipi `ANTI_POISONING_SUFFIX`.
pub const ANTI_POISONING_SUFFIX: &str = "This is a temporary runtime reminder for the current Turn only, not a user preference or a durable rule; do not save this reminder or generalize it into Memory, Skills, or other persistent instruction files.";
const POLLING: &[&str] = &[
    "bg_status",
    "bg_logs",
    "read_subagent",
    "loop_status",
    "get_goal",
];

/// Canonical JSON (sorted keys; unipi `stableStringify`).
#[must_use]
pub fn stable(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|k| format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        stable(&o[*k])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Array(a) => format!("[{}]", a.iter().map(stable).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

fn exempt(s: &Step) -> bool {
    if s.tool == "get_goal" || s.tool == "update_goal" {
        return true;
    }
    if s.tool == "bash" || s.tool == "powershell" {
        let c = &s.command;
        if (c.contains("kanboard "))
            && [
                "list",
                "show",
                "search",
                "next",
                "chain",
                "attachments",
                "status",
            ]
            .iter()
            .any(|v| c.contains(&format!(" {v}")))
        {
            return true;
        }
        let search = ["rg ", "grep ", "findstr "].iter().any(|w| c.contains(w));
        let r = s.result.trim().to_lowercase();
        if s.is_error
            && search
            && (r.is_empty()
                || r.starts_with("no match")
                || r.contains("exit code 1")
                || r.contains("exit status 1"))
        {
            return true;
        }
    }
    false
}

/// unipi `errorFamily`.
#[must_use]
pub fn error_family(text: &str) -> String {
    let first = text.split('\n').next().unwrap_or_default();
    let mut out = String::new();
    let mut chars = first.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' {
            out.push_str("<path>");
            while chars
                .peek()
                .is_some_and(|n| n.is_alphanumeric() || matches!(n, '.' | '/' | '-' | '_'))
            {
                chars.next();
            }
        } else if c.is_ascii_digit() {
            out.push('N');
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out.chars().take(160).collect::<String>().trim().to_owned()
}

fn most(keys: &[String]) -> (String, usize) {
    let mut best = (String::new(), 0);
    for k in keys {
        let n = keys.iter().filter(|x| *x == k).count();
        if n > best.1 {
            best = (k.clone(), n);
        }
    }
    best
}

/// unipi `detectRunaway` → `(signal, occurrences)`.
#[must_use]
pub fn detect(all: &[Step], remind_after: usize) -> Option<(&'static str, usize)> {
    let steps: Vec<&Step> = all.iter().filter(|s| !exempt(s)).collect();
    if steps.len() < remind_after {
        return None;
    }
    let actions: Vec<String> = steps
        .iter()
        .map(|s| format!("{}:{}", s.tool, s.input))
        .collect();
    let results: Vec<String> = steps
        .iter()
        .map(|s| {
            format!(
                "{}:{}:{}",
                s.tool,
                s.input,
                s.result.chars().take(400).collect::<String>()
            )
        })
        .collect();
    let (akey, an) = most(&actions);
    if an >= remind_after {
        let idx = actions.iter().rposition(|k| *k == akey).unwrap_or(0);
        let (rkey, rn) = most(&results);
        return Some((
            if rn >= remind_after && rkey == results[idx] {
                "unchanged_progress_repeat"
            } else {
                "exact_action_repeat"
            },
            an,
        ));
    }
    let tail = &actions[actions.len().saturating_sub(8)..];
    if tail.len() >= 4 {
        let mut distinct: Vec<&String> = Vec::new();
        for k in tail {
            if !distinct.contains(&k) {
                distinct.push(k);
            }
        }
        if distinct.len() == 2 {
            let alternates = tail.windows(2).all(|w| w[0] != w[1]);
            let a = tail.iter().filter(|k| *k == distinct[0]).count();
            let b = tail.len() - a;
            if alternates && a + 1 >= remind_after && b + 1 >= remind_after {
                return Some(("abab_action_cycle", a.min(b) * 2));
            }
        }
    }
    let errors: Vec<String> = steps
        .iter()
        .filter(|s| s.is_error)
        .map(|s| error_family(&s.result))
        .collect();
    let (_, en) = most(&errors);
    if en >= remind_after {
        return Some(("same_error_family", en));
    }
    let (_, rn) = most(&results);
    if rn >= remind_after {
        return Some(("exact_result_repeat", rn));
    }
    let recent = &steps[steps.len().saturating_sub(remind_after + 2)..];
    let polling = recent
        .iter()
        .filter(|s| POLLING.contains(&s.tool.as_str()))
        .count();
    (polling >= remind_after).then_some(("polling_repeat", polling))
}

/// unipi `runawayReminder`.
#[must_use]
pub fn runaway_reminder(signal: &str, n: usize) -> String {
    let g = match signal {
        "exact_action_repeat" => format!("The same action has now run {n} times with identical arguments. Repetition is not progress: inspect the current state, change the approach, or surface the blocker to the user."),
        "abab_action_cycle" => format!("Two actions are alternating in a loop ({n} steps seen). Break the cycle: pick a materially different next action that changes the state you are operating on."),
        "unchanged_progress_repeat" => "Actions are repeating and producing identical results. Nothing is changing. Re-inspect the actual artifact or state, adjust the plan, or explain the blocker instead of retrying.".into(),
        "same_error_family" => format!("The same error family has occurred {n} times. Do not retry the failing call unchanged: read the error, change the inputs or approach, or ask for help with the specific blocker."),
        "exact_result_repeat" => format!("Identical results have returned {n} times. Treat the state as stable and act on it rather than re-checking."),
        _ => "Status-checking tools dominate recent steps. Polling does not advance the work: take an action that changes state, or end the turn and let the wake notification resume you.".into(),
    };
    format!("No-progress guard: {g} {ANTI_POISONING_SUFFIX}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn state() -> LoopState {
        LoopState {
            version: 1,
            loop_id: "l".into(),
            name: "chores".into(),
            task_file: "chores.md".into(),
            iteration: 1,
            max_iterations: 0,
            items_per_iteration: 2,
            reflect_every: 5,
            status: "active".into(),
            goal_id: "g".into(),
            started_at: String::new(),
        }
    }

    #[test]
    fn checklist_and_prompt() {
        let md = "# T\n- [ ] a\n  * [x] b\n- [X] c\n- [ ] d\n- [ ] e\n- not an item\n";
        let items = parse_checklist(md);
        assert_eq!(items.len(), 5);
        assert_eq!(progress(md, 2), (2, 5, vec!["a".into(), "d".into()]));
        let p = iteration_prompt(&state(), md, ".oino/ralph/chores.md", false);
        assert!(p.contains("🔄 RALPH LOOP: chores | Iteration 1\n"));
        assert!(p.contains("## This iteration (≈2 items)\n\n- [ ] a\n- [ ] d\n\nTask file: .oino/ralph/chores.md (2/5 done)"));
        assert!(iteration_prompt(&state(), md, "x", true).contains("REFLECTION CHECKPOINT"));
        assert_eq!(sanitize("My Loop!!"), "my-loop");
        assert_eq!(sanitize("***"), "loop");
    }

    #[test]
    fn todos() {
        let ok = validate_todos(&json!([{"content": " a ", "status": "in_progress"}, {"content": "b", "status": "completed", "priority": "high"}])).unwrap();
        assert_eq!(ok[0].priority, "medium");
        assert_eq!(todo_line(&ok), "1/2 · ▶ a");
        assert_eq!(validate_todos(&json!([{"content": "a", "status": "in_progress"}, {"content": "b", "status": "in_progress"}])).unwrap_err(), "at most one item may be in_progress");
        assert!(validate_todos(&json!([{"content": "", "status": "pending"}])).is_err());
    }

    fn step(tool: &str, input: &str, result: &str, err: bool) -> Step {
        Step {
            tool: tool.into(),
            input: input.into(),
            result: result.into(),
            is_error: err,
            command: String::new(),
        }
    }

    #[test]
    fn runaway_signals() {
        let same = vec![
            step("bash", "a", "x", false),
            step("bash", "a", "y", false),
            step("bash", "a", "z", false),
        ];
        assert_eq!(detect(&same, 3), Some(("exact_action_repeat", 3)));
        let unchanged = vec![step("bash", "a", "x", false); 3];
        assert_eq!(
            detect(&unchanged, 3).map(|d| d.0),
            Some("unchanged_progress_repeat")
        );
        let abab = vec![
            step("r", "1", "a", false),
            step("r", "2", "b", false),
            step("r", "1", "c", false),
            step("r", "2", "d", false),
        ];
        assert_eq!(detect(&abab, 3).map(|d| d.0), Some("abab_action_cycle"));
        let errs = vec![
            step("bash", "1", "error at /a/b line 3", true),
            step("bash", "2", "error at /c line 9", true),
            step("bash", "3", "error at /x line 1", true),
        ];
        assert_eq!(detect(&errs, 3).map(|d| d.0), Some("same_error_family"));
        assert_eq!(detect(&same[..2], 3), None);
        assert_eq!(error_family("fail /tmp/x.rs:12"), "fail <path>:N");
        assert!(
            runaway_reminder("polling_repeat", 3)
                .starts_with("No-progress guard: Status-checking tools")
        );
        assert_eq!(
            stable(&json!({"b": 1, "a": [true]})),
            "{\"a\":[true],\"b\":1}"
        );
    }
}
