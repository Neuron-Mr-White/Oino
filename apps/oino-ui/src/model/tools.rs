//! Native presentation of tool calls (ui-protocol §6.4: built-in tools are rendered by the
//! frontend from their typed arguments and details; others fall back to render hints).

use oino_ai::{JsonObject, JsonValue, UserContent};

/// Lines of an inline diff shown in a collapsed transcript row (gpui-app §5.2).
pub const INLINE_DIFF_LINES: usize = 40;
/// Lines of a bash tail preview.
pub const TAIL_LINES: usize = 5;

/// What kind of tool a call is, for icons and native rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    /// `read`.
    Read,
    /// `write`.
    Write,
    /// `edit`.
    Edit,
    /// `bash` / `powershell`.
    Shell,
    /// `grep`, `find`, `ls`.
    Search,
    /// Subagent-like tools (`subagent`, `sidekick`, `btw`, `read_subagent`).
    Agent,
    /// Anything else.
    Other,
}

impl ToolKind {
    /// Kind of the tool named `name`.
    #[must_use]
    pub fn of(name: &str) -> Self {
        match name {
            "read" => Self::Read,
            "write" => Self::Write,
            "edit" => Self::Edit,
            "bash" | "powershell" => Self::Shell,
            "grep" | "find" | "ls" => Self::Search,
            n if n.contains("subagent") || n == "sidekick" || n == "btw" => Self::Agent,
            _ => Self::Other,
        }
    }
}

fn str_arg<'a>(args: &'a JsonObject, key: &str) -> Option<&'a str> {
    args.get(key).and_then(JsonValue::as_str)
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn shorten(text: &str, max: usize) -> String {
    let first = text.lines().next().unwrap_or_default();
    if first.chars().count() <= max && !text.contains('\n') {
        return first.to_owned();
    }
    let mut out: String = first.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// One-line title of a call (`Edit src/lib.rs`, `$ cargo test`).
#[must_use]
pub fn title(name: &str, args: &JsonObject) -> String {
    let path = str_arg(args, "path").or_else(|| str_arg(args, "file_path"));
    match ToolKind::of(name) {
        ToolKind::Read => format!("Read {}", path.unwrap_or("…")),
        ToolKind::Write => format!("Write {}", path.unwrap_or("…")),
        ToolKind::Edit => format!("Edit {}", path.unwrap_or("…")),
        ToolKind::Shell => format!("$ {}", shorten(str_arg(args, "command").unwrap_or(""), 120)),
        ToolKind::Search => {
            let pattern = str_arg(args, "pattern").unwrap_or("");
            let target = path.unwrap_or(".");
            if pattern.is_empty() {
                format!("{name} {target}")
            } else {
                format!("{name} {pattern} in {target}")
            }
        }
        ToolKind::Agent | ToolKind::Other => {
            let first = args.iter().find_map(|(key, value)| {
                value
                    .as_str()
                    .filter(|_| !matches!(key.as_str(), "content" | "newText" | "oldText"))
            });
            match first {
                Some(text) => format!("{name} {}", shorten(text, 80)),
                None => name.to_owned(),
            }
        }
    }
}

/// Streaming argument preview while the model is still emitting a call
/// (gpui-app §5.2: path and line counts for write/edit).
#[must_use]
pub fn args_preview(name: &str, args: &JsonObject) -> Option<String> {
    match ToolKind::of(name) {
        ToolKind::Write => str_arg(args, "content").map(|c| format!("{} lines", line_count(c))),
        ToolKind::Edit => {
            let edits = args.get("edits").and_then(JsonValue::as_array);
            let (removed, added) = match edits {
                Some(edits) => edits.iter().fold((0, 0), |(r, a), edit| {
                    let old = edit
                        .get("oldText")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    let new = edit
                        .get("newText")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("");
                    (r + line_count(old), a + line_count(new))
                }),
                None => (
                    str_arg(args, "oldText").map_or(0, line_count),
                    str_arg(args, "newText").map_or(0, line_count),
                ),
            };
            (removed + added > 0).then(|| format!("−{removed} +{added}"))
        }
        _ => None,
    }
}

/// Text blocks of a tool result joined with newlines.
#[must_use]
pub fn result_text(content: &[UserContent]) -> String {
    let mut out = String::new();
    for block in content {
        if let UserContent::Text(text) = block {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&text.text);
        }
    }
    out
}

/// The last `n` lines of `text`.
#[must_use]
pub fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].iter().map(|l| (*l).to_owned()).collect()
}

/// The unified patch of an edit result (`details.patch`, else `details.diff`).
#[must_use]
pub fn patch_of(details: Option<&JsonValue>) -> Option<String> {
    let details = details?;
    details
        .get("patch")
        .or_else(|| details.get("diff"))
        .and_then(JsonValue::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

/// Kind of a unified-diff line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    /// `+` line.
    Added,
    /// `-` line.
    Removed,
    /// `@@` hunk header.
    Hunk,
    /// `---`/`+++`/`diff`/`index` header.
    Header,
    /// Context.
    Context,
}

/// One classified diff line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// Kind.
    pub kind: DiffLineKind,
    /// Text without the trailing newline.
    pub text: String,
}

/// Summary of a patch: classified first `max` lines plus totals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffPreview {
    /// First `max` body lines (file headers skipped).
    pub lines: Vec<DiffLine>,
    /// Lines not shown.
    pub hidden: usize,
    /// Added lines in the whole patch.
    pub added: usize,
    /// Removed lines in the whole patch.
    pub removed: usize,
}

/// Classifies `patch` and keeps the first `max` body lines.
#[must_use]
pub fn diff_preview(patch: &str, max: usize) -> DiffPreview {
    let mut preview = DiffPreview::default();
    let mut body = 0usize;
    for line in patch.lines() {
        let kind = if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("Index:")
            || line.starts_with("====")
        {
            DiffLineKind::Header
        } else if line.starts_with("@@") {
            DiffLineKind::Hunk
        } else if line.starts_with('+') {
            preview.added += 1;
            DiffLineKind::Added
        } else if line.starts_with('-') {
            preview.removed += 1;
            DiffLineKind::Removed
        } else {
            DiffLineKind::Context
        };
        if kind == DiffLineKind::Header {
            continue;
        }
        body += 1;
        if preview.lines.len() < max {
            preview.lines.push(DiffLine {
                kind,
                text: line.to_owned(),
            });
        }
    }
    preview.hidden = body.saturating_sub(preview.lines.len());
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(value: JsonValue) -> JsonObject {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn titles_for_builtins_and_others() {
        assert_eq!(
            title("edit", &args(json!({"path":"src/a.rs"}))),
            "Edit src/a.rs"
        );
        assert_eq!(
            title("bash", &args(json!({"command":"cargo test\n--all"}))),
            "$ cargo test…"
        );
        assert_eq!(
            title("grep", &args(json!({"pattern":"fn main","path":"src"}))),
            "grep fn main in src"
        );
        assert_eq!(
            title("notify_user", &args(json!({"message":"hi"}))),
            "notify_user hi"
        );
        assert_eq!(title("x", &JsonObject::new()), "x");
        assert_eq!(ToolKind::of("read_subagent"), ToolKind::Agent);
    }

    #[test]
    fn previews_count_lines() {
        assert_eq!(
            args_preview("write", &args(json!({"path":"a","content":"1\n2\n3"}))),
            Some("3 lines".into())
        );
        assert_eq!(
            args_preview(
                "edit",
                &args(json!({"edits":[{"oldText":"a\nb","newText":"c"}]}))
            ),
            Some("−2 +1".into())
        );
        assert_eq!(args_preview("read", &JsonObject::new()), None);
    }

    #[test]
    fn diff_preview_classifies_and_truncates() {
        let patch = "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n+more\n";
        let preview = diff_preview(patch, 3);
        assert_eq!(preview.added, 2);
        assert_eq!(preview.removed, 1);
        assert_eq!(preview.lines.len(), 3);
        assert_eq!(preview.lines[0].kind, DiffLineKind::Hunk);
        assert_eq!(preview.hidden, 2);
        assert_eq!(
            patch_of(Some(&json!({"patch": patch, "diff": "x"}))).as_deref(),
            Some(patch)
        );
        assert_eq!(patch_of(None), None);
    }

    #[test]
    fn tail_and_result_text() {
        assert_eq!(tail("a\nb\nc", 2), vec!["b", "c"]);
        let content = vec![UserContent::text("one"), UserContent::text("two")];
        assert_eq!(result_text(&content), "one\ntwo");
    }
}
