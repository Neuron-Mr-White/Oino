#![doc = r#"Built-in coding tools for Oino.

The first tool set intentionally mirrors pi-coding-agent's default model tools:
`read`, `bash`, `edit`, and `write`. Tools depend on `ExecutionEnv` so future
sandboxed or remote runtimes can reuse the same model-visible surface.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use oino_agent_loop::{
    AbortSignal, BoxFuture, LoopError, LoopResult, Tool, ToolCall, ToolDefinition,
    ToolExecutionMode, ToolResult, ToolUpdateCallback,
};
use oino_env::{CommandOptions, EnvError, ExecutionEnv};
use oino_types::ContentBlock;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

const MAX_TEXT_LINES: usize = 2_000;
const MAX_TEXT_BYTES: usize = 50 * 1024;

pub const SESSION_TITLE_TOOL_NAME: &str = "set_session_title";
pub type SessionTitleSetter =
    Arc<dyn Fn(String, bool) -> BoxFuture<'static, LoopResult<()>> + Send + Sync>;

#[derive(Debug, Error)]
enum ToolInputError {
    #[error("missing or invalid argument `{0}`")]
    InvalidArgument(&'static str),
    #[error("offset {offset} is beyond end of file ({total} lines total)")]
    OffsetOutOfBounds { offset: usize, total: usize },
    #[error("edit tool input is invalid: edits must contain at least one replacement")]
    EmptyEdits,
    #[error("oldText for edit #{index} was not found in {path}")]
    EditNotFound { index: usize, path: String },
    #[error("oldText for edit #{index} matched {matches} times in {path}; it must be unique")]
    EditNotUnique {
        index: usize,
        matches: usize,
        path: String,
    },
    #[error("edit #{index} overlaps with another edit; merge nearby/overlapping changes into one replacement")]
    EditOverlap { index: usize },
}

/// Create Oino's default local coding tools: `read`, `bash`, `edit`, and `write`.
#[must_use]
pub fn default_tools(
    env: Arc<dyn ExecutionEnv>,
    cwd: impl Into<PathBuf>,
) -> BTreeMap<String, Arc<dyn Tool>> {
    let cwd = cwd.into();
    let mut tools: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
    for tool in [
        Arc::new(ReadTool::new(Arc::clone(&env), cwd.clone())) as Arc<dyn Tool>,
        Arc::new(BashTool::new(Arc::clone(&env), cwd.clone())) as Arc<dyn Tool>,
        Arc::new(EditTool::new(Arc::clone(&env), cwd.clone())) as Arc<dyn Tool>,
        Arc::new(WriteTool::new(env, cwd)) as Arc<dyn Tool>,
    ] {
        tools.insert(tool.definition().name, tool);
    }
    tools
}

#[must_use]
pub fn session_title_tool(setter: SessionTitleSetter) -> Arc<dyn Tool> {
    Arc::new(SessionTitleTool { setter })
}

#[derive(Clone)]
pub struct SessionTitleTool {
    setter: SessionTitleSetter,
}

#[derive(Debug, Deserialize)]
struct SessionTitleArgs {
    title: String,
}

#[async_trait]
impl Tool for SessionTitleTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: SESSION_TITLE_TOOL_NAME.into(),
            description: "Set a concise session title for the user when you have grasped their intent. If a session title already exists, this tool returns an error unless you pass `override: true` to override it.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["title"],
                "properties": {
                    "title": {"type": "string", "description": "New concise session title"},
                    "override": {
                        "description": "Use override:true to replace an existing session title.",
                        "oneOf": [
                            {"type": "boolean"},
                            {"type": "string", "enum": ["true", "false"]}
                        ]
                    }
                }
            }),
        }
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        abort_if_needed(&signal)?;
        let override_existing = boolish_arg(&call.arguments, "override");
        let mut arguments = call.arguments.clone();
        if let Some(object) = arguments.as_object_mut() {
            object.remove("override");
        }
        let args: SessionTitleArgs = serde_json::from_value(arguments)
            .map_err(|_| LoopError::Tool(ToolInputError::InvalidArgument("title").to_string()))?;
        let title = args.title.trim();
        if title.is_empty() {
            return Err(LoopError::Tool(
                ToolInputError::InvalidArgument("title").to_string(),
            ));
        }
        (self.setter)(title.to_string(), override_existing).await?;
        abort_if_needed(&signal)?;
        Ok(ToolResult::text(
            &call,
            format!("Session title set to {title}"),
        ))
    }
}

fn boolish_arg(arguments: &Value, key: &str) -> bool {
    match arguments.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

#[derive(Clone)]
pub struct ReadTool {
    env: Arc<dyn ExecutionEnv>,
    cwd: PathBuf,
}

impl ReadTool {
    #[must_use]
    pub fn new(env: Arc<dyn ExecutionEnv>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            env,
            cwd: cwd.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[async_trait]
impl Tool for ReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: format!(
                "Read the contents of a file. For text files, output is truncated to {MAX_TEXT_LINES} lines or {}KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.",
                MAX_TEXT_BYTES / 1024
            ),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path"],
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to read (relative or absolute)"},
                    "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
                    "limit": {"type": "number", "description": "Maximum number of lines to read"}
                }
            }),
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        abort_if_needed(&signal)?;
        let args: ReadArgs = serde_json::from_value(call.arguments.clone())
            .map_err(|_| LoopError::Tool(ToolInputError::InvalidArgument("path").to_string()))?;
        let path = resolve_to_cwd(&self.cwd, &args.path);
        if image_mime_from_path(&path).is_some() {
            let mime = image_mime_from_path(&path).unwrap_or("image/*");
            // The first OpenRouter adapter only supports text model-visible content, so avoid
            // returning image blocks that would break the next provider request.
            let _bytes = self.env.read_binary(&path).await.map_err(env_to_loop)?;
            return Ok(ToolResult::text(
                &call,
                format!(
                    "Read image file [{mime}]\n[Image omitted: Oino's current OpenRouter adapter does not support image tool results yet.]"
                ),
            ));
        }
        let content = self.env.read_text(&path).await.map_err(env_to_loop)?;
        abort_if_needed(&signal)?;
        let text = format_read_output(&args.path, &content, args.offset, args.limit)?;
        Ok(ToolResult::text(&call, text))
    }
}

#[derive(Clone)]
pub struct BashTool {
    env: Arc<dyn ExecutionEnv>,
    cwd: PathBuf,
}

impl BashTool {
    #[must_use]
    pub fn new(env: Arc<dyn ExecutionEnv>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            env,
            cwd: cwd.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
    timeout: Option<u64>,
}

#[async_trait]
impl Tool for BashTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".into(),
            description: format!(
                "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last {MAX_TEXT_LINES} lines or {}KB (whichever is hit first). Optionally provide a timeout in seconds.",
                MAX_TEXT_BYTES / 1024
            ),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["command"],
                "properties": {
                    "command": {"type": "string", "description": "Bash command to execute"},
                    "timeout": {"type": "number", "description": "Timeout in seconds (optional, no default timeout)"}
                }
            }),
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        abort_if_needed(&signal)?;
        let args: BashArgs = serde_json::from_value(call.arguments.clone())
            .map_err(|_| LoopError::Tool(ToolInputError::InvalidArgument("command").to_string()))?;
        let output = self
            .env
            .shell(
                &args.command,
                CommandOptions {
                    cwd: Some(self.cwd.clone()),
                    timeout_ms: args.timeout.map(|seconds| seconds.saturating_mul(1_000)),
                },
            )
            .await
            .map_err(env_to_loop)?;
        abort_if_needed(&signal)?;
        let text = format_command_output(output.stdout, output.stderr, output.status);
        let truncated = truncate_tail_with_notice(&text);
        if matches!(output.status, Some(code) if code != 0) {
            Ok(ToolResult::error(&call, truncated))
        } else {
            Ok(ToolResult::text(&call, truncated))
        }
    }
}

#[derive(Clone)]
pub struct WriteTool {
    env: Arc<dyn ExecutionEnv>,
    cwd: PathBuf,
}

impl WriteTool {
    #[must_use]
    pub fn new(env: Arc<dyn ExecutionEnv>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            env,
            cwd: cwd.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path", "content"],
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to write (relative or absolute)"},
                    "content": {"type": "string", "description": "Content to write to the file"}
                }
            }),
        }
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        abort_if_needed(&signal)?;
        let args: WriteArgs = serde_json::from_value(call.arguments.clone()).map_err(|_| {
            LoopError::Tool(ToolInputError::InvalidArgument("path/content").to_string())
        })?;
        let path = resolve_to_cwd(&self.cwd, &args.path);
        self.env
            .write_text(&path, &args.content)
            .await
            .map_err(env_to_loop)?;
        abort_if_needed(&signal)?;
        Ok(ToolResult::text(
            &call,
            format!(
                "Successfully wrote {} bytes to {}",
                args.content.len(),
                args.path
            ),
        ))
    }
}

#[derive(Clone)]
pub struct EditTool {
    env: Arc<dyn ExecutionEnv>,
    cwd: PathBuf,
}

impl EditTool {
    #[must_use]
    pub fn new(env: Arc<dyn ExecutionEnv>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            env,
            cwd: cwd.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditReplacement {
    old_text: String,
    new_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditArgs {
    path: String,
    #[serde(default)]
    edits: Vec<EditReplacement>,
}

#[async_trait]
impl Tool for EditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit".into(),
            description: "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["path", "edits"],
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to edit (relative or absolute)"},
                    "edits": {
                        "type": "array",
                        "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["oldText", "newText"],
                            "properties": {
                                "oldText": {"type": "string", "description": "Exact unique text to replace"},
                                "newText": {"type": "string", "description": "Replacement text"}
                            }
                        }
                    }
                }
            }),
        }
    }

    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Sequential
    }

    async fn prepare_arguments(&self, mut arguments: Value) -> LoopResult<Value> {
        if let Some(edits) = arguments.get_mut("edits") {
            if let Some(text) = edits.as_str() {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    *edits = parsed;
                }
            }
        }
        let old_text = arguments.get("oldText").and_then(Value::as_str);
        let new_text = arguments.get("newText").and_then(Value::as_str);
        if let (Some(old_text), Some(new_text)) = (old_text, new_text) {
            let mut edits = arguments
                .get("edits")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            edits.push(json!({"oldText": old_text, "newText": new_text}));
            if let Some(obj) = arguments.as_object_mut() {
                obj.remove("oldText");
                obj.remove("newText");
                obj.insert("edits".into(), Value::Array(edits));
            }
        }
        Ok(arguments)
    }

    async fn execute(
        &self,
        call: ToolCall,
        _updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        abort_if_needed(&signal)?;
        let args: EditArgs = serde_json::from_value(call.arguments.clone()).map_err(|_| {
            LoopError::Tool(ToolInputError::InvalidArgument("path/edits").to_string())
        })?;
        if args.edits.is_empty() {
            return Err(LoopError::Tool(ToolInputError::EmptyEdits.to_string()));
        }
        let path = resolve_to_cwd(&self.cwd, &args.path);
        let original = self.env.read_text(&path).await.map_err(env_to_loop)?;
        let edited = apply_exact_edits(&args.path, &original, &args.edits)?;
        abort_if_needed(&signal)?;
        self.env
            .write_text(&path, &edited)
            .await
            .map_err(env_to_loop)?;
        Ok(ToolResult {
            call_id: call.id,
            tool_name: call.name.clone(),
            content: vec![ContentBlock::Text {
                text: format!(
                    "Successfully replaced {} block(s) in {}.",
                    args.edits.len(),
                    args.path
                ),
            }],
            is_error: false,
            terminate: false,
            details: Some(json!({"replacements": args.edits.len()})),
        })
    }
}

fn resolve_to_cwd(cwd: &Path, path: &str) -> PathBuf {
    let input = PathBuf::from(path);
    if input.is_absolute() {
        input
    } else {
        cwd.join(input)
    }
}

fn abort_if_needed(signal: &AbortSignal) -> LoopResult<()> {
    if signal.is_aborted() {
        Err(LoopError::Aborted)
    } else {
        Ok(())
    }
}

fn env_to_loop(err: EnvError) -> LoopError {
    LoopError::Tool(err.to_string())
}

fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn format_read_output(
    display_path: &str,
    content: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> LoopResult<String> {
    let lines: Vec<&str> = content.split('\n').collect();
    let total = lines.len();
    let offset = offset.unwrap_or(1).max(1);
    let start = offset - 1;
    if start >= total {
        return Err(LoopError::Tool(
            ToolInputError::OffsetOutOfBounds { offset, total }.to_string(),
        ));
    }
    let end_by_limit = limit.map_or(total, |limit| start.saturating_add(limit).min(total));
    let selected = lines[start..end_by_limit].join("\n");
    let (truncated, shown_lines, truncated_reason) = truncate_head(&selected);
    let mut out = truncated;
    if let Some(reason) = truncated_reason {
        let end_line = start + shown_lines;
        let next_offset = end_line + 1;
        out.push_str(&format!(
            "\n\n[Showing lines {offset}-{end_line} of {total} ({reason}). Use offset={next_offset} to continue.]"
        ));
    } else if end_by_limit < total {
        let remaining = total - end_by_limit;
        let next_offset = end_by_limit + 1;
        out.push_str(&format!(
            "\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]"
        ));
    }
    if out.is_empty() {
        Ok(format!("{display_path} is empty"))
    } else {
        Ok(out)
    }
}

fn truncate_head(text: &str) -> (String, usize, Option<String>) {
    let mut bytes = 0usize;
    let mut selected = Vec::new();
    let mut total_lines = 0usize;
    for line in text.split('\n') {
        total_lines += 1;
        if selected.len() >= MAX_TEXT_LINES {
            return (
                selected.join("\n"),
                selected.len(),
                Some(format!("{MAX_TEXT_LINES} line limit")),
            );
        }
        let line_bytes = line.len() + usize::from(!selected.is_empty());
        if bytes + line_bytes > MAX_TEXT_BYTES {
            if selected.is_empty() {
                let clipped = clip_to_bytes(line, MAX_TEXT_BYTES);
                return (
                    clipped,
                    1,
                    Some(format!("{}KB limit", MAX_TEXT_BYTES / 1024)),
                );
            }
            return (
                selected.join("\n"),
                selected.len(),
                Some(format!("{}KB limit", MAX_TEXT_BYTES / 1024)),
            );
        }
        bytes += line_bytes;
        selected.push(line);
    }
    (selected.join("\n"), total_lines, None)
}

fn truncate_tail_with_notice(text: &str) -> String {
    let (truncated, total_lines, shown_lines, by) = truncate_tail(text);
    if let Some(reason) = by {
        let start = total_lines.saturating_sub(shown_lines).saturating_add(1);
        format!("{truncated}\n\n[Showing lines {start}-{total_lines} of {total_lines} ({reason}).]")
    } else {
        truncated
    }
}

fn truncate_tail(text: &str) -> (String, usize, usize, Option<String>) {
    let lines: Vec<&str> = text.split('\n').collect();
    let total = lines.len();
    let mut bytes = 0usize;
    let mut selected_rev = Vec::new();
    for line in lines.iter().rev() {
        if selected_rev.len() >= MAX_TEXT_LINES {
            selected_rev.reverse();
            return (
                selected_rev.join("\n"),
                total,
                selected_rev.len(),
                Some(format!("{MAX_TEXT_LINES} line limit")),
            );
        }
        let line_bytes = line.len() + usize::from(!selected_rev.is_empty());
        if bytes + line_bytes > MAX_TEXT_BYTES {
            if selected_rev.is_empty() {
                return (
                    clip_to_last_bytes(line, MAX_TEXT_BYTES),
                    total,
                    1,
                    Some(format!("{}KB limit", MAX_TEXT_BYTES / 1024)),
                );
            }
            selected_rev.reverse();
            return (
                selected_rev.join("\n"),
                total,
                selected_rev.len(),
                Some(format!("{}KB limit", MAX_TEXT_BYTES / 1024)),
            );
        }
        bytes += line_bytes;
        selected_rev.push(*line);
    }
    selected_rev.reverse();
    (selected_rev.join("\n"), total, total, None)
}

fn clip_to_bytes(text: &str, max_bytes: usize) -> String {
    let mut end = 0usize;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    text[..end].to_string()
}

fn clip_to_last_bytes(text: &str, max_bytes: usize) -> String {
    let target_start = text.len().saturating_sub(max_bytes);
    let mut start = text.len();
    for (idx, _) in text.char_indices() {
        if idx >= target_start {
            start = idx;
            break;
        }
    }
    text[start..].to_string()
}

fn format_command_output(stdout: String, stderr: String, status: Option<i32>) -> String {
    let mut sections = Vec::new();
    if !stdout.is_empty() {
        sections.push(format!("stdout:\n{stdout}"));
    }
    if !stderr.is_empty() {
        sections.push(format!("stderr:\n{stderr}"));
    }
    if sections.is_empty() {
        sections.push("(no output)".into());
    }
    if let Some(code) = status {
        if code != 0 {
            sections.push(format!("Command exited with code {code}"));
        }
    }
    sections.join("\n\n")
}

#[derive(Debug)]
struct LocatedEdit<'a> {
    index: usize,
    start: usize,
    end: usize,
    new_text: &'a str,
}

fn apply_exact_edits(path: &str, original: &str, edits: &[EditReplacement]) -> LoopResult<String> {
    let mut located = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        let matches: Vec<usize> = original
            .match_indices(&edit.old_text)
            .map(|(idx, _)| idx)
            .collect();
        match matches.len() {
            0 => {
                return Err(LoopError::Tool(
                    ToolInputError::EditNotFound {
                        index: index + 1,
                        path: path.into(),
                    }
                    .to_string(),
                ))
            }
            1 => located.push(LocatedEdit {
                index: index + 1,
                start: matches[0],
                end: matches[0] + edit.old_text.len(),
                new_text: &edit.new_text,
            }),
            matches => {
                return Err(LoopError::Tool(
                    ToolInputError::EditNotUnique {
                        index: index + 1,
                        matches,
                        path: path.into(),
                    }
                    .to_string(),
                ))
            }
        }
    }
    located.sort_by_key(|edit| edit.start);
    for pair in located.windows(2) {
        if pair[0].end > pair[1].start {
            return Err(LoopError::Tool(
                ToolInputError::EditOverlap {
                    index: pair[1].index,
                }
                .to_string(),
            ));
        }
    }
    let mut out = String::with_capacity(original.len());
    let mut cursor = 0usize;
    for edit in located {
        out.push_str(&original[cursor..edit.start]);
        out.push_str(edit.new_text);
        cursor = edit.end;
    }
    out.push_str(&original[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::ToolCall;
    use oino_env::LocalExecutionEnv;
    use serde_json::json;
    use uuid::Uuid;

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: Uuid::new_v4(),
            name: name.into(),
            arguments,
        }
    }

    #[tokio::test]
    async fn session_title_tool_calls_setter() {
        let seen = Arc::new(tokio::sync::Mutex::new((String::new(), false)));
        let tool_seen = Arc::clone(&seen);
        let tool = session_title_tool(Arc::new(move |title, override_existing| {
            let seen = Arc::clone(&tool_seen);
            Box::pin(async move {
                *seen.lock().await = (title, override_existing);
                Ok(())
            })
        }));
        let result = tool
            .execute(
                call(
                    SESSION_TITLE_TOOL_NAME,
                    json!({"title":"Design Review", "override":"true"}),
                ),
                ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("title tool failed: {err}"));
        assert_eq!(*seen.lock().await, ("Design Review".into(), true));
        assert_eq!(result.tool_name, SESSION_TITLE_TOOL_NAME);
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn read_supports_offset_limit_and_continuation_notice() {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let env = Arc::new(LocalExecutionEnv) as Arc<dyn ExecutionEnv>;
        env.write_text(dir.path().join("a.txt").as_path(), "one\ntwo\nthree")
            .await
            .unwrap_or_else(|err| panic!("write failed: {err}"));
        let tool = ReadTool::new(env, dir.path());
        let result = tool
            .execute(
                call("read", json!({"path":"a.txt", "offset": 2, "limit": 1})),
                ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("read failed: {err}"));
        assert_eq!(
            result.content,
            vec![ContentBlock::Text {
                text: "two\n\n[1 more lines in file. Use offset=3 to continue.]".into()
            }]
        );
    }

    #[tokio::test]
    async fn write_and_edit_modify_files() {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let env = Arc::new(LocalExecutionEnv) as Arc<dyn ExecutionEnv>;
        let write = WriteTool::new(Arc::clone(&env), dir.path());
        write
            .execute(
                call(
                    "write",
                    json!({"path":"src/a.txt", "content":"hello world"}),
                ),
                ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("write failed: {err}"));
        let edit = EditTool::new(Arc::clone(&env), dir.path());
        edit.execute(
            call(
                "edit",
                json!({"path":"src/a.txt", "edits":[{"oldText":"world", "newText":"oino"}]}),
            ),
            ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
            AbortSignal::new(),
        )
        .await
        .unwrap_or_else(|err| panic!("edit failed: {err}"));
        let content = env
            .read_text(dir.path().join("src/a.txt").as_path())
            .await
            .unwrap_or_else(|err| panic!("read failed: {err}"));
        assert_eq!(content, "hello oino");
    }

    #[tokio::test]
    async fn edit_rejects_non_unique_old_text() {
        let err = apply_exact_edits(
            "a.txt",
            "same\nsame",
            &[EditReplacement {
                old_text: "same".into(),
                new_text: "new".into(),
            }],
        )
        .err()
        .unwrap_or_else(|| panic!("expected edit error"));
        assert!(err.to_string().contains("must be unique"));
    }

    #[tokio::test]
    async fn bash_reports_stdout_and_nonzero_as_error() {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let env = Arc::new(LocalExecutionEnv) as Arc<dyn ExecutionEnv>;
        let tool = BashTool::new(env, dir.path());
        let result = tool
            .execute(
                call("bash", json!({"command":"printf hi && exit 2"})),
                ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("bash failed: {err}"));
        assert!(result.is_error);
        assert!(
            matches!(&result.content[0], ContentBlock::Text { text } if text.contains("stdout:\nhi") && text.contains("Command exited with code 2"))
        );
    }
}
