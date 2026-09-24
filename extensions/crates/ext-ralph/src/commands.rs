//! `/ralph` command parsing + execution, ported verbatim from the native
//! implementation (`crates/oino-tui/src/command.rs` parser and
//! `crates/oino-app/src/main.rs` `execute_ralph_command`) so outputs match.

use crate::state;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RalphCommand {
    Help,
    List,
    Status {
        name: Option<String>,
    },
    Panel {
        setting: Option<bool>,
    },
    Start {
        name: String,
        task: String,
    },
    Pause {
        name: String,
    },
    Resume {
        name: String,
    },
    Continue {
        name: Option<String>,
    },
    Once {
        name: Option<String>,
    },
    Steer {
        name: String,
        note: String,
    },
    Cancel {
        name: String,
    },
    Archive {
        name: String,
    },
    Clean {
        all: bool,
    },
    Nuke {
        yes: bool,
    },
    Record {
        name: String,
        promise: state::RalphPromise,
        note: String,
    },
}

pub fn parse_ralph_args(args: &str) -> Option<RalphCommand> {
    let rest = args.trim();
    if rest.is_empty() {
        return Some(RalphCommand::Help);
    }
    let (subject, tail) = split_head(rest);
    match subject {
        "help" => tail.is_empty().then_some(RalphCommand::Help),
        "list" => tail.is_empty().then_some(RalphCommand::List),
        "status" => Some(RalphCommand::Status {
            name: optional_single_arg(tail)?,
        }),
        "panel" => match tail.trim() {
            "" => Some(RalphCommand::Panel { setting: None }),
            "on" => Some(RalphCommand::Panel {
                setting: Some(true),
            }),
            "off" => Some(RalphCommand::Panel {
                setting: Some(false),
            }),
            _ => None,
        },
        "start" => {
            let (name, task) = split_head(tail);
            (!name.is_empty() && !task.trim().is_empty()).then_some(RalphCommand::Start {
                name: name.to_string(),
                task: task.trim().to_string(),
            })
        }
        "pause" | "stop" => single_arg(tail).map(|name| RalphCommand::Pause { name }),
        "resume" => single_arg(tail).map(|name| RalphCommand::Resume { name }),
        "continue" | "run" => Some(RalphCommand::Continue {
            name: optional_single_arg(tail)?,
        }),
        "once" => Some(RalphCommand::Once {
            name: optional_single_arg(tail)?,
        }),
        "steer" => {
            let (name, note) = split_head(tail);
            (!name.is_empty() && !note.trim().is_empty()).then_some(RalphCommand::Steer {
                name: name.to_string(),
                note: note.trim().to_string(),
            })
        }
        "cancel" => single_arg(tail).map(|name| RalphCommand::Cancel { name }),
        "archive" => single_arg(tail).map(|name| RalphCommand::Archive { name }),
        "clean" => match tail.trim() {
            "" => Some(RalphCommand::Clean { all: false }),
            "--all" => Some(RalphCommand::Clean { all: true }),
            _ => None,
        },
        "nuke" => match tail.trim() {
            "--yes" => Some(RalphCommand::Nuke { yes: true }),
            "" => Some(RalphCommand::Nuke { yes: false }),
            _ => None,
        },
        "record" => parse_ralph_record(tail),
        _ => None,
    }
}

fn parse_ralph_record(input: &str) -> Option<RalphCommand> {
    let (name, tail) = split_head(input);
    let (promise, note) = split_head(tail);
    if name.is_empty() || promise.is_empty() {
        return None;
    }
    let note = note.trim().to_string();
    let promise = match promise {
        "continue" => state::RalphPromise::Continue,
        "complete" => state::RalphPromise::Complete,
        "blocked" => state::RalphPromise::Blocked(note.clone()),
        "decide" => state::RalphPromise::Decide(note.clone()),
        "done" => {
            let (task_id, rest) = split_head(&note);
            if task_id.is_empty() {
                return None;
            }
            return Some(RalphCommand::Record {
                name: name.to_string(),
                promise: state::RalphPromise::TaskDone(task_id.to_string()),
                note: rest.trim().to_string(),
            });
        }
        _ => return None,
    };
    Some(RalphCommand::Record {
        name: name.to_string(),
        promise,
        note,
    })
}

fn split_head(input: &str) -> (&str, &str) {
    let input = input.trim();
    match input.find(char::is_whitespace) {
        Some(index) => (&input[..index], input[index..].trim()),
        None => (input, ""),
    }
}

fn single_arg(input: &str) -> Option<String> {
    let value = input.trim();
    (!value.is_empty() && !value.contains(char::is_whitespace)).then(|| value.to_string())
}

fn optional_single_arg(input: &str) -> Option<Option<String>> {
    let value = input.trim();
    if value.is_empty() {
        Some(None)
    } else {
        single_arg(value).map(Some)
    }
}

/// Execute a parsed command. `prompt` outputs signal the host to submit the
/// iteration prompt as a user turn (interactive path); `message` is the text
/// shown to the user — identical strings to the previous native executor.
pub struct CommandOutput {
    pub message: String,
    /// Iteration prompt the host should submit (start/resume/continue/once).
    pub prompt: Option<String>,
    /// Whether subsequent turn_end hooks should auto-continue the loop.
    pub auto_continue: bool,
    /// Loop name for the host panel/status bookkeeping.
    pub loop_name: Option<String>,
}

impl CommandOutput {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            prompt: None,
            auto_continue: false,
            loop_name: None,
        }
    }
}

pub fn load_ralph_target(
    project_root: &Path,
    name: Option<&str>,
) -> Result<state::RalphLoopState, state::RalphLoopError> {
    if let Some(name) = name {
        return state::load_state(project_root, name);
    }
    let active = state::list_states(project_root)?
        .into_iter()
        .find(|state| state.status == state::RalphLoopStatus::Active)
        .ok_or_else(|| state::RalphLoopError::NotFound("<active>".into()))?;
    Ok(active)
}

pub fn execute(
    command: &RalphCommand,
    project_root: &Path,
) -> Result<CommandOutput, state::RalphLoopError> {
    match command {
        RalphCommand::Help => Ok(CommandOutput::message(ralph_help_text())),
        RalphCommand::List => {
            let states = state::list_states(project_root)?;
            if states.is_empty() {
                Ok(CommandOutput::message(
                    "No Ralph loops found; start one with `/ralph start <name> <task>`",
                ))
            } else {
                Ok(CommandOutput::message(
                    states
                        .iter()
                        .map(state::status_line)
                        .collect::<Vec<_>>()
                        .join("\n"),
                ))
            }
        }
        RalphCommand::Status { name } => {
            if let Some(name) = name {
                let state = state::load_state(project_root, name)?;
                Ok(CommandOutput::message(state::status_line(&state)))
            } else {
                execute(&RalphCommand::List, project_root)
            }
        }
        RalphCommand::Panel { setting } => {
            let enabled = match setting {
                Some(on) => *on,
                None => !state::load_panel_settings(project_root)?.enabled,
            };
            state::set_panel_enabled(project_root, enabled)?;
            Ok(CommandOutput::message(format!(
                "Ralph progress floating panel {}",
                if enabled { "enabled" } else { "disabled" }
            )))
        }
        RalphCommand::Start { name, task } => {
            let state = state::start_loop(project_root, state::RalphLoopStart::new(name, task))?;
            let prompt = state::build_iteration_prompt(project_root, &state)?;
            Ok(CommandOutput {
                message: format!(
                    "Started Ralph loop `{}` and queued iteration {}/{}",
                    state.name,
                    state.iteration.saturating_add(1),
                    state.max_iterations
                ),
                prompt: Some(prompt),
                auto_continue: true,
                loop_name: Some(state.name.clone()),
            })
        }
        RalphCommand::Pause { name } => {
            let state = state::pause_loop(project_root, name)?;
            Ok(CommandOutput {
                loop_name: Some(state.name.clone()),
                ..CommandOutput::message(format!("Paused Ralph loop `{}`", state.name))
            })
        }
        RalphCommand::Resume { name } => {
            let state = state::resume_loop(project_root, name)?;
            let prompt = state::build_iteration_prompt(project_root, &state)?;
            Ok(CommandOutput {
                message: format!("Resumed Ralph loop `{}`", state.name),
                prompt: Some(prompt),
                auto_continue: true,
                loop_name: Some(state.name.clone()),
            })
        }
        RalphCommand::Continue { name } => {
            let state = load_ralph_target(project_root, name.as_deref())?;
            let prompt = state::build_iteration_prompt(project_root, &state)?;
            Ok(CommandOutput {
                message: format!("Continuing Ralph loop `{}`", state.name),
                prompt: Some(prompt),
                auto_continue: true,
                loop_name: Some(state.name.clone()),
            })
        }
        RalphCommand::Once { name } => {
            let state = load_ralph_target(project_root, name.as_deref())?;
            let prompt = state::build_iteration_prompt(project_root, &state)?;
            Ok(CommandOutput {
                message: format!("Running one Ralph iteration for `{}`", state.name),
                prompt: Some(prompt),
                auto_continue: false,
                loop_name: Some(state.name.clone()),
            })
        }
        RalphCommand::Steer { name, note } => {
            let state = state::append_steering(project_root, name, note)?;
            Ok(CommandOutput::message(format!(
                "Added steering to Ralph loop `{}` ({})",
                state.name, state.steering_file
            )))
        }
        RalphCommand::Cancel { name } => {
            let state = state::cancel_loop(project_root, name)?;
            Ok(CommandOutput {
                loop_name: Some(state.name.clone()),
                ..CommandOutput::message(format!("Cancelled Ralph loop `{}`", state.name))
            })
        }
        RalphCommand::Archive { name } => {
            let state = state::archive_loop(project_root, name)?;
            Ok(CommandOutput::message(format!(
                "Archived Ralph loop `{}`",
                state.name
            )))
        }
        RalphCommand::Clean { all } => {
            let summary = state::clean_loops(project_root, *all)?;
            if *all {
                Ok(CommandOutput::message(format!(
                    "Removed {} Ralph loop item{} ({} archived, {} complete/cancelled)",
                    summary.total_removed(),
                    if summary.total_removed() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    summary.archived_removed,
                    summary.terminal_removed
                )))
            } else {
                Ok(CommandOutput::message(format!(
                    "Removed {} archived Ralph loop item{}",
                    summary.archived_removed,
                    if summary.archived_removed == 1 {
                        ""
                    } else {
                        "s"
                    }
                )))
            }
        }
        RalphCommand::Nuke { yes } => {
            if !yes {
                return Err(state::RalphLoopError::NotFound(
                    "Refusing to delete all Ralph data without confirmation. Run `/ralph nuke --yes` to remove .oino/ralph entirely.".into(),
                ));
            }
            let removed = state::nuke_all(project_root)?;
            Ok(CommandOutput::message(if removed {
                "Deleted all Ralph loop data under .oino/ralph"
            } else {
                "No Ralph loop data found under .oino/ralph"
            }))
        }
        RalphCommand::Record {
            name,
            promise,
            note,
        } => {
            let note = if note.trim().is_empty() {
                format!("recorded {promise:?}")
            } else {
                note.clone()
            };
            let state = state::record_iteration(project_root, name, promise.clone(), note)?;
            Ok(CommandOutput::message(format!(
                "Recorded Ralph loop `{}` iteration {} ({:?})",
                state.name, state.iteration, state.status
            )))
        }
    }
}

pub fn ralph_help_text() -> String {
    [
        "Ralph loop commands:",
        "  /ralph start <name> <task>",
        "  /ralph list",
        "  /ralph status [name]",
        "  /ralph panel [on|off]",
        "  /ralph resume <name>",
        "  /ralph continue [name]",
        "  /ralph once [name]",
        "  /ralph steer <name> <urgent instruction>",
        "  /ralph pause <name>",
        "  /ralph cancel <name>",
        "  /ralph archive <name>",
        "  /ralph clean [--all]",
        "  /ralph nuke --yes",
        "  /ralph record <name> <continue|complete|blocked|decide|done> [note/task-id]",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from the old host test `executes_ralph_commands_against_project_state`.
    #[test]
    fn executes_ralph_commands_against_project_state() {
        let temp = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let help = execute(&RalphCommand::Help, temp.path()).unwrap_or_else(|e| panic!("{e}"));
        assert!(help.message.contains("/ralph start"));

        let started = execute(
            &RalphCommand::Start {
                name: "Demo Loop".into(),
                task: "Build iterative feature".into(),
            },
            temp.path(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(started.message.contains("demo-loop"));
        assert!(started.prompt.is_some());
        assert!(started.auto_continue);
        assert!(temp
            .path()
            .join(".oino/ralph/demo-loop/state.json")
            .is_file());

        let recorded = execute(
            &RalphCommand::Record {
                name: "demo-loop".into(),
                promise: state::RalphPromise::TaskDone("TASK-1".into()),
                note: "scaffolded".into(),
            },
            temp.path(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(recorded.message.contains("iteration 1"));

        let status = execute(
            &RalphCommand::Status {
                name: Some("demo-loop".into()),
            },
            temp.path(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(status.message.contains("demo-loop: Active iteration 1/60"));

        let panel = execute(&RalphCommand::Panel { setting: None }, temp.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(panel.message.contains("enabled"));
        assert!(temp.path().join(".oino/ralph/panel.json").is_file());
        let panel = execute(
            &RalphCommand::Panel {
                setting: Some(false),
            },
            temp.path(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(panel.message.contains("disabled"));

        let once = execute(&RalphCommand::Once { name: None }, temp.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(once.prompt.is_some());
        assert!(!once.auto_continue);

        execute(
            &RalphCommand::Archive {
                name: "demo-loop".into(),
            },
            temp.path(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let cleaned = execute(&RalphCommand::Clean { all: false }, temp.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(cleaned
            .message
            .contains("Removed 1 archived Ralph loop item"));

        assert!(execute(&RalphCommand::Nuke { yes: false }, temp.path()).is_err());
        let nuked = execute(&RalphCommand::Nuke { yes: true }, temp.path())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            nuked.message.contains("Deleted all Ralph loop data")
                || nuked.message.contains("No Ralph loop data")
        );
    }

    #[test]
    fn parses_all_ralph_subcommands() {
        assert_eq!(parse_ralph_args(""), Some(RalphCommand::Help));
        assert_eq!(parse_ralph_args("list"), Some(RalphCommand::List));
        assert_eq!(
            parse_ralph_args("status demo"),
            Some(RalphCommand::Status {
                name: Some("demo".into())
            })
        );
        assert_eq!(
            parse_ralph_args("panel on"),
            Some(RalphCommand::Panel {
                setting: Some(true)
            })
        );
        assert_eq!(
            parse_ralph_args("start demo build things"),
            Some(RalphCommand::Start {
                name: "demo".into(),
                task: "build things".into()
            })
        );
        assert_eq!(
            parse_ralph_args("continue"),
            Some(RalphCommand::Continue { name: None })
        );
        assert_eq!(
            parse_ralph_args("once demo"),
            Some(RalphCommand::Once {
                name: Some("demo".into())
            })
        );
        assert_eq!(
            parse_ralph_args("steer demo focus tests"),
            Some(RalphCommand::Steer {
                name: "demo".into(),
                note: "focus tests".into()
            })
        );
        assert_eq!(
            parse_ralph_args("clean --all"),
            Some(RalphCommand::Clean { all: true })
        );
        assert_eq!(
            parse_ralph_args("nuke --yes"),
            Some(RalphCommand::Nuke { yes: true })
        );
        assert_eq!(
            parse_ralph_args("record demo done TASK-7 noted"),
            Some(RalphCommand::Record {
                name: "demo".into(),
                promise: state::RalphPromise::TaskDone("TASK-7".into()),
                note: "noted".into()
            })
        );
    }
}
