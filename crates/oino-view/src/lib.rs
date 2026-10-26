//! `oino-view`: frontend-agnostic view models shared by the terminal client (`oino-tui`) and
//! the GPUI apps (modes.md §7.1).
//!
//! Nothing here knows about a UI toolkit. Every model is a plain state machine fed with
//! [`oino_core::events::ClientEvent`]s, RPC replies or user intents, and is unit-tested
//! without a terminal.
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`transcript`] | `interactive-mode.ts` `handleEvent`, `renderSessionItems` | [`transcript::Transcript`] rows keyed by tool-call id; reducer over session events |
//! | [`ui_state`] | `rpc-extension-ui`, footer/widgets/status | [`ui_state::UiState`]: statuses, widgets, frames, panels, dialog queue |
//! | [`ous`] | — (ui-protocol §6) | [`ous::OusNode`]: typed Oino UI Spec v1 nodes |
//! | [`tree`] | `tree-selector.ts` | [`tree::TreeModel`]: flattening, filter modes, folding, search |
//! | [`sessions`] | `session-selector.ts` | [`sessions::SessionListModel`]: scope, sort, name filter, search |
//! | [`settings_hub`] | — (ui-protocol §6.6) | extension settings hub: namespaces, flattened fields, display, cycling, parsing |
//! | [`composer`] | pi-tui `Editor` | [`composer::Composer`]: multi-line buffer, history, kill ring, undo, paste markers |
//! | [`commands`] | `slash-commands.ts` | built-in slash command table and parsing |
//! | [`keys`] | `keybindings.ts` | [`keys::KeyId`], [`keys::Keybindings`] with pi defaults |
//! | [`fuzzy`] | pi-tui `fuzzyFilter` | subsequence matcher |
//! | [`mod@format`] | `footer.ts` helpers | token, cwd, age formatting |
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
