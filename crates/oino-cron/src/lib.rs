//! The Oino cron scheduler: scheduled, unattended headless Oino runs.
//!
//! A **job** holds a prompt, a working directory, optional model/thinking overrides and a
//! schedule (a 6-field cron expression with seconds first, or `at:<RFC3339>` for a one-time
//! run, optionally written in plain language — "every day at 9:00"). The **scheduler**
//! ([`scheduler::Scheduler`]) ticks once per second while a single owner process runs
//! (`oino cron serve`, the installed user service, or the desktop app), fires due jobs as
//! `oino -p --mode json` child processes with their own session files, and records every
//! **run** with status, summary, cost, session path and a capped output log.
//!
//! Storage is plain JSON under `<agent dir>/cron/` (`~/.oino/cron/` by default), written
//! atomically, so the CLI, the daemon and the desktop can all read it; a file lock makes
//! one process the ticking owner ([`serve`]) and a loopback TCP socket
//! ([`client::ControlClient`]) carries control ops to it.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`paths`] | [`paths::CronPaths`]: every file the scheduler owns, atomic writes |
//! | [`model`] | [`model::Job`], [`model::Run`], [`model::CronSettings`], policies |
//! | [`schedule`] | schedule parsing/evaluation (cron + `at:` + plain language, tz/DST) |
//! | [`store`] | atomic JSON storage: jobs, runs, logs, settings, retention |
//! | [`scheduler`] | the ticking owner: due fires, overlap, concurrency, run lifecycle |
//! | [`control`] | [`control::CronControl`]: every operation, handled with or without a live scheduler |
//! | [`exec`] | running one fire as a headless `oino -p` child process |
//! | [`notify`] | per-job run notifications (ntfy / gotify / telegram) |
//! | [`client`] | the daemon's TCP control client ([`client::ControlClient`]) |
//! | [`serve`] | the always-on daemon: owner lock, tick loop, control socket |
//! | [`install`] | `oino cron install`/`uninstall`: systemd --user, launchd, Task Scheduler |

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

pub mod client;
pub mod control;
pub mod error;
pub mod exec;
pub mod install;
pub mod model;
pub mod notify;
pub mod paths;
pub mod schedule;
pub mod scheduler;
pub mod serve;
pub mod store;

pub use error::CronError;
pub use model::{Job, Run, RunStatus};
pub use paths::CronPaths;
pub use scheduler::Scheduler;
