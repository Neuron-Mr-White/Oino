//! The lossless (zero-LLM) summary (unipi `summarize.ts`): fixed section shares, every item
//! clipped, secrets redacted, the `session_recall` note at the end.

use std::collections::{BTreeMap, BTreeSet};

use crate::blocks::{Block, clen, extract_path, one_line, take};
use crate::brief::{FileOps, compile, select_ranked};
use crate::re::{self, regexes};
use crate::source::Source;

/// The note every lossless summary ends with.
pub const RECALL_NOTE: &str = "Everything above is condensed from the full session history, which is kept. Use `session_recall` to search it (refs like #123 are entry indices; pass them in `expand` for full content). Do not redo work that is already done.";
/// Transcript window.
pub const TRANSCRIPT_WINDOW_BLOCKS: usize = 160;

regexes! {
    TRIVIAL_REQUEST = r"(?i)^(?:continue|go on|go ahead|carry on|keep going|proceed|next|go|ok(?:ay)?|yes|yep|y|sure|thanks?|thank you|ty|k|done|resume|restarted[^.]*)[\s.!~]*$";
    IMPERATIVE_START = r"(?i)^(?:(?:please|also|and|then|but|so)[, ]+)?(?:don'?t|do not|never|always|keep|avoid|make sure|ensure|use|prefer|remember|only use|stop|no need|restore|revert|undo|preserve|stick to|stay with|go back to)\b";
    MODAL = r"(?i)\b(?:should(?:n'?t)?|must|prefer|instead of|instead|rather than|required|important|no need)\b";
    CORRECTION = r"(?i)\bI (?:did not|didn't|never) (?:ask|request|want|say)\b|\bnot what I (?:asked|wanted|meant)\b|\bwhy (?:did|do) you (?:change|remove|delete)\b";
    PASTED_AI = r"(?i)\bI(?:'m| am) an AI\b|\bas an AI\b|\bI can help (?:with|you)\b|\bif you'd like, I\b|\bI can'?t do that\b|\*\*[^*\n]+\*\*";
    CODE_FENCE = r"```[\s\S]*?(?:```|$)";
    CODE_FENCE_CLOSED = r"```[\s\S]*?```";
    QUOTE_LINE = r"(?m)^\s*>.*$";
    SENTENCE_SPLIT = r#"(?<=[.!?])\s+(?=[A-Z0-9"'`])"#;
    LIST_MARKER = r"^\s*(?:[-*+>]|\d+[.)])\s+";
    ENDS_QUESTION = r#"\?["')\]]*$"#;
    CONTEXT_FREE = r"(?i)^(?:it|this|that|these|those)\b";
    CODE_LIKE = r"^[`$]|[{};]\s*$";
    STANDING = r"(?i)\b(?:always|never|every|all (?:the )?\w+|by default|default|from now on|going forward|in general|globally|whenever|each time|unless|any time)\b";
    IMAGE_MD = r"!\[[^\]]*\]\([^)]*\)";
    EDIT_TOOLS = r"(?i)^(?:edit|multiedit|apply_patch|quick_edit|target_edit)$";
    WRITE_TOOLS = r"(?i)^(?:write|create)$";
    READ_TOOLS = r"(?i)^(?:read)$";
    NOISE_READ = r"(?i)(?:^/tmp/|\.(?:png|jpe?g|gif|webp|bmp|svg|ico)$)";
    MEMORY_TOOL = r"(?i)^(?:memory_store|memory_save|memory_add|save_memory|remember)$";
    LESSON_MARK = r"(?i)\b(?:root cause|turns out|found it|confirmed|gotcha|caveat|silently|no-?ops?|doesn'?t (?:support|work|accept)|not supported|only works|rejects?|ignores?|the fix (?:is|was)|workaround|breaks? (?:when|if)|must (?:use|be|run)|requires?|instead of|never|always)\b";
    DIAGNOSIS = r"(?i)\b(?:root cause|turns out|gotcha|caveat|silently|no-?ops?|doesn'?t (?:support|accept)|not supported|only works|rejects?|ignores?|the (?:real )?(?:issue|problem|bug|cause) (?:is|was)|the fix (?:is|was)|workaround)\b";
    DEBUG = r"(?i)\b(?:DBG|debug(?:ging)? (?:print|log|output)|print(?:ed)? (?:never|didn'?t)|never (?:fired|printed)|let me|I'?ll (?:check|look|try))\b";
    COMMENT_LINE = r"^\s*#+\s*(.+)$";
    CONFIRMED = r"(?i)\b(?:confirmed|found it)\b";
    ENDS_COLON = r":\s*$";
    SENTENCE_OR_NL = r"(?<=[.!?])\s+|\n+";
    NOTE_FILE = r"(?i)(?:^|/)(?:AGENTS?|CLAUDE|DESIGN|README|SKILL|CONTRIBUTING|RUNBOOK|NOTES)\.md$|(?:^|/)docs/.+\.md$|\.agents/skills/.+\.md$";
    TOOLING = r"^(?:npm|npx|pnpm|yarn|bun|bunx|cargo|mise|make|just|go|pytest|uv|python3? -m|docker|docker-compose|kubectl|helm|rsync|scp|ssh|git push|gh|tea|systemctl|terraform|deno|flutter|xcodebuild)\b";
    SEG_SPLIT = r"\s*(?:&&|;)\s*";
    CD_LIKE = r"^(?:cd|export|source|set|echo)\b";
    ENV_PREFIX = r"^(?:[A-Z_][A-Z0-9_]*=\S+\s+)+";
    PIPE_REDIRECT = r"\s*(?:\||2>&1|>\s*\S+|>>\s*\S+).*$";
    SSH_START = r"^ssh\b";
    SSH_REMOTE = r#"\s['"]"#;
    HEREDOC_OR_CONT = r"<<|\\$";
    URL_HOST = r"https?://([\w.-]+(?::\d+)?)";
    SSH_HOST = r"\b(?:ssh|scp|rsync)\b[^\n]*?\b([\w.-]+@[\w.-]+)";
    COMMIT_LINE = r"\[([\w./@-]+)(?: \(root-commit\))? ([0-9a-f]{7,40})\] (.+?)(?= \d+ files? changed|$)";
    COMMIT_CMD = r#"\bgit\s+commit\b[^"'\n]*?\s-[a-zA-Z]*m\s*(["'])((?:(?!\1).){3,}?)\1"#;
    COMMIT_TRAILER = r"(?i)\s+(?:Date|Author|Co-authored-by):.*$";
    SECRET_KV = r#"(?i)\b(password|passwd|passphrase|pwd|token|secret|api[_ -]?key)(\s*(?:is|=|:)\s*)(['"`]?)([^\s'"`]{4,})\3"#;
    SECRET_QUOTED = r#"(?i)\b(password|passwd|passphrase|pwd|token|secret|api[_ -]?key)(\s+)(['"`])([^'"`\n]{4,}?)\3"#;
    SECRET_SK = r"\bsk-[A-Za-z0-9_-]{16,}";
    SECRET_BEARER = r"(?i)\b(Bearer|X-API-KEY:?|Authorization:)\s+[A-Za-z0-9._~+/=-]{12,}";
    SECRET_GH = r"\bgh[pousr]_[A-Za-z0-9]{20,}";
    SECRET_AKIA = r"\bAKIA[0-9A-Z]{16}\b";
    SECRET_XOX = r"\bxox[abprs]-[A-Za-z0-9-]{10,}";
    SECRET_JWT = r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}";
    SECRET_PEM = r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)";
}

/// Which sections are on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors unipi SummarySections"
)]
pub struct Sections {
    /// Active work.
    pub active_work: bool,
    /// Requests.
    pub requests: bool,
    /// Latest state.
    pub state: bool,
    /// Decisions.
    pub decisions: bool,
    /// Files (+ project knowledge).
    pub files: bool,
    /// Commits.
    pub commits: bool,
    /// Errors.
    pub errors: bool,
    /// Lessons.
    pub lessons: bool,
    /// Transcript.
    pub transcript: bool,
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            active_work: true,
            requests: true,
            state: true,
            decisions: true,
            files: true,
            commits: true,
            errors: true,
            lessons: true,
            transcript: true,
        }
    }
}

/// Auto budget in tokens (1.5k–4k).
#[must_use]
pub fn auto_budget_tokens(blocks: usize) -> u64 {
    #[allow(clippy::cast_possible_truncation, reason = "bounded")]
    let b = blocks.min(1_000_000) as u64;
    (1500 + b * 8).clamp(1500, 4000)
}

/// `clip` with `…`.
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    if max <= 1 {
        return String::new();
    }
    if clen(text) <= max {
        text.to_owned()
    } else {
        format!("{}…", take(text, max - 1).trim_end())
    }
}

fn item_key(text: &str) -> String {
    take(&one_line(text), 80).to_lowercase()
}

fn fit_items(items: &[String], max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = 0;
    for it in items {
        let cost = clen(it) + 3;
        if used + cost > max {
            break;
        }
        out.push(it.clone());
        used += cost;
    }
    out
}

fn section(title: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "[{title}]\n{}",
        lines
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Requests shown and their input indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestSelection {
    /// Lines.
    pub lines: Vec<String>,
    /// Indices shown verbatim.
    pub shown: BTreeSet<usize>,
}

/// unipi `selectRequestLines`.
#[must_use]
pub fn select_request_lines(requests: &[String], max: usize) -> RequestSelection {
    let all: Vec<(String, usize)> = requests
        .iter()
        .enumerate()
        .map(|(i, r)| (one_line(r), i))
        .filter(|(t, _)| !t.is_empty() && !re::test(&TRIVIAL_REQUEST, t))
        .collect();
    let mut shown = BTreeSet::new();
    let Some((first, rest)) = all.split_first() else {
        return RequestSelection::default();
    };
    let per = (max / all.len().min(6)).saturating_sub(4).max(160);
    let mut lines = vec![clip(&first.0, 240)];
    shown.insert(first.1);
    let mut budget = max.saturating_sub(clen(&lines[0]) + 3);
    let mut recent: Vec<String> = Vec::new();
    for (t, i) in rest.iter().rev() {
        if recent.len() >= 5 {
            break;
        }
        let item = clip(t, per);
        if clen(&item) + 3 > budget {
            break;
        }
        budget -= clen(&item) + 3;
        recent.insert(0, item);
        shown.insert(*i);
    }
    let omitted = rest.len() - recent.len();
    if omitted > 0 {
        lines.push(format!(
            "({omitted} more request{} in between — session_recall has them)",
            if omitted == 1 { "" } else { "s" }
        ));
    }
    lines.extend(recent);
    RequestSelection { lines, shown }
}

fn is_decision(p: &str) -> bool {
    re::test(&IMPERATIVE_START, p) || re::test(&MODAL, p) || re::test(&CORRECTION, p)
}

/// unipi `selectDecisions`.
#[must_use]
pub fn select_decisions(requests: &[String], max: usize, skip: &BTreeSet<usize>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut found: Vec<String> = Vec::new();
    for (idx, req) in requests.iter().enumerate() {
        if skip.contains(&idx) || re::test(&TRIVIAL_REQUEST, &one_line(req)) {
            continue;
        }
        if req.contains(" → ") {
            let key = take(&req.to_lowercase(), 60);
            if seen.insert(key) {
                found.push(clip(&one_line(req), 260));
            }
            continue;
        }
        let stripped =
            re::replace_all(&QUOTE_LINE, &re::replace_all(&CODE_FENCE, req, "\n"), "").into_owned();
        let pieces: Vec<String> = stripped
            .split('\n')
            .filter(|l| !l.is_empty())
            .flat_map(|l| {
                re::split(&SENTENCE_SPLIT, l)
                    .into_iter()
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|p| re::replace_first(&LIST_MARKER, &p, "").trim().to_owned())
            .filter(|p| (24..=400).contains(&clen(p)))
            .collect();
        let mut pushed: Option<usize> = None;
        for piece in pieces {
            if !is_decision(&piece) {
                continue;
            }
            if let Some(at) = pushed
                && re::test(&CORRECTION, &piece)
                && !re::test(&IMPERATIVE_START, &piece)
                && !re::test(&MODAL, &piece)
            {
                found[at] = clip(&format!("{} {}", found[at], one_line(&piece)), 260);
                continue;
            }
            if re::test(&ENDS_QUESTION, &piece)
                || re::test(&PASTED_AI, &piece)
                || (re::test(&CONTEXT_FREE, &piece) && clen(&piece) < 60)
                || re::test(&CODE_LIKE, &piece)
            {
                continue;
            }
            let key = take(&piece.to_lowercase(), 60);
            if !seen.insert(key) {
                continue;
            }
            found.push(clip(&one_line(&piece), 220));
            pushed = Some(found.len() - 1);
        }
    }
    pick_decisions(&found, max)
}

fn pick_decisions(found: &[String], max: usize) -> Vec<String> {
    fn take_i(
        found: &[String],
        i: usize,
        chosen: &mut BTreeSet<usize>,
        used: &mut usize,
        max: usize,
    ) {
        let cost = clen(&found[i]) + 3;
        if chosen.contains(&i) || *used + cost > max {
            return;
        }
        chosen.insert(i);
        *used += cost;
    }
    let mut chosen = BTreeSet::new();
    let mut used = 0;
    for i in (0..found.len()).rev() {
        if used * 2 >= max {
            break;
        }
        take_i(found, i, &mut chosen, &mut used, max);
    }
    for i in 0..found.len() {
        if re::test(&STANDING, &found[i]) || re::test(&CORRECTION, &found[i]) {
            take_i(found, i, &mut chosen, &mut used, max);
        }
    }
    for i in (0..found.len()).rev() {
        take_i(found, i, &mut chosen, &mut used, max);
    }
    chosen.into_iter().map(|i| found[i].clone()).collect()
}

/// unipi `selectState`.
#[must_use]
pub fn select_state(reports: &[String], max: usize) -> Vec<String> {
    let flat: Vec<String> = reports
        .iter()
        .map(|r| one_line(&re::replace_all(&IMAGE_MD, r, "")))
        .filter(|r| !r.is_empty())
        .collect();
    if flat.is_empty() {
        return Vec::new();
    }
    let latest = flat.len() - 1;
    let report = flat.iter().rposition(|r| clen(r) >= 400);
    let step_len = max * 3 / 10;
    match report {
        Some(r) if r == latest => vec![format!(
            "Last report: {}",
            clip(&flat[r], max.saturating_sub(20))
        )],
        None => vec![format!("Latest step: {}", clip(&flat[latest], step_len))],
        Some(r) => {
            let step = format!("Latest step: {}", clip(&flat[latest], step_len));
            let age = latest - r;
            let label = format!(
                "Last full report ({age} message{} earlier — may be outdated)",
                if age == 1 { "" } else { "s" }
            );
            let room = max.saturating_sub(clen(&step) + clen(&label) + 10);
            vec![step.clone(), format!("{label}: {}", clip(&flat[r], room))]
        }
    }
}

fn rel(p: &str, cwd: Option<&str>) -> String {
    let Some(c) = cwd.filter(|_| p.starts_with('/')) else {
        return p.to_owned();
    };
    let c = c.trim_end_matches('/');
    match p.strip_prefix(c).and_then(|r| r.strip_prefix('/')) {
        Some(r) if !r.is_empty() => r.to_owned(),
        _ => p.to_owned(),
    }
}

fn list(label: &str, items: &[String], max: usize) -> String {
    if items.is_empty() {
        return String::new();
    }
    let fitted = fit_items(items, max);
    let more = items.len() - fitted.len();
    format!(
        "{label}: {}{}",
        fitted.join(", "),
        if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        }
    )
}

/// unipi `selectFiles`.
#[must_use]
pub fn select_files(blocks: &[Block], cwd: Option<&str>, max: usize) -> Vec<String> {
    let mut modified: Vec<(String, usize)> = Vec::new();
    let mut created: Vec<String> = Vec::new();
    let mut read: Vec<String> = Vec::new();
    for b in blocks {
        let Block::ToolCall { name, args, .. } = b else {
            continue;
        };
        let Some(raw) = extract_path(args) else {
            continue;
        };
        let path = rel(&raw, cwd);
        if re::test(&EDIT_TOOLS, name) {
            match modified.iter_mut().find(|(p, _)| *p == path) {
                Some(m) => m.1 += 1,
                None => modified.push((path, 1)),
            }
        } else if re::test(&WRITE_TOOLS, name) {
            if !created.contains(&path) {
                created.push(path);
            }
        } else if re::test(&READ_TOOLS, name)
            && !re::test(&NOISE_READ, &path)
            && !read.contains(&path)
        {
            read.push(path);
        }
    }
    created.retain(|p| !modified.iter().any(|(m, _)| m == p));
    read.retain(|p| !modified.iter().any(|(m, _)| m == p) && !created.contains(p));
    let mut by_count = modified.clone();
    by_count.sort_by_key(|x| std::cmp::Reverse(x.1));
    let by_count: Vec<String> = by_count
        .into_iter()
        .map(|(p, n)| if n > 1 { format!("{p} ×{n}") } else { p })
        .collect();
    read.reverse();
    [
        list("Modified", &by_count, max * 45 / 100),
        list("Created", &created, max * 3 / 10),
        list("Read", &read, max / 5),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LessonSource {
    Memory,
    Comment,
    Diagnosis,
}

/// unipi `selectLessons`.
#[must_use]
pub fn select_lessons(blocks: &[Block], max: usize) -> Vec<String> {
    let mut out: Vec<(String, String, LessonSource)> = Vec::new();
    let add = |text: &str, src: LessonSource, out: &mut Vec<(String, String, LessonSource)>| {
        let t = one_line(text);
        if clen(&t) < 30 {
            return;
        }
        let key = item_key(&t);
        out.retain(|(k, _, _)| *k != key);
        out.push((key, clip(&t, 320), src));
    };
    for b in blocks {
        match b {
            Block::ToolCall { name, args, .. } if re::test(&MEMORY_TOOL, name) => {
                let raw = ["content", "text", "memory", "note"]
                    .iter()
                    .find_map(|k| args.get(*k).and_then(serde_json::Value::as_str));
                if let Some(raw) = raw {
                    let mut text = String::new();
                    for l in raw.split('\n').map(str::trim).filter(|l| !l.is_empty()) {
                        text = if text.is_empty() {
                            l.to_owned()
                        } else {
                            format!("{text} {l}")
                        };
                        if clen(&text) >= 160 {
                            break;
                        }
                    }
                    add(&text, LessonSource::Memory, &mut out);
                }
            }
            Block::ToolCall { args, .. } => {
                if let Some(cmd) = args.get("command").and_then(serde_json::Value::as_str) {
                    for line in cmd.split('\n') {
                        if let Some(m) = re::cap(&COMMENT_LINE, line, 1)
                            && clen(&m) <= 300
                            && !re::test(&ENDS_COLON, &m)
                            && !re::test(&DEBUG, &m)
                            && (re::test(&DIAGNOSIS, &m) || re::test(&CONFIRMED, &m))
                            && re::test(&LESSON_MARK, &m)
                        {
                            add(&m, LessonSource::Comment, &mut out);
                        }
                    }
                }
            }
            Block::Assistant { text, .. } => {
                let no_code = re::replace_all(&CODE_FENCE_CLOSED, text, " ").into_owned();
                for s in re::split(&SENTENCE_OR_NL, &no_code) {
                    let t = s.trim();
                    if (50..=320).contains(&clen(t))
                        && !t.ends_with('?')
                        && !re::test(&DEBUG, t)
                        && re::test(&DIAGNOSIS, t)
                    {
                        add(t, LessonSource::Diagnosis, &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    let mut ranked: Vec<(usize, &(String, String, LessonSource))> =
        out.iter().enumerate().collect();
    ranked.sort_by(|a, b| a.1.2.cmp(&b.1.2).then(b.0.cmp(&a.0)));
    let mut chosen: Vec<usize> = Vec::new();
    let mut used = 0;
    for (i, c) in ranked {
        if used + clen(&c.1) + 3 > max {
            continue;
        }
        chosen.push(i);
        used += clen(&c.1) + 3;
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| out[i].1.clone()).collect()
}

fn command_core(command: &str) -> String {
    let first = command
        .split('\n')
        .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .unwrap_or_default();
    let segs: Vec<String> = re::split(&SEG_SPLIT, first)
        .into_iter()
        .map(|x| x.trim().to_owned())
        .filter(|x| !x.is_empty())
        .collect();
    let main = segs
        .iter()
        .find(|x| !re::test(&CD_LIKE, x))
        .cloned()
        .unwrap_or_default();
    let a = re::replace_first(&ENV_PREFIX, &main, "").into_owned();
    re::replace_first(&PIPE_REDIRECT, &a, "").trim().to_owned()
}

/// unipi `selectKnowledge`.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "notes, commands and hosts in one pass"
)]
pub fn select_knowledge(blocks: &[Block], cwd: Option<&str>, max: usize) -> Vec<String> {
    let mut edited: Vec<String> = Vec::new();
    let mut read: Vec<String> = Vec::new();
    let mut commands: Vec<(String, String, usize)> = Vec::new();
    let mut hosts: Vec<(String, usize)> = Vec::new();
    let add_host =
        |h: String, hosts: &mut Vec<(String, usize)>| match hosts.iter_mut().find(|(x, _)| *x == h)
        {
            Some(e) => e.1 += 1,
            None => hosts.push((h, 1)),
        };
    for b in blocks {
        let Block::ToolCall { name, args, .. } = b else {
            continue;
        };
        if let Some(p) = extract_path(args).filter(|p| re::test(&NOTE_FILE, p)) {
            let r = rel(&p, cwd);
            let target = if re::test(&EDIT_TOOLS, name) || re::test(&WRITE_TOOLS, name) {
                &mut edited
            } else {
                &mut read
            };
            if !target.contains(&r) {
                target.push(r);
            }
        }
        let Some(command) = args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .filter(|c| !c.is_empty())
        else {
            continue;
        };
        let mut core = command_core(command);
        if re::test(&SSH_START, &core) {
            let head = re::split(&SSH_REMOTE, &core)[0].trim().to_owned();
            core = head;
        }
        let quotes = core.chars().filter(|c| *c == '\'' || *c == '"').count();
        if re::test(&TOOLING, &core)
            && clen(&core) <= 160
            && !re::test(&HEREDOC_OR_CONT, &core)
            && quotes % 2 == 0
        {
            let key = core
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            match commands.iter_mut().find(|(k, _, _)| *k == key) {
                Some(c) => {
                    c.1.clone_from(&core);
                    c.2 += 1;
                }
                None => commands.push((key, core, 1)),
            }
        }
        for c in re::caps_all(&URL_HOST, command) {
            if let Some(h) = c.get(1).cloned().flatten() {
                add_host(h, &mut hosts);
            }
        }
        for c in re::caps_all(&SSH_HOST, command) {
            if let Some(h) = c.get(1).cloned().flatten() {
                add_host(h, &mut hosts);
            }
        }
    }
    read.retain(|p| !edited.contains(p));
    let mut lines = Vec::new();
    let mut cap = |label: &str, items: &[String], share_pct: usize| {
        if items.is_empty() {
            return;
        }
        let fitted = fit_items(items, max * share_pct / 100);
        let more = items.len() - fitted.len();
        if !fitted.is_empty() {
            lines.push(format!(
                "{label}: {}{}",
                fitted.join(", "),
                if more > 0 {
                    format!(" (+{more} more)")
                } else {
                    String::new()
                }
            ));
        }
    };
    cap("Notes written", &edited, 20);
    let last_read: Vec<String> = read
        .iter()
        .skip(read.len().saturating_sub(8))
        .cloned()
        .collect();
    cap("Notes read", &last_read, 15);
    let mut rep: Vec<&(String, String, usize)> = commands.iter().filter(|c| c.2 >= 2).collect();
    rep.sort_by_key(|c| std::cmp::Reverse(c.2));
    let rep: Vec<String> = rep
        .iter()
        .take(8)
        .map(|c| format!("`{}` ×{}", c.1, c.2))
        .collect();
    cap("Repeated commands", &rep, 45);
    let mut hs: Vec<&(String, usize)> = hosts.iter().filter(|h| h.1 >= 2).collect();
    hs.sort_by_key(|h| std::cmp::Reverse(h.1));
    let hs: Vec<String> = hs.iter().take(6).map(|h| h.0.clone()).collect();
    cap("Hosts", &hs, 20);
    lines
}

/// unipi `selectCommits`.
#[must_use]
pub fn select_commits(blocks: &[Block], max: usize) -> Vec<String> {
    let mut commits: Vec<(String, String)> = Vec::new();
    let set = |k: String, v: String, commits: &mut Vec<(String, String)>| match commits
        .iter_mut()
        .find(|(x, _)| *x == k)
    {
        Some(e) => e.1 = v,
        None => commits.push((k, v)),
    };
    let mut pending: Option<String> = None;
    for b in blocks {
        match b {
            Block::ToolCall { args, .. } => {
                let cmd = args
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                pending = re::cap(&COMMIT_CMD, cmd, 2);
            }
            Block::ToolResult { text, is_error, .. } => {
                let mut found = false;
                if !is_error {
                    for c in re::caps_all(&COMMIT_LINE, text) {
                        let hash = c.get(2).cloned().flatten().unwrap_or_default();
                        let subject = c.get(3).cloned().flatten().unwrap_or_default();
                        let subject = re::replace_first(&COMMIT_TRAILER, &subject, "")
                            .trim()
                            .to_owned();
                        set(take(&hash, 8), clip(&subject, 110), &mut commits);
                        found = true;
                    }
                }
                if !found
                    && !is_error
                    && let Some(p) = &pending
                {
                    let k = format!("~{}", commits.len());
                    set(k, clip(p.trim(), 110), &mut commits);
                }
                pending = None;
            }
            _ => {}
        }
    }
    let lines: Vec<String> = commits
        .into_iter()
        .map(|(h, s)| {
            if h.starts_with('~') {
                s
            } else {
                format!("{h} {s}")
            }
        })
        .collect();
    let mut last: Vec<String> = lines
        .iter()
        .skip(lines.len().saturating_sub(10))
        .cloned()
        .collect();
    last.reverse();
    let mut fitted = fit_items(&last, max);
    fitted.reverse();
    fitted
}

/// unipi `selectOpenErrors`.
#[must_use]
pub fn select_open_errors(blocks: &[Block], max: usize) -> Vec<String> {
    let mut last_target: BTreeMap<String, String> = BTreeMap::new();
    let mut open: Vec<(String, String, String, Option<usize>, u32)> = Vec::new();
    let start = blocks.len().saturating_sub(TRANSCRIPT_WINDOW_BLOCKS);
    for b in &blocks[start..] {
        match b {
            Block::ToolCall { name, args, .. } => {
                let target = extract_path(args).unwrap_or_else(|| {
                    take(
                        &one_line(
                            args.get("command")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default(),
                        ),
                        60,
                    )
                });
                last_target.insert(name.clone(), target);
            }
            Block::ToolResult {
                name,
                text,
                is_error,
                source,
            } => {
                let key = format!(
                    "{name}\u{0}{}",
                    last_target.get(name).cloned().unwrap_or_default()
                );
                open.retain(|e| e.0 != key);
                if *is_error {
                    open.push((
                        key,
                        name.clone(),
                        format!("[{name}] {}", clip(&one_line(text), 200)),
                        *source,
                        0,
                    ));
                    continue;
                }
                for e in &mut open {
                    if e.1 == *name {
                        e.4 += 1;
                    }
                }
                open.retain(|e| !(e.1 == *name && e.4 >= 3));
            }
            _ => {}
        }
    }
    let lines: Vec<String> = open
        .into_iter()
        .map(|(_, _, t, r, _)| r.map_or_else(|| t.clone(), |r| format!("{t} (#{r})")))
        .collect();
    let mut last: Vec<String> = lines
        .iter()
        .skip(lines.len().saturating_sub(5))
        .cloned()
        .collect();
    last.reverse();
    let mut fitted = fit_items(&last, max);
    fitted.reverse();
    fitted
}

/// unipi `selectTranscript`.
#[must_use]
pub fn select_transcript(blocks: &[Block], max: usize, ops: &FileOps) -> String {
    if max < 200 || blocks.is_empty() {
        return String::new();
    }
    let recent = &blocks[blocks.len().saturating_sub(TRANSCRIPT_WINDOW_BLOCKS)..];
    let selected = select_ranked(recent, Some(max), ops);
    let lines: Vec<String> = compile(&selected)
        .split('\n')
        .map(|l| clip(l, 300))
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut used = 0;
    for l in lines.iter().rev() {
        if used + clen(l) + 1 > max {
            break;
        }
        out.insert(0, l.clone());
        used += clen(l) + 1;
    }
    while out.first().is_some_and(|l| !l.starts_with('[')) {
        out.remove(0);
    }
    out.join("\n").trim().to_owned()
}

/// Redacts credentials (summaries are re-sent every turn).
#[must_use]
pub fn redact_secrets(text: &str) -> String {
    let rules: [(&re::Lazy, &str); 9] = [
        (&SECRET_KV, "$1$2[redacted]"),
        (&SECRET_QUOTED, "$1$2[redacted]"),
        (&SECRET_SK, "[redacted]"),
        (&SECRET_BEARER, "$1 [redacted]"),
        (&SECRET_GH, "[redacted]"),
        (&SECRET_AKIA, "[redacted]"),
        (&SECRET_XOX, "[redacted]"),
        (&SECRET_JWT, "[redacted]"),
        (&SECRET_PEM, "[redacted private key]"),
    ];
    rules.iter().fold(text.to_owned(), |t, (r, to)| {
        re::replace_all(r, &t, to).into_owned()
    })
}

/// Summary input.
pub struct Input<'a> {
    /// Source.
    pub source: &'a Source,
    /// Active work blocks `(id, text)`.
    pub active_work: &'a [(String, String)],
    /// Target size in characters.
    pub budget_chars: usize,
    /// cwd (relative paths).
    pub cwd: Option<&'a str>,
    /// File ops.
    pub ops: &'a FileOps,
    /// Sections.
    pub sections: Sections,
}

/// Summary output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Text.
    pub text: String,
    /// Section names.
    pub sections: Vec<String>,
}

/// unipi `buildLosslessSummary`.
#[must_use]
#[allow(clippy::too_many_lines, reason = "section assembly")]
pub fn build(input: &Input<'_>) -> Summary {
    let mut src = input.source.clone();
    src.requests = src.requests.iter().map(|r| redact_secrets(r)).collect();
    src.reports = src.reports.iter().map(|r| redact_secrets(r)).collect();
    for b in &mut src.blocks {
        b.map_text(redact_secrets);
    }
    let on = input.sections;
    let budget = input.budget_chars.max(2000);
    let cap = |pct: usize| budget * pct / 100;
    let mut parts: Vec<(&str, String)> = Vec::new();
    if on.active_work && !input.active_work.is_empty() {
        let per = cap(16) / input.active_work.len();
        let lines: Vec<String> = input
            .active_work
            .iter()
            .map(|(_, t)| clip(t.trim(), per.max(200)))
            .collect();
        parts.push((
            "Active Work",
            format!("[Active Work]\n{}", lines.join("\n\n")),
        ));
    }
    let req = if on.requests {
        select_request_lines(&src.requests, cap(14))
    } else {
        RequestSelection::default()
    };
    if on.requests {
        parts.push(("Your Requests", section("Your Requests", &req.lines)));
    }
    if on.state {
        parts.push((
            "Latest State",
            section("Latest State", &select_state(&src.reports, cap(12))),
        ));
    }
    if on.decisions {
        parts.push((
            "Decisions & Constraints",
            section(
                "Decisions & Constraints",
                &select_decisions(&src.requests, cap(16), &req.shown),
            ),
        ));
    }
    if on.files {
        parts.push((
            "Files",
            section("Files", &select_files(&src.blocks, input.cwd, cap(7))),
        ));
    }
    if on.lessons {
        parts.push((
            "Lessons",
            section("Lessons", &select_lessons(&src.blocks, cap(10))),
        ));
    }
    if on.files {
        parts.push((
            "Project Knowledge",
            section(
                "Project Knowledge",
                &select_knowledge(&src.blocks, input.cwd, cap(7)),
            ),
        ));
    }
    if on.commits {
        parts.push((
            "Commits",
            section("Commits", &select_commits(&src.blocks, cap(5))),
        ));
    }
    if on.errors {
        parts.push((
            "Open Errors",
            section("Open Errors", &select_open_errors(&src.blocks, cap(6))),
        ));
    }
    let head: Vec<&(&str, String)> = parts.iter().filter(|(_, t)| !t.is_empty()).collect();
    let head_text = head
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut names: Vec<String> = head.iter().map(|(n, _)| (*n).to_owned()).collect();
    let mut transcript = String::new();
    if on.transcript {
        let remaining = budget.saturating_sub(clen(&head_text) + clen(RECALL_NOTE) + 40);
        transcript = select_transcript(&src.blocks, (budget * 18 / 100).max(remaining), input.ops);
        if !transcript.is_empty() {
            names.push("Recent Transcript".into());
        }
    }
    let body = [
        head_text,
        if transcript.is_empty() {
            String::new()
        } else {
            format!("[Recent Transcript]\n{transcript}")
        },
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n");
    if body.is_empty() {
        return Summary {
            text: String::new(),
            sections: Vec::new(),
        };
    }
    Summary {
        text: format!("{}\n\n---\n\n{RECALL_NOTE}", redact_secrets(&body)),
        sections: names,
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
    use crate::source::collect_source;

    #[test]
    fn all_compile() {
        for (n, r) in all_patterns() {
            assert!(r.is_some(), "{n}");
        }
    }

    fn build_on(
        branch: &[serde_json::Value],
        budget: usize,
        active: &[(String, String)],
    ) -> Summary {
        let src = collect_source(branch, branch.len());
        build(&Input {
            source: &src,
            active_work: active,
            budget_chars: budget,
            cwd: Some("/repo"),
            ops: &FileOps::default(),
            sections: Sections::default(),
        })
    }

    #[test]
    fn rebuilds_from_raw_history() {
        let first = user("original request from before the first compaction");
        let fid = first["id"].as_str().unwrap_or_default().to_owned();
        let branch = vec![
            first,
            assistant("working", &[]),
            compaction(
                &format!("[User Preferences]\n- {}", "BLOAT ".repeat(4000)),
                &fid,
            ),
            user("later request"),
        ];
        let s = build_on(&branch, 8000, &[]).text;
        assert!(s.contains("original request from before the first compaction"));
        assert!(s.contains("later request"));
        assert!(!s.contains("BLOAT"));
    }

    #[test]
    fn leads_with_active_work() {
        let s = build_on(
            &working_session(),
            8000,
            &[(
                "kanboard".into(),
                "Kanboard task PIT-3 \"Login\" is in progress".into(),
            )],
        );
        assert!(s.text.starts_with("[Active Work]\nKanboard task PIT-3"));
        assert_eq!(
            &s.sections[..3],
            ["Active Work", "Your Requests", "Latest State"]
        );
        assert!(s.text.contains("Build a login page"));
        assert!(s.text.contains("Now add a password reset link."));
        assert!(s.text.ends_with(RECALL_NOTE));
        let t = build_on(&working_session(), 8000, &[]).text;
        assert!(t.contains("Modified: src/Login.tsx"), "{t}");
        assert!(t.contains("abc1234 feat: login page"), "{t}");
        assert!(!t.contains("[Open Errors]"), "{t}");
    }

    #[test]
    fn budget_and_empty() {
        let mut branch = vec![
            user(&format!(
                "Build the site. {}",
                "Keep it fast. ".repeat(2000)
            )),
            custom(
                "unipi-compactor-resume",
                &format!("<session_resume>{}</session_resume>", "y".repeat(20_000)),
            ),
            user(&format!(
                "<background-task-notification> {} </background-task-notification>",
                "z".repeat(5000)
            )),
            assistant(&"r".repeat(4000), &[]),
            tool_result("bash", &"e".repeat(10_000), true),
        ];
        for i in 0..200 {
            branch.push(assistant(
                &format!("step {i} {}", "w".repeat(300)),
                &[("bash", json!({"command": format!("echo {i}")}))],
            ));
        }
        let s = build_on(&branch, 6000, &[]).text;
        assert!(clen(&s) < 6000 * 5 / 4 + clen(RECALL_NOTE), "{}", clen(&s));
        assert!(!s.contains("yyyyyyyyyy") && !s.contains("zzzzzzzzzz"));
        assert_eq!(build_on(&[], 8000, &[]).text, "");
    }

    #[test]
    fn selectors() {
        let mut reqs: Vec<String> = vec!["first ask".into(), "continue".into(), "ok".into()];
        reqs.extend((0..9).map(|i| format!("ask {i}")));
        let lines = select_request_lines(&reqs, 2000).lines;
        assert_eq!(lines[0], "first ask");
        assert!(lines.iter().any(|l| l.contains("more requests in between")));
        assert_eq!(lines.last().map(String::as_str), Some("ask 8"));
        let d = select_decisions(
            &[
                "Please keep the orange theme. Should we add dark mode?".into(),
                "Never commit the API key.".into(),
            ],
            1000,
            &BTreeSet::new(),
        );
        assert!(
            d.contains(&"Please keep the orange theme.".to_owned()),
            "{d:?}"
        );
        assert!(d.contains(&"Never commit the API key.".to_owned()));
        assert!(!d.iter().any(|l| l.contains("dark mode")));
        let d = select_decisions(
            &[
                "Camera don't have permission, and no request for it too.".into(),
                "Don't use emoji in the agent replies.".into(),
                "**Deep dive**: the lead should orchestrate every stage.".into(),
                "It is on this port instead.".into(),
                "Use this as a prompt:\n```\nYou should always write short sentences.\n```".into(),
            ],
            2000,
            &BTreeSet::new(),
        );
        assert_eq!(d, ["Don't use emoji in the agent replies."]);
        let d = select_decisions(&["Also, restore the brown pink like background just now. Currently it turns to other colors already. I did not request for this change.".into()], 1000, &BTreeSet::new());
        assert_eq!(
            d,
            [
                "Also, restore the brown pink like background just now. I did not request for this change."
            ]
        );
        let st = select_state(
            &[
                "R".repeat(500),
                "short step one".into(),
                "Now checking the bloom defaults.".into(),
            ],
            1200,
        );
        assert_eq!(st[0], "Latest step: Now checking the bloom defaults.");
        assert!(st[1].starts_with("Last full report (2 messages earlier — may be outdated)"));
        let blocks = collect_source(
            &[
                assistant(
                    "",
                    &[(
                        "bash",
                        json!({"command": "git add -A && git commit -qm \"feat(webgl): field\""}),
                    )],
                ),
                tool_result("bash", "", false),
            ],
            2,
        )
        .blocks;
        assert_eq!(select_commits(&blocks, 500), ["feat(webgl): field"]);
        let mut branch = vec![
            assistant("", &[("bash", json!({"command": "cat missing"}))]),
            tool_result("bash", "ENOENT missing", true),
        ];
        assert!(
            select_open_errors(&collect_source(&branch, 2).blocks, 1000)[0]
                .contains("ENOENT missing")
        );
        for n in 1..=3 {
            branch.push(assistant(
                "",
                &[("bash", json!({"command": format!("ls {n}")}))],
            ));
            branch.push(tool_result("bash", "ok", false));
        }
        assert!(select_open_errors(&collect_source(&branch, branch.len()).blocks, 1000).is_empty());
    }

    #[test]
    fn redaction_and_knowledge() {
        let t = build_on(
            &[
                user("ssh to it, the password is '9801(*)!Pi', should be working fine though."),
                user("key sk-abcdefghijklmnopqrstuvwx please"),
            ],
            8000,
            &[],
        )
        .text;
        assert!(!t.contains("9801(*)!Pi") && !t.contains("sk-abcdefghijklmnopqrstuvwx"));
        assert!(t.contains("password is [redacted]"), "{t}");
        let q = build_on(
            &[user(
                "Sudo needs a terminal → Use password '9801(*)!Pi' for it",
            )],
            8000,
            &[],
        )
        .text;
        assert!(!q.contains("9801"));
        let call = |c: &str| assistant("", &[("bash", json!({"command": c}))]);
        let branch = vec![
            user("deploy it"),
            assistant(
                "",
                &[(
                    "write",
                    json!({"path": ".agents/skills/app-deploy/SKILL.md", "content": "x"}),
                )],
            ),
            call("cd app && pnpm typecheck 2>&1 | tail -5"),
            call("pnpm typecheck"),
            call("ssh -i keys/white root@1.2.3.4 'systemctl restart app'"),
            call("ssh -i keys/white root@1.2.3.4 'journalctl -n 5'"),
            call("curl -s https://app.example.com/health"),
            call("curl -s https://app.example.com/ready"),
            call("ls"),
            user("latest"),
        ];
        let t = build_on(&branch, 8000, &[]).text;
        assert!(t.contains("[Project Knowledge]"), "{t}");
        assert!(t.contains("Notes written: .agents/skills/app-deploy/SKILL.md"));
        assert!(t.contains("`pnpm typecheck` ×2"), "{t}");
        assert!(t.contains("`ssh -i keys/white root@1.2.3.4` ×2"), "{t}");
        assert!(t.contains("app.example.com"));
        assert!(!t.contains("`ls`"));
        assert_eq!(auto_budget_tokens(0), 1500);
        assert_eq!(auto_budget_tokens(1000), 4000);
    }
}
