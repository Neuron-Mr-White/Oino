//! `oino`: the headless Oino binary over `oino-core` (pi `main.ts`, modes.md §1–§3).
//!
//! pi 1.4.0 command line (same flags and semantics): interactive mode (the `oino-tui`
//! client), print mode, JSON event stream mode, RPC mode and ACP mode on stdio. Every mode
//! loads the bundled provider and unipi components ([`loader`]). Package subcommands
//! (`install`, `remove`, `update`, `list`, `config`) and the self-update (`oino update`) run
//! before flag parsing; `serve` is not available yet.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`cli::args`] | `cli/args.ts` | [`cli::args::parse_args`] → [`cli::args::Args`] |
//! | [`cli::help`] | `printHelp` | help text with extension flags |
//! | [`cli::files`] | `file-processor.ts`, `initial-message.ts` | `@file` arguments, first message |
//! | [`cli::list_models`] | `list-models.ts` | `--list-models` table |
//! | [`cli::packages`] | `package-manager.ts` | `install`/`remove`/`update`/`list`/`config` |
//! | [`cli::self_update`] | — | `oino update [--check]` from the release manifest |
//! | [`cli::cron`] | docs/cron.md | `oino cron …` (serve/install/list/add/run/…) |
//! | [`startup`] | `main.ts` `createRuntime` | services, session selection, model/tool options |
//! | [`loader`] | resource loader `-e` | bundled, installed and `-e` packages, `builtin:` components |
//! | [`modes::print`] | `print-mode.ts` | text and JSON output |
//! | [`modes::rpc`] | `rpc-mode.ts` | RPC server on stdio |
//! | [`modes::interactive`] | `interactive-mode.ts` | the TUI over an in-process backend |
//! | [`modes::acp`] | — | ACP agent on stdio (`oino-acp`) |
//! | `faux` (feature) | — | scripted provider + UI probe for tests |

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod browser;
pub mod cli;
#[cfg(feature = "faux")]
pub mod faux;
pub mod loader;
pub mod modes;
pub mod startup;

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use cli::args::{Diagnostic, ListModels, Severity, parse_args};
use cli::files::{build_initial_message, process_file_arguments};
use modes::print::{PrintInput, PrintOutput};
use startup::{AppMode, Environment, StartupError};

/// The version printed by `-v`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Writes a line to stderr (diagnostics never go to stdout).
pub(crate) fn eprintln_line(text: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{text}");
}

fn print_stdout(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
}

/// `Error: …` / `Warning: …` lines (pi `reportDiagnostics`).
pub(crate) fn report_lines(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|d| match d.severity {
            Severity::Error => format!("Error: {}", d.message),
            Severity::Warning => format!("Warning: {}", d.message),
        })
        .collect()
}

fn fail(lines: &[String]) -> i32 {
    for line in lines {
        eprintln_line(line);
    }
    1
}

/// pi `readPipedStdin`: all of stdin when it is not a terminal, trimmed; `None` if empty.
fn read_piped_stdin() -> Option<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut text = String::new();
    stdin.read_to_string(&mut text).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// pi `exportFromFile`.
fn export_from_file(input: &str, output: Option<&str>, cwd: &Path) -> Result<PathBuf, String> {
    let home = oino_settings::paths::home_dir();
    let input = oino_settings::paths::resolve_path(input, cwd, home.as_deref(), false);
    if !input.exists() {
        return Err(format!("File not found: {}", input.display()));
    }
    let root = input.parent().unwrap_or(cwd).to_path_buf();
    let manager =
        oino_session::SessionManager::open(&input, &root, Some(cwd)).map_err(|e| e.to_string())?;
    let branch: Vec<oino_session::SessionEntry> =
        manager.branch(None).into_iter().cloned().collect();
    let html = oino_core::export::render_html(
        manager.session_id(),
        manager.session_name().as_deref(),
        &branch,
    );
    let output = output.map_or_else(
        || {
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            PathBuf::from(format!("oino-session-{stem}.html"))
        },
        |o| oino_settings::paths::normalize_path(o, home.as_deref()),
    );
    std::fs::write(&output, html).map_err(|e| e.to_string())?;
    Ok(output)
}

fn environment(args: &cli::args::Args) -> Result<Environment, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("Error: {e}"))?;
    let agent_dir = match &args.agent_dir {
        Some(dir) => {
            let home = oino_settings::paths::home_dir();
            oino_settings::paths::resolve_path(dir, &cwd, home.as_deref(), false)
        }
        None => oino_settings::paths::agent_dir().map_err(|e| format!("Error: {e}"))?,
    };
    Ok(Environment { cwd, agent_dir })
}

/// Package manager subcommands (pi `handlePackageCommand`, runtime §15).
const PACKAGE_COMMANDS: [&str; 6] = ["install", "remove", "uninstall", "update", "list", "config"];

/// Subcommands handled before flag parsing that are not available yet (`auth`, Oino
/// `serve`/`ext`/`sessions`); owned by other work items.
const UNAVAILABLE_SUBCOMMANDS: [&str; 4] = ["auth", "mcp", "ext", "sessions"];

/// `oino install|remove|uninstall|update|list|config …`; bare `oino update [--check]` is the
/// self-update (`update <id>…` / `update --extensions` update packages).
async fn package_command(command: &str, rest: &[String]) -> i32 {
    let self_update = command == "update" && rest.iter().all(|a| a == "--check" || a == "--self");
    if self_update {
        let check = rest.iter().any(|a| a == "--check");
        return match cli::self_update::self_update(check, None).await {
            Ok(message) => {
                print_stdout(&format!("{message}\n"));
                0
            }
            Err(error) => fail(&[format!("Error: {error}")]),
        };
    }
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => return fail(&[format!("Error: {error}")]),
    };
    let agent_dir = match oino_settings::paths::agent_dir() {
        Ok(dir) => dir,
        Err(error) => return fail(&[format!("Error: {error}")]),
    };
    let context = cli::packages::PackageContext { agent_dir, cwd };
    cli::packages::run_command(command, rest, &context).await
}

/// Runs the CLI with `command_line` (without the program name); returns the exit code.
pub async fn run(command_line: Vec<String>) -> i32 {
    if let Some(command) = command_line
        .first()
        .filter(|c| PACKAGE_COMMANDS.contains(&c.as_str()))
    {
        return package_command(command, &command_line[1..]).await;
    }
    if command_line.first().is_some_and(|c| c == "cron") {
        return cli::cron::run(&command_line[1..]).await;
    }
    if let Some(command) = command_line
        .first()
        .filter(|c| UNAVAILABLE_SUBCOMMANDS.contains(&c.as_str()))
    {
        return fail(&[format!(
            "Error: \"oino {command}\" is not available in this build yet"
        )]);
    }
    let args = parse_args(&command_line);
    if !args.diagnostics.is_empty() {
        for line in report_lines(&args.diagnostics) {
            eprintln_line(&line);
        }
        if args
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            return 1;
        }
    }
    if args.version {
        print_stdout(&format!("{VERSION}\n"));
        return 0;
    }
    let env = match environment(&args) {
        Ok(env) => env,
        Err(message) => return fail(&[message]),
    };
    if let Some(input) = &args.export {
        return match export_from_file(input, args.messages.first().map(String::as_str), &env.cwd) {
            Ok(path) => {
                print_stdout(&format!("Exported to: {}\n", path.display()));
                0
            }
            Err(message) => fail(&[format!("Error: {message}")]),
        };
    }
    let mode = AppMode::resolve(
        &args,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    );
    if let Err(lines) = preflight(&args, mode) {
        return fail(&lines);
    }
    Box::pin(run_mode(args, mode, &env)).await
}

/// Mode checks before any service starts.
fn preflight(args: &cli::args::Args, mode: AppMode) -> Result<(), Vec<String>> {
    if mode == AppMode::Rpc && !args.file_args.is_empty() {
        return Err(vec![
            "Error: @file arguments are not supported in RPC mode".to_owned(),
        ]);
    }
    startup::validate_session_flags(args).map_err(|StartupError(lines)| lines)?;
    if args.help || args.list_models.is_some() {
        return Ok(());
    }
    let unavailable = match mode {
        AppMode::Interactive if args.remote.is_some() => Some("Remote interactive mode"),
        _ => None,
    };
    match unavailable {
        Some(what) => Err(vec![format!(
            "Error: {what} is not available in this build; use -p, --mode json or --mode rpc"
        )]),
        None => Ok(()),
    }
}

/// Starts the runtime and runs `--help`, `--list-models`, ACP, RPC, interactive or print/JSON
/// mode.
async fn run_mode(mut args: cli::args::Args, mode: AppMode, env: &Environment) -> i32 {
    if mode == AppMode::Acp && !args.help && args.list_models.is_none() {
        return modes::acp::run(&args, env).await;
    }
    let started = match startup::start(&args, mode, env).await {
        Ok(started) => started,
        Err(StartupError(lines)) => return fail(&lines),
    };
    if args.help {
        print_stdout(&cli::help::help_text(&started.extension_flags));
        return 0;
    }
    if let Some(list) = &args.list_models {
        let no_models = oino_models::auth::guidance::no_models_available_message(
            &startup::docs_root(&env.agent_dir).join("docs"),
        );
        let search = match list {
            ListModels::All => None,
            ListModels::Search(search) => Some(search.as_str()),
        };
        print_stdout(&cli::list_models::list_models_output(
            &started.runtime.session().models().available(),
            search,
            &no_models,
        ));
        started.runtime.dispose().await;
        return 0;
    }
    for line in report_lines(&started.diagnostics) {
        eprintln_line(&line);
    }
    if mode == AppMode::Rpc {
        return modes::rpc::run(started.runtime).await;
    }
    let stdin = read_piped_stdin();
    let files = match process_file_arguments(&args.file_args, &env.cwd) {
        Ok(files) => files,
        Err(message) => {
            started.runtime.dispose().await;
            return fail(&[message]);
        }
    };
    let initial = build_initial_message(&mut args.messages, files, stdin);
    if mode == AppMode::Interactive {
        let notices = report_lines(&started.diagnostics);
        let mut initial_messages: Vec<String> = initial.text.into_iter().collect();
        initial_messages.append(&mut args.messages);
        let options = oino_tui::TuiOptions {
            version: VERSION.to_owned(),
            initial_messages,
            initial_images: initial.images,
            notices,
            theme: None,
            tui_mode: args.tui_mode.map(|mode| match mode {
                cli::args::TuiMode::Regular => oino_tui::TuiMode::Regular,
                cli::args::TuiMode::Fullscreen => oino_tui::TuiMode::Fullscreen,
            }),
        };
        return modes::interactive::run(started, env, options).await;
    }
    let output = if mode == AppMode::Json {
        PrintOutput::Json
    } else {
        PrintOutput::Text
    };
    let input = PrintInput {
        initial,
        messages: args.messages,
    };
    modes::print::run(started.runtime, output, input).await
}
