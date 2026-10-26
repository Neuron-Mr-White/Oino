//! `session_recall` (unipi `vcc-recall.ts`, `search-entries.ts`, `drill-down.ts`,
//! `format-recall.ts`, `recall-scope.ts`, `touched-files.ts`): hybrid regex / BM25 search,
//! pages of 5, `expand`, `mode: touched`, `#N:path` drill-down, a 3 s search budget.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use fancy_regex::{Regex, RegexBuilder};
use serde_json::{Map, Value};

use crate::blocks::{Block, extract_path};

/// Max results.
pub const MAX_RECALL_RESULTS: usize = 50;
/// Max bytes of one expanded hit.
pub const MAX_EXPANDED_HIT_BYTES: usize = 16 * 1024;
/// Recent entries without a query.
pub const DEFAULT_RECENT: usize = 25;
/// Page size.
pub const PAGE_SIZE: usize = 5;
/// Search budget.
pub const SEARCH_BUDGET_MS: i64 = 3000;

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "can", "shall", "of",
    "in", "to", "for", "with", "on", "at", "from", "by", "as", "into", "through", "during",
    "before", "after", "above", "below", "between", "out", "off", "over", "under", "again",
    "further", "then", "once", "here", "there", "when", "where", "why", "how", "all", "both",
    "each", "few", "more", "most", "other", "some", "such", "no", "nor", "not", "only", "own",
    "same", "so", "than", "too", "very", "just", "about", "it", "its", "that", "this", "what",
    "which", "who", "whom", "these", "those",
];

fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn quantifier_at(p: &[char], i: usize) -> (usize, bool) {
    match p.get(i) {
        Some('+' | '*') => (1, true),
        Some('{') => {
            let Some(end) = p[i..].iter().position(|c| *c == '}').map(|e| e + i) else {
                return (0, false);
            };
            let body: String = p[i + 1..end].iter().collect();
            let (a, b) = body.split_once(',').unwrap_or((&body, ""));
            let ok = !a.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit());
            if ok {
                (end - i + 1, body.ends_with(','))
            } else {
                (0, false)
            }
        }
        _ => (0, false),
    }
}

/// An unbounded quantifier on a group that already contains one (`(a+)+`).
#[must_use]
pub fn has_nested_quantifier(pattern: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let mut groups: Vec<bool> = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        if c == '\\' {
            i += 2;
            continue;
        }
        if in_class {
            if c == ']' {
                in_class = false;
            }
            i += 1;
            continue;
        }
        match c {
            '[' => in_class = true,
            '(' => groups.push(false),
            ')' => {
                let inner = groups.pop().unwrap_or(false);
                let (len, unb) = quantifier_at(&p, i + 1);
                if inner && unb {
                    return true;
                }
                if let Some(g) = groups.last_mut() {
                    *g = *g || inner || unb;
                }
                i += len;
            }
            _ => {
                let (len, unb) = quantifier_at(&p, i);
                if unb && let Some(g) = groups.last_mut() {
                    *g = true;
                    i += len.saturating_sub(1);
                }
            }
        }
        i += 1;
    }
    false
}

fn ci(pattern: &str) -> Option<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .ok()
}

/// Compiles a query as a case-insensitive regex, falling back to the escaped literal.
#[must_use]
pub fn safe_regex(pattern: &str) -> Option<Regex> {
    if has_nested_quantifier(pattern) {
        return ci(&escape(pattern));
    }
    ci(pattern).or_else(|| ci(&escape(pattern)))
}

fn looks_like_regex(q: &str) -> bool {
    q.chars().any(|c| "|*+?{}()[]\\^$.".contains(c))
}

fn is_match(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

/// A search hit.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// Entry index.
    pub index: usize,
    /// Score.
    pub score: f64,
    /// Doc text.
    pub text: String,
    /// Block kind.
    pub kind: &'static str,
    /// Snippet.
    pub snippet: Option<String>,
    /// Files.
    pub files: Vec<String>,
}

fn block_text(b: &Block) -> String {
    match b {
        Block::ToolCall { name, args, .. } => format!("{name} {}", Value::Object(args.clone())),
        Block::ToolResult { name, text, .. } => format!("{name} {text}"),
        other => other.text().unwrap_or_default().to_owned(),
    }
}

fn block_files(b: &Block) -> Vec<String> {
    match b {
        Block::ToolCall { args, .. } => extract_path(args).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn line_snippet(text: &str, r: &Regex) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let m = lines.iter().position(|l| is_match(r, l))?;
    let start = m.saturating_sub(2);
    let end = (m + 3).min(lines.len());
    let mut parts = Vec::new();
    if start > 0 {
        parts.push(format!("...({start} lines above)"));
    }
    parts.extend(lines[start..end].iter().map(|l| (*l).to_owned()));
    if end < lines.len() {
        parts.push(format!("...({} lines below)", lines.len() - end));
    }
    Some(parts.join("\n"))
}

/// Search failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aborted;

impl std::fmt::Display for Aborted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Search aborted: query exceeded {SEARCH_BUDGET_MS}ms. Simplify the pattern — nested quantifiers such as (a+)+ can make matching blow up."
        )
    }
}

/// unipi `searchEntries`. `now` returns epoch ms (budget checkpoints).
///
/// # Errors
///
/// [`Aborted`] when the search exceeds its budget.
#[allow(clippy::too_many_lines, reason = "regex path then BM25 path")]
pub fn search(
    blocks: &[Block],
    query: Option<&str>,
    now: &dyn Fn() -> i64,
) -> Result<Vec<Hit>, Aborted> {
    let docs: Vec<(usize, String, &'static str, Vec<String>)> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            (
                b.source().unwrap_or(i),
                block_text(b),
                b.kind(),
                block_files(b),
            )
        })
        .collect();
    let q = query.map(str::trim).unwrap_or_default();
    let hit =
        |d: &(usize, String, &'static str, Vec<String>), score: f64, snippet: Option<String>| Hit {
            index: d.0,
            score,
            text: d.1.clone(),
            kind: d.2,
            snippet,
            files: d.3.clone(),
        };
    if q.is_empty() {
        return Ok(docs.iter().map(|d| hit(d, 0.0, None)).collect());
    }
    let deadline = now() + SEARCH_BUDGET_MS;
    let check = || {
        if now() > deadline {
            Err(Aborted)
        } else {
            Ok(())
        }
    };
    let hay = |d: &(usize, String, &'static str, Vec<String>)| {
        format!("{} {} {}", d.2, d.1, d.3.join(" "))
    };
    if looks_like_regex(q)
        && let Some(r) = safe_regex(q)
    {
        let mut hits = Vec::new();
        for d in &docs {
            check()?;
            if is_match(&r, &hay(d)) {
                hits.push(hit(d, 1.0, line_snippet(&d.1, &r)));
            }
        }
        if !hits.is_empty() {
            return Ok(hits);
        }
    }
    let raw: Vec<&str> = q.split_whitespace().collect();
    let meaningful: Vec<&str> = raw
        .iter()
        .copied()
        .filter(|t| !STOPWORDS.contains(&t.to_lowercase().as_str()) && t.chars().count() > 1)
        .collect();
    let terms = if meaningful.is_empty() {
        raw
    } else {
        meaningful
    };
    let term_res: Vec<Option<Regex>> = terms.iter().map(|t| safe_regex(t)).collect();
    let snip = ci(&term_res
        .iter()
        .flatten()
        .map(|r| r.as_str().to_owned())
        .collect::<Vec<_>>()
        .join("|"));
    let hays: Vec<String> = docs.iter().map(hay).collect();
    let n = hays.len();
    let mut df: BTreeMap<usize, usize> = BTreeMap::new();
    let mut total_len = 0usize;
    for h in &hays {
        check()?;
        total_len += h.split_whitespace().count().max(1);
        for (ti, r) in term_res.iter().enumerate() {
            if r.as_ref().is_some_and(|r| is_match(r, h)) {
                *df.entry(ti).or_default() += 1;
            }
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
    let avg = total_len as f64 / n.max(1) as f64;
    let mut scored: Vec<Hit> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        check()?;
        let h = &hays[i];
        let mc = term_res
            .iter()
            .filter(|r| r.as_ref().is_some_and(|r| is_match(r, h)))
            .count();
        if mc == 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
        let dl = h.split_whitespace().count().max(1) as f64;
        let mut score = 0.0;
        for (ti, r) in term_res.iter().enumerate() {
            let Some(r) = r else { continue };
            #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
            let tf = r.find_iter(h).flatten().count() as f64;
            if tf == 0.0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "counts fit f64")]
            let (dfv, nn) = (*df.get(&ti).unwrap_or(&0) as f64, n as f64);
            let idf = ((nn - dfv + 0.5) / (dfv + 0.5) + 1.0).ln();
            score += idf * (tf * 2.2) / (tf + 1.2 * (0.25 + 0.75 * dl / avg));
        }
        scored.push(hit(
            d,
            score,
            snip.as_ref().and_then(|s| line_snippet(&d.1, s)),
        ));
    }
    scored.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(scored)
}

/// `./rel` inside `cwd`, `.../a/b/c` when long.
#[must_use]
pub fn short_path(full: &str, cwd: &str) -> String {
    let n = full.replace('\\', "/");
    let c = cwd.replace('\\', "/");
    if let Some(r) = n.strip_prefix(&format!("{c}/")) {
        return format!("./{r}");
    }
    let parts: Vec<&str> = n.split('/').collect();
    if parts.len() > 3 {
        format!(".../{}", parts[parts.len() - 3..].join("/"))
    } else {
        n
    }
}

/// Files touched with their entries.
#[must_use]
pub fn touched_files(blocks: &[Block]) -> Vec<(String, Vec<(usize, String)>)> {
    let mut out: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for b in blocks {
        if let Block::ToolCall { name, args, source } = b
            && let Some(p) = extract_path(args)
        {
            let e = (source.unwrap_or(0), name.clone());
            match out.iter_mut().find(|(x, _)| *x == p) {
                Some(t) => t.1.push(e),
                None => out.push((p, vec![e])),
            }
        }
    }
    out
}

fn format_touched(t: &[(String, Vec<(usize, String)>)], page: usize, cwd: &str) -> String {
    if t.is_empty() {
        return "No file operations found in session history.".into();
    }
    let pages = t.len().div_ceil(PAGE_SIZE);
    let page = page.max(1);
    let start = (page - 1) * PAGE_SIZE;
    let header = if pages > 1 {
        format!("Page {page}/{pages} ({} total files)", t.len())
    } else {
        format!("{} files touched", t.len())
    };
    let lines: Vec<String> = t
        .iter()
        .skip(start)
        .take(PAGE_SIZE)
        .map(|(p, es)| {
            format!(
                "  {}    {}",
                short_path(p, cwd),
                es.iter()
                    .map(|(i, n)| format!("#{i} ({n})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    let mut r = format!("{header}:\n\n{}", lines.join("\n"));
    if page < pages {
        let _ = write!(r, "\n\n--- Use page:{} for more results ---", page + 1);
    }
    r
}

fn format_hits(hits: &[Hit], query: Option<&str>, header: Option<&str>) -> String {
    if hits.is_empty() {
        return query.map_or_else(
            || "No entries in session history.".to_owned(),
            |q| format!("No matches for \"{q}\" in session history."),
        );
    }
    let head = match (header, query) {
        (Some(h), q) => format!("{h} for \"{}\":", q.unwrap_or_default()),
        (None, Some(q)) => format!("Found {} matches for \"{q}\":", hits.len()),
        (None, None) => format!("Session history ({} entries):", hits.len()),
    };
    let lines: Vec<String> = hits
        .iter()
        .map(|h| {
            let files = if h.files.is_empty() {
                String::new()
            } else {
                format!(" files:[{}]", h.files.join(", "))
            };
            format!(
                "#{} [{}]{files} {}",
                h.index,
                h.kind,
                h.snippet.as_deref().unwrap_or(&h.text)
            )
        })
        .collect();
    format!("{head}\n\n{}", lines.join("\n\n"))
}

/// Parsed `#N:path[:full|:offset[:limit]]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrillDown {
    /// Entry index.
    pub index: usize,
    /// Path pattern.
    pub path: String,
    /// Full content.
    pub full: bool,
    /// Line offset.
    pub offset: Option<usize>,
    /// Line limit.
    pub limit: Option<usize>,
}

/// unipi `parseDrillDown` (whole query only).
#[must_use]
pub fn parse_drill_down(q: &str) -> Option<DrillDown> {
    let rest = q.strip_prefix('#')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let index = digits.parse().ok()?;
    let rest = rest[digits.len()..].strip_prefix(':')?;
    if rest.is_empty() {
        return None;
    }
    let mut d = DrillDown {
        index,
        path: rest.to_owned(),
        full: false,
        offset: None,
        limit: None,
    };
    if let Some(p) = rest.strip_suffix(":full").filter(|p| !p.is_empty()) {
        p.clone_into(&mut d.path);
        d.full = true;
        return Some(d);
    }
    let segs: Vec<&str> = rest.rsplitn(3, ':').collect();
    let num = |s: &str| {
        (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .then(|| s.parse::<usize>().ok())
            .flatten()
    };
    if segs.len() == 3 && !segs[2].is_empty() && num(segs[0]).is_some() && num(segs[1]).is_some() {
        segs[2].clone_into(&mut d.path);
        d.offset = num(segs[1]);
        d.limit = num(segs[0]);
    } else if segs.len() >= 2 && num(segs[0]).is_some() {
        let p = &rest[..rest.len() - segs[0].len() - 1];
        if !p.is_empty() {
            p.clone_into(&mut d.path);
            d.offset = num(segs[0]);
        }
    }
    Some(d)
}

fn content_bearing(args: &Map<String, Value>) -> bool {
    let has_path = ["path", "file_path", "filePath", "file"]
        .iter()
        .any(|k| args.get(*k).is_some_and(Value::is_string));
    if !has_path {
        return false;
    }
    let s = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    };
    s("content")
        || args
            .get("edits")
            .and_then(Value::as_array)
            .is_some_and(|e| !e.is_empty() && e.iter().all(Value::is_object))
        || (args.get("edits").is_none() && (s("oldText") || s("newText")))
}

fn format_call(
    name: &str,
    path: &str,
    args: &Map<String, Value>,
    idx: usize,
    d: &DrillDown,
) -> String {
    let st = |k: &str| args.get(k).and_then(Value::as_str);
    let body = if let Some(c) = st("content").filter(|c| !c.is_empty()) {
        c.to_owned()
    } else if let Some(edits) = args.get("edits").and_then(Value::as_array) {
        edits
            .iter()
            .enumerate()
            .map(|(i, e)| {
                format!(
                    "--- edit {} ---\n{}\n--- becomes ---\n{}",
                    i + 1,
                    e.get("oldText").and_then(Value::as_str).unwrap_or_default(),
                    e.get("newText").and_then(Value::as_str).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    } else if let (Some(o), Some(n)) = (
        st("oldText").filter(|v| !v.is_empty()),
        st("newText").filter(|v| !v.is_empty()),
    ) {
        format!("--- old ---\n{o}\n--- new ---\n{n}")
    } else {
        "(no file content found in tool call arguments)".to_owned()
    };
    let lines: Vec<&str> = body.split('\n').collect();
    let total = lines.len();
    let head = format!("File: {path}\nTool: {name}\n\n");
    if d.full {
        const MAX: usize = 50 * 1024;
        if body.len() > MAX {
            let mut cut = MAX;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            return format!(
                "{head}{}\n\n... ({} more bytes — file exceeds 50KB display limit. Use #{idx}:{path}:30 for next page.)",
                &body[..cut],
                body.len() - MAX
            );
        }
        return format!("{head}{body}");
    }
    if let Some(off) = d.offset {
        let max = d.limit.unwrap_or(30);
        let end = (off + max).min(total);
        if off >= total {
            return format!(
                "Offset {off} is beyond file length {total}. Use #{idx}:{path} for the first 30 lines."
            );
        }
        let mut r = format!(
            "File: {path}\nTool: {name}\nLines {}-{end} (of {total}):\n\n{}",
            off + 1,
            lines[off..end].join("\n")
        );
        if end < total {
            let _ = write!(
                r,
                "\n\n--- Use #{idx}:{path}:{end} or #{idx}:{path}:{end}:{max} for next {max} lines, #{idx}:{path}:full for complete ---"
            );
        } else if off > 0 {
            r.push_str("\n\n(End of file)");
        }
        return r;
    }
    if total > 30 {
        return format!(
            "{head}{}\n\n...({} more lines — use #{idx}:{path}:full for complete content, or #{idx}:{path}:30 for next 30 lines)",
            lines[..30].join("\n"),
            total - 30
        );
    }
    format!("{head}{body}")
}

/// unipi `expandEntryFile`.
#[must_use]
pub fn expand_entry_file(blocks: &[Block], d: &DrillDown) -> String {
    let idx = d.index;
    let entry: Vec<&Block> = blocks.iter().filter(|b| b.source() == Some(idx)).collect();
    if entry.is_empty() {
        return format!("Entry #{idx} not found in session history.");
    }
    let calls: Vec<(&str, String, &Map<String, Value>)> = entry
        .iter()
        .filter_map(|b| match b {
            Block::ToolCall { name, args, .. } if content_bearing(args) => {
                ["path", "file_path", "filePath", "file"]
                    .iter()
                    .find_map(|k| {
                        args.get(*k)
                            .and_then(Value::as_str)
                            .filter(|v| !v.is_empty())
                    })
                    .map(|p| (name.as_str(), p.to_owned(), args))
            }
            _ => None,
        })
        .collect();
    let listing = |cs: &[&(&str, String, &Map<String, Value>)]| {
        cs.iter()
            .map(|(n, p, _)| format!("  [#{idx}:{p}] {n}({p})"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if d.path == "file" {
        return match calls.len() {
            0 => format!("No file content found in entry #{idx}."),
            1 => format_call(calls[0].0, &calls[0].1, calls[0].2, idx, d),
            n => format!(
                "Entry #{idx} has {n} file operations:\n{}\n\nUse #{idx}:path to drill into a specific file.",
                listing(&calls.iter().collect::<Vec<_>>())
            ),
        };
    }
    let matched: Vec<&(&str, String, &Map<String, Value>)> =
        calls.iter().filter(|c| c.1.contains(&d.path)).collect();
    match matched.len() {
        0 => format!("No file content found in entry #{idx} for \"{}\".", d.path),
        1 => format_call(matched[0].0, &matched[0].1, matched[0].2, idx, d),
        n => format!(
            "Entry #{idx} has {n} file operations matching \"{}\":\n{}\n\nUse #{idx}:<more-specific-path> to drill into a specific file.",
            d.path,
            listing(&matched)
        ),
    }
}

fn truncate_expanded(text: &str) -> String {
    if text.len() <= MAX_EXPANDED_HIT_BYTES {
        return text.to_owned();
    }
    let mut cut = MAX_EXPANDED_HIT_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n… {} bytes omitted from this hit; narrow the query to inspect more specific context …",
        &text[..cut],
        text.len() - MAX_EXPANDED_HIT_BYTES
    )
}

/// Recall input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Query.
    pub query: Option<String>,
    /// `all` scope.
    pub all: bool,
    /// `touched` mode.
    pub touched: bool,
    /// Page (1-based).
    pub page: Option<usize>,
    /// Entries to expand.
    pub expand: Vec<i64>,
}

/// unipi `vccRecall`.
#[must_use]
#[allow(clippy::too_many_lines, reason = "drill-down, touched, expand, search")]
pub fn recall(blocks: &[Block], q: &Query, cwd: &str, now: &dyn Fn() -> i64) -> String {
    let scope_suffix = if q.all { " (scope: all)" } else { "" };
    let query = q.query.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(d) = query.and_then(parse_drill_down) {
        return expand_entry_file(blocks, &d);
    }
    if q.touched {
        return format_touched(&touched_files(blocks), q.page.unwrap_or(1), cwd);
    }
    if !q.expand.is_empty() && query.is_none() {
        let mut by: BTreeMap<usize, &Block> = BTreeMap::new();
        for (i, b) in blocks.iter().enumerate() {
            by.entry(b.source().unwrap_or(i)).or_insert(b);
        }
        let requested: BTreeSet<i64> = q.expand.iter().copied().collect();
        let invalid: Vec<String> = requested
            .iter()
            .filter(|i| {
                usize::try_from(**i)
                    .ok()
                    .is_none_or(|u| !by.contains_key(&u))
            })
            .map(ToString::to_string)
            .collect();
        if !invalid.is_empty() {
            return format!(
                "Cannot expand indices outside {}: {}",
                if q.all {
                    "session history"
                } else {
                    "active lineage"
                },
                invalid.join(", ")
            );
        }
        let lines: Vec<String> = requested
            .iter()
            .filter_map(|i| usize::try_from(*i).ok())
            .filter_map(|i| by.get(&i).map(|b| (i, *b)))
            .map(|(i, b)| {
                let t = match b {
                    Block::ToolCall { name, args, .. } => {
                        format!("{name} {}", Value::Object(args.clone()))
                    }
                    other => other.text().unwrap_or_default().to_owned(),
                };
                format!("#{i} [{}] {}", b.kind(), truncate_expanded(&t))
            })
            .collect();
        return format!(
            "{}{}",
            if q.all { "Scope: all\n\n" } else { "" },
            lines.join("\n\n")
        );
    }
    let all = match search(blocks, query, now) {
        Ok(h) => h,
        Err(e) => return e.to_string(),
    };
    if let Some(qs) = query {
        let all: Vec<Hit> = all.into_iter().take(MAX_RECALL_RESULTS).collect();
        let page = q.page.unwrap_or(1).max(1);
        let pages = all.len().div_ceil(PAGE_SIZE);
        let hits: Vec<Hit> = all
            .iter()
            .skip((page - 1) * PAGE_SIZE)
            .take(PAGE_SIZE)
            .cloned()
            .collect();
        if hits.is_empty() {
            return format!(
                "No matches for \"{qs}\"{scope_suffix}{}.",
                if page > 1 {
                    format!(" on page {page}")
                } else {
                    String::new()
                }
            );
        }
        let header = if pages > 1 {
            format!(
                "Page {page}/{pages} ({} total matches{scope_suffix})",
                all.len()
            )
        } else {
            format!("{} matches{scope_suffix}", all.len())
        };
        let footer = if page < pages {
            format!(
                "\n--- Use page:{}{} for more results ---",
                page + 1,
                if q.all { " with scope:'all'" } else { "" }
            )
        } else {
            String::new()
        };
        return format!("{}{footer}", format_hits(&hits, Some(qs), Some(&header)));
    }
    let recent: Vec<Hit> = all
        .iter()
        .skip(all.len().saturating_sub(DEFAULT_RECENT))
        .cloned()
        .collect();
    format!(
        "{}{}",
        if q.all { "Scope: all\n\n" } else { "" },
        format_hits(&recent, None, None)
    )
}

/// unipi `parseRecallScope` for the command form (`scope:all`, `page:N`).
#[must_use]
pub fn parse_command_args(raw: &str) -> (bool, usize, String) {
    let mut all = false;
    let mut page = 1;
    let mut words = Vec::new();
    let mut scope_seen = false;
    let mut page_seen = false;
    for w in raw.split_whitespace() {
        let l = w.to_lowercase();
        if !scope_seen && (l == "scope:all" || l == "scope:lineage") {
            all = l == "scope:all";
            scope_seen = true;
            continue;
        }
        if !page_seen
            && let Some(n) = l
                .strip_prefix("page:")
                .and_then(|n| n.parse::<usize>().ok())
        {
            page = n.max(1);
            page_seen = true;
            continue;
        }
        words.push(w);
    }
    (all, page, words.join(" "))
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

    fn blocks() -> Vec<Block> {
        vec![
            Block::User { text: "set up the redis cache for sessions".into(), source: Some(0) },
            Block::Assistant { text: "Decided: redis cache with ttl 60s".into(), source: Some(1) },
            Block::ToolCall { name: "write".into(), args: json!({"path": "/w/src/cache.ts", "content": (0..40).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n")}).as_object().cloned().unwrap_or_default(), source: Some(2) },
            Block::ToolResult { name: "bash".into(), text: "all good".into(), is_error: false, source: Some(3) },
        ]
    }

    fn now() -> i64 {
        0
    }

    #[test]
    fn searching() {
        let b = blocks();
        let hits = search(&b, Some("redis cache decision"), &now).unwrap_or_default();
        assert_eq!(hits[0].index, 1);
        assert!(
            search(&b, Some("ca.he"), &now)
                .unwrap_or_default()
                .iter()
                .any(|h| h.index == 0)
        );
        assert!(has_nested_quantifier("(a+)+"));
        assert!(!has_nested_quantifier("a+b*"));
        assert!(safe_regex("(unclosed").is_some());
        let t = recall(
            &b,
            &Query {
                query: Some("redis".into()),
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert!(t.starts_with("2 matches for \"redis\":"), "{t}");
        let t = recall(
            &b,
            &Query {
                query: Some("nothinghere".into()),
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert_eq!(t, "No matches for \"nothinghere\".");
        let t = recall(&b, &Query::default(), "/w", &now);
        assert!(t.starts_with("Session history (4 entries):"));
        let t = recall(
            &b,
            &Query {
                expand: vec![1, 9],
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert_eq!(t, "Cannot expand indices outside active lineage: 9");
        let t = recall(
            &b,
            &Query {
                expand: vec![1],
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert_eq!(t, "#1 [assistant] Decided: redis cache with ttl 60s");
        let t = recall(
            &b,
            &Query {
                touched: true,
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert!(t.contains("./src/cache.ts    #2 (write)"), "{t}");
        let slow = std::cell::Cell::new(0);
        let tick = || {
            slow.set(slow.get() + 5000);
            slow.get()
        };
        assert!(search(&b, Some("redis"), &tick).is_err());
    }

    #[test]
    fn drilling() {
        let b = blocks();
        assert_eq!(
            parse_drill_down("#2:cache.ts"),
            Some(DrillDown {
                index: 2,
                path: "cache.ts".into(),
                full: false,
                offset: None,
                limit: None
            })
        );
        assert_eq!(
            parse_drill_down("#2:cache.ts:full").map(|d| d.full),
            Some(true)
        );
        assert_eq!(
            parse_drill_down("#2:cache.ts:30").and_then(|d| d.offset),
            Some(30)
        );
        assert_eq!(
            parse_drill_down("#2:cache.ts:30:5").map(|d| (d.offset, d.limit)),
            Some((Some(30), Some(5)))
        );
        assert_eq!(parse_drill_down("see #2:x"), None);
        let t = recall(
            &b,
            &Query {
                query: Some("#2:cache.ts".into()),
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert!(
            t.starts_with("File: /w/src/cache.ts\nTool: write\n\nline 0"),
            "{t}"
        );
        assert!(t.contains("...(10 more lines"));
        let t = recall(
            &b,
            &Query {
                query: Some("#2:cache.ts:35:3".into()),
                ..Query::default()
            },
            "/w",
            &now,
        );
        assert!(t.contains("Lines 36-38 (of 40):"), "{t}");
        assert_eq!(
            expand_entry_file(
                &b,
                &DrillDown {
                    index: 7,
                    path: "x".into(),
                    full: false,
                    offset: None,
                    limit: None
                }
            ),
            "Entry #7 not found in session history."
        );
        assert_eq!(
            parse_command_args("redis scope:all page:2"),
            (true, 2, "redis".into())
        );
        assert_eq!(short_path("/a/b/c/d/e.ts", "/w"), ".../c/d/e.ts");
    }
}
