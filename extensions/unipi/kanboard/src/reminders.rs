//! Progress reminders (unipi `reminders.ts`; no LLM, never blocking).
//!
//! When the agent works board tasks by hand ("do UNI-5 and UNI-8"), the board should show it:
//! `start <ID>` moves a task to In Progress for this session. R1: on every file-changing tool
//! call of an agent turn, each mentioned task that is still Todo and has not been reminded
//! this turn gets one annotation (at most twice per task). "Mentioned" = task ids in the
//! user's prompts plus ids the agent `show`ed; only ids that exist on the board count. R1 is
//! silent in child sessions and when `kanboard.reminders` is off.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::shapes::Task;
use crate::shell::{collapse_quotes, invocations, is_assignment, shell_segments};

/// Custom type of the R1 reminder.
pub const REMINDER_CUSTOM_TYPE: &str = "unipi:kanboard-reminder";
/// R1 reminders per task id.
pub const MAX_REMINDERS_PER_TASK: u32 = 2;

/// Same wording long-horizon's runaway guard uses: the reminder must not stick.
pub const ANTI_POISONING_SUFFIX: &str = "This is a temporary runtime reminder for the current Turn only, not a user preference or a durable rule; do not save this reminder or generalize it into Memory, Skills, or other persistent instruction files.";

/// Tools that change files. bash/powershell count unless the command only reads.
const EDIT_TOOLS: &[&str] = &[
    "edit",
    "write",
    "multi_edit",
    "multiedit",
    "apply_patch",
    "notebook_edit",
];
const SHELL_TOOLS: &[&str] = &["bash", "powershell"];

/// First words of shell segments that never change files.
const READ_COMMANDS: &[&str] = &[
    "ls", "pwd", "cat", "head", "tail", "wc", "echo", "printf", "which", "type", "rg", "grep",
    "egrep", "fgrep", "find", "fd", "stat", "file", "du", "df", "env", "printenv", "date",
    "whoami", "tree", "less", "more", "true", "false", "test", "[", "diff", "cmp", "jq",
    "realpath", "dirname", "basename", "uname", "hostname", "id", "ps", "cd",
];
/// Interpreters/package managers whose version-check arms never write.
const VERSION_CHECKED: &[&str] = &["node", "npm", "npx", "python", "python3"];
const GIT_READS: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "remote",
    "rev-parse",
    "blame",
    "ls-files",
    "describe",
    "tag",
];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Board task ids in `text` (`PREFIX-123`, prefix = an upper-case letter then up to 15
/// upper-case letters/digits), first occurrence order, deduplicated.
#[must_use]
pub fn task_ids_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || !is_word(chars[i - 1]);
        if !(boundary && chars[i].is_ascii_uppercase()) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len()
            && j - i <= 15
            && (chars[j].is_ascii_uppercase() || chars[j].is_ascii_digit())
        {
            j += 1;
        }
        let matched = (|| {
            if chars.get(j) != Some(&'-') {
                return None;
            }
            let mut k = j + 1;
            while k < chars.len() && chars[k].is_ascii_digit() {
                k += 1;
            }
            (k > j + 1 && (k == chars.len() || !is_word(chars[k]))).then_some(k)
        })();
        if let Some(end) = matched {
            let id: String = chars[i..end].iter().collect();
            if !out.contains(&id) {
                out.push(id);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// unipi's `(^|[^0-9&>])>{1,2}\s*(?!&|\/dev\/null)[^\s|;&]` over the quote-collapsed command,
/// including its backtracking (so `>> /dev/null` still counts, as it did in unipi).
fn has_write_redirect(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let tail_ok = |q: usize| -> bool {
        let rest: String = chars[q..].iter().collect();
        if rest.starts_with('&') || rest.starts_with("/dev/null") {
            return false;
        }
        chars
            .get(q)
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, '|' | ';' | '&'))
    };
    let try_at = |gt: usize| -> bool {
        for count in [2usize, 1] {
            if gt + count > n || chars[gt..gt + count].iter().any(|c| *c != '>') {
                continue;
            }
            let after = gt + count;
            let mut ws = 0;
            while after + ws < n && chars[after + ws].is_whitespace() {
                ws += 1;
            }
            for w in (0..=ws).rev() {
                if tail_ok(after + w) {
                    return true;
                }
            }
        }
        false
    };
    for start in 0..n {
        // `^` alternative: the `>` run starts the text.
        if start == 0 && chars[0] == '>' && try_at(0) {
            return true;
        }
        let c = chars[start];
        if !(c.is_ascii_digit() || c == '&' || c == '>')
            && start + 1 < n
            && chars[start + 1] == '>'
            && try_at(start + 1)
        {
            return true;
        }
    }
    false
}

/// Whether a shell command may change files. Conservative toward "yes": any segment that is
/// not a known read (or a kanboard CLI call) counts, and so does an output redirection.
#[must_use]
pub fn shell_changes_files(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    if has_write_redirect(&collapse_quotes(trimmed)) {
        return true;
    }
    shell_segments(trimmed)
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .any(|segment| {
            let words: Vec<&str> = segment
                .split_whitespace()
                .filter(|w| !is_assignment(w))
                .collect();
            let first = words
                .first()
                .map_or("", |w| w.rsplit('/').next().unwrap_or(w));
            if first.is_empty() {
                return false;
            }
            if first == crate::shell::BINARY || first == format!("{}.exe", crate::shell::BINARY) {
                return false;
            }
            if READ_COMMANDS.contains(&first) {
                return false;
            }
            let second = words.get(1).copied().unwrap_or("");
            match first {
                "git" => !GIT_READS.contains(&second),
                "sed" => words.iter().any(|w| w.starts_with("-i")),
                "sort" => words.iter().any(|w| {
                    w.strip_prefix('-').is_some_and(|rest| {
                        rest.split('-').next().is_some_and(|run| run.contains('o'))
                    }) || w.starts_with("--output")
                }),
                f if VERSION_CHECKED.contains(&f) => !words[1..]
                    .iter()
                    .any(|w| matches!(*w, "--version" | "-v" | "-V")),
                _ => true,
            }
        })
}

/// A tool call that changes files.
#[must_use]
pub fn is_file_changing_call(tool_name: &str, input: &Value) -> bool {
    if EDIT_TOOLS.contains(&tool_name) {
        return true;
    }
    if !SHELL_TOOLS.contains(&tool_name) {
        return false;
    }
    shell_changes_files(input.get("command").and_then(Value::as_str).unwrap_or(""))
}

/// The R1 annotation text.
#[must_use]
pub fn r1_text(todo: &[String], cli: Option<&str>) -> String {
    let ids = todo.join(", ");
    let verb = if todo.len() == 1 { "is" } else { "are" };
    let how = cli.map_or_else(
        || "`start <ID>`".to_owned(),
        |c| format!("`{c} start <ID>`"),
    );
    format!(
        "[kanboard] {ids} {verb} still Todo. {how} the one you're on before changing files (it moves it to In Progress for this session), and `finish <ID> --comment \"<summary>\"` when done. {ANTI_POISONING_SUFFIX}"
    )
}

/// Per-session R1 bookkeeping.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    mentioned: Vec<String>,
    started: Vec<String>,
    r1: HashMap<String, u32>,
    reminded_this_turn: HashSet<String>,
}

impl Tracker {
    /// A user prompt (or `-do` request): remember the task ids it names. Our own reminders
    /// name started tasks; they are not new mentions.
    pub fn on_prompt(&mut self, text: &str) {
        if text.contains(ANTI_POISONING_SUFFIX) {
            return;
        }
        for id in task_ids_in(text) {
            self.mention(id);
        }
    }

    fn mention(&mut self, id: String) {
        if !self.mentioned.contains(&id) {
            self.mentioned.push(id);
        }
    }

    /// A new agent turn: re-arm R1.
    pub fn on_turn_start(&mut self) {
        self.reminded_this_turn.clear();
    }

    /// Records `show`/`start` from a tool result and returns the R1 candidates when a
    /// reminder may be due (a file-changing call, not silent, something mentioned).
    pub fn observe(
        &mut self,
        tool_name: &str,
        input: &Value,
        is_error: bool,
        silent: bool,
    ) -> Option<Vec<String>> {
        if SHELL_TOOLS.contains(&tool_name) {
            let command = input.get("command").and_then(Value::as_str).unwrap_or("");
            for inv in invocations(command) {
                let Some(id) = inv.first_positional().map(ToOwned::to_owned) else {
                    continue;
                };
                if inv.sub == "show" {
                    self.mention(id.clone());
                }
                if inv.sub == "start" && !is_error && !self.started.contains(&id) {
                    self.started.push(id);
                }
            }
        }
        if !is_file_changing_call(tool_name, input) || silent || self.mentioned.is_empty() {
            return None;
        }
        let candidates: Vec<String> = self
            .mentioned
            .iter()
            .filter(|id| {
                !self.reminded_this_turn.contains(*id)
                    && self.r1.get(*id).copied().unwrap_or(0) < MAX_REMINDERS_PER_TASK
            })
            .cloned()
            .collect();
        (!candidates.is_empty()).then_some(candidates)
    }

    /// The candidates still Todo on the board; counts them as reminded. `None` = no reminder.
    pub fn remind(&mut self, candidates: &[String], tasks: &[Task]) -> Option<Vec<String>> {
        let todo: Vec<String> = candidates
            .iter()
            .filter(|id| tasks.iter().any(|t| &t.id == *id && t.status == "todo"))
            .cloned()
            .collect();
        if todo.is_empty() {
            return None;
        }
        for id in &todo {
            self.reminded_this_turn.insert(id.clone());
            *self.r1.entry(id.clone()).or_insert(0) += 1;
        }
        Some(todo)
    }

    /// Mentioned ids (tests / status).
    #[must_use]
    pub fn mentioned(&self) -> &[String] {
        &self.mentioned
    }

    /// Ids this session ran `start` on.
    #[must_use]
    pub fn started(&self) -> &[String] {
        &self.started
    }
}

/// `tool_result` override carrying the R1 annotation (unipi `harnessToolResultDetails`:
/// the annotation is appended to the content and recorded under
/// `details.unipiHarnessAnnotations`).
#[must_use]
pub fn r1_result(
    content: &Value,
    details: &Value,
    todo: &[String],
    cli: Option<&str>,
    id: &str,
) -> Value {
    let annotation = format!("\n\n{}", r1_text(todo, cli));
    let mut blocks = content.as_array().cloned().unwrap_or_default();
    blocks.push(json!({"type": "text", "text": annotation}));
    let mut details = match details {
        Value::Object(map) => map.clone(),
        _ => serde_json::Map::new(),
    };
    let mut list = details
        .get("unipiHarnessAnnotations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.push(json!({
        "meta": {
            "version": 1,
            "id": id,
            "delivery": "boundary",
            "source": "Kanboard",
            "title": "R1 progress reminder",
            "synopsis": format!("{} still Todo", todo.join(", ")),
            "severity": "warning",
        },
        "text": annotation,
    }));
    details.insert("unipiHarnessAnnotations".into(), Value::Array(list));
    json!({"content": blocks, "details": details})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: &str) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            status: status.into(),
            ..Task::default()
        }
    }

    fn bash(cmd: &str) -> Value {
        json!({"command": cmd})
    }

    #[test]
    fn finds_board_ids() {
        assert_eq!(
            task_ids_in("do UNI-5 and UNI-8, then UNI-5 again"),
            ["UNI-5", "UNI-8"]
        );
        assert_eq!(
            task_ids_in("K2-10 x-1 ab-2 AB-c A_B-1 (KB-3)."),
            ["K2-10", "KB-3"]
        );
        assert_eq!(
            task_ids_in("UNI-5x UNI- -5 ABCDEFGHIJKLMNOPQ-1"),
            [] as [std::string::String; 0]
        );
        assert_eq!(task_ids_in("ABCDEFGHIJKLMNOP-1"), ["ABCDEFGHIJKLMNOP-1"]);
    }

    #[test]
    fn classifies_file_changing_calls() {
        assert!(is_file_changing_call("edit", &json!({})));
        assert!(is_file_changing_call("write", &json!({})));
        assert!(!is_file_changing_call("read", &json!({})));
        for read in [
            "ls -la",
            "cat a | grep b",
            "git status",
            "git log --oneline",
            "rg x && wc -l f",
            "node --version",
            "npx tsx --version",
            "oino-kanboard start KB-1",
            "FOO=1 grep x f",
            "echo hi 2>&1",
            "cat f >/dev/null",
            r#"grep -c "a;b" f"#,
            r#"echo "a > b""#,
            "sort f",
            "sed -n 1p f",
            "/usr/bin/oino-kanboard list",
        ] {
            assert!(!shell_changes_files(read), "{read}");
        }
        for write in [
            "echo hi > f",
            "cat a >> b",
            "git commit -m x",
            "npm install",
            "node script.js",
            "sed -i s/a/b/ f",
            "sort -o out f",
            "sort --output=o f",
            "touch x",
            "ls; rm -rf x",
            "python3 build.py",
            "echo x >> /dev/null",
        ] {
            assert!(shell_changes_files(write), "{write}");
        }
        assert!(!shell_changes_files("  "));
        assert!(is_file_changing_call("bash", &bash("make")));
        assert!(!is_file_changing_call("bash", &bash("ls")));
    }

    #[test]
    fn fires_once_per_turn_naming_mentioned_todo_ids() {
        let mut t = Tracker::default();
        t.on_prompt("please do KB-1 and KB-2");
        let board = [task("KB-1", "todo"), task("KB-2", "todo")];
        let c = t.observe("edit", &json!({}), false, false).unwrap();
        assert_eq!(t.remind(&c, &board).unwrap(), ["KB-1", "KB-2"]);
        assert!(
            t.observe("edit", &json!({}), false, false).is_none(),
            "once per turn"
        );
        t.on_turn_start();
        let c = t.observe("edit", &json!({}), false, false).unwrap();
        assert_eq!(t.remind(&c, &board).unwrap(), ["KB-1", "KB-2"]);
        t.on_turn_start();
        assert!(
            t.observe("edit", &json!({}), false, false).is_none(),
            "per-task cap"
        );
    }

    #[test]
    fn keeps_nudging_the_unstarted_task() {
        let mut t = Tracker::default();
        t.on_prompt("KB-1 then KB-2");
        t.observe("bash", &bash("oino-kanboard start KB-1"), false, false);
        assert_eq!(t.started(), ["KB-1"]);
        let board = [task("KB-1", "in_progress"), task("KB-2", "todo")];
        let c = t.observe("edit", &json!({}), false, false).unwrap();
        assert_eq!(t.remind(&c, &board).unwrap(), ["KB-2"]);
    }

    #[test]
    fn silent_cases() {
        let mut t = Tracker::default();
        assert!(
            t.observe("edit", &json!({}), false, false).is_none(),
            "no mentions"
        );
        t.on_prompt("KB-1");
        assert!(
            t.observe("edit", &json!({}), false, true).is_none(),
            "silent"
        );
        let c = t.observe("edit", &json!({}), false, false).unwrap();
        assert!(t.remind(&c, &[task("KB-1", "done")]).is_none(), "non-todo");
        assert!(t.remind(&c, &[]).is_none(), "unknown id");
    }

    #[test]
    fn own_reminders_are_not_mentions_and_shows_are() {
        let mut t = Tracker::default();
        t.on_prompt(&r1_text(&["KB-9".into()], None));
        assert_eq!(t.mentioned(), [] as [String; 0]);
        t.observe(
            "bash",
            &bash("oino-kanboard --json show KB-4"),
            false,
            false,
        );
        assert_eq!(t.mentioned(), ["KB-4"]);
    }

    #[test]
    fn r1_text_and_result() {
        let text = r1_text(&["KB-1".into()], Some("/b --actor agent --project p"));
        assert!(text.starts_with(
            "[kanboard] KB-1 is still Todo. `/b --actor agent --project p start <ID>`"
        ));
        assert!(text.ends_with(ANTI_POISONING_SUFFIX));
        assert!(
            r1_text(&["A-1".into(), "A-2".into()], None)
                .contains("A-1, A-2 are still Todo. `start <ID>`")
        );
        let out = r1_result(
            &json!([{"type": "text", "text": "ok"}]),
            &json!({"unipiHarnessAnnotations": [{"x": 1}], "keep": true}),
            &["KB-1".into()],
            None,
            "r1-1",
        );
        assert!(
            out["content"][1]["text"]
                .as_str()
                .unwrap()
                .starts_with("\n\n[kanboard]")
        );
        assert_eq!(out["details"]["keep"], true);
        assert_eq!(
            out["details"]["unipiHarnessAnnotations"][1]["meta"]["synopsis"],
            "KB-1 still Todo"
        );
    }
}
