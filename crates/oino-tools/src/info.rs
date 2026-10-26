//! Static pi metadata of the built-in tools (names, descriptions, schemas, prompt text).

use oino_agent::{ExecutionMode, ToolDefinitionMeta};
use oino_ai::{ConstrainedSampling, ConstrainedSamplingConfig, JsonValue, StrictMode};
use serde_json::json;

use crate::truncate::{DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, GREP_MAX_LINE_LENGTH};

/// Built-in tool names (pi `ToolName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BuiltinToolName {
    /// `read`
    Read,
    /// `bash`
    Bash,
    /// `edit`
    Edit,
    /// `write`
    Write,
    /// `grep`
    Grep,
    /// `find`
    Find,
    /// `ls`
    Ls,
    /// `powershell` (Windows only)
    Powershell,
}

impl BuiltinToolName {
    /// Every built-in available on this platform, in pi `createAllTools` order.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let mut names = vec![Self::Read, Self::Bash];
        if cfg!(windows) {
            names.push(Self::Powershell);
        }
        names.extend([Self::Edit, Self::Write, Self::Grep, Self::Find, Self::Ls]);
        names
    }

    /// Model-facing name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Bash => "bash",
            Self::Edit => "edit",
            Self::Write => "write",
            Self::Grep => "grep",
            Self::Find => "find",
            Self::Ls => "ls",
            Self::Powershell => "powershell",
        }
    }

    /// Parses a model-facing name (only names available on this platform).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|candidate| candidate.as_str() == name)
    }
}

impl std::fmt::Display for BuiltinToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tools active by default (setting `defaultTools`, pi `createCodingTools`).
pub const DEFAULT_ACTIVE_TOOLS: [BuiltinToolName; 4] = [
    BuiltinToolName::Read,
    BuiltinToolName::Bash,
    BuiltinToolName::Edit,
    BuiltinToolName::Write,
];

/// pi read-only set (`createReadOnlyTools`): `read`, `grep`, `find`, `ls`.
pub const READ_ONLY_TOOLS: [BuiltinToolName; 4] = [
    BuiltinToolName::Read,
    BuiltinToolName::Grep,
    BuiltinToolName::Find,
    BuiltinToolName::Ls,
];

/// The pi metadata core uses for the system prompt and `ToolInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinToolInfo {
    /// Tool name.
    pub name: BuiltinToolName,
    /// Human label.
    pub label: &'static str,
    /// Model-facing description.
    pub description: String,
    /// One-line summary in the system prompt tool list (pi `promptSnippet`).
    pub prompt_snippet: Option<&'static str>,
    /// Guideline bullets (pi `promptGuidelines`; `None` when pi leaves it undefined).
    pub prompt_guidelines: Option<Vec<String>>,
    /// Scheduling (every pi built-in is parallel; file mutations use the mutation queue).
    pub execution_mode: ExecutionMode,
    /// JSON Schema of the arguments.
    pub parameters: JsonValue,
    /// JSON Schema of `structuredContent`.
    pub output_schema: Option<JsonValue>,
    /// Constrained sampling preference (`{type:"json_schema", strict:"prefer"}` on the
    /// read/bash/edit/write family).
    pub constrained_sampling: Option<ConstrainedSampling>,
}

impl BuiltinToolInfo {
    /// The loop-facing metadata.
    #[must_use]
    pub fn definition_meta(&self) -> ToolDefinitionMeta {
        let mut meta = ToolDefinitionMeta::new(
            self.name.as_str(),
            self.label,
            self.description.clone(),
            self.parameters.clone(),
        );
        meta.output_schema.clone_from(&self.output_schema);
        meta.execution_mode = self.execution_mode;
        meta.constrained_sampling
            .clone_from(&self.constrained_sampling);
        meta
    }
}

/// Guideline advertising the session environment (D13 substitution of pi's `PI_*` text).
pub const SESSION_ENV_GUIDELINE: &str = "You can inspect OINO_* environment variables (also exported as PI_*) for current model and session details.";

fn strict_prefer() -> ConstrainedSampling {
    ConstrainedSampling::Config(ConstrainedSamplingConfig {
        kind: "json_schema".to_owned(),
        strict: Some(StrictMode::Prefer),
        variants: None,
    })
}

fn kb(bytes: u64) -> u64 {
    bytes / 1024
}

fn info(
    name: BuiltinToolName,
    description: String,
    prompt_snippet: &'static str,
    prompt_guidelines: Option<Vec<String>>,
    parameters: JsonValue,
) -> BuiltinToolInfo {
    BuiltinToolInfo {
        name,
        label: name.as_str(),
        description,
        prompt_snippet: Some(prompt_snippet),
        prompt_guidelines,
        execution_mode: ExecutionMode::Parallel,
        parameters,
        output_schema: None,
        constrained_sampling: None,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// pi `createReadToolDefinition` metadata.
#[must_use]
pub fn read_info() -> BuiltinToolInfo {
    let mut info = info(
        BuiltinToolName::Read,
        format!(
            "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to {DEFAULT_MAX_LINES} lines or {}KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.",
            kb(DEFAULT_MAX_BYTES)
        ),
        "Read file contents",
        Some(strings(&[
            "Use read to examine files instead of cat or sed.",
        ])),
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {"type": "string", "description": "Path to the file to read (relative or absolute)"},
                "offset": {"type": "number", "description": "Line number to start reading from (1-indexed)"},
                "limit": {"type": "number", "description": "Maximum number of lines to read"}
            }
        }),
    );
    info.output_schema = Some(json!({
        "anyOf": [
            {"type": "string"},
            {
                "type": "object",
                "required": ["type", "data", "mimeType", "note"],
                "properties": {
                    "type": {"type": "string", "const": "image"},
                    "data": {"type": "string"},
                    "mimeType": {"type": "string"},
                    "note": {"type": "string"}
                }
            }
        ]
    }));
    info.constrained_sampling = Some(strict_prefer());
    info
}

/// pi `createWriteToolDefinition` metadata.
#[must_use]
pub fn write_info() -> BuiltinToolInfo {
    let mut info = info(
        BuiltinToolName::Write,
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.".to_owned(),
        "Create or overwrite files",
        Some(strings(&["Use write only for new files or complete rewrites."])),
        json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": {"type": "string", "description": "Path to the file to write (relative or absolute)"},
                "content": {"type": "string", "description": "Content to write to the file"}
            }
        }),
    );
    info.constrained_sampling = Some(strict_prefer());
    info
}

/// pi `createEditToolDefinition` metadata.
#[must_use]
pub fn edit_info() -> BuiltinToolInfo {
    let mut info = info(
        BuiltinToolName::Edit,
        "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes.".to_owned(),
        "Make precise file edits with exact text replacement, including multiple disjoint edits in one call",
        Some(strings(&[
            "Use edit for precise changes (edits[].oldText must match exactly)",
            "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls",
            "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.",
            "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.",
        ])),
        json!({
            "type": "object",
            "required": ["path", "edits"],
            "properties": {
                "path": {"type": "string", "description": "Path to the file to edit (relative or absolute)"},
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["oldText", "newText"],
                        "properties": {
                            "oldText": {"type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call."},
                            "newText": {"type": "string", "description": "Replacement text for this targeted edit."}
                        }
                    },
                    "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead."
                }
            }
        }),
    );
    info.constrained_sampling = Some(strict_prefer());
    info
}

/// Shell tool flavour (pi `ShellToolConfig`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    /// `bash`.
    Bash,
    /// `powershell` (Windows).
    PowerShell,
}

impl ShellKind {
    /// Tool name.
    #[must_use]
    pub fn tool_name(self) -> BuiltinToolName {
        match self {
            Self::Bash => BuiltinToolName::Bash,
            Self::PowerShell => BuiltinToolName::Powershell,
        }
    }

    /// Shell name in descriptions and errors (pi `shellName`).
    #[must_use]
    pub fn shell_name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::PowerShell => "PowerShell",
        }
    }

    /// Temp file prefix (pi `pi-bash` / `pi-powershell`, D13).
    #[must_use]
    pub fn temp_file_prefix(self) -> &'static str {
        match self {
            Self::Bash => "oino-bash",
            Self::PowerShell => "oino-powershell",
        }
    }

    fn prompt_snippet(self) -> &'static str {
        match self {
            Self::Bash => "Execute bash commands (ls, grep, find, etc.)",
            Self::PowerShell => "Execute PowerShell commands",
        }
    }
}

/// pi `createShellToolDefinition` metadata. Guidelines are only present when the session
/// environment is exposed.
#[must_use]
pub fn shell_info(kind: ShellKind, expose_session_environment: bool) -> BuiltinToolInfo {
    let mut info = info(
        kind.tool_name(),
        format!(
            "Execute a {} command in the current working directory. Returns stdout and stderr. Output is truncated to last {DEFAULT_MAX_LINES} lines or {}KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.",
            kind.shell_name(),
            kb(DEFAULT_MAX_BYTES)
        ),
        kind.prompt_snippet(),
        expose_session_environment.then(|| strings(&[SESSION_ENV_GUIDELINE])),
        json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {"type": "string", "description": "Shell command to execute"},
                "timeout": {"type": "number", "description": "Timeout in seconds (optional, no default timeout)"}
            }
        }),
    );
    info.output_schema = Some(json!({
        "type": "object",
        "required": ["output", "truncated", "exit_code", "wall_time_seconds"],
        "properties": {
            "output": {"type": "string", "description": "Combined stdout and stderr, possibly truncated"},
            "truncated": {"type": "boolean"},
            "full_output_path": {"type": "string", "description": "Full output, when truncated"},
            "exit_code": {"type": "number"},
            "wall_time_seconds": {"type": "number"}
        }
    }));
    info.constrained_sampling = Some(strict_prefer());
    info
}

/// Default `grep` match limit.
pub(crate) const GREP_DEFAULT_LIMIT: f64 = 100.0;
/// Default `find` result limit.
pub(crate) const FIND_DEFAULT_LIMIT: f64 = 1000.0;
/// Default `ls` entry limit.
pub(crate) const LS_DEFAULT_LIMIT: f64 = 500.0;

/// pi `createGrepToolDefinition` metadata.
#[must_use]
pub fn grep_info() -> BuiltinToolInfo {
    info(
        BuiltinToolName::Grep,
        format!(
            "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore. Output is truncated to {GREP_DEFAULT_LIMIT} matches or {}KB (whichever is hit first). Long lines are truncated to {GREP_MAX_LINE_LENGTH} chars.",
            kb(DEFAULT_MAX_BYTES)
        ),
        "Search file contents for patterns (respects .gitignore)",
        None,
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {"type": "string", "description": "Search pattern (regex or literal string)"},
                "path": {"type": "string", "description": "Directory or file to search (default: current directory)"},
                "glob": {"type": "string", "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'"},
                "ignoreCase": {"type": "boolean", "description": "Case-insensitive search (default: false)"},
                "literal": {"type": "boolean", "description": "Treat pattern as literal string instead of regex (default: false)"},
                "context": {"type": "number", "description": "Number of lines to show before and after each match (default: 0)"},
                "limit": {"type": "number", "description": "Maximum number of matches to return (default: 100)"}
            }
        }),
    )
}

/// pi `createFindToolDefinition` metadata.
#[must_use]
pub fn find_info() -> BuiltinToolInfo {
    info(
        BuiltinToolName::Find,
        format!(
            "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore. Output is truncated to {FIND_DEFAULT_LIMIT} results or {}KB (whichever is hit first).",
            kb(DEFAULT_MAX_BYTES)
        ),
        "Find files by glob pattern (respects .gitignore)",
        None,
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'"},
                "path": {"type": "string", "description": "Directory to search in (default: current directory)"},
                "limit": {"type": "number", "description": "Maximum number of results (default: 1000)"}
            }
        }),
    )
}

/// pi `createLsToolDefinition` metadata.
#[must_use]
pub fn ls_info() -> BuiltinToolInfo {
    info(
        BuiltinToolName::Ls,
        format!(
            "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. Includes dotfiles. Output is truncated to {LS_DEFAULT_LIMIT} entries or {}KB (whichever is hit first).",
            kb(DEFAULT_MAX_BYTES)
        ),
        "List directory contents",
        None,
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Directory to list (default: current directory)"},
                "limit": {"type": "number", "description": "Maximum number of entries to return (default: 500)"}
            }
        }),
    )
}

/// Metadata of a built-in by name.
#[must_use]
pub fn builtin_tool_info(
    name: BuiltinToolName,
    expose_session_environment: bool,
) -> BuiltinToolInfo {
    match name {
        BuiltinToolName::Read => read_info(),
        BuiltinToolName::Bash => shell_info(ShellKind::Bash, expose_session_environment),
        BuiltinToolName::Powershell => {
            shell_info(ShellKind::PowerShell, expose_session_environment)
        }
        BuiltinToolName::Edit => edit_info(),
        BuiltinToolName::Write => write_info(),
        BuiltinToolName::Grep => grep_info(),
        BuiltinToolName::Find => find_info(),
        BuiltinToolName::Ls => ls_info(),
    }
}
