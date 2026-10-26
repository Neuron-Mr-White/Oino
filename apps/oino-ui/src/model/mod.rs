//! Toolkit-free view models (gpui-app.md §3.2). Every model is a state machine over
//! client events or user intents and is unit-tested without GPUI.

pub mod coalesce;
pub mod composer;
pub mod dialog;
pub mod interaction;
pub mod kanban;
pub mod ous;
pub mod palette;
pub mod session;
pub mod sessions;
pub mod status;
pub mod timeline;
pub mod tools;
pub mod transcript;
pub mod ui_state;

/// JSON value type used throughout (re-exported from `oino-ai`).
pub use oino_ai::{JsonObject, JsonValue};
