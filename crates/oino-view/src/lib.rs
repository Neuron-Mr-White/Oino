//! `oino-view`: frontend-agnostic view models shared by the terminal client (`oino-tui`) and
//! the GPUI apps (modes.md §7.1).
//!
//! Nothing here knows about a UI toolkit. Every model is a plain state machine fed with
//! [`oino_core::events::ClientEvent`]s, RPC replies or user intents, and is unit-tested
//! without a terminal.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`transcript`] | [`transcript::Transcript`] rows keyed by tool-call id; reducer over session events |
//! | [`ui_state`] | [`ui_state::RegistryMirror`]: v2 snapshot/patch mirror (ui-protocol §6, §9.2) with the dialog queue; [`ui_state::UiState`]: the transitional v1 request reducer |
//! | [`ous`] | [`ous::OusNode`]: typed Oino UI Spec v1 nodes (deleted with the last v1 emitter, WP-U7) |
//! | [`tree`] | [`tree::TreeModel`]: flattening, filter modes, folding, search |
//! | [`sessions`] | [`sessions::SessionListModel`]: scope, sort, name filter, search |
//! | [`settings_hub`] | extension settings hub: namespaces, flattened fields, display, cycling, parsing, `visibleIf`/`disabledIf` |
//! | [`composer`] | [`composer::Composer`]: multi-line buffer, history, kill ring, undo, paste markers |
//! | [`commands`] | built-in slash command table and parsing |
//! | [`keys`] | [`keys::KeyId`], [`keys::Keybindings`] with defaults |
//! | [`fuzzy`] | subsequence matcher |
//! | [`mod@format`] | token, cwd, age formatting |
//!
//! Wire types come from `oino-core`/`oino-session`/`oino-ai` until `oino-protocol` exists
//! (deviations/tui-client.md).

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

pub mod commands;
pub mod composer;
pub mod format;
pub mod fuzzy;
pub mod keys;
pub mod ous;
pub mod sessions;
pub mod settings_hub;
pub mod transcript;
pub mod tree;
pub mod ui_state;

/// JSON value type used throughout (re-exported from `oino-ai`).
pub use oino_ai::{JsonObject, JsonValue};
