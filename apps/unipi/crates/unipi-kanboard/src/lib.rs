//! Oino kanboard — task storage, transition rules, dependency graph and CLI (round 5:
//! in-process; the daemon is gone). Every write goes through this library's validated
//! commands under a per-project file lock, so the `unipi.kanboard` extension (via
//! [`service::exec`]/`kanboard.exec`), the CLI binary and the desktop board all enforce
//! exactly the same rules.
//!
//! | Module | Boundary |
//! |---|---|
//! | [`model`], [`format`](mod@format) | statuses, priorities, actors, runs; task file parser/renderer |
//! | [`transitions`], [`deps`], [`order`] | transition table, dependency readiness, lane order |
//! | [`store`], [`board`], [`attachments`] | home layout (`<agent dir>/state/unipi.kanboard`), locks, atomic writes |
//! | [`commands`] | one operation per subcommand (JSON payloads) |
//! | [`cli`], [`run`] | clap definitions, dispatcher and human rendering |
//! | [`service`], [`sys`] | in-process `exec` (argv/env/cwd/stdin → `--json` payload), file watch, pid/agent-dir helpers |
//! | [`error`] | `Error` (`usage` exit 2; `rule`, `not_found`, `io` exit 1) |

#![allow(
    clippy::missing_errors_doc,
    reason = "every fallible fn returns crate::Error; its variants (usage, rule, not_found, io) are documented once on Error"
)]

pub mod attachments;
pub mod board;
pub mod cli;
pub mod commands;
pub mod deps;
pub mod error;
pub mod format;
pub mod model;
pub mod order;
pub mod run;
pub mod service;
pub mod store;
pub mod sys;
pub mod transitions;

pub use error::{Error, Problem, Result};
pub use model::{Actor, ChainGate, Priority, RunMode, Status, Task};
pub use store::{Layout, Project};

pub mod dispatcher;
