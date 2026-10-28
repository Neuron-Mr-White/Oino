//! Timeline presentation (design.md §4.9–§4.10): one-line tool summaries (verb · subject ·
//! meta), "Explored N files" grouping of read-only runs, per-turn file change summaries,
//! subagent rows and query-rail excerpts. Pure functions over transcript rows.

use oino_ai::{AgentMessage, JsonObject, JsonValue};

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
    // QA-8: when the model omits `agent`/`profile`/`name` (the subagents extension resolves
    // the profile server-side), the name comes from the tool details the extension streams
    // (`profileLabel`, else the raw `profile` id) instead of falling back to the generic
    // tool label. An explicitly named agent wins; the pretty label beats the raw id the
    // schema makes the model pass, so the row matches the subagent-step cards either way.
    let details_name = tool
        .details
        .as_ref()
        .and_then(|d| {
            d.get("profileLabel")
                .or_else(|| d.get("profile"))
                .and_then(JsonValue::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .filter(|_| !sidekick);
    let fallback = || {
        if sidekick {
            "Sidekick".to_owned()
        } else {
            tool_label(&tool.name)
        }
    };
    let name = ["agent", "name", "subagent"]
        .iter()
        .find_map(|k| s(&tool.args, k))
        .or(details_name.as_deref())
        .or_else(|| s(&tool.args, "profile"))
        .map_or_else(fallback, str::to_owned);
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

/// Whether `row` folds into its turn's fold line (A2.10): tool calls other than agents, and
/// assistant rows without visible text (thinking only, or blank).
#[must_use]
pub fn foldable(row: &Row) -> bool {
    match &row.kind {
        RowKind::Tool(t) => ToolKind::of(&t.name) != ToolKind::Agent,
        RowKind::Assistant(a) => a.text.trim().is_empty() && a.error.is_none(),
        _ => false,
    }
}

fn fold_content(row: &Row) -> bool {
    match &row.kind {
        RowKind::Tool(_) => true,
        RowKind::Assistant(a) => !a.thinking.trim().is_empty(),
        _ => false,
    }
}

/// The fold run containing row `ix`: `(first, last)` content rows of the maximal run of
/// [`foldable`] rows around it. `None` when `ix` is not inside one (blank rows at the run
/// edges are not part of it).
#[must_use]
pub fn fold_run(rows: &[Row], ix: usize) -> Option<(usize, usize)> {
    let row = rows.get(ix)?;
    if !foldable(row) {
        return None;
    }
    let mut start = ix;
    while start > 0 && foldable(&rows[start - 1]) {
        start -= 1;
    }
    let mut end = ix;
    while end + 1 < rows.len() && foldable(&rows[end + 1]) {
        end += 1;
    }
    while start < end && !fold_content(&rows[start]) {
        start += 1;
    }
    while end > start && !fold_content(&rows[end]) {
        end -= 1;
    }
    (fold_content(&rows[start]) && (start..=end).contains(&ix)).then_some((start, end))
}

/// The fold line of one run (A2.10): "Ran commands · 2 steps", or the live action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldSummary {
    /// "Read files, edited files", "Ran commands", "Thought".
    pub title: String,
    /// "2 steps · 3.4s".
    pub detail: String,
    /// Latest action while running ("Reading · src/x.ts", "Thinking…").
    pub live: Option<String>,
    /// Failed tool calls.
    pub failed: usize,
    /// Dominant tool kind (icon).
    pub kind: Option<ToolKind>,
}

fn category(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "Read files",
        ToolKind::Write | ToolKind::Edit => "Edited files",
        ToolKind::Shell => "Ran commands",
        ToolKind::Search => "Searched",
        ToolKind::Agent | ToolKind::Other => "Used tools",
    }
}

fn subject_text(subject: &Subject) -> Option<String> {
    match subject {
        Subject::Path { name, dir } => Some(format!("{dir}{name}")),
        Subject::Command(c) | Subject::Text(c) => Some(c.clone()),
        Subject::None => None,
    }
}

/// Summary of the fold run `start..=end`.
#[must_use]
pub fn fold_summary(rows: &[Row], start: usize, end: usize) -> FoldSummary {
    let mut cats: Vec<&'static str> = Vec::new();
    let mut steps = 0usize;
    let mut failed = 0usize;
    let mut ms = 0u64;
    let mut live = None;
    let mut kind = None;
    let mut thought = false;
    for row in &rows[start..=end] {
        match &row.kind {
            RowKind::Tool(t) => {
                let k = ToolKind::of(&t.name);
                kind.get_or_insert(k);
                steps += 1;
                let c = category(k);
                if !cats.contains(&c) {
                    cats.push(c);
                }
                if t.status == ToolStatus::Error {
                    failed += 1;
                }
                ms += t.duration_ms.unwrap_or(0);
                if matches!(t.status, ToolStatus::Running | ToolStatus::Preparing) {
                    let tl = tool_line(t);
                    live = Some(match subject_text(&tl.subject) {
                        Some(subject) => format!("{} · {subject}", tl.verb),
                        None => tl.verb,
                    });
                }
            }
            RowKind::Assistant(a) if !a.thinking.trim().is_empty() => {
                thought = true;
                if a.streaming && a.text.is_empty() {
                    live = Some("Thinking…".to_owned());
                }
            }
            _ => {}
        }
    }
    let title = match cats.as_slice() {
        [] if thought => "Thought".to_owned(),
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first, rest @ ..] => {
            let mut out = (*first).to_owned();
            for c in rest.iter().take(2) {
                out.push_str(", ");
                out.push_str(&c.to_lowercase());
            }
            out
        }
    };
    let mut detail = match steps {
        0 => String::new(),
        1 => "1 step".to_owned(),
        n => format!("{n} steps"),
    };
    if ms >= 1000 {
        if !detail.is_empty() {
            detail.push_str(" · ");
        }
        detail.push_str(&duration(ms));
    }
    FoldSummary {
        title,
        detail,
        live,
        failed,
        kind,
    }
}

/// A harness card (A2.10): `(source, title, body)` of an extension message, collapsed to
/// one line by default. `<tag>value</tag>` texts (background task notifications) become a
/// summary title plus `key: value` lines.
#[must_use]
pub fn harness_card(
    custom_type: &str,
    provenance: Option<&str>,
    text: &str,
) -> (String, String, String) {
    let source =
        provenance.map_or_else(|| tool_label(&custom_type.replace('-', "_")), str::to_owned);
    let trimmed = text.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        let mut fields: Vec<(String, String)> = Vec::new();
        for line in trimmed.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix('<')
                && let Some((tag, tail)) = rest.split_once('>')
                && let Some(value) = tail.strip_suffix(&format!("</{tag}>"))
            {
                fields.push((tag.to_owned(), value.trim().to_owned()));
            }
        }
        if !fields.is_empty() {
            let get = |k: &str| fields.iter().find(|(t, _)| t == k).map(|(_, v)| v.clone());
            let title = get("summary")
                .or_else(|| get("task-name"))
                .unwrap_or_else(|| fields[0].1.clone());
            let body = fields
                .iter()
                .filter(|(t, _)| t != "summary" && t != "guidance")
                .map(|(t, v)| format!("{}: {v}", t.replace('-', " ")))
                .collect::<Vec<_>>()
                .join("\n");
            return (source, unescape(&title), unescape(&body));
        }
    }
    let title = trimmed
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches('#')
                .trim()
                .trim_matches('*')
                .trim()
        })
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    (source, excerpt(title, 1, 120), trimmed.to_owned())
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Live child agents of the session (`_oino/agents` feed, GA-15 / GA-44): their tree info and
/// a reduced transcript per agent, so agent rows show the child's latest action and the
/// Agent tab can render the child timeline.
#[derive(Clone, Debug, Default)]
pub struct AgentFeed {
    /// Agents in first-seen order.
    pub agents: Vec<AgentLive>,
}

/// One child agent.
#[derive(Clone, Debug)]
pub struct AgentLive {
    /// Tree info.
    pub info: oino_core::agents::AgentInfo,
    /// Child transcript.
    pub transcript: crate::model::transcript::Transcript,
    /// When it was first seen.
    pub started: std::time::Instant,
}

impl AgentFeed {
    /// Replaces the tree (`agents_changed`); keeps transcripts of known agents.
    pub fn set_tree(&mut self, agents: &[oino_core::agents::AgentInfo]) {
        for info in agents {
            if let Some(a) = self.agents.iter_mut().find(|a| a.info.id == info.id) {
                a.info = info.clone();
            } else {
                self.agents.push(AgentLive {
                    info: info.clone(),
                    transcript: crate::model::transcript::Transcript::new(),
                    started: std::time::Instant::now(),
                });
            }
        }
        // Closed agents leave the tree; keep the last 32 for late-opened tabs.
        for a in &mut self.agents {
            if !agents.iter().any(|i| i.id == a.info.id) {
                a.info.state = oino_core::agents::AgentRunState::Closed;
            }
        }
        if self.agents.len() > 32 {
            let drop = self.agents.len() - 32;
            self.agents.drain(..drop);
        }
    }

    /// Applies a nested event (`agent_event`). Returns whether anything changed.
    pub fn apply(&mut self, agent_id: &str, event: &oino_core::events::ClientEvent) -> bool {
        let oino_core::events::ClientEvent::Session(event) = event else {
            return false;
        };
        let Some(a) = self.agents.iter_mut().find(|a| a.info.id == agent_id) else {
            return false;
        };
        !a.transcript.apply(event).is_empty()
    }

    /// Agent by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&AgentLive> {
        self.agents.iter().find(|a| a.info.id == id)
    }

    /// Loads a child transcript from `_oino/agent_messages` (frontend-api §3): the backlog of
    /// a tab opened after the child settled, or for a child the live feed never streamed
    /// (btw / dream children, which have no tool row). A transcript that already has rows is
    /// kept — the live feed is newer than a fetched backlog. Returns whether rows changed.
    pub fn load_messages(&mut self, agent_id: &str, messages: &[AgentMessage]) -> bool {
        let Some(agent) = self.agents.iter_mut().find(|a| a.info.id == agent_id) else {
            return false;
        };
        if !agent.transcript.is_empty() || messages.is_empty() {
            return false;
        }
        agent.transcript.load(messages);
        true
    }

    /// The agent behind a subagent tool row: by child id when the row knows it, else the
    /// `nth` agent (in start order) owned by an agent-spawning extension.
    #[must_use]
    pub fn for_row(&self, child: Option<&str>, nth: usize) -> Option<&AgentLive> {
        if let Some(id) = child {
            return self.get(id);
        }
        // Only agents a tool row can start (subagents, sidekicks); helper children such as the
        // auto-rename or btw sessions have no row.
        self.agents
            .iter()
            .filter(|a| {
                a.info.parent_id.is_none()
                    && (a.info.owner.contains("subagent") || a.info.owner.contains("fusion"))
            })
            .nth(nth)
    }
}

impl AgentLive {
    /// The child's latest action ("Reading · src/x.rs", "Thinking…", last text line).
    #[must_use]
    pub fn latest(&self) -> Option<String> {
        let rows = self.transcript.rows();
        for row in rows.iter().rev() {
            match &row.kind {
                RowKind::Tool(t) => {
                    let line = tool_line(t);
                    return Some(match subject_text(&line.subject) {
                        Some(s) => format!("{} · {s}", line.verb),
                        None => line.verb,
                    });
                }
                RowKind::Assistant(a) if !a.text.trim().is_empty() => {
                    return Some(excerpt(a.text.lines().last().unwrap_or_default(), 1, 80));
                }
                RowKind::Assistant(a) if a.streaming => return Some("Thinking…".to_owned()),
                _ => {}
            }
        }
        None
    }

    /// Tool calls the child has made.
    #[must_use]
    pub fn steps(&self) -> usize {
        self.transcript
            .rows()
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Tool(_)))
            .count()
    }
}

/// Whether an error came from the model or provider, so "Switch model" helps (GA-9).
#[must_use]
pub fn is_model_error(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "model",
        "provider",
        "rate limit",
        "429",
        "401",
        "403",
        "500",
        "502",
        "503",
        "529",
        "api key",
        "overloaded",
        "quota",
        "context length",
        "context window",
        "too many tokens",
        "request failed",
        "connection",
        "timed out",
        "unauthorized",
    ]
    .iter()
    .any(|k| t.contains(k))
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
    text.replace("**", "").trim().to_owned()
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

    fn thought(text: &str) -> Row {
        Row {
            id: "t".into(),
            version: 0,
            kind: RowKind::Assistant(AssistantRow {
                thinking: text.into(),
                ..AssistantRow::default()
            }),
        }
    }

    #[test]
    fn fold_runs_and_summaries() {
        let mut rows = vec![
            user("q"),
            thought("hmm"),
            tool("r", "read", json!({"path":"src/a.rs"}), ToolStatus::Done),
            blank(),
            tool("b", "bash", json!({"command":"ls"}), ToolStatus::Done),
            tool(
                "b2",
                "bash",
                json!({"command":"cargo test"}),
                ToolStatus::Error,
            ),
            blank(),
        ];
        assert_eq!(fold_run(&rows, 0), None);
        assert_eq!(fold_run(&rows, 1), Some((1, 5)));
        assert_eq!(fold_run(&rows, 4), Some((1, 5)));
        assert_eq!(fold_run(&rows, 6), None, "trailing blank rows are outside");
        let s = fold_summary(&rows, 1, 5);
        assert_eq!(s.title, "Read files, ran commands");
        assert_eq!(s.detail, "3 steps");
        assert_eq!(s.failed, 1);
        assert_eq!(s.live, None);
        rows.push(tool(
            "r2",
            "read",
            json!({"path":"x.ts"}),
            ToolStatus::Running,
        ));
        assert_eq!(fold_run(&rows, 7), Some((1, 7)));
        assert_eq!(
            fold_summary(&rows, 1, 7).live.as_deref(),
            Some("Reading · x.ts")
        );
        assert_eq!(fold_summary(&rows, 1, 1).title, "Thought");
    }

    #[test]
    fn agent_feed_maps_rows_to_spawned_agents() {
        let info = |id: &str, owner: &str| -> oino_core::agents::AgentInfo {
            serde_json::from_value(json!({
                "id": id, "label": id, "owner": owner, "state": "running", "depth": 1
            }))
            .unwrap()
        };
        let mut feed = AgentFeed::default();
        feed.set_tree(&[info("r", "unipi.utility"), info("s1", "unipi.subagents")]);
        assert_eq!(
            feed.for_row(None, 0).map(|a| a.info.id.as_str()),
            Some("s1")
        );
        assert_eq!(
            feed.for_row(Some("r"), 0).map(|a| a.info.id.as_str()),
            Some("r")
        );
        let event: oino_core::events::ClientEvent = serde_json::from_value(json!({
            "type": "tool_execution_start", "toolCallId": "c1", "toolName": "read",
            "args": {"path": "src/a.rs"}
        }))
        .unwrap();
        assert!(feed.apply("s1", &event));
        let live = feed.get("s1").unwrap();
        assert_eq!(live.latest().as_deref(), Some("Reading · src/a.rs"));
        assert_eq!(live.steps(), 1);
        feed.set_tree(&[]);
        assert_eq!(
            feed.get("s1").unwrap().info.state,
            oino_core::agents::AgentRunState::Closed
        );
    }

    /// The nested `agent_event` wire shape (frontend-api §3) reduces into the child
    /// transcript: `message_start` creates the row, `message_end` settles it.
    #[test]
    fn nested_agent_events_reduce_into_the_child_transcript() {
        let event = |v: serde_json::Value| -> oino_core::events::ClientEvent {
            serde_json::from_value(v).unwrap()
        };
        let mut feed = AgentFeed::default();
        feed.set_tree(
            &serde_json::from_value::<Vec<oino_core::agents::AgentInfo>>(json!([
                {"id": "c1", "label": "Subagent explorer", "owner": "unipi.subagents",
                 "state": "running", "depth": 1}
            ]))
            .unwrap(),
        );
        let nested = |inner: serde_json::Value| -> oino_core::events::ClientEvent {
            match event(json!({"type": "agent_event", "agentId": "c1", "event": inner})) {
                oino_core::events::ClientEvent::Oino(
                    oino_core::events::OinoEvent::AgentEvent { event, .. },
                ) => *event,
                other => panic!("not an agent_event: {other:?}"),
            }
        };
        let user = nested(json!({"type": "message_start",
            "message": {"role": "user", "content": "map the parser", "timestamp": 0}}));
        assert!(feed.apply("c1", &user));
        let end = nested(json!({"type": "message_end",
            "message": {"role": "assistant", "content": [{"type": "text", "text": "two sites"}],
                "api": "x", "provider": "p", "model": "m",
                "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0,
                    "totalTokens": 0,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0,
                        "total": 0}},
                "stopReason": "stop", "timestamp": 0}}));
        assert!(feed.apply("c1", &end));
        let rows = feed.get("c1").unwrap().transcript.rows();
        assert_eq!(rows.len(), 2, "user + assistant rows: {rows:#?}");
        assert_eq!(
            crate::components::timeline::row_text(rows.first()),
            "map the parser"
        );
    }

    /// A late-opened Agent tab loads its backlog (`_oino/agent_messages`); live rows win.
    #[test]
    fn agent_messages_fill_a_late_opened_tab() {
        let info = |id: &str| {
            serde_json::from_value::<oino_core::agents::AgentInfo>(json!({
                "id": id, "label": "btw", "owner": "unipi.btw", "state": "idle", "depth": 1
            }))
            .unwrap()
        };
        let mut feed = AgentFeed::default();
        feed.set_tree(&[info("b1")]);
        let message = |text: &str| {
            serde_json::from_value::<AgentMessage>(json!({
                "role": "user", "content": text, "timestamp": 0
            }))
            .unwrap()
        };
        assert!(feed.load_messages("b1", &[message("what does parse_array do?")]));
        assert_eq!(feed.get("b1").unwrap().transcript.rows().len(), 1);
        // A second fetch never replaces live rows, and an unknown agent is ignored.
        assert!(!feed.load_messages("b1", &[message("stale")]));
        assert!(!feed.load_messages("nope", &[message("x")]));
    }

    #[test]
    fn model_errors() {
        assert!(is_model_error("429 Too Many Requests"));
        assert!(!is_model_error("Compaction failed: Nothing to compact"));
    }

    #[test]
    fn harness_cards_parse_tagged_text() {
        let text = "<background-task-notification>\n  <task-id>t7</task-id>\n  <task-name>sleep 45</task-name>\n  <status>completed</status>\n  <summary>Background task &quot;sleep 45&quot; completed</summary>\n  <guidance>x</guidance>\n</background-task-notification>";
        let (source, title, body) = harness_card("background-task-notification", None, text);
        assert_eq!(source, "Background task notification");
        assert_eq!(title, "Background task \"sleep 45\" completed");
        assert!(body.contains("task id: t7"));
        assert!(!body.contains("guidance"));
        let (source, title, _) = harness_card("x", Some("Memory"), "# Wake-up text\n\nbody");
        assert_eq!(
            (source.as_str(), title.as_str()),
            ("Memory", "Wake-up text")
        );
    }

    #[test]
    fn agent_row_names_itself_from_subagent_details() {
        // QA-8: the model may omit `agent`/`profile`; the extension streams the resolved
        // profile in the tool details and the row must show it, not the generic label.
        let RowKind::Tool(mut t) = tool(
            "s",
            "run_subagent",
            json!({"title": "Read README", "task": "Read README.md first line"}),
            ToolStatus::Running,
        )
        .kind
        else {
            panic!()
        };
        assert_eq!(agent_line(&t).unwrap().name, "Run subagent");
        t.details = Some(json!({
            "owner": "subagents",
            "profile": "subagent_explore",
            "profileLabel": "Explore subagent"
        }));
        let a = agent_line(&t).unwrap();
        assert_eq!(a.name, "Explore subagent");
        assert_eq!(a.task, "Read README.md first line");
        // A raw profile id still beats the generic label (sessions from before profileLabel).
        t.details = Some(json!({"profile": "subagent_explore"}));
        assert_eq!(agent_line(&t).unwrap().name, "subagent_explore");
        assert_eq!(agent_line(&t).unwrap().name, "subagent_explore");
        // An explicit custom agent name always wins.
        t.args = serde_json::from_value(json!({"agent": "explorer"})).unwrap();
        assert_eq!(agent_line(&t).unwrap().name, "explorer");
        // The pretty label beats the raw profile id the schema makes the model pass.
        t.args = serde_json::from_value(json!({"profile": "subagent_explore"})).unwrap();
        t.details = Some(json!({"profileLabel": "Explore subagent"}));
        assert_eq!(agent_line(&t).unwrap().name, "Explore subagent");
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
            vec![("Locate loop".into(), false), ("Patch `,`".into(), false)]
        );
        let checks = plan_steps("- [x] done\n- [ ] todo\n1. ignored");
        assert_eq!(checks, vec![("done".into(), true), ("todo".into(), false)]);
    }
}
