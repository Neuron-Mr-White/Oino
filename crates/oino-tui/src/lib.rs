//! `oino-tui`: the interactive terminal client over
//! [`oino_client::SessionBackend`].
//!
//! The binary constructs the backend (in-process or remote) and a [`TuiHost`] and calls
//! [`run`]. Everything visible is driven by [`App`], a pure state machine over key presses,
//! client events and command replies; [`render::draw`] paints it with ratatui. Extension UI
//! (dialogs, widgets, status, frames, panels, OUS specs) is rendered natively from the
//! frontend-agnostic protocol (ui-protocol.md).
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`app`] | [`App`], [`AppEvent`], [`Effect`], [`TuiOptions`] |
//! | [`render`] | [`render::draw`] |
//! | [`host`] | [`TuiHost`], [`host::LocalHost`] |
//! | [`input`] | crossterm → [`oino_view::keys::KeyId`] |
//! | [`theme`] | [`theme::Theme`] |
//! | `runner` | `TUI.start` | terminal lifecycle, event loop, effect execution |

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

pub mod app;
pub mod host;
pub mod input;
pub mod render;
mod runner;
pub mod theme;

pub use app::{App, AppEvent, Effect, Tag, TuiMode, TuiOptions};
pub use host::TuiHost;
pub use runner::run;

/// Failure of the interactive client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TuiError {
    /// Terminal I/O failed.
    #[error("terminal: {0}")]
    Io(#[from] std::io::Error),
}
