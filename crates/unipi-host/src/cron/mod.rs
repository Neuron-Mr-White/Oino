//! Host-owned cron scheduler (UNI-314, Cron C1).
//!
//! `unipi-host serve` runs every enabled job's `pi -p "<prompt>"` on its
//! own schedule — no pi window is needed; the scheduler is always on
//! while the host is. Storage, rules and the control ops are documented
//! in `docs/m7/CRON.md`.
//!
//! Modules:
//! - [`model`]: `Job`, `Run`, `CronSettings` (the `cron` block of
//!   `config.json`).
//! - [`schedule`]: parses/evaluates `schedule` (6-field cron, seconds
//!   first, or `at:<RFC3339>`) + `tz`.
//! - [`store`]: atomic JSON storage under `<host dir>/cron/`.
//! - [`exec`]: runs one fire as a child `pi` process.
//! - [`scheduler`]: the owner — ticks, decides fires (overlap/missed/
//!   concurrency/pause), and the control ops for the CLI.

pub mod exec;
pub mod model;
pub mod notify;
pub mod scheduler;
pub mod schedule;
pub mod store;
pub mod wake;

pub use model::{CronSettings, Job, Run, RunStatus};
pub use scheduler::{CronControl, CronEvent, Scheduler};
