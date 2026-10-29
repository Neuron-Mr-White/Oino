//! The board tab marker (round-5 #7): `/kanboard open` sends an `openPanel`
//! with this id — a marker only. The pane renders `unipi.kanboard/board`
//! itself from the in-process kanboard service, so there is no OUS spec, no
//! keys list and no SSE-driven republish.

use serde_json::{Value, json};

/// Panel id, scoped by extension like the other unipi panels (frontends find
/// the board by it).
pub const PANEL_ID: &str = "unipi.kanboard/board";

/// The `openPanel` request — the marker the side pane turns into the native
/// board tab (`focus`, `closeOnEscape` carry over; `spec` stays empty).
#[must_use]
pub fn open_request() -> Value {
    json!({
        "panelId": PANEL_ID,
        "placement": "sidePane",
        "title": "Board",
        "spec": {"t": "empty"},
        "focus": true,
        "closeOnEscape": true,
    })
}
