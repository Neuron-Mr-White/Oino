//! Shared helpers for the kanboard tests.
//!
//! Each integration test binary uses a subset of these helpers, so dead-code
//! analysis across binaries would otherwise flag the rest.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    missing_docs,
    unreachable_pub,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::format_push_string,
    clippy::used_underscore_binding,
    clippy::needless_continue,
    reason = "integration tests: helpers outside #[test] fns may panic on fixture failures"
)]
#![allow(dead_code)]

use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;
use unipi_kanboard::commands::{self, Common};
use unipi_kanboard::model::{Actor, ChainGate, Priority, Status, Task};
use unipi_kanboard::store::{Layout, Project};

// In-process tests read `UNIPI_KANBOARD_MAX_SESSIONS` & co. from the process
// environment (`commands::max_sessions()`); run them from a shell that does not
// export the `UNIPI_KANBOARD_*` tuning vars (an Oino session sets them only for
// the agent's own bash calls). Child-process helpers override them explicitly.

pub(crate) struct Fixture {
    pub _home: TempDir,
    pub _root: TempDir,
    pub layout: Layout,
    pub project: Project,
    pub common: Common,
}

impl Fixture {
    pub(crate) fn new() -> Fixture {
        let home = TempDir::new().expect("home");
        let root = TempDir::new().expect("root");
        let layout = Layout::with_home(home.path());
        let project =
            Project::create(&layout, root.path(), Some("Fixture"), Some("FIX")).expect("project");
        Fixture {
            _home: home,
            _root: root,
            layout,
            project,
            common: Common::new(Actor::User, ChainGate::InReview),
        }
    }

    pub(crate) fn root(&self) -> PathBuf {
        self._root.path().to_path_buf()
    }

    pub(crate) fn add(&self, title: &str) -> Task {
        self.add_with(title, Status::Backlog, Priority::None, &[])
    }

    pub(crate) fn add_with(
        &self,
        title: &str,
        status: Status,
        priority: Priority,
        after: &[String],
    ) -> Task {
        self.add_labeled(title, status, priority, after, &[])
    }

    /// `add` with labels — the shape the UNI-60 create path uses.
    pub(crate) fn add_labeled(
        &self,
        title: &str,
        status: Status,
        priority: Priority,
        after: &[String],
        labels: &[String],
    ) -> Task {
        let value = commands::add(
            &self.layout,
            self.project.clone(),
            &self.common,
            title,
            Some("body"),
            Some(status),
            priority,
            after,
            &[],
            labels,
            true,
        )
        .expect("add");
        task_from(&value)
    }

    pub(crate) fn tasks(&self) -> Vec<Task> {
        let board =
            unipi_kanboard::board::Board::open(&self.layout, self.project.clone()).expect("board");
        board.tasks().expect("tasks")
    }

    pub(crate) fn list_json(&self) -> Value {
        self.list_statuses(&Status::ALL)
    }

    /// `list` over an explicit set of lanes (the UNI-62 CLI default is a
    /// subset of these).
    pub(crate) fn list_statuses(&self, statuses: &[Status]) -> Value {
        commands::list(
            &self.layout,
            self.project.clone(),
            ChainGate::InReview,
            statuses,
            false,
        )
        .expect("list")
    }

    pub(crate) fn move_to(&self, id: &str, to: Status) -> Value {
        commands::move_task(
            &self.layout,
            self.project.clone(),
            &self.common,
            id,
            to,
            None,
        )
        .expect("move")
    }

    /// `start <ID>` as the agent of `session` (pid owns the claim) — the way
    /// a task gets into `in_progress` now.
    pub(crate) fn start(&self, id: &str, session: &str, pid: u32) -> Value {
        self.try_start(id, session, pid).expect("start")
    }

    pub(crate) fn try_start(
        &self,
        id: &str,
        session: &str,
        pid: u32,
    ) -> unipi_kanboard::error::Result<Value> {
        let host = commands::hostname();
        commands::start(
            &self.layout,
            self.project.clone(),
            ChainGate::InReview,
            id,
            &commands::StartArgs {
                session,
                pid,
                host: &host,
                max_sessions: commands::max_sessions(),
            },
            self.common.now,
        )
    }
}

pub(crate) fn task_from(value: &Value) -> Task {
    serde_json::from_value(value.clone()).expect("task json")
}

pub(crate) fn id_of(value: &Value) -> String {
    value
        .get("id")
        .and_then(|id| id.as_str())
        .expect("id in payload")
        .to_string()
}

pub(crate) fn write_task_file(fixture: &Fixture, name: &str, contents: &str) -> PathBuf {
    let dir = fixture.layout.tasks_dir(&fixture.project.slug);
    std::fs::create_dir_all(&dir).expect("tasks dir");
    let path = dir.join(format!("{name}.md"));
    std::fs::write(&path, contents).expect("write task");
    path
}

pub(crate) const VALID_TASK: &str = "---
id: FIX-900
title: Hand written
status: todo
priority: none
order: 1000
deps: []
labels: []
created: 2026-09-24T10:00:00Z
updated: 2026-09-24T10:00:00Z
run:
---

Body text.

## Activity
- 2026-09-24T10:00:00Z [user] created
";

/// A pid that is definitely alive on this host — the test process itself.
/// Literal pids like 1/4242 are init/system on unix but absent on Windows,
/// where a "live claimer" would silently read as stale.
pub(crate) fn alive_pid() -> u32 {
    std::process::id()
}

/// The compiled CLI binary.
pub(crate) fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_unipi-kanboard")
}

/// Run the CLI against this fixture's home.
/// Every `UNIPI_KANBOARD_*` the binary reads that a test might need to be
/// absent by default (actor/session/pid/limits) — cleared on every spawn
/// before the test's own `.env(...)` calls, so an ambient value exported by
/// the *caller's* shell (e.g. a wrapping agent session with its own
/// `UNIPI_KANBOARD_SESSION`/`UNIPI_KANBOARD_MAX_SESSIONS` set for its own
/// board) never leaks into a child process the test spawns. `HOME` and
/// `PROJECT` are not here: every test sets them explicitly right after.
pub(crate) const AMBIENT_KANBOARD_ENV: &[&str] = &[
    "UNIPI_KANBOARD_ACTOR",
    "UNIPI_KANBOARD_SESSION",
    "UNIPI_KANBOARD_PID",
    "UNIPI_KANBOARD_MAX_SESSIONS",
    "UNIPI_KANBOARD_ARCHIVE_AFTER_DAYS",
    "UNIPI_KANBOARD_RETENTION_DAYS",
    "UNIPI_KANBOARD_CHAIN_GATE",
];

pub(crate) fn cli(fixture: &Fixture, args: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(bin());
    command.args(args);
    for var in AMBIENT_KANBOARD_ENV {
        command.env_remove(var);
    }
    command
        .env("HOME", fixture.layout.home.as_os_str())
        .env("UNIPI_KANBOARD_HOME", fixture.layout.home.as_os_str())
        .env("UNIPI_KANBOARD_PROJECT", &fixture.project.slug)
        .current_dir(fixture.root())
        .output()
        .expect("run unipi-kanboard")
}

/// Like `cli` but with extra environment overrides (limits, session) — those
/// apply after the ambient-env clear, so a test that wants a specific value
/// still gets exactly that value regardless of what the parent shell has.
pub(crate) fn cli_with_env(
    fixture: &Fixture,
    args: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    let mut command = std::process::Command::new(bin());
    command.args(args);
    for var in AMBIENT_KANBOARD_ENV {
        command.env_remove(var);
    }
    command
        .env("HOME", fixture.layout.home.as_os_str())
        .env("UNIPI_KANBOARD_HOME", fixture.layout.home.as_os_str())
        .env("UNIPI_KANBOARD_PROJECT", &fixture.project.slug)
        .current_dir(fixture.root());
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("run unipi-kanboard")
}
