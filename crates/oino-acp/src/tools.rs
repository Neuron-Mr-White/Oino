//! Tool-call presentation: ACP kind, title, locations, content and diffs (modes §6.3).

use std::path::{Path, PathBuf};

use oino_agent::AgentToolResult;
use oino_ai::{JsonObject, JsonValue, UserContent};
use serde_json::json;

/// ACP `ToolKind` of a tool (built-ins by name; extension tools are `other`).
#[must_use]
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "read" => "read",
        "write" | "edit" => "edit",
        "bash" | "powershell" => "execute",
        "grep" | "find" | "ls" => "search",
        _ => "other",
    }
}

fn arg<'a>(args: &'a JsonObject, key: &str) -> Option<&'a str> {
    args.get(key).and_then(JsonValue::as_str)
}

/// Resolves a tool path argument against the session cwd (pi `resolveToCwd`, `~` expanded).
#[must_use]
pub fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let path = path.strip_prefix('@').unwrap_or(path);
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = oino_settings::paths::home_dir()
    {
        return home.join(rest);
    }
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// The file a built-in file tool works on.
#[must_use]
pub fn file_path(name: &str, args: &JsonObject, cwd: &Path) -> Option<PathBuf> {
    match name {
        "read" | "write" | "edit" => arg(args, "path").map(|p| resolve_path(cwd, p)),
        _ => None,
    }
}

/// Human title of a call (`Read src/main.rs`, `` `cargo test` ``, …).
#[must_use]
pub fn tool_title(name: &str, args: &JsonObject) -> String {
    let path = arg(args, "path");
    match (name, path) {
        ("read", Some(path)) => format!("Read {path}"),
        ("write", Some(path)) => format!("Write {path}"),
        ("edit", Some(path)) => format!("Edit {path}"),
        ("ls", path) => format!("List {}", path.unwrap_or(".")),
        ("grep", path) => format!(
            "Search `{}` in {}",
            arg(args, "pattern").unwrap_or_default(),
            path.unwrap_or(".")
        ),
        ("find", path) => format!(
            "Find `{}` in {}",
            arg(args, "pattern").unwrap_or_default(),
            path.unwrap_or(".")
        ),
        ("bash" | "powershell", _) => match arg(args, "command") {
            Some(command) => format!("`{}`", command.replace('`', "'")),
            None => name.to_owned(),
        },
        _ => name.to_owned(),
    }
}

/// `locations` for follow-along (`[{path, line?}]`).
#[must_use]
pub fn tool_locations(name: &str, args: &JsonObject, cwd: &Path) -> Vec<JsonValue> {
    let Some(path) = file_path(name, args, cwd) else {
        return Vec::new();
    };
    let mut location = json!({ "path": path });
    if name == "read"
        && let Some(offset) = args.get("offset").and_then(JsonValue::as_u64)
    {
        location["line"] = json!(offset.max(1));
    }
    vec![location]
}

/// ACP `ContentBlock` of an Oino content block.
#[must_use]
pub fn content_block(content: &UserContent) -> JsonValue {
    match content {
        UserContent::Text(text) => json!({ "type": "text", "text": text.text }),
        UserContent::Image(image) => {
            json!({ "type": "image", "data": image.data, "mimeType": image.mime_type })
        }
    }
}

/// `tool_call_update.content` items for a tool result.
#[must_use]
pub fn result_content(result: &AgentToolResult) -> Vec<JsonValue> {
    result
        .content
        .iter()
        .map(|block| json!({ "type": "content", "content": content_block(block) }))
        .collect()
}

/// Text of a result (for terminals that show the tool's own output).
#[must_use]
pub fn result_text(result: &AgentToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| match block {
            UserContent::Text(text) => Some(text.text.as_str()),
            UserContent::Image(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pre-execution snapshot of the file an `edit`/`write` changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSnapshot {
    /// Absolute path.
    pub path: PathBuf,
    /// Content before the call (`None` = the file did not exist).
    pub old_text: Option<String>,
}

impl FileSnapshot {
    /// Snapshots the target of `name` when it mutates a file.
    pub async fn take(name: &str, args: &JsonObject, cwd: &Path) -> Option<Self> {
        if !matches!(name, "edit" | "write") {
            return None;
        }
        let path = file_path(name, args, cwd)?;
        let old_text = tokio::fs::read_to_string(&path).await.ok();
        Some(Self { path, old_text })
    }

    /// The `{type: diff}` content after the call (`None` when nothing changed).
    pub async fn diff(&self) -> Option<JsonValue> {
        let new_text = tokio::fs::read_to_string(&self.path).await.ok()?;
        if self.old_text.as_deref() == Some(new_text.as_str()) {
            return None;
        }
        let mut diff = json!({ "type": "diff", "path": self.path, "newText": new_text });
        if let Some(old) = &self.old_text {
            diff["oldText"] = json!(old);
        }
        Some(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: JsonValue) -> JsonObject {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn kinds_titles_locations() {
        assert_eq!(tool_kind("edit"), "edit");
        assert_eq!(tool_kind("bash"), "execute");
        assert_eq!(tool_kind("grep"), "search");
        assert_eq!(tool_kind("ask_user"), "other");
        let read = args(json!({"path": "src/a.rs", "offset": 10}));
        assert_eq!(tool_title("read", &read), "Read src/a.rs");
        assert_eq!(
            tool_locations("read", &read, Path::new("/w")),
            vec![json!({"path": "/w/src/a.rs", "line": 10})]
        );
        assert_eq!(
            tool_title("bash", &args(json!({"command": "ls `x`"}))),
            "`ls 'x'`"
        );
        assert_eq!(
            tool_locations("bash", &read, Path::new("/w")),
            Vec::<JsonValue>::new()
        );
    }
}
