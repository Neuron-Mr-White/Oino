//! `oino-ui`: shared GPUI views and view models for Oino Desktop (and, later, Oino Mobile).
//!
//! The crate has two layers (gpui-app.md §3):
//!
//! - [`model`] — toolkit-free view models fed by [`oino_core::events::ClientEvent`]s: the
//!   transcript (rows keyed by message/tool-call id, streaming patches only the tail), the
//!   extension UI state (status, frame, widgets, panels, dialog queue), OUS v1 nodes, the
//!   session list and composer intents. They are plain state machines with unit tests and build
//!   without GPUI (`--no-default-features`).
//! - `views` (feature `views`, default) — the ZCode-inspired GPUI shell built on gpui-kit:
//!   sidebar, conversation (virtualised `list`), tool rows with inline diffs, composer, model
//!   picker, command palette, dialogs/approvals, OUS renderer, side pane, settings.
//!
//! The app shell (`oino-desktop`) supplies a [`host::SessionHost`] (session list, open/create
//! sessions as [`oino_client::SessionBackend`]s, extensions) and owns the tokio runtime that
//! `oino-core` runs on; views never block on core.
//!
//! # API map
//!
//! | Item | Contents |
//! |---|---|
//! | [`model::transcript`] | [`model::transcript::Transcript`], rows, minimal [`model::transcript::RowChange`]s |
//! | [`model::ui_state`] | [`model::ui_state::UiState`] reducer over `extension_ui_request` + effects |
//! | [`model::dialog`] | typed extension dialogs and their `extension_ui_response` answers |
//! | [`model::ous`] | OUS v1 node tree (ui-protocol §6) |
//! | [`model::session`] | [`model::session::SessionModel`]: transcript + UI + run state per open session |
//! | [`model::sessions`] | sidebar session list grouped by workspace |
//! | [`model::composer`] | submit intents (prompt / steer / follow-up), history, drafts |
//! | [`model::coalesce`] | per-frame event coalescing |
//! | [`host`] | [`host::SessionHost`] contract implemented by the app shell |
//! | `rt` (feature `views`) | tokio ↔ GPUI bridge (`Tokio::spawn` → `gpui::Task`) |
//! | `views` (feature `views`) | GPUI views |
//! | `mobile` (feature `mobile`) | phone shell: stacked screens, bottom sheets, safe areas |

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::assert_is_empty,
        clippy::needless_pass_by_value,
        reason = "tests assert by unwrapping"
    )
)]

pub mod host;
#[cfg(feature = "mobile")]
pub mod mobile;
pub mod model;
#[cfg(feature = "views")]
pub mod rt;
#[cfg(feature = "views")]
pub mod views;
