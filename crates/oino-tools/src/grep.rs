//! `grep` (pi `tools/grep.ts`).
//!
//! pi runs `rg --no-config --json --line-number --color=never --hidden [--ignore-case]
//! [--fixed-strings] [--glob G] -- PATTERN PATH`; Oino uses the ripgrep library crates with
//! the same settings: hidden files searched, `.gitignore` (inside git repositories), `.ignore`
//! and `.rgignore` respected, binary files skipped, one match per matching line. Results are
//! reported in sorted path order (ripgrep's parallel order is not deterministic).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::FutureExt;
use futures::future::BoxFuture;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::JsonValue;
use serde_json::{Map, json};
use tokio_util::sync::CancellationToken;

use crate::config::ToolConfig;
use crate::info::{GREP_DEFAULT_LIMIT, grep_info};
use crate::js::number_to_string;
use crate::path_utils::resolve_to_cwd;
use crate::truncate::{
    DEFAULT_MAX_BYTES, GREP_MAX_LINE_LENGTH, TruncationOptions, format_size, truncate_head,
    truncate_line,
};
use crate::{aborted, args, js_number, node_path};

/// The `grep` tool.
#[derive(Debug)]
pub struct GrepTool {
    meta: ToolDefinitionMeta,
    config: ToolConfig,
}

impl GrepTool {
    /// A `grep` tool.
    #[must_use]
    pub fn new(config: ToolConfig) -> Self {
        Self {
            meta: grep_info().definition_meta(),
            config,
        }
    }
}

impl AgentTool for GrepTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn execute(
        &self,
        call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        async move {
            if cancel.is_cancelled() {
                return Err(aborted());
            }
            let stop = Arc::new(AtomicBool::new(false));
            let result = tokio::select! {
                result = run(&config, &call, Arc::clone(&stop)) => result,
                () = cancel.cancelled() => Err(aborted()),
            };
            stop.store(true, Ordering::Relaxed);
            result
        }
        .boxed()
    }
}

/// A matching line found by the search.
struct Match {
    path: String,
    line_number: u64,
    /// The line when it is valid UTF-8 (ripgrep JSON `lines.text`).
    text: Option<String>,
}

struct SearchSpec {
    pattern: String,
    root: String,
    glob: Option<String>,
    ignore_case: bool,
    literal: bool,
    limit: f64,
}

struct SearchOutcome {
    matches: Vec<Match>,
    limit_reached: bool,
}

struct LineSink<'a> {
    path: &'a str,
    matches: &'a mut Vec<Match>,
    limit: f64,
    stop: &'a AtomicBool,
}

impl Sink for LineSink<'_> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        found: &SinkMatch<'_>,
    ) -> Result<bool, Self::Error> {
        self.matches.push(Match {
            path: self.path.to_owned(),
            line_number: found.line_number().unwrap_or(0),
            text: std::str::from_utf8(found.bytes()).ok().map(str::to_owned),
        });
        #[allow(
            clippy::cast_precision_loss,
            reason = "match counts are far below 2^53"
        )]
        let reached = self.matches.len() as f64 >= self.limit;
        Ok(!reached && !self.stop.load(Ordering::Relaxed))
    }
}

/// Runs the search like `rg` would; errors are rendered as ripgrep prints them.
fn search(spec: &SearchSpec, stop: &AtomicBool) -> Result<SearchOutcome, String> {
    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(spec.ignore_case)
        .fixed_strings(spec.literal)
        .line_terminator(Some(b'\n'))
        .build(&spec.pattern)
        .map_err(|error| format!("rg: {error}"))?;
    let mut walker = WalkBuilder::new(&spec.root);
    walker
        .hidden(false)
        .follow_links(false)
        .require_git(true)
        .add_custom_ignore_filename(".rgignore")
        .sort_by_file_name(Ord::cmp);
    if let Some(glob) = &spec.glob {
        let mut overrides = OverrideBuilder::new(&spec.root);
        overrides
            .add(glob)
            .map_err(|error| format!("rg: {error}"))?;
        walker.overrides(overrides.build().map_err(|error| format!("rg: {error}"))?);
    }
    let mut found_lines = Vec::new();
    let mut errors = Vec::new();
    for entry in walker.build() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("rg: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let explicit = entry.depth() == 0;
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(if explicit {
                BinaryDetection::convert(b'\x00')
            } else {
                BinaryDetection::quit(b'\x00')
            })
            .build();
        let path = entry.path().to_string_lossy().into_owned();
        let sink = LineSink {
            path: &path,
            matches: &mut found_lines,
            limit: spec.limit,
            stop,
        };
        if let Err(error) = searcher.search_path(&matcher, entry.path(), sink) {
            errors.push(format!("rg: {path}: {error}"));
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "match counts are far below 2^53"
        )]
        if found_lines.len() as f64 >= spec.limit {
            return Ok(SearchOutcome {
                matches: found_lines,
                limit_reached: true,
            });
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(SearchOutcome {
        matches: found_lines,
        limit_reached: false,
    })
}

/// Formats matches like pi: `path:line: text`, or blocks with `path-line- text` context lines.
async fn format_matches(
    env: &dyn crate::env::ExecutionEnv,
    matches: &[Match],
    context: f64,
    format_path: &(dyn Fn(&str) -> String + Sync),
) -> (Vec<String>, bool) {
    let mut cache: HashMap<String, Vec<String>> = HashMap::new();
    let mut lines_truncated = false;
    let mut output_lines: Vec<String> = Vec::new();
    for found in matches {
        let relative = format_path(&found.path);
        if let (true, Some(text)) = (context == 0.0, &found.text) {
            let sanitized = text.replace("\r\n", "\n").replace('\r', "");
            let sanitized = sanitized.strip_suffix('\n').unwrap_or(&sanitized);
            let line = truncate_line(sanitized, GREP_MAX_LINE_LENGTH);
            lines_truncated |= line.was_truncated;
            output_lines.push(format!("{relative}:{}: {}", found.line_number, line.text));
            continue;
        }
        if !cache.contains_key(&found.path) {
            let lines = match env.read_file(&found.path).await {
                Ok(bytes) => String::from_utf8_lossy(&bytes)
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .split('\n')
                    .map(str::to_owned)
                    .collect(),
                Err(_) => Vec::new(),
            };
            cache.insert(found.path.clone(), lines);
        }
        let lines = cache.get(&found.path).map_or(&[][..], Vec::as_slice);
        if lines.is_empty() {
            output_lines.push(format!(
                "{relative}:{}: (unable to read file)",
                found.line_number
            ));
            continue;
        }
        #[allow(
            clippy::cast_precision_loss,
            reason = "line numbers are far below 2^53"
        )]
        let (line_number, total) = (found.line_number as f64, lines.len() as f64);
        let (start, end) = if context > 0.0 {
            (
                (line_number - context).max(1.0),
                total.min(line_number + context),
            )
        } else {
            (line_number, line_number)
        };
        let mut current = start;
        while current <= end {
            let text = if current.fract() == 0.0 && current >= 1.0 {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "whole, positive"
                )]
                let index = current as usize - 1;
                lines.get(index).map_or("", String::as_str)
            } else {
                ""
            };
            let line = truncate_line(&text.replace('\r', ""), GREP_MAX_LINE_LENGTH);
            lines_truncated |= line.was_truncated;
            let shown = number_to_string(current);
            if (current - line_number).abs() < f64::EPSILON {
                output_lines.push(format!("{relative}:{shown}: {}", line.text));
            } else {
                output_lines.push(format!("{relative}-{shown}- {}", line.text));
            }
            current += 1.0;
        }
    }
    (output_lines, lines_truncated)
}

async fn run(
    config: &ToolConfig,
    call: &ToolInvocation,
    stop: Arc<AtomicBool>,
) -> Result<AgentToolResult, ToolError> {
    let pattern = args::required_str(&call.args, "pattern")?.to_owned();
    let search_dir = args::optional_str(&call.args, "path").filter(|path| !path.is_empty());
    let glob = args::optional_str(&call.args, "glob").filter(|glob| !glob.is_empty());
    let context = args::optional_f64(&call.args, "context")
        .filter(|context| *context > 0.0)
        .unwrap_or(0.0);
    let limit = args::optional_f64(&call.args, "limit")
        .unwrap_or(GREP_DEFAULT_LIMIT)
        .max(1.0);
    let env = config.env.as_ref();
    if !env.is_local() {
        return Err(ToolError::new(
            "grep is only available on the local file system",
        ));
    }
    let root = resolve_to_cwd(search_dir.unwrap_or("."), &config.effective_cwd())
        .map_err(|error| ToolError::new(error.to_string()))?;
    let is_directory = env
        .stat(&root)
        .await
        .map_err(|_| ToolError::new(format!("Path not found: {root}")))?
        .is_dir;
    let spec = SearchSpec {
        pattern,
        root: root.clone(),
        glob: glob.map(str::to_owned),
        ignore_case: args::flag(&call.args, "ignoreCase"),
        literal: args::flag(&call.args, "literal"),
        limit,
    };
    let outcome = tokio::task::spawn_blocking(move || search(&spec, &stop))
        .await
        .map_err(|error| ToolError::new(error.to_string()))?
        .map_err(ToolError::new)?;
    if outcome.matches.is_empty() {
        return Ok(AgentToolResult::text("No matches found"));
    }
    let format_path = |file: &str| {
        if is_directory {
            let relative = node_path::relative(&root, file);
            if !relative.is_empty() && !relative.starts_with("..") {
                return relative.replace('\\', "/");
            }
        }
        node_path::basename(file)
    };
    let (output_lines, lines_truncated) =
        format_matches(env, &outcome.matches, context, &format_path).await;
    let truncation = truncate_head(&output_lines.join("\n"), TruncationOptions::bytes_only());
    let mut output = truncation.content.clone();
    let mut details = Map::new();
    let mut notices = Vec::new();
    if outcome.limit_reached {
        notices.push(format!(
            "{} matches limit reached. Use limit={} for more, or refine pattern",
            number_to_string(limit),
            number_to_string(limit * 2.0)
        ));
        details.insert("matchLimitReached".to_owned(), js_number(limit));
    }
    if truncation.truncated {
        notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        details.insert("truncation".to_owned(), json!(truncation));
    }
    if lines_truncated {
        notices.push(format!(
            "Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"
        ));
        details.insert("linesTruncated".to_owned(), JsonValue::Bool(true));
    }
    if !notices.is_empty() {
        output = format!("{output}\n\n[{}]", notices.join(". "));
    }
    Ok(AgentToolResult {
        details: (!details.is_empty()).then_some(JsonValue::Object(details)),
        ..AgentToolResult::text(output)
    })
}
