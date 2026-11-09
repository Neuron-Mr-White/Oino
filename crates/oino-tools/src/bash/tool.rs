//! The `bash` and `powershell` tools.

use futures::FutureExt;
use futures::future::BoxFuture;
use oino_agent::{
    AgentTool, AgentToolResult, ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::{JsonValue, UserContent};
use serde_json::{Map, json};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::accumulator::{OutputAccumulator, OutputSnapshot};
use super::detach::DetachedProcess;
use super::exec::{ExecEnd, ExecError, ExecRequest, OutputSink, ShellSpec, exec};
use super::shell::{shell_env, tool_env};
use super::temp_dir;
use crate::config::ToolConfig;
use crate::info::{ShellKind, shell_info};
use crate::{args, js_number};

/// Update throttle.
pub const BASH_UPDATE_THROTTLE: std::time::Duration = std::time::Duration::from_millis(100);
/// Limit of `structuredContent.output`.
const STRUCTURED_OUTPUT_MAX_BYTES: u64 = 32 * 1024 - 1024;
/// Prepended to PowerShell commands.
const POWERSHELL_UTF8_PREFIX: &str =
    "try { [Console]::OutputEncoding=[System.Text.Encoding]::UTF8 } catch {}\n";

/// A shell tool: `bash`, or `powershell` on Windows.
#[derive(Debug)]
pub struct ShellTool {
    meta: ToolDefinitionMeta,
    kind: ShellKind,
    config: ToolConfig,
}

impl ShellTool {
    /// The `bash` tool.
    #[must_use]
    pub fn bash(config: ToolConfig) -> Self {
        Self::new(ShellKind::Bash, config)
    }

    /// The `powershell` tool (Windows; elsewhere every call fails with a fixed message).
    #[must_use]
    pub fn powershell(config: ToolConfig) -> Self {
        Self::new(ShellKind::PowerShell, config)
    }

    fn new(kind: ShellKind, config: ToolConfig) -> Self {
        Self {
            meta: shell_info(kind, config.shell.expose_session_environment).definition_meta(),
            kind,
            config,
        }
    }
}

impl AgentTool for ShellTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn prepare_arguments(&self, raw: oino_ai::JsonObject) -> oino_ai::JsonObject {
        crate::legacy::normalize_call(self.meta.name.as_str(), &raw, &self.config.effective_cwd())
            .map_or(raw, |call| call.args)
    }

    fn execute(
        &self,
        call: ToolInvocation,
        updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let config = self.config.clone();
        let kind = self.kind;
        async move { run(&config, kind, &call, updates, &cancel).await }.boxed()
    }
}

/// Feeds output into the accumulator and emits throttled updates.
struct Streaming {
    output: OutputAccumulator,
    updates: UpdateSink,
    dirty: bool,
    last_update: Option<Instant>,
    timer: Option<Instant>,
}

impl Streaming {
    fn emit(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.last_update = Some(Instant::now());
        let snapshot = self.output.snapshot(true);
        let mut details = Map::new();
        if snapshot.truncation.truncated {
            details.insert("truncation".to_owned(), json!(snapshot.truncation));
        }
        if let Some(path) = &snapshot.full_output_path {
            details.insert("fullOutputPath".to_owned(), json!(path));
        }
        self.updates.send(AgentToolResult {
            content: vec![UserContent::text(snapshot.content)],
            details: Some(JsonValue::Object(details)),
            ..AgentToolResult::default()
        });
    }

    fn finish(&mut self) -> Result<OutputSnapshot, ToolError> {
        self.output.finish();
        self.timer = None;
        self.emit();
        let snapshot = self.output.snapshot(true);
        self.output
            .close_temp_file()
            .map_err(|error| ToolError::new(error.to_string()))?;
        Ok(snapshot)
    }
}

impl OutputSink for Streaming {
    fn data(&mut self, chunk: &[u8]) {
        self.output.append(chunk);
        self.dirty = true;
        let due = self
            .last_update
            .map_or_else(Instant::now, |last| last + BASH_UPDATE_THROTTLE);
        if due <= Instant::now() {
            self.timer = None;
            self.emit();
        } else {
            self.timer.get_or_insert(due);
        }
    }

    fn next_flush(&self) -> Option<Instant> {
        self.timer
    }

    fn flush(&mut self) {
        self.timer = None;
        self.emit();
    }
}

/// The visible text with the truncation footer, and `details`.
fn format_output(
    snapshot: &OutputSnapshot,
    last_line_bytes: u64,
    empty_text: &str,
) -> (String, Option<JsonValue>) {
    let truncation = &snapshot.truncation;
    let mut text = if snapshot.content.is_empty() {
        empty_text.to_owned()
    } else {
        snapshot.content.clone()
    };
    if !truncation.truncated {
        return (text, None);
    }
    let path = snapshot.full_output_path.clone().unwrap_or_default();
    let mut details = Map::new();
    details.insert("truncation".to_owned(), json!(truncation));
    if let Some(path) = &snapshot.full_output_path {
        details.insert("fullOutputPath".to_owned(), json!(path));
    }
    let _ = last_line_bytes;
    let footer = format!("\n\n[Output truncated to 32 KiB head+tail. Full output: {path}]");
    text.push_str(&footer);
    (text, Some(JsonValue::Object(details)))
}

fn append_status(text: &str, status: &str) -> String {
    if text.is_empty() {
        status.to_owned()
    } else {
        format!("{text}\n\n{status}")
    }
}

async fn background(
    config: &ToolConfig,
    kind: ShellKind,
    call: &ToolInvocation,
    command: String,
    vars: Vec<(String, String)>,
    timeout: Option<f64>,
    risk: &str,
) -> Result<AgentToolResult, ToolError> {
    let mut arguments = call.args.clone();
    arguments.insert(
        "shell".to_owned(),
        json!(if kind == ShellKind::PowerShell {
            "powershell"
        } else {
            "bash"
        }),
    );
    arguments.insert("timeout".to_owned(), json!(timeout));
    arguments.insert("riskLevel".to_owned(), json!(risk));
    arguments
        .entry("summary".to_owned())
        .or_insert(json!("(legacy call)"));
    let shell = match kind {
        ShellKind::Bash => {
            super::shell::resolve_shell_config(
                config.env.as_ref(),
                config.shell.shell_path.as_deref(),
            )
            .await
        }
        ShellKind::PowerShell => super::shell::resolve_powershell_config(config.env.as_ref()).await,
    }
    .map_err(ToolError::new)?;
    let stdin = shell.transport == super::shell::CommandTransport::Stdin;
    let mut shell_args = shell.args;
    if !stdin {
        shell_args.push(command.clone());
    }
    arguments.insert("program".to_owned(), json!(shell.shell));
    arguments.insert("args".to_owned(), json!(shell_args));
    arguments.insert("vars".to_owned(), json!(vars));
    arguments.insert("stdin".to_owned(), json!(stdin.then_some(command.clone())));
    config
        .context
        .background_shell(JsonValue::Object(arguments))
        .await
}

async fn run(
    config: &ToolConfig,
    kind: ShellKind,
    call: &ToolInvocation,
    updates: UpdateSink,
    cancel: &CancellationToken,
) -> Result<AgentToolResult, ToolError> {
    let command = args::required_str(&call.args, "command")?;
    let seconds = args::optional_f64(&call.args, "timeout").unwrap_or(60.0);
    let timeout = Some(seconds);
    super::exec::resolve_timeout(timeout).map_err(ToolError::new)?;
    let risk = args::optional_str(&call.args, "riskLevel").unwrap_or("high");
    if !["low", "medium", "high"].contains(&risk) {
        return Err(ToolError::new("riskLevel must be low, medium or high"));
    }
    let command = match (kind, config.shell.command_prefix.as_deref()) {
        (ShellKind::PowerShell, _) => format!("{POWERSHELL_UTF8_PREFIX}{command}"),
        (ShellKind::Bash, Some(prefix)) if !prefix.is_empty() => format!("{prefix}\n{command}"),
        (ShellKind::Bash, _) => command.to_owned(),
    };
    let session = config
        .shell
        .expose_session_environment
        .then(|| config.context.session_env())
        .flatten();
    let vars = tool_env(
        shell_env(config.process_env.as_deref(), &config.agent_dir),
        session.as_ref(),
    );
    if call.args.get("fireAndForget").and_then(JsonValue::as_bool) == Some(true) {
        return background(config, kind, call, command, vars, timeout, risk).await;
    }

    let mut streaming = Streaming {
        output: OutputAccumulator::new(temp_dir(&config.agent_dir), kind.temp_file_prefix()),
        updates: updates.clone(),
        dirty: false,
        last_update: None,
        timer: None,
    };
    updates.send(AgentToolResult::default());
    let (_registration, mut detach) = config.bash_detach.register(&call.tool_call_id);
    let started = std::time::Instant::now();
    let request = ExecRequest {
        env: config.env.as_ref(),
        shell: match kind {
            ShellKind::Bash => ShellSpec::Bash(config.shell.shell_path.as_deref()),
            ShellKind::PowerShell => ShellSpec::PowerShell,
        },
        command,
        cwd: config.effective_cwd(),
        vars,
        timeout,
    };
    let outcome = exec(request, &mut streaming, cancel, Some(&mut detach)).await;
    let exit_code = match outcome {
        Err(error) => {
            let snapshot = streaming.finish()?;
            let (text, _) = format_output(&snapshot, streaming.output.last_line_bytes(), "");
            return Err(ToolError::new(match error {
                ExecError::Aborted => append_status(&text, "Command aborted"),
                ExecError::Timeout(seconds) => {
                    append_status(&text, &format!("Command timed out after {seconds} seconds"))
                }
                ExecError::Failed(message) => message,
            }));
        }
        Ok(ExecEnd::Detached(request, process)) => {
            let snapshot = streaming.finish()?;
            let (text, details) =
                format_output(&snapshot, streaming.output.last_line_bytes(), "(no output)");
            let marker = format!(
                "\n\n[Detached to background task {}: {}]",
                request.task_id, request.reason
            );
            let reply = DetachedProcess {
                process,
                output_so_far: text.clone(),
                full_output_path: snapshot.full_output_path,
                remaining_timeout: std::time::Duration::from_secs_f64(seconds)
                    .saturating_sub(started.elapsed()),
            };
            // A dropped requester simply lets the process run unobserved.
            let _ = request.reply.send(reply);
            return Ok(AgentToolResult {
                details,
                ..AgentToolResult::text(format!("{text}{marker}"))
            });
        }
        Ok(ExecEnd::Exited(code)) => code,
    };
    exited(&mut streaming, exit_code, started)
}

/// The result of a command that exited (the success / non-zero-exit branch).
fn exited(
    streaming: &mut Streaming,
    exit_code: i32,
    started: std::time::Instant,
) -> Result<AgentToolResult, ToolError> {
    let snapshot = streaming.finish()?;
    let (text, details) =
        format_output(&snapshot, streaming.output.last_line_bytes(), "(no output)");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let wall_time_seconds = (elapsed_ms / 100.0).round() / 10.0;
    let full = streaming
        .output
        .read_full_output(STRUCTURED_OUTPUT_MAX_BYTES)
        .map_err(|error| ToolError::new(error.to_string()))?;
    let mut structured = Map::new();
    structured.insert("output".to_owned(), json!(full.content));
    structured.insert("truncated".to_owned(), json!(full.truncated));
    if full.truncated
        && let Some(path) = &snapshot.full_output_path
    {
        structured.insert("full_output_path".to_owned(), json!(path));
    }
    structured.insert("exit_code".to_owned(), json!(exit_code));
    structured.insert("wall_time_seconds".to_owned(), js_number(wall_time_seconds));
    let is_error = exit_code != 0;
    let text = if is_error {
        append_status(&text, &format!("Command exited with code {exit_code}"))
    } else {
        text
    };
    Ok(AgentToolResult {
        content: vec![UserContent::text(text)],
        details,
        structured_content: Some(JsonValue::Object(structured)),
        is_error,
        ..AgentToolResult::default()
    })
}
