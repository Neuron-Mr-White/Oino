//! Timeline presentation (design.md §4.9–§4.10): one-line tool summaries (verb · subject ·
//! meta), "Explored N files" grouping of read-only runs, per-turn file change summaries,
//! subagent rows and query-rail excerpts. Pure functions over transcript rows.

use oino_ai::{JsonObject, JsonValue};

use crate::model::tools::{self, ToolKind};
use crate::model::transcript::{Row, RowKind, ToolRow, ToolStatus};

/// Minimum read-only calls in a row before they collapse into "Explored N files".
pub const EXPLORE_MIN: usize = 4;

/// The subject of a tool row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    /// A file: basename + directory.
    Path {
        /// Basename.
        name: String,
        /// Directory (with trailing `/`), may be empty.
        dir: String,
    },
    /// A command line (mono).
    Command(String),
    /// Free text.
    Text(String),
    /// Nothing.
    None,
}

/// One-line summary of a tool row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLine {
    /// Verb (`Read`, `Edited`, `Ran`, …).
    pub verb: String,
    /// Subject.
    pub subject: Subject,
    /// Trailing meta (`+8 −3`, `3.4s`, `14 matches`).
    pub meta: Option<String>,
}

fn s<'a>(args: &'a JsonObject, key: &str) -> Option<&'a str> {
    args.get(key).and_then(JsonValue::as_str)
}

/// Splits a path into `(basename, dir/)`.
#[must_use]
pub fn split_path(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind(['/', '\\']) {
        Some(i) => (trimmed[i + 1..].to_owned(), trimmed[..=i].to_owned()),
        None => (trimmed.to_owned(), String::new()),
    }
}

fn path_subject(path: Option<&str>) -> Subject {
    match path {
        Some(p) if !p.is_empty() => {
            let (name, dir) = split_path(p);
            Subject::Path { name, dir }
        }
        _ => Subject::None,
    }
}

/// Human duration (`120ms`, `3.4s`, `2m05s`).
#[must_use]
pub fn duration(ms: u64) -> String {
    match ms {
        0..1000 => format!("{ms}ms"),
        1000..60_000 => format!("{}.{}s", ms / 1000, (ms % 1000) / 100),
        _ => format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1000),
    }
}

fn verb(name: &str, kind: ToolKind, done: bool) -> String {
    let (running, finished) = match (kind, name) {
        (ToolKind::Read, _) => ("Reading", "Read"),
        (ToolKind::Write, _) => ("Writing", "Wrote"),
        (ToolKind::Edit, _) => ("Editing", "Edited"),
        (ToolKind::Shell, _) => ("Running", "Ran"),
        (ToolKind::Search, "ls") => ("Listing", "Listed"),
        (ToolKind::Search, _) => ("Searching", "Searched"),
        (_, n) if n.contains("fetch") => ("Fetching", "Fetched"),
        (_, n) if n.contains("search") => ("Searching", "Searched web"),
        (_, "ask_user") => ("Asking", "Asked"),
        (ToolKind::Agent, _) => ("Delegating", "Delegated"),
        (ToolKind::Other, n) => return tool_label(n),
    };
    (if done { finished } else { running }).to_owned()
}

/// `mcp__server__tool` → `server · tool`; `snake_case` → Title case.
#[must_use]
pub fn tool_label(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("mcp__")
        && let Some((server, tool)) = rest.split_once("__")
    {
        return format!("{server} · {tool}");
    }
    let mut out = name.replace('_', " ");
    if let Some(first) = out.get(..1) {
        let up = first.to_uppercase();
        out.replace_range(..1, &up);
    }
    out
}

fn count_lines(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// The one-line summary of `tool`.
#[must_use]
pub fn tool_line(tool: &ToolRow) -> ToolLine {
    let kind = ToolKind::of(&tool.name);
    let done = matches!(tool.status, ToolStatus::Done | ToolStatus::Error);
    let args = &tool.args;
    let path = s(args, "path").or_else(|| s(args, "file_path"));
    let subject = match kind {
        ToolKind::Read | ToolKind::Write | ToolKind::Edit => path_subject(path),
        ToolKind::Shell => Subject::Command(
            s(args, "command")
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
        ),
        ToolKind::Search => match s(args, "pattern") {
            Some(p) if !p.is_empty() => Subject::Text(p.to_owned()),
            _ => path_subject(path.or(Some("."))),
        },
        ToolKind::Agent | ToolKind::Other => ["url", "query", "task", "prompt", "message", "title"]
            .iter()
            .find_map(|k| s(args, k))
            .map_or(Subject::None, |t| {
                Subject::Text(t.lines().next().unwrap_or_default().to_owned())
            }),
    };
    let meta = match (kind, tool.status) {
        (ToolKind::Edit | ToolKind::Write, ToolStatus::Preparing) => {
            tools::args_preview(&tool.name, args)
        }
        (ToolKind::Edit | ToolKind::Write, _) => tool.patch().map(|p| {
            let d = tools::diff_preview(&p, 0);
            format!("+{} −{}", d.added, d.removed)
        }),
        (ToolKind::Search, ToolStatus::Done) => {
            let n = count_lines(&tool.output);
            Some(format!(
                "{n} {}",
                if tool.name == "grep" {
                    "matches"
                } else {
                    "results"
                }
            ))
        }
        _ => tool.duration_ms.filter(|ms| *ms >= 100).map(duration),
    };
    ToolLine {
        verb: verb(&tool.name, kind, done),
        subject,
        meta,
    }
}

/// Whether a tool is read-only (groups into "Explored").
#[must_use]
pub fn is_read_only(name: &str) -> bool {
    matches!(name, "read" | "grep" | "find" | "ls")
}

fn transparent(row: &Row) -> bool {
    matches!(&row.kind, RowKind::Assistant(a) if a.text.trim().is_empty() && a.thinking.is_empty() && a.error.is_none())
}

fn explorer(row: &Row) -> bool {
    matches!(&row.kind, RowKind::Tool(t) if is_read_only(&t.name) && t.parent.is_none())
}

/// The read-only run containing row `ix`: `(first row, last row, tool count)` when it has at
/// least [`EXPLORE_MIN`] calls. Blank assistant rows inside the run are transparent.
#[must_use]
pub fn explore_run(rows: &[Row], ix: usize) -> Option<(usize, usize, usize)> {
    let row = rows.get(ix)?;
    if !explorer(row) && !transparent(row) {
        return None;
    }
    let mut start = ix;
    while start > 0 && (explorer(&rows[start - 1]) || transparent(&rows[start - 1])) {
        start -= 1;
    }
    let mut end = ix;
    while end + 1 < rows.len() && (explorer(&rows[end + 1]) || transparent(&rows[end + 1])) {
        end += 1;
    }
    while start < end && !explorer(&rows[start]) {
        start += 1;
    }
    while end > start && !explorer(&rows[end]) {
        end -= 1;
    }
    let count = rows[start..=end].iter().filter(|r| explorer(r)).count();
    (count >= EXPLORE_MIN && (start..=end).contains(&ix)).then_some((start, end, count))
}

/// Label of an explore group (`Explored 6 files`, `Exploring 3 files`).
#[must_use]
pub fn explore_label(rows: &[Row], start: usize, end: usize) -> String {
    let tools: Vec<&ToolRow> = rows[start..=end]
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::Tool(t) => Some(t.as_ref()),
            _ => None,
        })
        .collect();
    let running = tools
        .iter()
        .any(|t| matches!(t.status, ToolStatus::Preparing | ToolStatus::Running));
    let verb = if running { "Exploring" } else { "Explored" };
    let n = tools.len();
    format!("{verb} {n} {}", if n == 1 { "file" } else { "files" })
}

/// One changed file in a turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// Path as given to the tool.
    pub path: String,
    /// Added lines.
    pub added: usize,
    /// Removed lines.
    pub removed: usize,
    /// Patches (for the Review tab).
    pub patches: Vec<String>,
}

/// Files changed by edit/write calls in rows `range`.
#[must_use]
pub fn file_changes(rows: &[Row]) -> Vec<FileChange> {
    let mut out: Vec<FileChange> = Vec::new();
    for row in rows {
        let RowKind::Tool(tool) = &row.kind else {
            continue;
        };
        if !matches!(ToolKind::of(&tool.name), ToolKind::Edit | ToolKind::Write)
            || tool.status != ToolStatus::Done
        {
            continue;
        }
        let Some(path) = s(&tool.args, "path").or_else(|| s(&tool.args, "file_path")) else {
            continue;
        };
        let diff = tool.patch();
        let (added, removed) = match &diff {
            Some(p) => {
                let d = tools::diff_preview(p, 0);
                (d.added, d.removed)
            }
            None => (s(&tool.args, "content").map_or(0, |c| c.lines().count()), 0),
        };
        let entry = if let Some(e) = out.iter_mut().find(|f| f.path == path) {
            e
        } else {
            out.push(FileChange {
                path: path.to_owned(),
                added: 0,
                removed: 0,
                patches: Vec::new(),
            });
            let last = out.len() - 1;
            &mut out[last]
        };
        entry.added += added;
        entry.removed += removed;
        entry.patches.extend(diff);
    }
    out
}

/// The turn of row `ix`: rows from its user prompt (or the start) up to the next prompt.
#[must_use]
pub fn turn_bounds(rows: &[Row], ix: usize) -> (usize, usize) {
    let start = rows[..=ix.min(rows.len().saturating_sub(1))]
        .iter()
        .rposition(|r| matches!(r.kind, RowKind::User(_)))
        .unwrap_or(0);
    let end = rows[ix + 1..]
        .iter()
        .position(|r| matches!(r.kind, RowKind::User(_)))
        .map_or(rows.len(), |p| ix + 1 + p);
    (start, end)
}

/// Whether row `ix` is the last row of its turn (`TurnFooter` goes after it).
#[must_use]
pub fn is_turn_end(rows: &[Row], ix: usize, running: bool) -> bool {
    let last = ix + 1 == rows.len();
    let before_prompt = rows
        .get(ix + 1)
        .is_some_and(|r| matches!(r.kind, RowKind::User(_)));
    (last && !running) || before_prompt
}

/// A subagent / sidekick row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLine {
    /// Agent name (`explorer`, `Sidekick`).
    pub name: String,
    /// Task text.
    pub task: String,
    /// Sidekick (fusion) rather than a subagent.
    pub sidekick: bool,
    /// Child session id when known (opens the Agent tab).
    pub child: Option<String>,
}

/// Agent details of a subagent-like tool row.
#[must_use]
pub fn agent_line(tool: &ToolRow) -> Option<AgentLine> {
    if ToolKind::of(&tool.name) != ToolKind::Agent {
        return None;
    }
    let sidekick = tool.name == "sidekick";
    let name = ["agent", "profile", "name", "subagent"]
        .iter()
        .find_map(|k| s(&tool.args, k))
        .map_or_else(
            || {
                if sidekick {
                    "Sidekick".to_owned()
                } else {
                    tool_label(&tool.name)
                }
            },
            str::to_owned,
        );
    let task = ["task", "prompt", "message", "question"]
        .iter()
        .find_map(|k| s(&tool.args, k))
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    let child = tool
        .details
        .as_ref()
        .and_then(|d| {
            d.get("sessionId")
                .or_else(|| d.get("childSessionId"))
                .or_else(|| d.get("agentId"))
        })
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    Some(AgentLine {
        name,
        task,
        sidekick,
        child,
    })
}

/// First `lines` lines of `text`, each truncated to `width` chars.
#[must_use]
pub fn excerpt(text: &str, lines: usize, width: usize) -> String {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .take(lines)
        .map(|l| {
            if l.chars().count() > width {
                let mut t: String = l.chars().take(width.saturating_sub(1)).collect();
                t.push('…');
                t
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Query-rail entry for the user row at `ix`: `(query excerpt, answer excerpt)`.
#[must_use]
pub fn rail_excerpt(rows: &[Row], ix: usize) -> (String, String) {
    let query = match rows.get(ix).map(|r| &r.kind) {
        Some(RowKind::User(u)) => excerpt(&u.text, 2, 60),
        _ => String::new(),
    };
    let answer = rows[ix + 1..]
        .iter()
        .take_while(|r| !matches!(r.kind, RowKind::User(_)))
        .find_map(|r| match &r.kind {
            RowKind::Assistant(a) if !a.text.trim().is_empty() => Some(excerpt(&a.text, 2, 60)),
            _ => None,
        })
        .unwrap_or_default();
    (query, answer)
}

/// Plan steps parsed from markdown (`- [ ]` / `- [x]` checklists, else a numbered list).
#[must_use]
pub fn plan_steps(markdown: &str) -> Vec<(String, bool)> {
    let mut checklist = Vec::new();
    let mut numbered = Vec::new();
    for line in markdown.lines() {
        let t = line.trim_start();
        if let Some(rest) = t
            .strip_prefix("- [ ] ")
            .or_else(|| t.strip_prefix("* [ ] "))
        {
            checklist.push((clean(rest), false));
        } else if let Some(rest) = t
            .strip_prefix("- [x] ")
            .or_else(|| t.strip_prefix("- [X] "))
            .or_else(|| t.strip_prefix("* [x] "))
        {
            checklist.push((clean(rest), true));
        } else if let Some((n, rest)) = t.split_once(". ")
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
            && line.len() == t.len()
        {
            numbered.push((clean(rest), false));
        }
    }
    if checklist.is_empty() {
        numbered
    } else {
        checklist
    }
}

fn clean(text: &str) -> String {
    text.replace("**", "").replace('`', "").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::transcript::{AssistantRow, UserRow};
    use serde_json::json;

    fn tool(id: &str, name: &str, args: serde_json::Value, status: ToolStatus) -> Row {
        let row: ToolRow = ToolRow {
            call_id: id.into(),
            name: name.into(),
            args: serde_json::from_value(args).unwrap(),
            status,
            output: String::new(),
            details: None,
            duration_ms: None,
            parent: None,
            expanded: false,
        };
        Row {
            id: id.into(),
            version: 0,
            kind: RowKind::Tool(Box::new(row)),
        }
    }

    fn user(text: &str) -> Row {
        Row {
            id: format!("u-{text}"),
            version: 0,
            kind: RowKind::User(UserRow {
                text: text.into(),
                images: 0,
            }),
        }
    }

    fn blank() -> Row {
        Row {
            id: "a".into(),
            version: 0,
            kind: RowKind::Assistant(AssistantRow::default()),
        }
    }

    #[test]
    fn tool_lines_have_verbs_subjects_and_meta() {
        let RowKind::Tool(mut edit) = tool(
            "e",
            "edit",
            json!({"path":"src/parser.rs"}),
            ToolStatus::Done,
        )
        .kind
        else {
            panic!()
        };
        edit.details = Some(json!({"patch":"@@ -1 +1,2 @@\n-a\n+b\n+c\n"}));
        let line = tool_line(&edit);
        assert_eq!(line.verb, "Edited");
        assert_eq!(
            line.subject,
            Subject::Path {
                name: "parser.rs".into(),
                dir: "src/".into()
            }
        );
        assert_eq!(line.meta.as_deref(), Some("+2 −1"));
        let RowKind::Tool(mut bash) = tool(
            "b",
            "bash",
            json!({"command":"cargo test parser\n--x"}),
            ToolStatus::Running,
        )
        .kind
        else {
            panic!()
        };
        assert_eq!(tool_line(&bash).verb, "Running");
        bash.status = ToolStatus::Done;
        bash.duration_ms = Some(3420);
        let line = tool_line(&bash);
        assert_eq!(line.verb, "Ran");
        assert_eq!(line.subject, Subject::Command("cargo test parser".into()));
        assert_eq!(line.meta.as_deref(), Some("3.4s"));
        assert_eq!(
            tool_label("mcp__github__search_issues"),
            "github · search_issues"
        );
        assert_eq!(tool_label("notify_user"), "Notify user");
        assert_eq!(duration(125_000), "2m05s");
    }

    #[test]
    fn explore_runs_group_four_or_more_read_only_calls() {
        let mut rows = vec![user("q")];
        for i in 0..3 {
            rows.push(tool(
                &format!("r{i}"),
                "read",
                json!({"path":"a"}),
                ToolStatus::Done,
            ));
        }
        rows.push(blank());
        rows.push(tool("g", "grep", json!({"pattern":"x"}), ToolStatus::Done));
        rows.push(tool("e", "edit", json!({"path":"a"}), ToolStatus::Done));
        assert_eq!(explore_run(&rows, 1), Some((1, 5, 4)));
        assert_eq!(explore_run(&rows, 4), Some((1, 5, 4)));
        assert_eq!(explore_run(&rows, 6), None);
        assert_eq!(explore_label(&rows, 1, 5), "Explored 4 files");
        rows.truncate(4);
        assert_eq!(explore_run(&rows, 1), None, "three calls stay ungrouped");
    }

    #[test]
    fn turn_summaries_and_bounds() {
        let RowKind::Tool(mut e) =
            tool("e", "edit", json!({"path":"src/a.rs"}), ToolStatus::Done).kind
        else {
            panic!()
        };
        e.details = Some(json!({"patch":"@@\n+x\n-y\n"}));
        let rows = vec![
            user("one"),
            Row {
                id: "e".into(),
                version: 0,
                kind: RowKind::Tool(e.clone()),
            },
            Row {
                id: "e2".into(),
                version: 0,
                kind: RowKind::Tool(e),
            },
            tool(
                "w",
                "write",
                json!({"path":"b.rs","content":"1\n2"}),
                ToolStatus::Done,
            ),
            user("two"),
        ];
        let changes = file_changes(&rows[0..4]);
        assert_eq!(changes.len(), 2);
        assert_eq!((changes[0].added, changes[0].removed), (2, 2));
        assert_eq!(changes[1].added, 2);
        assert_eq!(turn_bounds(&rows, 2), (0, 4));
        assert!(is_turn_end(&rows, 3, true));
        assert!(!is_turn_end(&rows, 2, false));
        assert!(!is_turn_end(&rows, 4, true));
        assert!(is_turn_end(&rows, 4, false));
    }

    #[test]
    fn agents_excerpts_and_plans() {
        let RowKind::Tool(t) = tool(
            "s",
            "run_subagent",
            json!({"agent":"explorer","task":"map crate deps\nmore"}),
            ToolStatus::Running,
        )
        .kind
        else {
            panic!()
        };
        let a = agent_line(&t).unwrap();
        assert_eq!(
            (a.name.as_str(), a.task.as_str()),
            ("explorer", "map crate deps")
        );
        assert!(!a.sidekick);
        let RowKind::Tool(k) = tool(
            "k",
            "sidekick",
            json!({"message":"hi"}),
            ToolStatus::Running,
        )
        .kind
        else {
            panic!()
        };
        assert!(agent_line(&k).unwrap().sidekick);
        assert_eq!(excerpt("a\n\nbbbbbb\nc", 2, 4), "a\nbbb…");
        let steps = plan_steps("Plan:\n1. **Locate** loop\n2. Patch `,`\n  3. nested");
        assert_eq!(
            steps,
            vec![("Locate loop".into(), false), ("Patch ,".into(), false)]
        );
        let checks = plan_steps("- [x] done\n- [ ] todo\n1. ignored");
        assert_eq!(checks, vec![("done".into(), true), ("todo".into(), false)]);
    }
}
