//! `oino.search` — read-only `grep`/`find` tools over the preopened project
//! directory. Respects `.gitignore` via the `ignore` crate; regex via `regex`.
//! Output is sorted (path, then line) and capped at 30k characters.

use ignore::WalkBuilder;
use oino_extension_guest::{
    export_extension,
    ext::oino::extension::types::{ContentBlock, ToolDeclaration},
    Extension, Registration, ToolOutput,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_OUTPUT_CHARS: usize = 30_000;
const DEFAULT_GREP_MAX: u64 = 100;
const DEFAULT_FIND_MAX: u64 = 200;

struct Search;

impl Extension for Search {
    fn init(_config: Value) -> Result<Registration, String> {
        Ok(Registration {
            tools: vec![
                ToolDeclaration {
                    name: "grep".into(),
                    description: "Search file contents with a regex. Respects .gitignore; results are sorted by path then line and capped at 30k characters.".into(),
                    input_schema: Some(
                        r#"{"type":"object","additionalProperties":false,"required":["pattern"],"properties":{"pattern":{"type":"string","description":"Regular expression to search for"},"path":{"type":"string","description":"Directory or file to search (default: project root)"},"glob":{"type":"string","description":"Only search files whose path matches this glob"},"case_insensitive":{"type":"boolean","description":"Case-insensitive matching"},"max_results":{"type":"number","description":"Maximum matching lines (default 100)"}}}"#.into(),
                    ),
                },
                ToolDeclaration {
                    name: "find".into(),
                    description: "Find files by glob pattern. Respects .gitignore; results are sorted by path and capped at 30k characters.".into(),
                    input_schema: Some(
                        r#"{"type":"object","additionalProperties":false,"required":["pattern"],"properties":{"pattern":{"type":"string","description":"Glob pattern matched against file names (e.g. `*.rs`)"},"path":{"type":"string","description":"Directory to search (default: project root)"},"max_results":{"type":"number","description":"Maximum results (default 200)"}}}"#.into(),
                    ),
                },
            ],
            ..Registration::default()
        })
    }

    fn run_tool(name: &str, _call_id: &str, args: Value) -> Result<ToolOutput, String> {
        match name {
            "grep" => run_grep(&args),
            "find" => run_find(&args),
            _ => Err(format!("unknown tool `{name}`")),
        }
    }
}

fn run_grep(args: &Value) -> Result<ToolOutput, String> {
    let pattern = args["pattern"].as_str().unwrap_or_default();
    if pattern.is_empty() {
        return Err("missing `pattern`".into());
    }
    let case_insensitive = args["case_insensitive"].as_bool().unwrap_or(false);
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|err| format!("invalid regex: {err}"))?;
    let max = args["max_results"].as_u64().unwrap_or(DEFAULT_GREP_MAX) as usize;
    let root = search_root(args);
    let glob_filter = glob_matcher(args["glob"].as_str());

    // path -> (line_no, text) pairs, sorted for determinism
    let mut matches: BTreeMap<PathBuf, Vec<(u64, String)>> = BTreeMap::new();
    let mut total = 0usize;
    for entry in walk_entries(&root) {
        if total >= max {
            break;
        }
        let path = entry.path();
        if !entry.file_type().is_some_and(|ty| ty.is_file()) {
            continue;
        }
        if !glob_filter(path) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue; // binary/unreadable files are skipped, not errors
        };
        let mut hits = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if re.is_match(line) {
                hits.push((index as u64 + 1, line.to_string()));
                total += 1;
                if total >= max {
                    break;
                }
            }
        }
        if !hits.is_empty() {
            matches.insert(display_path(&root, path), hits);
        }
    }
    let mut out = String::new();
    for (path, hits) in &matches {
        for (line_no, line) in hits {
            out.push_str(&format!("{}:{}: {}\n", path.display(), line_no, line));
        }
    }
    let text = cap_output(
        out,
        &format!("call grep with a narrower pattern or path for {root}"),
    );
    Ok(ToolOutput {
        content: vec![ContentBlock::Text(text)],
        ..ToolOutput::default()
    })
}

fn run_find(args: &Value) -> Result<ToolOutput, String> {
    let pattern = args["pattern"].as_str().unwrap_or_default();
    if pattern.is_empty() {
        return Err("missing `pattern`".into());
    }
    let max = args["max_results"].as_u64().unwrap_or(DEFAULT_FIND_MAX) as usize;
    let root = search_root(args);
    let filter = glob_matcher(Some(pattern));

    let mut paths: Vec<PathBuf> = walk_entries(&root)
        .into_iter()
        .filter(|entry| entry.file_type().is_some_and(|ty| ty.is_file()))
        .filter(|entry| filter(entry.path()))
        .map(|entry| display_path(&root, entry.path()))
        .collect();
    paths.sort();
    let mut out = String::new();
    for path in paths.iter().take(max) {
        out.push_str(&path.display().to_string());
        out.push('\n');
    }
    let text = cap_output(
        out,
        &format!("call find with a narrower pattern or path for {root}"),
    );
    Ok(ToolOutput {
        content: vec![ContentBlock::Text(text)],
        ..ToolOutput::default()
    })
}

fn search_root(args: &Value) -> String {
    args["path"].as_str().unwrap_or(".").to_string()
}

/// Files below `root`, honoring `.gitignore` and friends (hidden files are
/// still walked when explicitly required — skip hidden by default like ripgrep).
fn walk_entries(root: &str) -> Vec<ignore::DirEntry> {
    WalkBuilder::new(root)
        .standard_filters(true)
        // Honor .gitignore even outside a git repo (preopened dirs may not
        // carry .git).
        .require_git(false)
        .build()
        .filter_map(Result::ok)
        .collect()
}

fn display_path(root: &str, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Minimal `*`/`?`/literal-suffix glob match on the path string.
fn glob_matcher(pattern: Option<&str>) -> Box<dyn Fn(&Path) -> bool> {
    match pattern {
        Some(pattern) if !pattern.trim().is_empty() => {
            let pattern = glob::Pattern::new(pattern);
            let pattern = match pattern {
                Ok(p) => p,
                Err(_) => return Box::new(|_| true),
            };
            Box::new(move |path| {
                pattern.matches_path(path)
                    || path
                        .file_name()
                        .map(|name| pattern.matches(&name.to_string_lossy()))
                        .unwrap_or(false)
            })
        }
        _ => Box::new(|_| true),
    }
}

fn cap_output(mut out: String, hint: &str) -> String {
    if out.len() <= MAX_OUTPUT_CHARS {
        return out;
    }
    let kept = out.len() - MAX_OUTPUT_CHARS;
    out.truncate(MAX_OUTPUT_CHARS);
    out.push_str(&format!("[truncated: {kept} more chars; {hint}]"));
    out
}

export_extension!(Search);

// glob matching via `glob` crate (tiny, no platform deps)
mod glob {
    pub use glob::Pattern;
}
