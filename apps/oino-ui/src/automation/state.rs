//! App-level state for the driver: the screen, the visible session, pending dialogs, and the
//! focus handles that elements registered during prepaint (docs/dev/v1/gui-driver.md §2).

use std::cell::RefCell;

use gpui_kit::{App, FocusHandle, Global, Window};
use serde_json::{Value, json};

/// What `state` reports about the app (filled by the shell).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The main-area screen: `sessions`, `settings`, `board`, `cron`, `extensions`,
    /// `onboarding`.
    pub route: String,
    /// The visible session's id.
    pub session: Option<String>,
    /// The visible session's display name.
    pub session_name: Option<String>,
    /// Queued dialogs, front first.
    pub dialogs: Vec<DialogInfo>,
    /// Open (live) sessions.
    pub panes: usize,
    /// The visible session is running a turn.
    pub busy: bool,
    /// Extra surfaces that are open (palette, switch sheet, side pane, bottom frame, panels).
    pub open: Vec<String>,
    /// The extension chrome the shell shows: statuses, above/below-composer widgets and the
    /// frame segments (footer / glance). Scenario assertions read it as `state.chrome`.
    pub chrome: Value,
}

/// One pending dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogInfo {
    /// Request id.
    pub id: String,
    /// Extension UI method (`approval`, `form`, `custom`, …).
    pub method: String,
    /// Title.
    pub title: String,
}

impl State {
    /// The `state` JSON shape.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "route": self.route,
            "session_id": self.session,
            "session_name": self.session_name,
            "dialogs": self.dialogs.iter().map(|d| json!({
                "id": d.id,
                "method": d.method,
                "title": d.title,
            })).collect::<Vec<_>>(),
            "panes": self.panes,
            "busy": self.busy,
            "open": self.open,
            "chrome": self.chrome,
        })
    }
}

/// A focus handle registered by an element during prepaint.
#[derive(Clone)]
struct Registered {
    path: String,
    handle: FocusHandle,
}

thread_local! {
    /// Handles registered by this thread's prepaint pass (moved into [`FocusHandles`] once the
    /// frame's render starts, where the app is at hand).
    static PENDING: RefCell<Vec<Registered>> = const { RefCell::new(Vec::new()) };
}

/// The focus handles of the last finished frame, keyed by global element id path.
#[derive(Default)]
pub struct FocusHandles(Vec<Registered>);

impl Global for FocusHandles {}

/// Records a focus handle seen during prepaint (UI thread only).
pub(crate) fn stash_focus(path: &str, handle: FocusHandle) {
    PENDING.with(|pending| {
        pending.borrow_mut().push(Registered {
            path: path.to_owned(),
            handle,
        });
    });
}

/// Publishes the handles stashed by the last prepaint pass to the app.
pub(crate) fn publish(cx: &mut App) {
    let handles = PENDING.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    cx.set_global(FocusHandles(handles));
}

/// Moves keyboard focus to the element at `path`; `false` when it registered no handle.
pub fn focus_path(window: &mut Window, cx: &mut App, path: &str) -> bool {
    let Some(handles) = cx.try_global::<FocusHandles>() else {
        return false;
    };
    let Some(entry) = handles.0.iter().find(|r| r.path == path) else {
        return false;
    };
    let handle = entry.handle.clone();
    window.focus(&handle, cx);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_json_carries_the_dialogs() {
        let state = State {
            route: "sessions".into(),
            session: Some("abc".into()),
            session_name: Some("Fix the parser".into()),
            dialogs: vec![DialogInfo {
                id: "demo-ui-1".into(),
                method: "approval".into(),
                title: "Allow bash?".into(),
            }],
            panes: 2,
            busy: true,
            open: vec!["side".into()],
            chrome: json!({"statuses": [{"key": "unipi.workflow/mode", "text": "Plan"}]}),
        };
        let json = state.to_json();
        assert_eq!(json["route"], json!("sessions"));
        assert_eq!(json["session_id"], json!("abc"));
        assert_eq!(json["dialogs"][0]["method"], json!("approval"));
        assert_eq!(json["panes"], json!(2));
        assert_eq!(json["chrome"]["statuses"][0]["text"], json!("Plan"));
        assert_eq!(json["open"][0], json!("side"));
    }
}
