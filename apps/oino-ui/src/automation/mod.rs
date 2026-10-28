//! GUI automation layer: a deterministic driver for Oino Desktop (docs/dev/v1/gui-driver.md).
//!
//! Compiled in by default, inert until the app runs with `OINO_DESKTOP_AUTOMATION=<socket>`.
//! Then the app serves a JSON-lines RPC on that Unix socket ([`protocol`], [`server`]) and
//! answers it on the UI thread ([`driver`]):
//!
//! * **Read** — [`registry`] collects every interactive element's id, role, label, state and
//!   bounds during prepaint, which GPUI already runs each frame. Components wrap their element
//!   with [`probe`]; while automation is off the wrapper is one relaxed atomic load.
//! * **Write** — pointer and keyboard events are handed to `Window::dispatch_event`, the same
//!   entry point the platform backends use, so hit testing, focus transfer, click synthesis,
//!   keymap dispatch and IME behave exactly as they do for a person at the machine.
//!
//! This replaces the two open harness gaps of `STATUS.md` §1b.7 on Xvfb: pointer-only surfaces
//! (board FAB, `+` launcher, settings switches, browser take-over, mascot toggle) are now
//! clickable, and the semantic tree replaces OCR for QA assertions.

pub mod driver;
pub mod input;
pub mod protocol;
pub mod registry;
#[cfg(unix)]
pub mod server;
pub mod state;

pub use driver::install;
pub use registry::{Automatable, Meta, Node, Role, begin_frame, end_frame, end_marker, probe};
pub use state::State;
