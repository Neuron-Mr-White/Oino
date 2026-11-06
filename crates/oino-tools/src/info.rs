//! Static metadata of the built-in tools (names, descriptions, schemas, prompt text).

use oino_agent::{ExecutionMode, ToolDefinitionMeta};
use oino_ai::{ConstrainedSampling, ConstrainedSamplingConfig, JsonValue, StrictMode};
use serde_json::json;

/// Built-in tool names.
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
    /// `glob`
    Glob,
    /// Codex patch format.
    ApplyPatch,
    /// `ls`
    Ls,
    /// Sandboxed JavaScript.
    Codemode,
    /// Deferred schema discovery.
    ToolSearch,
    /// `powershell` (Windows only)
    Powershell,
}

impl BuiltinToolName {
    /// Every built-in available on this platform, in declaration order.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let mut names = vec![Self::Read, Self::Bash];
        if cfg!(windows) {
            names.push(Self::Powershell);
        }
        names.extend([
            Self::Edit,
            Self::Write,
            Self::Grep,
            Self::Glob,
            Self::Ls,
            Self::ApplyPatch,
            Self::Codemode,
            Self::ToolSearch,
        ]);
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
            Self::Glob => "glob",
            Self::ApplyPatch => "apply_patch",
            Self::Ls => "ls",
            Self::Powershell => "powershell",
            Self::Codemode => "codemode",
            Self::ToolSearch => "tool_search",
        }
    }

    /// Parses a model-facing name (only names available on this platform).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        if name == "find" {
            return Some(Self::Find);
        }
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

/// Tools active by default (setting `defaultTools`).
pub const DEFAULT_ACTIVE_TOOLS: [BuiltinToolName; 4] = [
    BuiltinToolName::Read,
    BuiltinToolName::Bash,
    BuiltinToolName::Edit,
    BuiltinToolName::Write,
];

/// Read-only set (`createReadOnlyTools`): `read`, `grep`, `find`, `ls`.
pub const READ_ONLY_TOOLS: [BuiltinToolName; 4] = [
    BuiltinToolName::Read,
    BuiltinToolName::Grep,
    BuiltinToolName::Glob,
    BuiltinToolName::Ls,
];

/// The metadata core uses for the system prompt and `ToolInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinToolInfo {
    /// Tool name.
    pub name: BuiltinToolName,
    /// Human label.
    pub label: &'static str,
    /// Model-facing description.
    pub description: String,
    /// One-line summary in the system prompt tool list.
    pub prompt_snippet: Option<&'static str>,
    /// Guideline bullets (`promptGuidelines`; `None` when undefined).
    pub prompt_guidelines: Option<Vec<String>>,
    /// Scheduling (every built-in is parallel; file mutations use the mutation queue).
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

/// Guideline advertising the session environment (the `OINO_*` text).
pub const SESSION_ENV_GUIDELINE: &str = "You can inspect OINO_* environment variables (also exported as PI_*) for current model and session details.";

fn strict_prefer() -> ConstrainedSampling {
    ConstrainedSampling::Config(ConstrainedSamplingConfig {
        kind: "json_schema".to_owned(),
        strict: Some(StrictMode::Prefer),
        variants: None,
    })
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

/// The read tool metadata.
#[must_use]
pub fn read_info() -> BuiltinToolInfo {
    let mut metadata = info(BuiltinToolName::Read,
        "Read a file using an absolute file_path. Bare reads of long files show a numbered declaration outline; use offset and limit (up to 2400 lines) to page. line_numbers prefixes text with 12→text. Images up to 5 MB and PDFs up to 3 MB are supported; image_quality=high preserves more detail.".to_owned(),
        "Read file contents", Some(strings(&["Use read instead of cat, nl or sed to read and number files."])),
        json!({"type":"object","properties":{"file_path":{"type":"string"},"offset":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"default":2400},"line_numbers":{"type":"boolean"},"image_quality":{"type":"string","enum":["default","high"],"default":"default"}},"required":["file_path"],"additionalProperties":false}));
    metadata.constrained_sampling = Some(strict_prefer());
    metadata
}

/// The write tool metadata.
#[must_use]
pub fn write_info() -> BuiltinToolInfo {
    let mut metadata = info(BuiltinToolName::Write,
        "Create or overwrite a file atomically, creating parent directories. Preserves existing encoding, BOM and line endings and ensures a trailing newline. Directory paths are refused.".to_owned(),
        "Create or overwrite files", Some(strings(&["Use write for new files or complete rewrites."])),
        json!({"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}},"required":["file_path","content"],"additionalProperties":false}));
    metadata.constrained_sampling = Some(strict_prefer());
    metadata
}

/// The edit tool metadata.
#[must_use]
pub fn edit_info() -> BuiltinToolInfo {
    let mut metadata = info(BuiltinToolName::Edit,
        "Edit a file with batched exact replacements. All edits match the original file and must not overlap. Each oldText must be unique unless its change_all is true. Refuses files changed externally since last read. Writes atomically preserving encoding, BOM and line endings, returning a diff summary.".to_owned(),
        "Make precise batched file edits", Some(strings(&["Read files before editing. Merge overlapping or nearby changes into one edit."])),
        json!({"type":"object","properties":{"file_path":{"type":"string"},"edits":{"type":"array","minItems":1,"items":{"type":"object","properties":{"oldText":{"type":"string"},"newText":{"type":"string"},"change_all":{"type":"boolean","default":false}},"required":["oldText","newText"],"additionalProperties":false}}},"required":["file_path","edits"],"additionalProperties":false}));
    metadata.constrained_sampling = Some(strict_prefer());
    metadata
}

/// Shell tool flavour.
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

    /// Shell name in descriptions and errors.
    #[must_use]
    pub fn shell_name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::PowerShell => "PowerShell",
        }
    }

    /// Temp file prefix (`oino-bash` / `oino-powershell`).
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

/// The shell tool metadata. Guidelines are only present when the session
/// environment is exposed.
#[must_use]
pub fn shell_info(kind: ShellKind, expose_session_environment: bool) -> BuiltinToolInfo {
    let mut info = info(
        kind.tool_name(),
        format!(
            "Execute a {} command in a NEW shell in the session working directory. Shell state does not persist. Timeout defaults to 60 seconds. Output keeps its head and tail within 32 KiB, with the complete output in a log. Use fireAndForget for background work; its task id and log path return immediately and the exit result is delivered automatically. Declare riskLevel from plausible consequences: low for read-only local inspection, medium for bounded reversible changes, high for destructive, privileged, remote or hard-to-recover changes. Chained commands take the maximum risk; file redirection is at least medium. User authorization does not lower risk.",
            kind.shell_name()
        ),
        kind.prompt_snippet(),
        expose_session_environment.then(|| strings(&[SESSION_ENV_GUIDELINE])),
        json!({
            "type": "object",
            "required": ["command", "summary", "riskLevel"],
            "properties": {
                "command": {"type": "string", "description": "Shell command to execute"},
                "summary": {"type": "string"},
                "riskLevel": {"type": "string", "enum": ["low", "medium", "high"]},
                "timeout": {"type": "number", "exclusiveMinimum": 0, "default": 60},
                "fireAndForget": {"type": "boolean", "default": false}
            },
            "additionalProperties": false
        }),
    );
    info.constrained_sampling = Some(strict_prefer());
    info
}

/// Default `grep` match limit.
pub(crate) const GREP_DEFAULT_LIMIT: f64 = 100.0;
/// Default `find` result limit.
pub(crate) const FIND_DEFAULT_LIMIT: f64 = 1000.0;
/// Default `ls` entry limit.
pub(crate) const LS_DEFAULT_LIMIT: f64 = 500.0;

/// The grep tool metadata.
#[must_use]
pub fn grep_info() -> BuiltinToolInfo {
    info(BuiltinToolName::Grep,
        "Search file contents using in-process ripgrep. Defaults to files_with_matches; content returns matching lines and optional context, count returns counts per file. Supports file types, globs, literal and multiline searches. Respects ignore files.".to_owned(),
        "Search file contents", None,
        json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob_pattern":{"type":"string"},"output_mode":{"type":"string","enum":["content","files_with_matches","count"],"default":"files_with_matches"},"case_insensitive":{"type":"boolean"},"type":{"type":"string"},"context_before":{"type":"integer","minimum":0},"context_after":{"type":"integer","minimum":0},"context":{"type":"integer","minimum":0},"line_numbers":{"type":"boolean"},"head_limit":{"type":"integer","minimum":0},"multiline":{"type":"boolean"},"fixed_string":{"type":"boolean"}},"required":["pattern"],"additionalProperties":false}))
}

/// The find tool metadata.
#[must_use]
pub fn find_info() -> BuiltinToolInfo {
    let mut metadata = glob_info();
    metadata.name = BuiltinToolName::Find;
    metadata.label = "find";
    metadata
}

/// The glob tool metadata. The find alias is never advertised.
#[must_use]
pub fn glob_info() -> BuiltinToolInfo {
    info(BuiltinToolName::Glob, "Find files by one or more glob patterns, excluding excludePatterns. Paths are relative to folder (default cwd). Respects ignore files.".to_owned(),
        "Find files by glob patterns", None,
        json!({"type":"object","properties":{"patterns":{"type":"array","items":{"type":"string"},"minItems":1},"excludePatterns":{"type":"array","items":{"type":"string"}},"folder":{"type":"string"}},"required":["patterns"],"additionalProperties":false}))
}

/// Codex patch metadata; registration is model-gated.
#[must_use]
pub fn apply_patch_info() -> BuiltinToolInfo {
    info(BuiltinToolName::ApplyPatch, "Apply a Codex *** Begin Patch / *** End Patch patch. Supports Add File, Update File (with optional Move to), and Delete File operations.".to_owned(),
        "Apply file patches", None,
        json!({"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false}))
}

/// The ls tool metadata.
#[must_use]
pub fn ls_info() -> BuiltinToolInfo {
    info(BuiltinToolName::Ls, "List directory entries, sorted alphabetically, including dotfiles. Directories have a / suffix. Defaults to cwd; ignorePatterns excludes matching entries.".to_owned(),
        "List directory contents", None,
        json!({"type":"object","properties":{"directory_path":{"type":"string"},"ignorePatterns":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}))
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
        BuiltinToolName::Glob => glob_info(),
        BuiltinToolName::ApplyPatch => apply_patch_info(),
        BuiltinToolName::Ls => ls_info(),
        BuiltinToolName::Codemode => codemode_info(),
        BuiltinToolName::ToolSearch => tool_search_info(),
    }
}

/// Sandboxed script metadata. Providers use the JSON envelope for raw source.
#[must_use]
pub fn codemode_info() -> BuiltinToolInfo {
    let mut value = info(BuiltinToolName::Codemode,
        "Run JavaScript as an async function in an isolated QuickJS sandbox (no filesystem, network, Node or timers). Pass raw source in code. Use await tools.<name>(args), text(value), image(value), console.log, return, exit(), store/load, ALL_TOOLS, searchTools, describeTool and describeNamespace. Non-identifier tool characters become _. Discover deferred tools without changing the declaration prefix. Every tool call is permission checked; failed calls reject and prior effects are not undone. First-line // @options: {\"max_output_tokens\":10000,\"timeout_ms\":60000} controls output and deadline. Storage commits only on success. Scripts cannot call codemode.".into(),
        "Execute sandboxed JavaScript using other tools", None,
        json!({"type":"object","properties":{"code":{"type":"string"}},"required":["code"],"additionalProperties":false}));
    value.execution_mode = ExecutionMode::Sequential;
    value
}

/// Append-only deferred tool discovery metadata.
#[must_use]
pub fn tool_search_info() -> BuiltinToolInfo {
    info(BuiltinToolName::ToolSearch,
        "Load tool schemas by select:<comma-separated exact names> (ignores max_results) or keywords matched against tool names (default five). Unknown or unavailable exact names are errors. Already loaded schemas are not appended again.".into(),
        "Find and load deferred tools", None,
        json!({"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":["integer","null"],"minimum":1}},"required":["query"],"additionalProperties":false}))
}
