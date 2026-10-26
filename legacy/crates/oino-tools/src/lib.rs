#![doc = r#"Built-in coding tools for Oino.

The default model-visible tools are `read`, `bash`, `edit`, and `write`.
Tools depend on `ExecutionEnv` so future
sandboxed or remote runtimes can reuse the same model-visible surface.

## Boundary

`oino-tools` owns model-visible tool definitions, JSON input schemas, argument
normalization, truncation messages, exact edit validation, current-working-directory
resolution, and conversion from environment failures into tool errors. Actual process
and filesystem access belongs to `oino-env`; provider serialization, session storage,
TUI confirmations, extension permissions, and app-level policy live outside this crate.

## Public API map

- [`default_tools`] builds the standard local coding tool set around a shared
  `ExecutionEnv` and startup working directory.
- [`ReadTool`] reads text with offset/limit continuation notices and returns a text
  placeholder for image files until providers support image tool results.
- [`BashTool`] runs shell commands through `ExecutionEnv`, formats stdout/stderr/status,
  and truncates long output from the tail so recent command output remains visible.
- [`EditTool`] applies exact, unique, non-overlapping replacements against the original
  file and keeps writes sequential.
- [`WriteTool`] creates or overwrites a file through `ExecutionEnv`, including parent
  directory creation delegated to the environment adapter.
- [`session_title_tool`], [`SessionTitleTool`], [`SessionTitleSetter`], and
  [`SESSION_TITLE_TOOL_NAME`] bridge the app's session-title callback into the same
  tool protocol without making this crate depend on session storage.

## Contributor rules

Keep tool names, schemas, and output wording stable unless the TUI help, user docs,
and tests are updated together. Route every side effect through `ExecutionEnv`; do not
call local filesystem or process APIs directly from tool implementations. Preserve abort
checks before and after environment work, keep `write`/`edit`/session-title operations
sequential, and prefer typed `Deserialize` argument structs with `serde_json::from_value`
over ad-hoc JSON probing except for compatibility shims.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use oino_agent_loop::{
    AbortSignal, BoxFuture, LoopError, LoopResult, Tool, ToolCall, ToolDefinition,
    ToolExecutionMode, ToolResult, ToolUpdateCallback,
};
use oino_env::{CommandOptions, EnvError, ExecutionEnv};
use oino_types::{ContentBlock, ToolDeclaration};
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
    #[error("oldText for edit #{index} matched {matches} times in {path} (lines {lines:?}); it must be unique")]
    EditNotUnique {
        index: usize,
        matches: usize,
        lines: Vec<usize>,
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
    path: Option<String>,
    #[serde(default)]
    paths: Vec<ReadPathArg>,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReadPathArg {
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

/// Combined output cap for multi-path reads.
const MULTI_READ_MAX_CHARS: usize = 30_000;

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
                "properties": {
                    "path": {"type": "string", "description": "Path to the file to read (relative or absolute). Use `paths` to read several files in one call."},
                    "paths": {
                        "type": "array",
                        "description": "Read several files in one call. Output is `==> path <==` sections with per-file errors inline; combined output is capped at 30k characters.",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["path"],
                            "properties": {
                                "path": {"type": "string", "description": "Path to the file to read"},
                                "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
                                "limit": {"type": "number", "description": "Maximum number of lines to read"}
                            }
                        }
                    },
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
        if !args.paths.is_empty() {
            return self.execute_multi(&call, &args.paths, &signal).await;
        }
        let Some(arg_path) = args.path.as_deref() else {
            return Err(LoopError::Tool(
                ToolInputError::InvalidArgument("path").to_string(),
            ));
        };
        let path = resolve_to_cwd(&self.cwd, arg_path);
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
        let text = format_read_output(arg_path, &content, args.offset, args.limit)?;
        Ok(ToolResult::text(&call, text))
    }
}

impl ReadTool {
    /// Multi-file read: `==> path <==` sections, per-file errors inline, and a
    /// shared 30k character cap with an explicit truncation footer.
    async fn execute_multi(
        &self,
        call: &ToolCall,
        paths: &[ReadPathArg],
        signal: &AbortSignal,
    ) -> LoopResult<ToolResult> {
        let mut out = String::new();
        let mut remaining = MULTI_READ_MAX_CHARS;
        for entry in paths {
            abort_if_needed(signal)?;
            if remaining == 0 {
                break;
            }
            let resolved = resolve_to_cwd(&self.cwd, &entry.path);
            let section = match self.env.read_text(&resolved).await {
                Ok(content) => {
                    match format_read_output(&entry.path, &content, entry.offset, entry.limit) {
                        Ok(text) => text,
                        Err(err) => format!("<error: {err}>"),
                    }
                }
                Err(err) => format!("<error: {err}>"),
            };
            let header = format!("==> {} <==\n", entry.path);
            let available = remaining.saturating_sub(header.len());
            if section.len() > available {
                let kept = &section[..available];
                out.push_str(&header);
                out.push_str(kept);
                out.push_str(&format!(
                    "\n[truncated: {} more chars; call read with offset/limit for {}]",
                    section.len() - available,
                    entry.path
                ));
                break;
            }
            out.push_str(&header);
            out.push_str(&section);
            out.push('\n');
            remaining = remaining.saturating_sub(header.len() + section.len());
        }
        Ok(ToolResult::text(call, out))
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
                signature: None,
            }],
            is_error: false,
            terminate: false,
            details: Some(json!({"replacements": args.edits.len()})),
            added_tool_names: Vec::new(),
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
        let positions: Vec<usize> = original
            .match_indices(&edit.old_text)
            .map(|(idx, _)| idx)
            .collect();
        match positions.len() {
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
                start: positions[0],
                end: positions[0] + edit.old_text.len(),
                new_text: &edit.new_text,
            }),
            count => {
                let lines: Vec<usize> = positions
                    .iter()
                    .map(|start| original[..*start].matches('\n').count() + 1)
                    .collect();
                return Err(LoopError::Tool(
                    ToolInputError::EditNotUnique {
                        index: index + 1,
                        matches: count,
                        lines,
                        path: path.into(),
                    }
                    .to_string(),
                ));
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
                text: "two\n\n[1 more lines in file. Use offset=3 to continue.]".into(),
                signature: None
            }]
        );
    }

    #[tokio::test]
    async fn read_multi_path_sections_errors_and_cap() {
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let env = Arc::new(LocalExecutionEnv) as Arc<dyn ExecutionEnv>;
        env.write_text(dir.path().join("a.txt").as_path(), "alpha")
            .await
            .unwrap_or_else(|err| panic!("write failed: {err}"));
        env.write_text(dir.path().join("b.txt").as_path(), "beta")
            .await
            .unwrap_or_else(|err| panic!("write failed: {err}"));
        let tool = ReadTool::new(env.clone(), dir.path());
        let result = tool
            .execute(
                call(
                    "read",
                    json!({"paths":[{"path":"a.txt"},{"path":"missing.txt"},{"path":"b.txt"}]}),
                ),
                ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("read failed: {err}"));
        let ContentBlock::Text { text, .. } = &result.content[0] else {
            panic!("text block expected");
        };
        assert!(
            text.starts_with(
                "==> a.txt <==
alpha"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "==> missing.txt <==
<error:"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "==> b.txt <==
beta"
            ),
            "{text}"
        );

        // cap: big file truncates with the explicit footer
        let big = "x".repeat(31_000);
        env.write_text(dir.path().join("big.txt").as_path(), &big)
            .await
            .unwrap_or_else(|err| panic!("write failed: {err}"));
        let result = tool
            .execute(
                call("read", json!({"paths":[{"path":"big.txt"}]})),
                ToolUpdateCallback::new(Arc::new(oino_agent_loop::NoopEventSink), Uuid::new_v4()),
                AbortSignal::new(),
            )
            .await
            .unwrap_or_else(|err| panic!("read failed: {err}"));
        let ContentBlock::Text { text, .. } = &result.content[0] else {
            panic!("text block expected");
        };
        assert!(text.len() < 31_000 + 512, "len {}", text.len());
        assert!(
            text.contains("[truncated:") && text.contains("offset/limit for big.txt"),
            "{text}"
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
            matches!(&result.content[0], ContentBlock::Text { text, .. } if text.contains("stdout:\nhi") && text.contains("Command exited with code 2"))
        );
    }
}

/// Host-owned `tool_search`: lists deferred (registered but hidden) tools and
/// loads them into the model-visible set. Registered only when at least one
/// deferred tool exists.
pub struct ToolSearchTool {
    deferred: Arc<dyn Fn() -> Vec<ToolDeclaration> + Send + Sync>,
    load: Arc<dyn Fn(Vec<String>) -> Vec<String> + Send + Sync>,
}

impl ToolSearchTool {
    /// `deferred` lists the deferred declarations; `load` marks names loaded
    /// and returns the confirmed subset.
    #[must_use]
    pub fn new(
        deferred: Arc<dyn Fn() -> Vec<ToolDeclaration> + Send + Sync>,
        load: Arc<dyn Fn(Vec<String>) -> Vec<String> + Send + Sync>,
    ) -> Self {
        Self { deferred, load }
    }
}

#[derive(Debug, Deserialize)]
struct ToolSearchArgs {
    query: Option<String>,
    load: Option<Vec<String>>,
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "tool_search".into(),
            description: "Search deferred tools and load them into the visible set. Without `load`, returns up to 10 deferred tools matching `query` (name + description). With `load`, marks the named tools available and returns the confirmed list.".into(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "query": {"type": "string", "description": "Fuzzy match against deferred tool names and descriptions"},
                    "load": {"type": "array", "items": {"type": "string"}, "description": "Tool names to make available"}
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
        let args: ToolSearchArgs = serde_json::from_value(call.arguments.clone())
            .map_err(|_| LoopError::Tool(ToolInputError::InvalidArgument("query").to_string()))?;
        if let Some(load) = args.load {
            let confirmed = (self.load)(load);
            let mut result = ToolResult::text(
                &call,
                if confirmed.is_empty() {
                    "No matching deferred tools loaded.".to_string()
                } else {
                    format!("Loaded tools: {}", confirmed.join(", "))
                },
            );
            result.added_tool_names = confirmed;
            return Ok(result);
        }
        let deferred = (self.deferred)();
        let query = args.query.unwrap_or_default();
        let matches = fuzzy_tool_matches(&deferred, &query, 10);
        let text = if matches.is_empty() {
            "No deferred tools found.".to_string()
        } else {
            matches
                .iter()
                .map(|tool| format!("{} — {}", tool.name, tool.description))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(ToolResult::text(&call, text))
    }
}

fn fuzzy_tool_matches(
    tools: &[ToolDeclaration],
    query: &str,
    limit: usize,
) -> Vec<ToolDeclaration> {
    let query = query.trim();
    if query.is_empty() {
        return tools.iter().take(limit).cloned().collect();
    }
    let mut nucleo = nucleo::Nucleo::new(nucleo::Config::DEFAULT, Arc::new(|| {}), Some(1), 1);
    let injector = nucleo.injector();
    for (index, tool) in tools.iter().enumerate() {
        let haystack = format!("{} {}", tool.name, tool.description);
        injector.push(index, move |_, columns| {
            columns[0] = nucleo::Utf32String::from(haystack);
        });
    }
    drop(injector);
    nucleo.pattern.reparse(
        0,
        query,
        nucleo::pattern::CaseMatching::Ignore,
        nucleo::pattern::Normalization::Smart,
        false,
    );
    while nucleo.tick(10).running {}
    nucleo
        .snapshot()
        .matched_items(..)
        .filter_map(|item| tools.get(*item.data).cloned())
        .take(limit)
        .collect()
}
