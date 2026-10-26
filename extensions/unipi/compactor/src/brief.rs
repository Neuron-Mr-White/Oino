//! The brief transcript and ranked block selection (unipi `brief.ts`, `rank.ts`,
//! `skill-collapse.ts`).

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::blocks::{Block, clen, extract_path, one_line, take};
use crate::re::{self, regexes};

regexes! {
    SELF_TALK = r"(?i)^\s*(?:hmm|wait|actually|oh|okay|ok|well|so)[,.!\s-]+";
    PIPE_TAIL = r"\s*\|\s*(?:head|tail|sort|wc|column|tr|cut|awk|uniq|python3|node|bun)(?:\s[^|]*)?$";
    TRIVIAL_LINE = r"^(?:set\s+[-+]|cd\s+\S+$|export\s+\w+=|(?:source|\.)\s+\S+|pwd$|true$|:$|#)";
    HEREDOC_OPEN = r#"(?<![\w)])<<-?\s*["']?([A-Za-z_]\w*)["']?"#;
    FILEWRITER = r"(?:^|[|&;]\s*)(?:cat|tee|dd)\b";
    BODY_NOISE = r#"^(?:import\s|from\s+\S+\s+import|require\(|const\s+\w+\s*=\s*require|#|//|"""|'''|"use strict"|use\s+strict|<\?php)"#;
    CD_PREFIX = r"^cd\s+\S+\s*&&\s*";
    SKILL_BLOCK = r#"<skill\s+name="([^"]+)"[^>]*>[\s\S]*?(?:</skill>|$)"#;
    REF_END = r"\(#(\d+)\)$";
    REPEAT_TAIL = r"^(.*) \((#[\d, #]+)\) x(\d+)$";
    REF_SUFFIX = r"\s*\(#\d+\)$";
    ERR_HEADER = r"^\[tool_error\]\s+(\S+?)(?:\s*\(#(\d+)\))?$";
    ERR_HEADER_MULTI = r"^\[tool_error\]\s+(\S+?)\s*\(((?:#\d+(?:,\s*)?)+)\)(?:\s*x(\d+))?$";
    EDIT_TOOL = r"(?i)^(edit|write|multiedit|quick_edit|target_edit|apply_patch)$";
    READ_TOOL = r"(?i)^(read|glob|grep|ls|find|semantic_query|semantic_grep|semantic_show)$";
    TEST_CMD = r"(?i)\b(?:bun|npm|pnpm|yarn|node|pytest|cargo|go|mvn|gradle)\b[^\n]*(?:test|spec|check|lint|build|tsc)";
    GH_PR_POLL = r"(?i)(?:^|\s)gh\s+pr\s+(?:view|checks)\s+(\d+)\b";
    WORKFLOW_CMD = r"(?i)(?:^|\s)(?:gh\s+(?:pr|issue)\s+[a-z-]+|git\s+(?:commit|push|merge|rebase|revert|cherry-pick|tag|reset|checkout|branch)\b)";
    TRIVIAL_BASH_LINE = r"^(?:set\s+[-+]|cd(?:\s+\S+)?$|export\s+\w+=|(?:source|\.)\s+\S+|pwd$|true$|:$|#|ls(?:\s|$)|echo\b|clear$|sleep\b)";
}

const TRUNCATE_USER: usize = 256;
const CLOSING_HEAD: usize = 120;
const CLOSING_TAIL: usize = 120;
const HEAD: usize = 80;
const TAIL: usize = 120;
const BASH_CAP: usize = 240;
const HEREDOC_BODY_CAP: usize = 80;
const TOOL_CALLS_PER_TURN: usize = 8;

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
    "need", "must", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into",
    "through", "during", "before", "after", "above", "below", "between", "under", "over", "and",
    "but", "or", "nor", "not", "so", "yet", "both", "either", "neither", "each", "every", "all",
    "any", "few", "more", "most", "other", "some", "such", "no", "that", "this", "these", "those",
    "it", "its", "i", "me", "my", "we", "our", "you", "your", "he", "him", "his", "she", "her",
    "they", "them", "their", "who", "which", "what", "if", "then", "than", "when", "where", "how",
    "just", "also",
];

/// unipi `clip` from `content.ts` (`...` suffix).
#[must_use]
pub fn clip3(text: &str, max: usize) -> String {
    if clen(text) <= max {
        text.to_owned()
    } else {
        format!("{}...", take(text, max.saturating_sub(3)))
    }
}

/// Collapses `<skill name="X">…</skill>` blocks.
#[must_use]
pub fn collapse_skill_text(text: &str) -> String {
    re::replace_all(&SKILL_BLOCK, text, "[skill: $1]").into_owned()
}

fn normalize_for_budget(text: &str) -> String {
    let t = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    let mut ws = false;
    for c in t.chars() {
        if c != '\n' && c.is_whitespace() {
            ws = true;
            continue;
        }
        if ws {
            out.push(' ');
            ws = false;
        }
        out.push(c);
    }
    if ws {
        out.push(' ');
    }
    let mut collapsed = String::new();
    let mut nl = 0;
    for c in out.chars() {
        if c == '\n' {
            nl += 1;
            if nl <= 2 {
                collapsed.push(c);
            }
        } else {
            nl = 0;
            collapsed.push(c);
        }
    }
    collapsed.trim().to_owned()
}

/// Word spans (byte ranges) — an approximation of `Intl.Segmenter` word segmentation.
fn word_spans(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let inner_apos = (c == '\'' || c == '’')
            && start.is_some()
            && chars.get(k + 1).is_some_and(|(_, n)| n.is_alphanumeric());
        if c.is_alphanumeric() || c == '_' || inner_apos {
            start.get_or_insert(i);
        } else if let Some(st) = start.take() {
            out.push((st, i));
        }
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

fn significant(s: &str) -> Vec<(usize, usize)> {
    word_spans(s)
        .into_iter()
        .filter(|(a, b)| !STOP_WORDS.contains(&s[*a..*b].to_lowercase().as_str()))
        .collect()
}

fn truncate_tokens(text: &str, limit: usize) -> String {
    let flat = normalize_for_budget(text);
    let words = significant(&flat);
    if words.len() <= limit {
        return flat;
    }
    // End at the start of the first word over the limit (JS keeps everything before it).
    let cut = words[limit].0;
    format!("{}...(truncated)", flat[..cut].trim_end())
}

fn truncate_head_tail(text: &str, head: usize, tail: usize) -> String {
    let flat = normalize_for_budget(text);
    let words = significant(&flat);
    if words.len() <= head + tail {
        return flat;
    }
    let h = flat[..words[head - 1].1].trim_end();
    let t = flat[words[words.len() - tail].0..].trim_start();
    format!("{h}\n...(middle truncated)...\n{t}")
}

/// Line index of the heredoc terminator opened on `lines[i]`, if present downstream.
#[must_use]
pub fn heredoc_close(lines: &[&str], i: usize) -> Option<usize> {
    let term = re::cap(&HEREDOC_OPEN, lines[i], 1)?;
    (i + 1..lines.len()).find(|j| lines[*j].trim() == term)
}

fn strip_pipe_tail(line: &str) -> String {
    let mut c = line.to_owned();
    for _ in 0..3 {
        let s = re::replace_first(&PIPE_TAIL, &c, "").into_owned();
        if s == c {
            break;
        }
        c = s;
    }
    c.trim().to_owned()
}

fn strip_cd(line: &str) -> String {
    re::replace_first(&CD_PREFIX, line, "").trim().to_owned()
}

/// Semantic compression of a bash command (unipi `compressBash`).
#[must_use]
pub fn compress_bash(raw: &str) -> String {
    let raw_lines: Vec<&str> = raw.split('\n').collect();
    let mut kept: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw_lines.len() {
        let line = raw_lines[i];
        kept.push(line.to_owned());
        if let Some(close) = heredoc_close(&raw_lines, i) {
            let want = !re::test(&FILEWRITER, line);
            let preview = if want {
                raw_lines[i + 1..close]
                    .iter()
                    .map(|l| l.trim())
                    .find(|t| !t.is_empty() && !re::test(&BODY_NOISE, t))
            } else {
                None
            };
            if let Some(p) = preview {
                let clipped = if clen(p) > HEREDOC_BODY_CAP {
                    format!("{}\u{2026}", take(p, HEREDOC_BODY_CAP - 1))
                } else {
                    p.to_owned()
                };
                if let Some(last) = kept.last_mut() {
                    *last = format!("{} {clipped}", line.trim());
                }
            }
            i = close;
        }
        i += 1;
    }
    let lines: Vec<String> = kept
        .iter()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return raw.trim().to_owned();
    }
    let meaningful: Vec<String> = lines
        .iter()
        .filter(|l| !re::test(&TRIVIAL_LINE, l))
        .map(|l| strip_pipe_tail(&strip_cd(l)))
        .filter(|l| !l.is_empty())
        .collect();
    let chosen = if meaningful.is_empty() {
        let f = strip_pipe_tail(&strip_cd(&lines[0]));
        if f.is_empty() { Vec::new() } else { vec![f] }
    } else {
        meaningful
    };
    let cmd = chosen.join("; ");
    if clen(&cmd) > BASH_CAP {
        format!("{}...", take(&cmd, BASH_CAP - 3))
    } else {
        cmd
    }
}

fn tool_one_liner(name: &str, args: &Map<String, Value>) -> String {
    let field = match name {
        "Read" | "Edit" | "Write" | "read" | "edit" | "write" => Some("file_path"),
        "Glob" | "Grep" => Some("pattern"),
        _ => None,
    };
    if let Some(v) = field.and_then(|f| args.get(f)).and_then(Value::as_str) {
        return format!("* {name} \"{v}\"");
    }
    if let Some(p) = extract_path(args) {
        return format!("* {name} \"{p}\"");
    }
    if name == "bash" || name == "Bash" {
        let raw = args
            .get("command")
            .or_else(|| args.get("description"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return format!("* {name} \"{}\"", compress_bash(raw));
    }
    if let Some(q) = args.get("query").and_then(Value::as_str) {
        return format!("* {name} \"{}\"", clip3(q, 60));
    }
    format!("* {name}")
}

/// One brief section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// `[user]`, `[assistant]`, `[tool_error] …`.
    pub header: String,
    /// Lines.
    pub lines: Vec<String>,
}

fn is_closing_assistant(blocks: &[Block], i: usize) -> bool {
    if blocks[i].kind() != "assistant" {
        return false;
    }
    blocks[i + 1..]
        .iter()
        .find(|b| b.kind() != "tool_result")
        .is_none_or(|b| b.kind() == "user")
}

fn reference(b: &Block) -> String {
    b.source().map_or_else(String::new, |s| format!(" (#{s})"))
}

/// unipi `buildBriefSections`.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one pass per unipi stage")]
pub fn build_sections(blocks: &[Block]) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut last_header = String::new();
    let push = |sections: &mut Vec<Section>, last: &mut String, header: &str, line: String| {
        if header == last.as_str()
            && let Some(s) = sections.last_mut()
        {
            s.lines.push(line);
            return;
        }
        sections.push(Section {
            header: header.to_owned(),
            lines: vec![line],
        });
        header.clone_into(last);
    };
    let push_text =
        |sections: &mut Vec<Section>, last: &mut String, header: &str, text: &str, r: &str| {
            let mut lines: Vec<String> = text.split('\n').map(ToOwned::to_owned).collect();
            if !r.is_empty()
                && let Some(l) = lines.last_mut()
            {
                l.push_str(r);
            }
            for l in lines {
                push(sections, last, header, l);
            }
        };
    for (i, b) in blocks.iter().enumerate() {
        match b {
            Block::User { text, .. } => {
                if text.trim().is_empty() {
                    continue;
                }
                let t = truncate_tokens(&collapse_skill_text(text), TRUNCATE_USER);
                if !t.is_empty() {
                    push_text(&mut sections, &mut last_header, "[user]", &t, &reference(b));
                }
                "[user]".clone_into(&mut last_header);
            }
            Block::Assistant { text, .. } => {
                let mut raw = text.clone();
                for _ in 0..2 {
                    let s = re::replace_first(&SELF_TALK, &raw, "").into_owned();
                    if s == raw {
                        break;
                    }
                    raw = s;
                }
                let t = if is_closing_assistant(blocks, i) {
                    truncate_head_tail(&raw, CLOSING_HEAD, CLOSING_TAIL)
                } else {
                    truncate_head_tail(&raw, HEAD, TAIL)
                };
                if !t.is_empty() {
                    push_text(
                        &mut sections,
                        &mut last_header,
                        "[assistant]",
                        &t,
                        &reference(b),
                    );
                }
            }
            Block::ToolCall { name, args, .. } => {
                if name.trim().is_empty() {
                    continue;
                }
                let line = format!("{}{}", tool_one_liner(name, args), reference(b));
                push(&mut sections, &mut last_header, "[assistant]", line);
            }
            Block::ToolResult {
                name,
                text,
                is_error: true,
                ..
            } => {
                let body = text
                    .split('\n')
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or_default()
                    .to_owned();
                if body.is_empty() || body == "(no output)" {
                    continue;
                }
                let header = format!("[tool_error] {name}{}", reference(b));
                push(&mut sections, &mut last_header, &header, body);
                last_header = header;
            }
            Block::ToolResult { .. } | Block::Thinking { .. } => {}
        }
    }
    // Collapse consecutive identical tool lines.
    for sec in sections.iter_mut().filter(|s| s.header == "[assistant]") {
        let mut out: Vec<String> = Vec::new();
        for line in std::mem::take(&mut sec.lines) {
            if !line.starts_with("* ") {
                out.push(line);
                continue;
            }
            let r = re::cap(&REF_END, &line, 1).unwrap_or_default();
            let base = if r.is_empty() {
                line.clone()
            } else {
                line[..line.len() - (r.len() + 3)].trim_end().to_owned()
            };
            let last = out.last().cloned().unwrap_or_default();
            let m = re::caps_all(&REPEAT_TAIL, &last).into_iter().next();
            if let Some(m) =
                m.filter(|m| m.get(1).cloned().flatten().as_deref() == Some(base.as_str()))
            {
                let refs = m.get(2).cloned().flatten().unwrap_or_default();
                let n: u32 = m
                    .get(3)
                    .cloned()
                    .flatten()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(1);
                if let Some(l) = out.last_mut() {
                    *l = format!("{base} ({refs}, #{r}) x{}", n + 1);
                }
            } else if re::test(&REF_END, &last) && re::replace_first(&REF_SUFFIX, &last, "") == base
            {
                let prev = re::cap(&REF_END, &last, 1).unwrap_or_default();
                if let Some(l) = out.last_mut() {
                    *l = format!("{base} (#{prev}, #{r}) x2");
                }
            } else {
                out.push(line);
            }
        }
        sec.lines = out;
    }
    // Cap tool calls per [assistant] turn.
    for sec in sections.iter_mut().filter(|s| s.header == "[assistant]") {
        let idxs: Vec<usize> = sec
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with("* "))
            .map(|(i, _)| i)
            .collect();
        if idxs.len() <= TOOL_CALLS_PER_TURN {
            continue;
        }
        let drop = idxs.len() - TOOL_CALLS_PER_TURN;
        let dropped: BTreeSet<usize> = idxs[..drop].iter().copied().collect();
        let first = idxs[drop];
        let mut next = Vec::new();
        for (i, l) in sec.lines.iter().enumerate() {
            if dropped.contains(&i) {
                continue;
            }
            if i == first {
                next.push(format!("* ({drop} earlier tool-call entries omitted)"));
            }
            next.push(l.clone());
        }
        sec.lines = next;
    }
    // Collapse consecutive identical [tool_error] sections.
    let mut collapsed: Vec<Section> = Vec::new();
    for sec in sections {
        let m = re::caps_all(&ERR_HEADER, &sec.header).into_iter().next();
        let Some(m) = m.filter(|_| sec.lines.len() == 1) else {
            collapsed.push(sec);
            continue;
        };
        let tool = m.get(1).cloned().flatten().unwrap_or_default();
        let r = m.get(2).cloned().flatten();
        let merged = collapsed.last_mut().and_then(|prev| {
            let pm = re::caps_all(&ERR_HEADER_MULTI, &prev.header)
                .into_iter()
                .next()?;
            (pm.get(1).cloned().flatten().as_deref() == Some(tool.as_str())
                && prev.lines.len() == 1
                && prev.lines[0] == sec.lines[0])
                .then(|| {
                    let refs = format!(
                        "{}{}",
                        pm.get(2).cloned().flatten().unwrap_or_default(),
                        r.as_ref().map(|r| format!(", #{r}")).unwrap_or_default()
                    );
                    let count = pm
                        .get(3)
                        .cloned()
                        .flatten()
                        .and_then(|c| c.parse::<u32>().ok())
                        .map_or(2, |c| c + 1);
                    prev.header = format!("[tool_error] {tool} ({refs}) x{count}");
                })
        });
        if merged.is_none() {
            collapsed.push(sec);
        }
    }
    collapsed
}

/// Renders sections.
#[must_use]
pub fn stringify(sections: &[Section]) -> String {
    let tools_only =
        |s: &Section| s.header == "[assistant]" && s.lines.iter().all(|l| l.starts_with("* "));
    let mut out: Vec<&str> = Vec::new();
    for (i, s) in sections.iter().enumerate() {
        if i > 0 && !(tools_only(&sections[i - 1]) && tools_only(s)) {
            out.push("");
        }
        out.push(&s.header);
        out.extend(s.lines.iter().map(String::as_str));
    }
    out.join("\n")
}

/// unipi `compileBrief`.
#[must_use]
pub fn compile(blocks: &[Block]) -> String {
    stringify(&build_sections(blocks))
}

// ── ranking ──────────────────────────────────────────────────────────────

const MAX_BLOCKS: usize = 80;
const RECENT_BLOCKS: usize = 16;

fn bash_command(b: &Block) -> Option<&str> {
    match b {
        Block::ToolCall { name, args, .. } if name.eq_ignore_ascii_case("bash") => {
            args.get("command").and_then(Value::as_str)
        }
        _ => None,
    }
}

fn trivial_only_bash(raw: &str) -> bool {
    let lines: Vec<&str> = raw.split('\n').collect();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        kept.push(lines[i]);
        if let Some(c) = heredoc_close(&lines, i) {
            i = c;
        }
        i += 1;
    }
    kept.iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .all(|l| re::test(&TRIVIAL_BASH_LINE, l))
}

/// File activity hints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileOps {
    /// Read paths.
    pub read: BTreeSet<String>,
    /// Modified paths.
    pub modified: BTreeSet<String>,
}

#[derive(Clone, Debug)]
struct Ranked {
    score: i64,
    reasons: Vec<&'static str>,
}

fn score(b: &Block, i: usize, total: usize, ops: &FileOps) -> Ranked {
    let mut r = Ranked {
        score: 0,
        reasons: Vec::new(),
    };
    let mut add = |p: i64, why: &'static str| {
        r.score += p;
        r.reasons.push(why);
    };
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "small indices"
    )]
    let recency = if total <= 1 {
        0
    } else {
        ((i as f64 / (total - 1) as f64) * 12.0).round() as i64
    };
    add(recency, "recency");
    match b {
        Block::User { .. } => add(18, "user-turn"),
        Block::Assistant { .. } => add(10, "assistant-context"),
        Block::ToolResult { .. } => add(1, "tool-result-low-value"),
        _ => {}
    }
    if let Block::ToolCall { name, .. } = b {
        let cmd = bash_command(b);
        if re::test(&EDIT_TOOL, name) {
            add(34, "edit-tool");
        } else if cmd.is_some_and(|c| re::test(&TEST_CMD, c)) {
            add(26, "test-command");
        } else if re::test(&READ_TOOL, name) {
            add(6, "read-tool");
        } else {
            add(12, "tool-call");
        }
        if cmd.is_some_and(|c| re::test(&WORKFLOW_CMD, c)) {
            add(14, "workflow-command");
        }
        if cmd.is_some_and(trivial_only_bash) {
            add(-16, "trivial-bash");
        }
        if let Block::ToolCall { args, .. } = b
            && let Some(p) = extract_path(args)
        {
            if ops.modified.contains(&p) {
                add(18, "hook-modified-file");
            }
            if ops.read.contains(&p) {
                add(6, "hook-read-file");
            }
        }
    }
    if let Block::ToolResult { text, .. } = b
        && clen(text) > 1000
    {
        add(-8, "long-tool-result");
    }
    r
}

fn rank(blocks: &[Block], ops: &FileOps) -> Vec<Ranked> {
    let mut ranked: Vec<Ranked> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| score(b, i, blocks.len(), ops))
        .collect();
    let important: Vec<usize> = ranked
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.score >= 34
                || r.reasons
                    .iter()
                    .any(|x| matches!(*x, "edit-tool" | "test-command" | "nonzero-exit"))
        })
        .map(|(i, _)| i)
        .collect();
    let bump = |r: &mut Ranked, p: i64, why: &'static str| {
        r.score += p;
        r.reasons.push(why);
    };
    for idx in important {
        if let Some(i) = (idx.saturating_sub(8)..idx)
            .rev()
            .find(|i| blocks[*i].kind() == "user")
        {
            bump(&mut ranked[i], 10, "near-important-event");
        }
        if let Some(i) = (idx.saturating_sub(4)..idx)
            .rev()
            .find(|i| blocks[*i].kind() == "assistant")
        {
            bump(&mut ranked[i], 7, "near-important-event");
        }
        if let Some(i) = (idx + 1..=(idx + 4).min(blocks.len().saturating_sub(1)))
            .find(|i| matches!(blocks[*i].kind(), "assistant" | "tool_call"))
        {
            bump(&mut ranked[i], 5, "after-important-event");
        }
    }
    for i in 0..blocks.len() {
        if let Block::Assistant { text, .. } = &blocks[i]
            && clen(text.trim()) >= 120
            && blocks[i + 1..]
                .iter()
                .find(|b| b.kind() != "tool_result")
                .is_none_or(|b| b.kind() == "user")
        {
            bump(&mut ranked[i], 14, "segment-closing-assistant");
        }
    }
    ranked
}

fn dedup_key(b: &Block) -> Option<String> {
    if let Some(cmd) = bash_command(b) {
        if let Some(n) = re::cap(&GH_PR_POLL, cmd, 1) {
            return Some(format!("gh-pr-poll:{n}"));
        }
        let n = one_line(cmd);
        return (!n.is_empty()).then(|| format!("bash:{n}"));
    }
    if let Block::ToolCall { name, args, .. } = b {
        return extract_path(args).map(|p| format!("tool:{}:{p}", name.to_lowercase()));
    }
    None
}

/// unipi `selectRankedBriefBlocks` with a char budget.
#[must_use]
pub fn select_ranked(blocks: &[Block], max_chars: Option<usize>, ops: &FileOps) -> Vec<Block> {
    if blocks.len() <= MAX_BLOCKS && max_chars.is_none() {
        return blocks.to_vec();
    }
    let preserve = RECENT_BLOCKS.min(MAX_BLOCKS);
    let ranked = rank(blocks, ops);
    let mut selected: BTreeSet<usize> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let costs: Option<Vec<usize>> = max_chars.map(|_| {
        blocks
            .iter()
            .map(|b| {
                if b.kind() == "tool_result" {
                    0
                } else {
                    clen(&compile(std::slice::from_ref(b))) + 1
                }
            })
            .collect()
    });
    let mut used = 0;
    for i in (blocks.len().saturating_sub(preserve)..blocks.len()).rev() {
        if blocks[i].kind() == "tool_result" || selected.contains(&i) {
            continue;
        }
        if let (Some(c), Some(m)) = (&costs, max_chars)
            && used + c[i] > m
        {
            continue;
        }
        selected.insert(i);
        if let Some(c) = &costs {
            used += c[i];
        }
        if let Some(k) = dedup_key(&blocks[i]) {
            seen.insert(k);
        }
    }
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by(|a, b| ranked[*b].score.cmp(&ranked[*a].score).then(b.cmp(a)));
    for i in order {
        if selected.len() >= MAX_BLOCKS {
            break;
        }
        if selected.contains(&i) || blocks[i].kind() == "tool_result" {
            continue;
        }
        let key = dedup_key(&blocks[i]);
        if key.as_ref().is_some_and(|k| seen.contains(k)) {
            continue;
        }
        if let (Some(c), Some(m)) = (&costs, max_chars) {
            if used + c[i] > m {
                continue;
            }
            used += c[i];
        }
        selected.insert(i);
        if let Some(k) = key {
            seen.insert(k);
        }
    }
    selected.into_iter().map(|i| blocks[i].clone()).collect()
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

    #[test]
    fn all_compile() {
        for (n, r) in all_patterns() {
            assert!(r.is_some(), "{n}");
        }
    }

    fn call(name: &str, args: Value, s: usize) -> Block {
        Block::ToolCall {
            name: name.into(),
            args: args.as_object().cloned().unwrap_or_default(),
            source: Some(s),
        }
    }

    #[test]
    fn bash_compression() {
        assert_eq!(
            compress_bash("set -euo pipefail\ncd /x && git commit -m 'a' | tail -3"),
            "git commit -m 'a'"
        );
        assert_eq!(
            compress_bash("cat > f.py <<EOF\nprint(1)\nEOF\npython3 f.py"),
            "cat > f.py <<EOF; python3 f.py"
        );
        assert_eq!(
            compress_bash("python3 - <<PY\nimport os\nprint(os.getcwd())\nPY"),
            "python3 - <<PY print(os.getcwd())"
        );
        assert_eq!(
            compress_bash("echo $((8 << 20))\nls"),
            "echo $((8 << 20)); ls"
        );
        assert_eq!(compress_bash("ls"), "ls");
        assert!(compress_bash(&"x".repeat(300)).ends_with("..."));
    }

    #[test]
    fn brief() {
        let blocks = vec![
            Block::User {
                text: "fix it".into(),
                source: Some(0),
            },
            call("read", json!({"path": "a.ts"}), 1),
            call("read", json!({"path": "a.ts"}), 2),
            call("read", json!({"path": "a.ts"}), 3),
            Block::ToolResult {
                name: "bash".into(),
                text: "boom".into(),
                is_error: true,
                source: Some(4),
            },
            Block::ToolResult {
                name: "bash".into(),
                text: "boom".into(),
                is_error: true,
                source: Some(5),
            },
        ];
        let text = compile(&blocks);
        assert!(text.contains("[user]\nfix it (#0)"), "{text}");
        assert!(text.contains("* read \"a.ts\" (#1, #2, #3) x3"), "{text}");
        assert!(
            text.contains("[tool_error] bash (#4, #5) x2\nboom"),
            "{text}"
        );
        assert_eq!(
            collapse_skill_text("a <skill name=\"x\" v=1>body</skill> b"),
            "a [skill: x] b"
        );
        let long = format!("{} end", "word ".repeat(300));
        assert!(truncate_tokens(&long, 256).ends_with("...(truncated)"));
        let ht = truncate_head_tail(&"alpha ".repeat(300), 80, 120);
        assert!(ht.contains("...(middle truncated)..."));
        let many: Vec<Block> = (0..12)
            .map(|i| call("bash", json!({"command": format!("echo {i}")}), i))
            .collect();
        assert!(compile(&many).contains("* (4 earlier tool-call entries omitted)"));
    }

    #[test]
    fn ranking() {
        let mut blocks: Vec<Block> = (0..100)
            .map(|i| Block::Assistant {
                text: format!("step {i}"),
                source: Some(i),
            })
            .collect();
        blocks.push(call("edit", json!({"path": "x"}), 100));
        let sel = select_ranked(&blocks, None, &FileOps::default());
        assert_eq!(sel.len(), MAX_BLOCKS);
        assert!(sel.iter().any(|b| b.kind() == "tool_call"));
        let sel = select_ranked(&blocks, Some(200), &FileOps::default());
        assert!(compile(&sel).chars().count() <= 260);
    }
}
