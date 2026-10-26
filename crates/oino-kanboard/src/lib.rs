//! Oino kanboard — task storage, transition rules, dependency graph, CLI and the board
//! daemon (JSON API + SSE; no web UI). The binary `oino-kanboard` is the only writer of
//! board files; the `unipi.kanboard` extension and the daemon call the same library, so the
//! panel and the terminal enforce exactly the same rules.
//!
//! | Module | Boundary |
//! |---|---|
//! | [`model`], [`format`](mod@format) | statuses, priorities, actors, runs; task file parser/renderer |
//! | [`transitions`], [`deps`], [`order`] | transition table, dependency readiness, lane order |
//! | [`store`], [`board`], [`attachments`] | home layout (`<agent dir>/state/unipi.kanboard`), locks, atomic writes |
//! | [`commands`] | one operation per subcommand (JSON payloads) |
//! | [`cli`], [`run`] | clap definitions, dispatcher and human rendering |
//! | [`daemon`], [`serve`] | `daemon.json`, single instance, the axum JSON API + SSE |
//! | [`error`] | `Error` (`usage` exit 2; `rule`, `not_found`, `io` exit 1) |

#![allow(
    clippy::missing_errors_doc,
    reason = "every fallible fn returns crate::Error; its variants (usage, rule, not_found, io) are documented once on Error"
)]

pub mod attachments;
pub mod board;
pub mod cli;
pub mod commands;
pub mod daemon;
pub mod deps;
pub mod error;
pub mod format;
pub mod model;
pub mod order;
pub mod run;
pub mod serve;
pub mod store;
pub mod transitions;

pub use error::{Error, Problem, Result};
pub use model::{Actor, ChainGate, Priority, RunMode, Status, Task};
pub use store::{Layout, Project};
