//! pi `cli/args.ts` `parseArgs`: the hand-written parser, ported rule for rule (multi-letter
//! short flags such as `-nbt`, `-p` taking an optional message, `@file` arguments, unknown
//! long flags kept for extensions).

use indexmap::IndexMap;
use oino_ai::{JsonValue, ThinkingLevel};

/// pi `VALID_THINKING_LEVELS`.
pub const VALID_THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// `--mode` values (Oino adds `acp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// Print mode text output.
    Text,
    /// JSON event stream.
    Json,
    /// RPC over stdio.
    Rpc,
    /// ACP agent over stdio.
    Acp,
}

/// Severity of a parse diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Exit 1 after reporting.
    Error,
    /// Reported only.
    Warning,
}

/// A parse or startup diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// pi's text.
    pub message: String,
}

impl Diagnostic {
    /// An error.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    /// A warning.
    #[must_use]
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// `--list-models [search]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListModels {
    /// Every available model.
    All,
    /// Fuzzy filtered.
    Search(String),
}

/// `--tui-mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiMode {
    /// Inline.
    Regular,
    /// Alternate screen.
    Fullscreen,
}

/// pi `Args`.
#[allow(
    clippy::struct_excessive_bools,
    reason = "one field per pi CLI switch, as in pi's Args"
)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    /// `--provider`.
    pub provider: Option<String>,
    /// `--model`.
    pub model: Option<String>,
    /// `--api-key`.
    pub api_key: Option<String>,
    /// `--system-prompt`.
    pub system_prompt: Option<String>,
    /// `--append-system-prompt` (repeatable).
    pub append_system_prompt: Option<Vec<String>>,
    /// `--thinking`.
    pub thinking: Option<ThinkingLevel>,
    /// `-c`.
    pub continue_session: bool,
    /// `-r`.
    pub resume: bool,
    /// `-h`.
    pub help: bool,
    /// `-v`.
    pub version: bool,
    /// `--mode`.
    pub mode: Option<OutputMode>,
    /// `-n/--name`.
    pub name: Option<String>,
    /// `--no-session`.
    pub no_session: bool,
    /// `--session`.
    pub session: Option<String>,
    /// `--session-id`.
    pub session_id: Option<String>,
    /// `--fork`.
    pub fork: Option<String>,
    /// `--session-dir`.
    pub session_dir: Option<String>,
    /// `--models`.
    pub models: Option<Vec<String>>,
    /// `-t/--tools`.
    pub tools: Option<Vec<String>>,
    /// `-xt/--exclude-tools`.
    pub exclude_tools: Option<Vec<String>>,
    /// `-nt`.
    pub no_tools: bool,
    /// `-nbt`.
    pub no_builtin_tools: bool,
    /// `-e` (repeatable).
    pub extensions: Option<Vec<String>>,
    /// `-ne`.
    pub no_extensions: bool,
    /// `--no-mcp` (accepted, no core MCP).
    pub no_mcp: bool,
    /// `-p`.
    pub print: bool,
    /// `--export <in>`.
    pub export: Option<String>,
    /// `--skill` (repeatable).
    pub skills: Option<Vec<String>>,
    /// `--prompt-template` (repeatable).
    pub prompt_templates: Option<Vec<String>>,
    /// `--theme` (repeatable).
    pub themes: Option<Vec<String>>,
    /// `--use-theme`.
    pub use_theme: Option<String>,
    /// `-ns`.
    pub no_skills: bool,
    /// `-np`.
    pub no_prompt_templates: bool,
    /// `--no-themes`.
    pub no_themes: bool,
    /// `-nc`.
    pub no_context_files: bool,
    /// `--list-models`.
    pub list_models: Option<ListModels>,
    /// `--tui-mode`.
    pub tui_mode: Option<TuiMode>,
    /// `--verbose`.
    pub verbose: bool,
    /// `-a` / `-na`.
    pub project_trust_override: Option<bool>,
    /// `--offline`.
    pub offline: bool,
    /// Oino `--remote <url>`.
    pub remote: Option<String>,
    /// Oino `--agent-dir <dir>`.
    pub agent_dir: Option<String>,
    /// Positional messages.
    pub messages: Vec<String>,
    /// `@file` arguments (without `@`).
    pub file_args: Vec<String>,
    /// Unknown long flags for extensions (`true` or a string value).
    pub unknown_flags: IndexMap<String, JsonValue>,
    /// Parse diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn push(list: &mut Option<Vec<String>>, value: String) {
    list.get_or_insert_with(Vec::new).push(value);
}

/// pi `normalizeSessionName`.
#[must_use]
pub fn normalize_session_name(value: &str) -> Option<String> {
    let name = value.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// pi `parseArgs`.
#[allow(
    clippy::too_many_lines,
    reason = "one branch per pi flag, in pi's order, so the port stays reviewable"
)]
#[must_use]
pub fn parse_args(args: &[String]) -> Args {
    let mut result = Args::default();
    let mut i = 0;
    let value_at = |i: usize| args.get(i).cloned();
    while i < args.len() {
        let arg = args[i].as_str();
        let has_next = i + 1 < args.len();
        match arg {
            "--" => {
                for positional in &args[i + 1..] {
                    match positional.strip_prefix('@') {
                        Some(file) => result.file_args.push(file.to_owned()),
                        None => result.messages.push(positional.clone()),
                    }
                }
                break;
            }
            "--help" | "-h" => result.help = true,
            "--version" | "-v" => result.version = true,
            "--mode" => match value_at(i + 1) {
                Some(mode) if !mode.starts_with('-') => {
                    i += 1;
                    result.mode = match mode.as_str() {
                        "text" => Some(OutputMode::Text),
                        "json" => Some(OutputMode::Json),
                        "rpc" => Some(OutputMode::Rpc),
                        "acp" => Some(OutputMode::Acp),
                        _ => {
                            result.diagnostics.push(Diagnostic::error(format!(
                                "Invalid mode \"{mode}\". Valid values: text, json, rpc, acp"
                            )));
                            result.mode
                        }
                    };
                }
                _ => result
                    .diagnostics
                    .push(Diagnostic::error("--mode requires text, json, rpc, or acp")),
            },
            "--continue" | "-c" => result.continue_session = true,
            "--resume" | "-r" => result.resume = true,
            "--provider" if has_next => {
                i += 1;
                result.provider = value_at(i);
            }
            "--model" if has_next => {
                i += 1;
                result.model = value_at(i);
            }
            "--api-key" if has_next => {
                i += 1;
                result.api_key = value_at(i);
            }
            "--system-prompt" if has_next => {
                i += 1;
                result.system_prompt = value_at(i);
            }
            "--append-system-prompt" if has_next => {
                i += 1;
                push(&mut result.append_system_prompt, args[i].clone());
            }
            "--name" | "-n" => {
                if has_next {
                    i += 1;
                    result.name = value_at(i);
                } else {
                    result
                        .diagnostics
                        .push(Diagnostic::error("--name requires a value"));
                }
            }
            "--no-session" => result.no_session = true,
            "--session" if has_next => {
                i += 1;
                result.session = value_at(i);
            }
            "--session-id" if has_next => {
                i += 1;
                result.session_id = value_at(i);
            }
            "--fork" if has_next => {
                i += 1;
                result.fork = value_at(i);
            }
            "--session-dir" if has_next => {
                i += 1;
                result.session_dir = value_at(i);
            }
            "--models" if has_next => {
                i += 1;
                result.models = Some(split_list(&args[i]));
            }
            "--no-tools" | "-nt" => result.no_tools = true,
            "--no-builtin-tools" | "-nbt" => result.no_builtin_tools = true,
            "--tools" | "-t" if has_next => {
                i += 1;
                let tools = split_list(&args[i]);
                match oino_settings::tools::tool_list_error(&tools) {
                    Some(error) => result
                        .diagnostics
                        .push(Diagnostic::error(format!("{arg}: {error}"))),
                    None => result.tools = Some(tools),
                }
            }
            "--exclude-tools" | "-xt" if has_next => {
                i += 1;
                result.exclude_tools = Some(split_list(&args[i]));
            }
            "--thinking" if has_next => {
                i += 1;
                let level = &args[i];
                match level.parse::<ThinkingLevel>() {
                    Ok(parsed) if VALID_THINKING_LEVELS.contains(&level.as_str()) => {
                        result.thinking = Some(parsed);
                    }
                    _ => result.diagnostics.push(Diagnostic::warning(format!(
                        "Invalid thinking level \"{level}\". Valid values: {}",
                        VALID_THINKING_LEVELS.join(", ")
                    ))),
                }
            }
            "--print" | "-p" => {
                result.print = true;
                if let Some(next) = args.get(i + 1)
                    && !next.starts_with('@')
                    && (!next.starts_with('-') || next.starts_with("---"))
                {
                    result.messages.push(next.clone());
                    i += 1;
                }
            }
            "--export" if has_next => {
                i += 1;
                result.export = value_at(i);
            }
            "--extension" | "-e" if has_next => {
                i += 1;
                push(&mut result.extensions, args[i].clone());
            }
            "--no-extensions" | "-ne" => result.no_extensions = true,
            "--no-mcp" => result.no_mcp = true,
            "--skill" if has_next => {
                i += 1;
                push(&mut result.skills, args[i].clone());
            }
            "--prompt-template" if has_next => {
                i += 1;
                push(&mut result.prompt_templates, args[i].clone());
            }
            "--theme" if has_next => {
                i += 1;
                push(&mut result.themes, args[i].clone());
            }
            "--use-theme" => match args.get(i + 1) {
                Some(name) if !name.starts_with('-') => {
                    result.use_theme = Some(name.clone());
                    i += 1;
                }
                _ => result
                    .diagnostics
                    .push(Diagnostic::error("--use-theme requires a theme name")),
            },
            "--no-skills" | "-ns" => result.no_skills = true,
            "--no-prompt-templates" | "-np" => result.no_prompt_templates = true,
            "--no-themes" => result.no_themes = true,
            "--no-context-files" | "-nc" => result.no_context_files = true,
            "--list-models" => match args.get(i + 1) {
                Some(search) if !search.starts_with('-') && !search.starts_with('@') => {
                    result.list_models = Some(ListModels::Search(search.clone()));
                    i += 1;
                }
                _ => result.list_models = Some(ListModels::All),
            },
            "--tui-mode" => match args.get(i + 1).map(String::as_str) {
                Some("regular") => {
                    result.tui_mode = Some(TuiMode::Regular);
                    i += 1;
                }
                Some("fullscreen") => {
                    result.tui_mode = Some(TuiMode::Fullscreen);
                    i += 1;
                }
                Some(mode) if !mode.starts_with('-') => {
                    i += 1;
                    result.diagnostics.push(Diagnostic::error(format!(
                        "Invalid TUI mode \"{mode}\". Valid values: regular, fullscreen"
                    )));
                }
                _ => result.diagnostics.push(Diagnostic::error(
                    "--tui-mode requires regular or fullscreen",
                )),
            },
            "--verbose" => result.verbose = true,
            "--approve" | "-a" => result.project_trust_override = Some(true),
            "--no-approve" | "-na" => result.project_trust_override = Some(false),
            "--offline" => result.offline = true,
            "--remote" if has_next => {
                i += 1;
                result.remote = value_at(i);
            }
            "--agent-dir" if has_next => {
                i += 1;
                result.agent_dir = value_at(i);
            }
            _ => {
                if let Some(file) = arg.strip_prefix('@') {
                    result.file_args.push(file.to_owned());
                } else if let Some(flag) = arg.strip_prefix("--") {
                    if let Some((name, value)) = flag.split_once('=') {
                        result
                            .unknown_flags
                            .insert(name.to_owned(), JsonValue::from(value));
                    } else {
                        match args.get(i + 1) {
                            Some(next) if !next.starts_with('-') && !next.starts_with('@') => {
                                result
                                    .unknown_flags
                                    .insert(flag.to_owned(), JsonValue::from(next.as_str()));
                                i += 1;
                            }
                            _ => {
                                result
                                    .unknown_flags
                                    .insert(flag.to_owned(), JsonValue::Bool(true));
                            }
                        }
                    }
                } else if arg.starts_with('-') {
                    result
                        .diagnostics
                        .push(Diagnostic::error(format!("Unknown option: {arg}")));
                } else {
                    result.messages.push(arg.to_owned());
                }
            }
        }
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        parse_args(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn print_takes_an_optional_message() {
        let args = parse(&["-p", "hello", "world"]);
        assert!(args.print);
        assert_eq!(args.messages, vec!["hello", "world"]);
        let args = parse(&["-p", "@a.md", "x"]);
        assert_eq!(args.messages, vec!["x"]);
        assert_eq!(args.file_args, vec!["a.md"]);
        let args = parse(&["-p", "--", "- point"]);
        assert_eq!(args.messages, vec!["- point"]);
        let args = parse(&["-p", "---x"]);
        assert_eq!(args.messages, vec!["---x"]);
    }

    #[test]
    fn short_multi_letter_flags_and_lists() {
        let args = parse(&[
            "-nbt",
            "-nc",
            "-t",
            "read, grep ,",
            "-xt",
            "bash",
            "-ne",
            "-na",
        ]);
        assert!(args.no_builtin_tools && args.no_context_files && args.no_extensions);
        assert_eq!(args.tools, Some(vec!["read".into(), "grep".into()]));
        assert_eq!(args.exclude_tools, Some(vec!["bash".into()]));
        assert_eq!(args.project_trust_override, Some(false));
    }

    #[test]
    fn modes_and_diagnostics() {
        assert_eq!(parse(&["--mode", "rpc"]).mode, Some(OutputMode::Rpc));
        assert_eq!(
            parse(&["--mode", "x"]).diagnostics,
            vec![Diagnostic::error(
                "Invalid mode \"x\". Valid values: text, json, rpc, acp"
            )]
        );
        assert_eq!(
            parse(&["--mode"]).diagnostics[0].message,
            "--mode requires text, json, rpc, or acp"
        );
        assert_eq!(
            parse(&["-z"]).diagnostics,
            vec![Diagnostic::error("Unknown option: -z")]
        );
        let thinking = parse(&["--thinking", "huge"]);
        assert_eq!(thinking.diagnostics[0].severity, Severity::Warning);
        assert_eq!(
            thinking.diagnostics[0].message,
            "Invalid thinking level \"huge\". Valid values: off, minimal, low, medium, high, xhigh, max"
        );
        assert_eq!(
            parse(&["--thinking", "high"]).thinking,
            Some(ThinkingLevel::High)
        );
    }

    #[test]
    fn unknown_long_flags_go_to_extensions() {
        let args = parse(&["--plan", "--depth=3", "--level", "5", "msg"]);
        assert_eq!(args.unknown_flags["plan"], JsonValue::Bool(true));
        assert_eq!(args.unknown_flags["depth"], JsonValue::from("3"));
        assert_eq!(args.unknown_flags["level"], JsonValue::from("5"));
        assert_eq!(args.messages, vec!["msg"]);
    }

    #[test]
    fn list_models_and_repeatables() {
        assert_eq!(parse(&["--list-models"]).list_models, Some(ListModels::All));
        assert_eq!(
            parse(&["--list-models", "son"]).list_models,
            Some(ListModels::Search("son".into()))
        );
        let args = parse(&["-e", "a", "--extension", "b", "--append-system-prompt", "x"]);
        assert_eq!(args.extensions, Some(vec!["a".into(), "b".into()]));
        assert_eq!(args.append_system_prompt, Some(vec!["x".into()]));
        assert_eq!(
            parse(&["--name"]).diagnostics[0].message,
            "--name requires a value"
        );
    }
}
