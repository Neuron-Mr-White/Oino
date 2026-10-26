//! Built-in tool rendering per style (unipi `src/render/{tools,simple}.ts`) as OUS render
//! specs for `read`, `bash`, `edit`, `write`, `grep`, `find`, `ls`.
//!
//! | style | collapsed | expanded |
//! |---|---|---|
//! | `simple` | one mcode row `└ • Ran  npm test · 14 output lines` | the advanced view |
//! | `advanced` | `◆ Ran command` + command / diff / content blocks + output tail + exit line | full output |
//! | `regular` | `None` (the frontend's own rendering) | same |

use serde_json::{Value, json};

use super::parse::{
    as_json, diff_stats, exit_code, lang_for_path, parse_diff, split_command, strip_status,
    test_summary, to_patch,
};
use crate::hostfs::short_path;
use crate::settings::RenderStyle;

/// Built-ins with utility renderers.
pub const BUILTINS: [&str; 7] = ["read", "bash", "edit", "write", "grep", "find", "ls"];
const COMMAND_LINES: usize = 14;
const OUTPUT_TAIL: usize = 10;
const DIFF_ROWS: usize = 28;
const WRITE_LINES: usize = 12;
const READ_LINES: usize = 200;
const PATH_KEYS: [&str; 7] = [
    "path",
    "file_path",
    "file",
    "cwd",
    "dir",
    "directory",
    "notebook_path",
];

/// `[running, done, failed]` verbs (mcode `tool-definitions.ts`).
#[must_use]
pub fn verbs_for(name: &str) -> [String; 3] {
    let t = |a: &str, b: &str, c: &str| [a.to_owned(), b.to_owned(), c.to_owned()];
    match name.trim().to_lowercase().as_str() {
        "bash" | "powershell" => t("Running", "Ran", "Command failed"),
        "read" => t("Reading", "Read", "Read failed"),
        "edit" => t("Editing", "Edited", "Edit failed"),
        "write" => t("Writing", "Wrote", "Write failed"),
        "grep" => t("Searching", "Searched", "Search failed"),
        "find" | "glob" | "ls" | "list" => t("Listing", "Listed", "List failed"),
        "web_search" => t("Using WebSearch", "Used WebSearch", "WebSearch failed"),
        "web_fetch" => t("Using WebFetch", "Used WebFetch", "WebFetch failed"),
        "task" | "delegate" | "spawn_agent" | "run_subagent" => {
            t("Delegating", "Delegated", "Delegation failed")
        }
        "ask_user" => t("Asking user", "Asked user", "Ask failed"),
        _ => {
            let title = title_case_tool(name);
            [title.clone(), title.clone(), format!("{title} failed")]
        }
    }
}

/// mcode's fallback for unknown tools: `get_goal` → `Get Goal`.
#[must_use]
pub fn title_case_tool(name: &str) -> String {
    let n = name.trim().to_lowercase();
    let words: Vec<String> = n
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or_else(String::new, |f| {
                f.to_uppercase().collect::<String>() + c.as_str()
            })
        })
        .collect();
    if words.is_empty() {
        "Tool".into()
    } else {
        words.join(" ")
    }
}

fn s<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}

fn raw_target(name: &str, args: &Value, cwd: &str, home: Option<&str>) -> String {
    if name == "bash" || name == "powershell" {
        return s(args, "command")
            .or_else(|| s(args, "script"))
            .unwrap_or_default()
            .to_owned();
    }
    if name == "grep" {
        let pattern = s(args, "pattern")
            .or_else(|| s(args, "query"))
            .unwrap_or_default();
        return match s(args, "path") {
            Some(p) => format!("{pattern} in {}", short_path(p, cwd, home)),
            None => pattern.to_owned(),
        };
    }
    if let Some(p) = PATH_KEYS.iter().find_map(|k| s(args, k)) {
        return short_path(p, cwd, home);
    }
    if let Some(v) = [
        "query", "pattern", "url", "skill", "command", "title", "name", "id", "question",
    ]
    .iter()
    .find_map(|k| s(args, k))
    {
        return v.to_owned();
    }
    args.as_object()
        .into_iter()
        .flatten()
        .find_map(|(_, v)| match v {
            Value::String(x) if !x.is_empty() => Some(x.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The single argument a row shows (always one line).
#[must_use]
pub fn target_arg(name: &str, args: &Value, cwd: &str, home: Option<&str>) -> String {
    let raw = raw_target(name, args, cwd, home);
    let first = raw
        .split('\n')
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_owned();
    if raw.trim().contains('\n') {
        format!("{first} …")
    } else {
        first
    }
}

/// Text content of a tool result.
#[must_use]
pub fn text_of(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ` · N output lines` (empty when the result has no text).
#[must_use]
pub fn output_meta(result: Option<&Value>) -> String {
    let text = result.map(text_of).unwrap_or_default();
    if text.is_empty() {
        return String::new();
    }
    let n = text.split('\n').count();
    format!(" · {n} output line{}", if n == 1 { "" } else { "s" })
}

/// A render request's tool data.
#[derive(Clone, Debug, Default)]
pub struct ToolView {
    /// Tool name.
    pub name: String,
    /// Arguments.
    pub args: Value,
    /// Final or partial result.
    pub result: Option<Value>,
    /// The result is partial (still running).
    pub running: bool,
    /// Failed.
    pub failed: bool,
    /// Call cwd (host).
    pub cwd: String,
    /// `$HOME`.
    pub home: Option<String>,
}

impl ToolView {
    /// From `{toolCall, args, result?, partialResult?, isError?}`.
    #[must_use]
    pub fn from_data(name: &str, data: &Value, cwd: &str, home: Option<&str>) -> Self {
        let args = data
            .get("args")
            .cloned()
            .or_else(|| data.pointer("/toolCall/arguments").cloned())
            .unwrap_or(Value::Null);
        let result = data.get("result").filter(|r| !r.is_null()).cloned();
        let partial = data.get("partialResult").filter(|r| !r.is_null()).cloned();
        let failed = data
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || result
                .as_ref()
                .and_then(|r| r.get("isError"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
        Self {
            name: name.to_owned(),
            args,
            running: result.is_none(),
            result: result.or(partial),
            failed,
            cwd: cwd.to_owned(),
            home: home.map(ToOwned::to_owned),
        }
    }

    fn details(&self, key: &str) -> Option<&Value> {
        self.result.as_ref()?.get("details")?.get(key)
    }

    fn detached(&self) -> Option<String> {
        self.details("detachedToTask")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    fn path(&self) -> String {
        s(&self.args, "path")
            .or_else(|| s(&self.args, "file_path"))
            .unwrap_or_default()
            .to_owned()
    }
}

fn span(text: impl Into<String>, tone: Option<&str>) -> Value {
    let mut v = json!({"text": text.into()});
    if let Some(t) = tone {
        v["tone"] = json!(t);
    }
    v
}

/// The simple-style row.
#[must_use]
pub fn simple_row(v: &ToolView) -> Value {
    let [running_v, done_v, failed_v] = verbs_for(&v.name);
    let verb = if v.failed {
        failed_v
    } else if v.running {
        running_v
    } else {
        done_v
    };
    let marker = span(
        if v.failed { "×" } else { "•" },
        Some(if v.failed {
            "error"
        } else if v.running {
            "accent"
        } else {
            "success"
        }),
    );
    let target = target_arg(&v.name, &v.args, &v.cwd, v.home.as_deref());
    let meta = match v.detached() {
        Some(id) => format!(" · → background task {id}"),
        None => output_meta(v.result.as_ref()),
    };
    let mut spans = vec![
        span("└ ", Some("muted")),
        marker,
        span(" ", None),
        json!({"text": verb, "bold": true}),
    ];
    if !target.is_empty() {
        if v.name == "bash" || v.name == "powershell" {
            spans.push(span(format!("  {target}"), None));
        } else {
            spans.push(span(format!(" ({target})"), Some("muted")));
        }
    }
    if v.running {
        spans.push(span(
            if meta.is_empty() {
                " …".to_owned()
            } else {
                meta
            },
            Some("muted"),
        ));
    } else if !meta.is_empty() {
        spans.push(span(meta, Some(if v.failed { "error" } else { "muted" })));
    }
    json!({"t": "text", "value": spans, "wrap": false})
}

fn header(title: &str, detail: &str, running: bool, bad: bool, meta: Option<&str>) -> Value {
    let mut spans = vec![
        span(
            "◆ ",
            Some(if running {
                "muted"
            } else if bad {
                "error"
            } else {
                "accent"
            }),
        ),
        json!({"text": title, "bold": true}),
    ];
    if !detail.is_empty() {
        spans.push(span(format!(" {detail}"), Some("accent")));
    }
    if let Some(m) = meta.filter(|m| !m.is_empty()) {
        spans.push(span(
            format!(" · {m}"),
            Some(if bad { "error" } else { "muted" }),
        ));
    }
    json!({"t": "text", "value": spans})
}

fn dim(text: impl Into<String>) -> Value {
    json!({"t": "text", "value": [{"text": text.into(), "dim": true}]})
}

fn code(value: &str, lang: Option<&str>, line_numbers: bool) -> Value {
    let mut v = json!({"t": "code", "value": value, "lineNumbers": line_numbers});
    if let Some(l) = lang {
        v["language"] = json!(l);
    }
    v
}

fn stack(children: &[Value]) -> Value {
    json!({"t": "stack", "direction": "vertical", "children": children})
}

fn command_nodes(command: &str, expanded: bool) -> Vec<Value> {
    let lines: Vec<&str> = command.split('\n').collect();
    let shown = if expanded {
        lines.len()
    } else {
        lines.len().min(COMMAND_LINES)
    };
    let text = lines[..shown].join("\n");
    let mut out = Vec::new();
    // Heredoc bodies start on their own line: render them in their own language.
    let segs = split_command(&text);
    let heredocs_only = segs
        .iter()
        .enumerate()
        .all(|(i, seg)| !seg.code || (i > 0 && segs[i - 1].text.ends_with('\n')));
    if segs.iter().any(|seg| seg.code) && heredocs_only {
        for seg in segs {
            let body = seg.text.trim_end_matches('\n');
            if !body.is_empty() {
                out.push(code(
                    body,
                    Some(if seg.code { seg.lang.as_str() } else { "bash" }),
                    false,
                ));
            }
        }
    } else {
        out.push(code(&text, Some("bash"), false));
    }
    if shown < lines.len() {
        out.push(dim(format!("  … {} more lines", lines.len() - shown)));
    }
    out
}

fn bash_advanced(v: &ToolView, expanded: bool) -> Value {
    let command = s(&v.args, "command").unwrap_or_default();
    let mut out = vec![header(
        if v.running {
            "Running command"
        } else {
            "Ran command"
        },
        "",
        v.running,
        v.failed,
        None,
    )];
    out.extend(command_nodes(command, expanded));
    let text = v.result.as_ref().map(text_of).unwrap_or_default();
    let output = strip_status(&text);
    if !output.is_empty() {
        let pretty = as_json(&output);
        let body = pretty.clone().unwrap_or_else(|| output.clone());
        let lines: Vec<&str> = body.split('\n').collect();
        let shown = if expanded {
            lines.len()
        } else {
            lines.len().min(OUTPUT_TAIL)
        };
        if shown < lines.len() {
            out.push(dim(format!("… {} earlier lines", lines.len() - shown)));
        }
        let tail = lines[lines.len() - shown..].join("\n");
        if pretty.is_some() {
            out.push(code(&tail, Some("json"), false));
        } else {
            out.push(json!({"t": "log", "lines": lines[lines.len() - shown..], "tail": true}));
        }
    }
    if !v.running {
        if let Some((p, f)) = test_summary(&output) {
            out.push(json!({"t": "text", "value": [span(format!("{} {p} passed · {f} failed", if f > 0 { "✗" } else { "✓" }), Some(if f > 0 { "error" } else { "success" }))]}));
        }
        let detached = v.detached();
        let code = exit_code(&text, v.failed);
        let (status, tone) = match (&detached, code) {
            (Some(id), _) => (format!("→ background task {id}"), "accent"),
            (None, None) => ("Stopped".to_owned(), "error"),
            (None, Some(0)) => ("Exited with code 0".to_owned(), "success"),
            (None, Some(c)) => (format!("Exited with code {c}"), "error"),
        };
        out.push(
            json!({"t": "text", "value": [span("└ ", Some("muted")), span(status, Some(tone))]}),
        );
    }
    stack(&out)
}

fn read_range(args: &Value) -> String {
    let offset = args.get("offset").and_then(Value::as_u64);
    let limit = args.get("limit").and_then(Value::as_u64);
    if offset.is_none() && limit.is_none() {
        return String::new();
    }
    let start = offset.unwrap_or(1);
    match limit {
        Some(l) => format!(" (L{start}–{})", start + l.saturating_sub(1)),
        None => format!(" (L{start}+)"),
    }
}

fn edit_view(v: &ToolView, verb: &str, shown_path: &str, expanded: bool) -> Value {
    let diff = v
        .details("diff")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let rows = parse_diff(diff);
    let (a, r) = diff_stats(&rows);
    let meta = (!v.running).then(|| format!("+{a} -{r}"));
    let mut out = vec![header(verb, shown_path, v.running, false, meta.as_deref())];
    if !rows.is_empty() {
        let shown = if expanded {
            rows.len()
        } else {
            rows.len().min(DIFF_ROWS)
        };
        out.push(json!({"t": "diff", "patch": to_patch(&rows[..shown]), "path": v.path()}));
        if shown < rows.len() {
            out.push(dim(format!("   … {} more lines", rows.len() - shown)));
        }
    }
    stack(&out)
}

fn write_view(v: &ToolView, verb: &str, shown_path: &str, expanded: bool) -> Value {
    let content = s(&v.args, "content").unwrap_or_default();
    let lines: Vec<&str> = content.split('\n').collect();
    let n = lines.len();
    let meta = format!("{n} line{}", if n == 1 { "" } else { "s" });
    let mut out = vec![header(verb, shown_path, v.running, false, Some(&meta))];
    let shown = if expanded { n } else { n.min(WRITE_LINES) };
    out.push(code(
        &lines[..shown].join("\n"),
        lang_for_path(&v.path()),
        true,
    ));
    if shown < n {
        out.push(dim(format!("… {} more lines", n - shown)));
    }
    stack(&out)
}

fn read_view(v: &ToolView, verb: &str, shown_path: &str, text: &str, expanded: bool) -> Value {
    let lines: Vec<&str> = text.split('\n').collect();
    let meta = format!(
        "{} line{}",
        lines.len(),
        if lines.len() == 1 { "" } else { "s" }
    );
    let detail = format!("{shown_path}{}", read_range(&v.args));
    let mut out = vec![header(
        verb,
        &detail,
        v.running,
        false,
        (!v.running).then_some(meta.as_str()),
    )];
    let content = v
        .result
        .as_ref()
        .and_then(|r| r.get("content"))
        .and_then(Value::as_array);
    for img in content
        .into_iter()
        .flatten()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("image"))
    {
        out.push(json!({"t": "image", "data": img.get("data"), "mimeType": img.get("mimeType"), "alt": shown_path}));
    }
    if expanded && !text.is_empty() {
        let mut c = code(
            &lines[..lines.len().min(READ_LINES)].join("\n"),
            lang_for_path(&v.path()),
            true,
        );
        c["startLine"] = json!(v.args.get("offset").and_then(Value::as_u64).unwrap_or(1));
        out.push(c);
        if lines.len() > READ_LINES {
            out.push(dim(format!("… {} more lines", lines.len() - READ_LINES)));
        }
    }
    stack(&out)
}

fn file_advanced(v: &ToolView, expanded: bool) -> Value {
    let shown_path = short_path(&v.path(), &v.cwd, v.home.as_deref());
    let [running_v, done_v, _] = verbs_for(&v.name);
    let verb = if v.running && v.name != "read" {
        running_v
    } else {
        done_v
    };
    let text = v.result.as_ref().map(text_of).unwrap_or_default();
    if v.failed {
        let mut out = vec![header(&verb, &shown_path, false, true, Some("failed"))];
        out.extend(
            text.split('\n')
                .map(|l| json!({"t": "text", "value": [span(format!("  {l}"), Some("error"))]})),
        );
        return stack(&out);
    }
    match v.name.as_str() {
        "edit" => edit_view(v, &verb, &shown_path, expanded),
        "write" => write_view(v, &verb, &shown_path, expanded),
        "read" => read_view(v, &verb, &shown_path, &text, expanded),
        _ => {
            let target = target_arg(&v.name, &v.args, &v.cwd, v.home.as_deref());
            let mut out = vec![header(&verb, &target, v.running, false, None)];
            if expanded && !text.is_empty() {
                out.push(json!({"t": "log", "lines": text.split('\n').collect::<Vec<_>>()}));
            }
            stack(&out)
        }
    }
}

/// The advanced view.
#[must_use]
pub fn advanced(v: &ToolView, expanded: bool) -> Value {
    if v.name == "bash" {
        bash_advanced(v, expanded)
    } else {
        file_advanced(v, expanded)
    }
}

/// Harness annotation header (`details.unipiHarnessAnnotations` / `oinoProvenance`).
#[must_use]
pub fn annotation(result: Option<&Value>) -> Option<Value> {
    let d = result?.get("details")?;
    let list = d
        .get("unipiHarnessAnnotations")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| d.get("oinoProvenance").map(|p| vec![p.clone()]))?;
    let first = list.first()?;
    let warn = first.get("severity").and_then(Value::as_str) == Some("warning");
    let label = format!(
        "{}UniPi · {} · {}",
        if warn { "⚠ " } else { "" },
        first
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("harness"),
        first
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    Some(
        json!({"t": "text", "value": [span(label, Some(if warn { "warning" } else { "harness" }))]}),
    )
}

/// Renders a built-in tool call; `None` = the frontend's own rendering.
#[must_use]
pub fn render(style: RenderStyle, v: &ToolView, expanded: bool) -> Option<Value> {
    let body = match style {
        RenderStyle::Regular => return None,
        RenderStyle::Simple if !expanded => simple_row(v),
        RenderStyle::Simple | RenderStyle::Advanced => advanced(v, expanded),
    };
    Some(match annotation(v.result.as_ref()) {
        Some(a) => stack(&[a, body]),
        None => body,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    fn view(name: &str, args: Value, result: Option<Value>) -> ToolView {
        let mut data = json!({"args": args});
        if let Some(r) = result {
            data["result"] = r;
        }
        ToolView::from_data(name, &data, "/w", Some("/home/u"))
    }

    fn text(v: &Value) -> String {
        fn walk(v: &Value, out: &mut String) {
            match v {
                Value::String(s) => out.push_str(s),
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                Value::Object(o) => o
                    .iter()
                    .filter(|(k, _)| {
                        matches!(
                            k.as_str(),
                            "value" | "text" | "children" | "lines" | "patch"
                        )
                    })
                    .for_each(|(_, x)| walk(x, out)),
                _ => {}
            }
        }
        let mut s = String::new();
        walk(v, &mut s);
        s
    }

    #[test]
    fn verbs_and_titles() {
        assert_eq!(verbs_for("bash")[1], "Ran");
        assert_eq!(verbs_for("LS")[0], "Listing");
        assert_eq!(
            verbs_for("get_goal"),
            [
                "Get Goal".to_owned(),
                "Get Goal".to_owned(),
                "Get Goal failed".to_owned()
            ]
        );
        assert_eq!(title_case_tool(""), "Tool");
    }

    #[test]
    fn target_args() {
        assert_eq!(
            target_arg("bash", &json!({"command": "\nnpm test\nmore"}), "/w", None),
            "npm test …"
        );
        assert_eq!(
            target_arg(
                "grep",
                &json!({"pattern": "fn", "path": "/w/src"}),
                "/w",
                None
            ),
            "fn in ./src"
        );
        assert_eq!(
            target_arg(
                "read",
                &json!({"path": "/home/u/x.md"}),
                "/w",
                Some("/home/u")
            ),
            "~/x.md"
        );
        assert_eq!(
            target_arg("web_search", &json!({"query": "rust"}), "/w", None),
            "rust"
        );
        assert_eq!(target_arg("other", &json!({"n": 3}), "/w", None), "3");
    }

    #[test]
    fn simple_rows() {
        let done = view(
            "bash",
            json!({"command": "npm test"}),
            Some(json!({"content": [{"type": "text", "text": "a\nb"}]})),
        );
        assert_eq!(
            text(&simple_row(&done)),
            "└ • Ran  npm test · 2 output lines"
        );
        let running = view("read", json!({"path": "/w/a.rs"}), None);
        assert_eq!(text(&simple_row(&running)), "└ • Reading (./a.rs) …");
        let failed = view(
            "edit",
            json!({"path": "/w/a.rs"}),
            Some(json!({"content": [{"type": "text", "text": "no match"}], "isError": true})),
        );
        let row = simple_row(&failed);
        assert_eq!(row["value"][1]["text"], "×");
        assert!(text(&row).contains("Edit failed"));
        let detached = view(
            "bash",
            json!({"command": "sleep 99"}),
            Some(json!({"content": [], "details": {"detachedToTask": "t1"}})),
        );
        assert!(text(&simple_row(&detached)).ends_with("→ background task t1"));
    }

    #[test]
    fn advanced_bash() {
        let out = (1..=15)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let v = view(
            "bash",
            json!({"command": "cargo test"}),
            Some(
                json!({"content": [{"type": "text", "text": format!("{out}\ntest result: ok. 12 passed; 0 failed\n\nCommand exited with code 0")}]}),
            ),
        );
        let r = advanced(&v, false);
        let t = text(&r);
        assert!(t.contains("Ran command"));
        assert!(t.contains("… 6 earlier lines"));
        assert!(t.contains("✓ 12 passed · 0 failed"));
        assert!(t.contains("Exited with code 0"));
        let heredoc = view(
            "bash",
            json!({"command": "python - <<'EOF'\nprint(1)\nEOF"}),
            None,
        );
        let nodes = advanced(&heredoc, true);
        assert!(
            nodes["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["language"] == "python")
        );
        let json_out = view(
            "bash",
            json!({"command": "cat a.json"}),
            Some(json!({"content": [{"type": "text", "text": "{\"a\":1}"}]})),
        );
        assert!(
            advanced(&json_out, false)["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["language"] == "json")
        );
    }

    #[test]
    fn advanced_files() {
        let edit = view(
            "edit",
            json!({"path": "/w/a.rs"}),
            Some(json!({"content": [], "details": {"diff": " 1 a\n+2 b\n-3 c"}})),
        );
        let e = advanced(&edit, false);
        assert!(text(&e).contains("Edited ./a.rs · +1 -1"));
        assert_eq!(e["children"][1]["t"], "diff");
        let write = view(
            "write",
            json!({"path": "/w/a.rs", "content": (0..20).map(|i| i.to_string()).collect::<Vec<_>>().join("\n")}),
            Some(json!({"content": []})),
        );
        let w = advanced(&write, false);
        assert_eq!(w["children"][1]["language"], "rust");
        assert!(text(&w).contains("… 8 more lines"));
        let read = view(
            "read",
            json!({"path": "/w/a.md", "offset": 5, "limit": 3}),
            Some(json!({"content": [{"type": "text", "text": "x\ny"}]})),
        );
        assert!(text(&advanced(&read, false)).contains("./a.md (L5–7) · 2 lines"));
        assert_eq!(advanced(&read, true)["children"][1]["startLine"], 5);
        let bad = view(
            "write",
            json!({"path": "/w/a"}),
            Some(json!({"content": [{"type": "text", "text": "EACCES"}], "isError": true})),
        );
        assert!(text(&advanced(&bad, false)).contains("EACCES"));
    }

    #[test]
    fn style_dispatch_and_annotations() {
        let v = view(
            "ls",
            json!({"path": "/w"}),
            Some(
                json!({"content": [{"type": "text", "text": "a"}], "details": {"unipiHarnessAnnotations": [{"source": "Watchdog", "title": "Slow", "severity": "warning"}]}}),
            ),
        );
        assert!(render(RenderStyle::Regular, &v, false).is_none());
        let r = render(RenderStyle::Simple, &v, false).unwrap();
        assert!(text(&r).starts_with("⚠ UniPi · Watchdog · Slow"));
        assert_eq!(
            render(RenderStyle::Simple, &v, true).unwrap()["children"][1]["t"],
            "stack"
        );
        assert!(
            render(
                RenderStyle::Advanced,
                &view("grep", json!({"pattern": "x"}), None),
                false
            )
            .is_some()
        );
    }
}
