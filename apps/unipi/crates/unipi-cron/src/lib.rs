//! The `UniPi` cron scheduler: scheduled, unattended headless Oino runs (v2 ext-abi §6).
//!
//! A **job** holds a prompt, a working directory, optional model/thinking overrides and a
//! schedule (a 6-field cron expression with seconds first, or `at:<RFC3339>` for a
//! one-time run, optionally written in plain language — "every day at 9:00"). The
//! **scheduler** ([`scheduler::Scheduler`]) ticks once per second while a single owner
//! process runs, fires due jobs as `oino -p --mode json` child processes with their own
//! session files, and records every **run** with status, summary, cost, session path and
//! a capped output log.
//!
//! **Who runs it (v2):** unipi-host embeds the scheduler as the single owner
//! ([`embedded::Embedded`], the `scheduler.lock` flock) and registers
//! [`host_service::CronHostService`] as the `"cron"` host service for the extension's
//! live ops. Under a plain `oino` TUI nothing fires; job files can still be created and
//! edited (the `unipi.cron` extension writes them through its fs preopens) — its
//! `run`/`stop`/`status` surface then reports "no scheduler running".
//!
//! Storage is plain JSON files under `<agent dir>/cron/` (`~/.oino/cron/` by default):
//! one file per job, written atomically, so the extension, the `unipi-host cron` CLI and
//! the host all read and write the same thing. The owner watches `jobs/` and rescans on
//! change, so edits land without a restart.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`paths`] | [`paths::CronPaths`]: every file the scheduler owns, atomic writes |
//! | [`model`] | [`model::Job`], [`model::Run`], [`model::CronSettings`], policies |
//! | [`schedule`] | re-export of [`unipi_cron_schedule`]: schedule parsing/evaluation |
//! | [`store`] | one JSON file per job + migration from `jobs.json`, runs, logs, settings |
//! | [`scheduler`] | the ticking owner: due fires, overlap, concurrency, run lifecycle |
//! | [`embedded`] | [`embedded::Embedded`]: what unipi-host embeds (owner lock, tick loop, `jobs/` watcher) |
//! | [`control`] | [`control::CronControl`]/[`control::handle_control`]: every operation, with or without a live scheduler |
//! | [`exec`] | running one fire as a headless `oino -p` child process |
//! | [`host_service`] | [`host_service::CronHostService`]: the `"cron"` host service (WP-X1 trait) |
//! | [`notify`] | per-job run notifications (ntfy / gotify / telegram) |

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

pub mod control;
pub mod embedded;
pub mod error;
pub mod exec;
pub mod host_service;
pub mod model;
pub mod notify;
pub mod paths;
pub mod scheduler;
pub mod store;

pub use unipi_cron_schedule as schedule;

pub use error::CronError;
pub use model::{Job, Run, RunStatus};
pub use paths::CronPaths;
pub use scheduler::Scheduler;
