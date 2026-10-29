//! CLI dispatcher: turns parsed arguments into library calls and renders the
//! result (JSON for `--json`, one-line summaries otherwise).

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::{Cli, Command, ProjectCommand};
use crate::commands::{self, Common, EditArgs, OrderTarget};
use crate::error::{Error, Result};
use crate::model::{Actor, ChainGate, Priority, Status};
use crate::store::{self, Layout};

/// The environment one dispatch runs under (round 5, #7): the CLI uses
/// [`Exec::process`], in-process hosts (`service::exec`, the board service)
/// pass the request's env/cwd/stdin so nothing reads the host process's state.
pub struct Exec<'e> {
    /// Environment lookup — the request's vars on top of the host's.
    pub var: &'e dyn Fn(&str) -> Option<String>,
    /// Working directory project resolution happens in.
    pub cwd: &'e Path,
    /// What a `-` argument reads; `None` falls back to real stdin (CLI).
    pub stdin: Option<&'e str>,
    /// The pid a `start` claim belongs to when neither `--pid` nor
    /// `OINO_KANBOARD_PID` says otherwise (the host process, in-process).
    pub owner_pid: u32,
}

impl Exec<'_> {
    /// Trimmed env lookup.
    fn var_string(&self, name: &str) -> Option<String> {
        (self.var)(name).map(|value| value.trim().to_string())
    }
}

/// The pid `Exec::process` falls back to: the caller's parent on unix (the
/// shell that ran the CLI outlives it), the process itself elsewhere.
fn process_owner_pid() -> u32 {
    #[cfg(unix)]
    {
        std::os::unix::process::parent_id()
    }
    #[cfg(not(unix))]
    {
        std::process::id()
    }
}

/// Runs the parsed command under the process environment.
pub fn dispatch(cli: &Cli) -> Result<Value> {
    let cwd = std::env::current_dir()?;
    let exec = Exec {
        var: &|name| std::env::var(name).ok(),
        cwd: &cwd,
        stdin: None,
        owner_pid: process_owner_pid(),
    };
    dispatch_in(cli, &exec)
}

/// Runs the parsed command under an explicit environment and returns its JSON
/// payload.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per subcommand: the dispatcher mirrors the CLI table"
)]
pub fn dispatch_in(cli: &Cli, exec: &Exec<'_>) -> Result<Value> {
    let layout = layout_in(exec)?;
    let actor = resolve_actor_in(cli.actor.as_deref(), exec);
    let gate: ChainGate = cli.gate.parse().map_err(|_| {
        Error::usage(format!(
            "unknown --gate \"{}\" (expected in_review|done)",
            cli.gate
        ))
    })?;
    let mut common = Common::new(actor, gate);
    common.session = cli
        .session
        .clone()
        .or_else(|| exec.var_string("OINO_KANBOARD_SESSION"))
        .filter(|value| !value.is_empty());
    let env_project = exec.var_string(store::PROJECT_ENV);
    let project = |layout: &Layout, explicit: Option<&str>| -> Result<store::Project> {
        store::resolve_project_in(layout, explicit, env_project.as_deref(), exec.cwd)
    };

    match &cli.command {
        Command::Project(ProjectCommand::Add(args)) => commands::project_add(
            &layout,
            args.root.as_deref(),
            args.name.as_deref(),
            args.prefix.as_deref(),
            exec.cwd,
        ),
        Command::Project(ProjectCommand::List) => commands::project_list(&layout),
        Command::Project(ProjectCommand::Archive { slug } | ProjectCommand::Unarchive { slug }) => {
            if common.actor == Actor::Agent {
                return Err(Error::rule("project archive/unarchive is user only"));
            }
            let archived = matches!(
                cli.command,
                Command::Project(ProjectCommand::Archive { .. })
            );
            commands::project_set_archived(&layout, slug, archived)
        }
        Command::Project(ProjectCommand::Show) => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::project_show(&layout, &project)
        }
        Command::Project(ProjectCommand::Rebind { slug, root }) => {
            if common.actor == Actor::Agent {
                return Err(Error::rule("project rebind is user only"));
            }
            commands::project_rebind(&layout, slug, root)
        }

        Command::Add {
            title,
            body,
            body_file,
            attach,
            attach_existing,
            status,
            priority,
            after,
            labels,
            new_label,
        } => {
            let mut attach = attach.clone();
            for path in existing_files(attach_existing) {
                if !attach.contains(&path) {
                    attach.push(path);
                }
            }
            let project = project(&layout, cli.project.as_deref())?;
            let status = status
                .as_deref()
                .map(crate::cli::parse_status)
                .transpose()?;
            let priority = priority
                .as_deref()
                .map(crate::cli::parse_priority)
                .transpose()?
                .unwrap_or(Priority::None);
            let body = match body_file {
                Some(file) => Some(std::fs::read_to_string(file).map_err(|err| {
                    Error::usage(format!("cannot read {}: {err}", file.display()))
                })?),
                None => read_body(body.as_deref(), exec)?,
            };
            commands::add(
                &layout,
                project,
                &common,
                title.as_deref().unwrap_or(""),
                body.as_deref(),
                status,
                priority,
                after,
                &attach,
                labels,
                *new_label,
            )
        }

        Command::List { status, all, ready } => {
            let project = project(&layout, cli.project.as_deref())?;
            let statuses: Vec<Status> = match status {
                Some(value) => vec![crate::cli::parse_status(value)?],
                // Bare `list` shows the active lanes; `--all` is everything
                // including backlog, cancelled and archived (UNI-62).
                None if *all => Status::ALL.to_vec(),
                None => vec![
                    Status::Todo,
                    Status::InProgress,
                    Status::Blocked,
                    Status::InReview,
                    Status::Done,
                ],
            };
            commands::list(&layout, project, gate, &statuses, *ready)
        }

        Command::Show { id } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::show(&layout, project, id, gate)
        }

        Command::Move {
            id,
            status,
            comment,
            attach,
        } => {
            let project = project(&layout, cli.project.as_deref())?;
            let to = crate::cli::parse_status(status)?;
            commands::move_task_with(
                &layout,
                project,
                &common,
                id,
                to,
                comment.as_deref(),
                attach,
            )
        }

        Command::Note { id, text, attach } => {
            let text = read_body(Some(text), exec)?.unwrap_or_default();
            let project = project(&layout, cli.project.as_deref())?;
            commands::note_with(&layout, project, &common, id, &text, attach)
        }

        Command::Attach {
            id,
            file,
            note,
            name,
        } => {
            let project = project(&layout, cli.project.as_deref())?;
            let bytes = std::fs::read(file)
                .map_err(|err| Error::usage(format!("cannot read {}: {err}", file.display())))?;
            let original = name.clone().unwrap_or_else(|| {
                file.file_name()
                    .map_or_else(|| "file".into(), |n| n.to_string_lossy().to_string())
            });
            commands::attach(
                &layout,
                project,
                &common,
                id,
                &original,
                &bytes,
                note.as_deref(),
            )
        }

        Command::Attachments { id } => {
            let project = project(&layout, cli.project.as_deref())?;
            let board = crate::board::Board::open(&layout, project)?;
            let task = board.get(id)?;
            Ok(serde_json::json!(crate::attachments::list(
                &layout,
                &board.project.slug,
                &task.id
            )))
        }

        Command::Edit {
            id,
            title,
            body,
            priority,
            labels,
            new_label,
        } => {
            let project = project(&layout, cli.project.as_deref())?;
            let body = read_body(body.as_deref(), exec)?;
            let priority = priority
                .as_deref()
                .map(crate::cli::parse_priority)
                .transpose()?;
            let labels = labels.as_ref().map(|raw| {
                raw.split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect::<Vec<String>>()
            });
            commands::edit(
                &layout,
                project,
                &common,
                id,
                EditArgs {
                    title: title.as_deref(),
                    body: body.as_deref(),
                    priority,
                    labels,
                    new_label: *new_label,
                },
            )
        }

        Command::Link { id, after } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::link(&layout, project, &common, gate, id, after, false)
        }

        Command::Unlink { id, after } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::link(&layout, project, &common, gate, id, after, true)
        }

        Command::Order {
            id,
            before,
            after_pos,
            top,
            bottom,
        } => {
            let project = project(&layout, cli.project.as_deref())?;
            let target = match (before.as_deref(), after_pos.as_deref(), top, bottom) {
                (Some(target), None, false, false) => OrderTarget::Before(target),
                (None, Some(target), false, false) => OrderTarget::AfterPos(target),
                (None, None, true, false) => OrderTarget::Top,
                (None, None, false, true) => OrderTarget::Bottom,
                _ => {
                    return Err(Error::usage(
                        "order needs exactly one of --before ID, --after-pos ID, --top, --bottom",
                    ));
                }
            };
            commands::order(&layout, project, &common, id, target)
        }

        Command::Start { id, pid } => {
            let project = project(&layout, cli.project.as_deref())?;
            let session = require_session(&common, "start")?;
            let pid = owner_pid(*pid, exec)?;
            let host = commands::hostname();
            let args = commands::StartArgs {
                session: &session,
                pid,
                host: &host,
                max_sessions: exec
                    .var_string(commands::SESSION_CAP_ENV)
                    .and_then(|value| value.parse().ok())
                    .filter(|cap| *cap > 0)
                    .unwrap_or(commands::SESSION_CAP_DEFAULT),
            };
            commands::start(&layout, project, gate, id, &args, common.now)
        }

        Command::Finish {
            id,
            comment,
            attach,
        } => {
            let project = project(&layout, cli.project.as_deref())?;
            let session = require_session(&common, "finish")?;
            commands::finish(
                &layout, project, gate, id, &session, comment, attach, common.now,
            )
        }

        Command::Next => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::next(&layout, project, gate, common.now)
        }

        Command::Reap { dry_run } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::reap(&layout, project, *dry_run, common.now)
        }

        Command::Chain { id } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::chain(&layout, project, gate, id)
        }

        Command::Search { text: needle, all } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::search(&layout, project, gate, needle, *all)
        }

        Command::Release { id, to, comment } => {
            let project = project(&layout, cli.project.as_deref())?;
            let to = crate::cli::parse_status(to)?;
            commands::release(&layout, project, id, to, comment, gate, common.now)
        }

        Command::Duplicate { id } => {
            let project = project(&layout, cli.project.as_deref())?;
            commands::duplicate(&layout, project, &common, id)
        }

        Command::ArchiveSweep {
            after_days,
            retention_days,
        } => {
            let project = project(&layout, cli.project.as_deref())?;
            let days = after_days
                .or_else(|| {
                    exec.var_string("OINO_KANBOARD_ARCHIVE_AFTER_DAYS")
                        .and_then(|value| value.parse().ok())
                })
                .unwrap_or(0);
            let retention = retention_days
                .or_else(|| {
                    exec.var_string("OINO_KANBOARD_RETENTION_DAYS")
                        .and_then(|value| value.parse().ok())
                })
                .unwrap_or(0);
            commands::archive_sweep(&layout, project, days, retention, common.now)
        }

        // `status`: in-process health check (round 5, #7) — there is no
        // daemon to ask; `ok` means the files are readable and writable.
        Command::Status => commands::board_status(&layout),

        Command::Validate { fix } => {
            let project = project(&layout, cli.project.as_deref())?;
            let result = commands::validate(&layout, project, *fix)?;
            Ok(json!({
                "ok": result.problems.is_empty(),
                "problems": result.problems.iter().map(|problem| json!({
                    "file": problem.file,
                    "line": problem.line,
                    "message": problem.message,
                    "fixable": problem.fixable,
                })).collect::<Vec<_>>(),
                "fixed": result.fixed,
            }))
        }
    }
}

/// The `--attach-existing` paths that name existing files (`~/` → `$HOME`).
fn existing_files(paths: &[String]) -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    paths
        .iter()
        .filter_map(|raw| match (raw.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => Some(home.join(rest)),
            (Some(_), None) => None,
            (None, _) => Some(std::path::PathBuf::from(raw)),
        })
        .filter(|path| path.is_file())
        .collect()
}

/// The board home `OINO_KANBOARD_HOME`/`OINO_AGENT_DIR` resolves to — under
/// `exec`'s environment, so in-process hosts see the request's home.
fn layout_in(exec: &Exec<'_>) -> Result<Layout> {
    if let Some(home) = exec.var_string(store::HOME_ENV).filter(|h| !h.is_empty()) {
        return Ok(Layout::with_home(PathBuf::from(home)));
    }
    if let Some(agent) = exec
        .var_string("OINO_AGENT_DIR")
        .filter(|agent| !agent.is_empty())
    {
        let base = crate::sys::tilde_home(&agent).ok_or_else(|| {
            Error::usage(format!("OINO_AGENT_DIR `{agent}` needs $HOME to expand"))
        })?;
        return Ok(Layout::with_home(base.join("state").join("unipi.kanboard")));
    }
    Layout::from_env()
}

/// `--actor`, else `$OINO_KANBOARD_ACTOR` (under `exec`), else `user`.
fn resolve_actor_in(explicit: Option<&str>, exec: &Exec<'_>) -> Actor {
    if let Some(value) = explicit
        && let Ok(actor) = value.trim().parse()
    {
        return actor;
    }
    if let Some(value) = exec.var_string(store::ACTOR_ENV)
        && let Ok(actor) = value.parse()
    {
        return actor;
    }
    Actor::User
}

/// The pid that owns a `start` claim: `--pid`, else `$OINO_KANBOARD_PID`
/// (the pi extension exports its own pid), else `exec.owner_pid` — the
/// caller's parent for the CLI, the host process for in-process calls.
#[allow(clippy::unnecessary_wraps, reason = "kept uniform with older fallible shape")]
fn owner_pid(explicit: Option<u32>, exec: &Exec<'_>) -> Result<u32> {
    if let Some(pid) = explicit.filter(|pid| *pid > 0) {
        return Ok(pid);
    }
    if let Some(pid) = exec
        .var_string("OINO_KANBOARD_PID")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
    {
        return Ok(pid);
    }
    Ok(exec.owner_pid)
}

/// Session-scoped commands fail loudly without one.
fn require_session(common: &Common, command: &str) -> Result<String> {
    common.session.clone().ok_or_else(|| {
        Error::usage(format!(
            "{command} needs a session: pass --session or set OINO_KANBOARD_SESSION"
        ))
    })
}

/// A `-` body/text value reads `exec.stdin` (in-process) or real stdin (CLI).
fn read_body(value: Option<&str>, exec: &Exec<'_>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some("-") => {
            if let Some(text) = exec.stdin {
                return Ok(Some(text.to_string()));
            }
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|err| Error::Io(format!("cannot read stdin: {err}")))?;
            Ok(Some(buffer))
        }
        Some(text) => Ok(Some(text.to_string())),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

fn text(value: &Value, key: &str) -> String {
    match field(value, key) {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The title a human sees: the derived `displayTitle` when the raw title is
/// empty (older payloads without the key fall back to `title`).
fn display_title(value: &Value) -> String {
    let derived = text(value, "displayTitle");
    if derived.is_empty() {
        text(value, "title")
    } else {
        derived
    }
}

fn strings(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn ids(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| text(item, "id"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// One-line (or few-line) human summary of a command's payload.
fn indent_block(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    text.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// First non-empty body line, ≤100 chars, indented — the `show` excerpt.
fn body_excerpt(task: &Value) -> String {
    let body = task.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let line = body
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.is_empty() {
        return String::new();
    }
    let trimmed: String = line.chars().take(100).collect();
    format!("\n    {trimmed}")
}

/// Human-readable rendering of a command's JSON payload.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one arm per subcommand: the human renderer mirrors the CLI table"
)]
pub fn human(cli: &Cli, payload: &Value) -> String {
    match &cli.command {
        Command::Project(ProjectCommand::Add(args)) => format!(
            "project {} registered (root {}{})",
            text(payload, "slug"),
            text(payload, "root"),
            args.name
                .as_deref()
                .map(|name| format!(", name {name}"))
                .unwrap_or_default()
        ),
        Command::Project(ProjectCommand::List) => {
            let items = payload.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return "no projects registered".to_string();
            }
            items
                .iter()
                .map(|project| format!("{}  {}", text(project, "slug"), text(project, "root")))
                .collect::<Vec<_>>()
                .join("\n")
        }
        Command::Project(ProjectCommand::Archive { slug }) => format!("project {slug} archived"),
        Command::Project(ProjectCommand::Unarchive { slug }) => {
            format!("project {slug} unarchived")
        }
        Command::Project(ProjectCommand::Rebind { slug, root }) => {
            format!("project {slug} rebound to {}", root.display())
        }
        Command::Project(ProjectCommand::Show) => {
            let counts = field(payload, "counts");
            let lanes = Status::ALL
                .iter()
                .map(|status| format!("{}={}", status.as_str(), field(counts, status.as_str())))
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "{} ({} tasks)\n  {lanes}",
                text(field(payload, "project"), "slug"),
                field(payload, "total")
            )
        }

        Command::Add { .. } => format!(
            "{} created in {} ({})",
            text(payload, "id"),
            text(payload, "status"),
            display_title(payload)
        ),
        Command::List { .. } => {
            // K4 turned the JSON payload into `{tasks, problems}`; this table kept
            // reading a bare array and printed "no tasks" on every board that had
            // tasks (found on coffee: `list --json` returned 18, `list` said none).
            let items = payload
                .get("tasks")
                .and_then(|value| value.as_array())
                .cloned()
                .or_else(|| payload.as_array().cloned())
                .unwrap_or_default();
            let problems = payload
                .get("problems")
                .and_then(|value| value.as_array())
                .map_or(0, Vec::len);
            if items.is_empty() && problems == 0 {
                return "no tasks".to_string();
            }
            let mut lines = items
                .iter()
                .map(|task| {
                    let ready = field(task, "ready").as_bool().unwrap_or(false);
                    let waiting = ids(field(task, "waitingFor"));
                    format!(
                        "{}  [{}] {}{}{}{}",
                        text(task, "id"),
                        text(task, "status"),
                        display_title(task),
                        if field(task, "priority") == &json!("none") {
                            String::new()
                        } else {
                            format!(" ({})", text(task, "priority"))
                        },
                        if !ready && !waiting.is_empty() {
                            format!("  waiting on {waiting}")
                        } else {
                            String::new()
                        },
                        body_excerpt(task)
                    )
                })
                .collect::<Vec<_>>();
            if items.is_empty() {
                lines.push("no tasks".to_string());
            }
            if problems > 0 {
                lines.push(format!(
                    "⚠ {problems} task file(s) need repair — oino-kanboard validate --fix"
                ));
            }
            lines.join("\n")
        }
        Command::Show { .. } => {
            let status = text(payload, "status");
            let deps = ids(field(payload, "deps"));
            let mut out = format!(
                "{} [{}] {}\n  priority: {}  order: {}  created: {}{}",
                text(payload, "id"),
                status,
                display_title(payload),
                text(payload, "priority"),
                text(payload, "order"),
                text(payload, "created"),
                if deps.is_empty() {
                    String::new()
                } else {
                    format!("  deps: {deps}")
                }
            );
            let body = payload
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if !body.is_empty() {
                let _ = write!(out, "\n{}\n", indent_block(body, 2));
            }
            if let Some(run) = payload.get("run").filter(|value| !value.is_null()) {
                let _ = write!(
                    out,
                    "\n  run: session {} pid {} on {} ({})",
                    text(run, "session"),
                    text(run, "pid"),
                    text(run, "host"),
                    text(payload, "staleness")
                );
            }
            let activity = payload
                .get("activity")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for entry in activity
                .iter()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .iter()
                .rev()
            {
                let _ = write!(
                    out,
                    "\n  - {} [{}] {}",
                    text(entry, "at"),
                    text(entry, "actor"),
                    text(entry, "text")
                );
            }
            out
        }

        Command::Move { id, .. } => format!("{id} → {}", text(payload, "status")),
        Command::Start { id, .. } => format!("{id} started (in_progress, claimed by this session)"),
        Command::Finish { id, .. } => format!("{id} finished → in_review"),
        Command::Note { id, .. } => format!("{id}: note added"),
        Command::Attach { id, .. } => format!(
            "{id}: attached {}\n  {}",
            text(field(payload, "attachment"), "path"),
            text(field(payload, "attachment"), "markdown")
        ),
        Command::Attachments { id } => {
            let items = payload.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                format!("{id}: no attachments")
            } else {
                items
                    .iter()
                    .map(|item| {
                        format!(
                            "{}  {}  ({} bytes)",
                            text(item, "kind"),
                            text(item, "path"),
                            field(item, "size")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        Command::Edit { id, .. } => format!("{id}: updated"),
        Command::Link { id, after } => format!("{id}: now depends on {after}"),
        Command::Unlink { id, after } => format!("{id}: no longer depends on {after}"),
        Command::Order { id, .. } => format!(
            "{id}: order {} (rebalanced {})",
            text(payload, "order"),
            field(payload, "rebalanced")
        ),
        Command::Next => {
            let task = field(payload, "task");
            if task.is_null() {
                let waiting = field(payload, "waiting")
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|item| {
                        format!(
                            "{} (waiting {})",
                            text(item, "id"),
                            ids(field(item, "waitingFor"))
                        )
                    })
                    .collect::<Vec<_>>();
                if waiting.is_empty() {
                    "no ready task".to_string()
                } else {
                    format!("no ready task; todo but waiting: {}", waiting.join(", "))
                }
            } else {
                format!(
                    "next: {} [{}] {}",
                    text(task, "id"),
                    text(task, "priority"),
                    display_title(task)
                )
            }
        }
        Command::Reap { dry_run } => {
            let released = strings(field(payload, "released"));
            let unknown = strings(field(payload, "unknown"));
            let mut out = if released.is_empty() {
                if *dry_run {
                    "reap (dry-run): nothing to release".to_string()
                } else {
                    "reap: nothing to release".to_string()
                }
            } else {
                format!(
                    "{}{released}",
                    if *dry_run {
                        "would release: "
                    } else {
                        "released to todo: "
                    }
                )
            };
            if !unknown.is_empty() {
                let _ = write!(out, "\n  claimed on other hosts (untouched): {unknown}");
            }
            out
        }
        Command::Settings { .. } => {
            if field(payload, "maxSessions").is_null() {
                let argv = field(payload, "piCommand")
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let joined: Vec<String> = argv
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                format!("pi-command set to {:?}", joined.join(" "))
            } else {
                let argv = field(payload, "piCommand")
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let joined: Vec<String> = argv
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                format!(
                    "pi-command: {}\nmodels: {} reported\nsummary model: {}\ncustom summary instruction: {}\nmaxSessions: {}",
                    if joined.is_empty() {
                        "(not set)".to_string()
                    } else {
                        joined.join(" ")
                    },
                    field(payload, "models").as_array().map_or(0, Vec::len),
                    {
                        let m = text(payload, "summaryModel");
                        if m.is_empty() {
                            "pi default".to_string()
                        } else {
                            m
                        }
                    },
                    field(payload, "summaryInstructionSet"),
                    text(payload, "maxSessions")
                )
            }
        }
        Command::RotateToken => format!("rotate-token: {}", text(payload, "note")),
        Command::Chain { id } => {
            let upstream = field(payload, "upstream")
                .as_array()
                .cloned()
                .unwrap_or_default();
            let downstream = field(payload, "downstream")
                .as_array()
                .cloned()
                .unwrap_or_default();
            let row = |item: &Value| {
                format!(
                    "{} [{}] {}",
                    text(item, "id"),
                    text(item, "status"),
                    display_title(item)
                )
            };
            let mut out = format!(
                "{id} [{}] {}",
                text(payload, "status"),
                display_title(payload)
            );
            if !upstream.is_empty() {
                out.push_str("\n  upstream:");
                for item in &upstream {
                    let _ = write!(out, "\n    {}", row(item));
                }
            }
            if !downstream.is_empty() {
                out.push_str("\n  downstream:");
                for item in &downstream {
                    let _ = write!(out, "\n    {}", row(item));
                }
            }
            out
        }
        Command::Search { .. } => {
            let items = field(payload, "tasks")
                .as_array()
                .cloned()
                .unwrap_or_default();
            if items.is_empty() {
                return "no matches".to_string();
            }
            items
                .iter()
                .map(|task| {
                    format!(
                        "{}  [{}] {}{}",
                        text(task, "id"),
                        text(task, "status"),
                        display_title(task),
                        body_excerpt(task)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Command::Release { id, to, .. } => format!("{id} released to {to}"),
        Command::Duplicate { .. } => format!("{} created (duplicate)", text(payload, "id")),
        Command::ArchiveSweep { .. } => {
            let archived = ids(field(payload, "archived"));
            let cold = ids(field(payload, "cold"));
            let mut line = if archived.is_empty() {
                format!(
                    "nothing archived{}",
                    payload
                        .get("skipped")
                        .and_then(|value| value.as_str())
                        .map(|reason| format!(" ({reason})"))
                        .unwrap_or_default()
                )
            } else {
                format!("archived: {archived}")
            };
            if !cold.is_empty() {
                let _ = write!(line, " · cold: {cold}");
            }
            line
        }
        Command::Serve { .. } => {
            if field(payload, "alreadyRunning").as_bool().unwrap_or(false) {
                let daemon = field(payload, "daemon");
                let mut line = format!(
                    "kanboard already running on {}:{} (pid {})",
                    text(daemon, "host"),
                    text(daemon, "port"),
                    text(daemon, "pid")
                );
                if field(payload, "bindingChanged").as_bool().unwrap_or(false) {
                    let _ = write!(
                        line,
                        " — requested {}:{}; stop it first (oino-kanboard stop) to rebind",
                        text(field(payload, "requested"), "host"),
                        text(field(payload, "requested"), "port")
                    );
                }
                line
            } else if field(payload, "stopped").as_bool().unwrap_or(false) {
                let daemon = field(payload, "daemon");
                format!(
                    "kanboard stopped (was pid {} on port {})",
                    text(daemon, "pid"),
                    text(daemon, "port")
                )
            } else {
                let daemon = field(payload, "daemon");
                format!(
                    "kanboard listening on {}:{} (pid {}){}",
                    text(daemon, "host"),
                    text(daemon, "port"),
                    text(daemon, "pid"),
                    if field(daemon, "token").is_null() {
                        String::new()
                    } else {
                        " · remote access requires the token from daemon.json".to_string()
                    }
                )
            }
        }
        Command::Status => {
            let daemon = field(payload, "daemon");
            if daemon.is_null() {
                "no daemon recorded".to_string()
            } else {
                format!(
                    "pid {} · port {} · version {} · started {} · alive={}",
                    text(daemon, "pid"),
                    text(daemon, "port"),
                    text(daemon, "version"),
                    text(daemon, "startedAt"),
                    field(payload, "alive")
                )
            }
        }
        Command::Stop { .. } => {
            if field(payload, "stopped").as_bool().unwrap_or(false) {
                format!("stopped pid {}", text(payload, "pid"))
            } else {
                format!(
                    "not stopped: {}",
                    payload
                        .get("reason")
                        .and_then(|reason| reason.as_str())
                        .unwrap_or("unknown")
                )
            }
        }
        Command::Validate { .. } => {
            let problems = field(payload, "problems")
                .as_array()
                .cloned()
                .unwrap_or_default();
            let fixed = ids(field(payload, "fixed"));
            let mut out = if problems.is_empty() {
                "validate: ok".to_string()
            } else {
                problems
                    .iter()
                    .map(|problem| {
                        format!(
                            "{}:{}: {}",
                            text(problem, "file"),
                            text(problem, "line"),
                            text(problem, "message")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if !fixed.is_empty() {
                let _ = write!(out, "\nfixed formatting: {fixed}");
            }
            out
        }
    }
}

/// Exit code: rule violations and validation findings are failures.
#[must_use]
pub fn exit_code(cli: &Cli, payload: &Value) -> i32 {
    if let Command::Validate { .. } = cli.command
        && field(payload, "problems")
            .as_array()
            .is_some_and(|problems| !problems.is_empty())
    {
        return 1;
    }
    0
}

/// Resolve the project without touching the board (used by `main` for context).
pub fn project_for(layout: &Layout, cli: &Cli) -> Result<store::Project> {
    store::resolve_project(layout, cli.project.as_deref())
}

/// Re-exported for `main`/daemon use.
pub fn layout() -> Result<Layout> {
    Layout::from_env()
}

/// Kept public so K2 can build the same `Value` payloads for the JSON API.
pub fn dispatch_value(cli: &Cli) -> Result<Value> {
    dispatch(cli)
}

/// Helper for the daemon: the JSON of a task list is the API payload.
pub fn list_payload(cli: &Cli) -> Result<Value> {
    dispatch(cli)
}

/// Unused-import guard for `Priority` (kept for API symmetry in K2).
#[allow(dead_code)]
fn _priority_marker(_: Priority) {}

/// Convenience for tests and the daemon.
pub fn project_counts(layout: &Layout, project: &store::Project) -> Result<Value> {
    let board = crate::board::Board::open(layout, project.clone())?;
    let tasks = board.tasks()?;
    let mut counts = serde_json::Map::new();
    for status in Status::ALL {
        counts.insert(
            status.as_str().to_string(),
            json!(tasks.iter().filter(|task| task.status == status).count()),
        );
    }
    Ok(Value::Object(counts))
}
