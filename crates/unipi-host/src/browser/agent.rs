//! "Agent is driving" + Take over (UNI-296): what agents — CDP clients
//! other than the app's own viewers — do with one profile's browser, and
//! the user's take-over that pauses them.
//!
//! The app's viewers drive the browser through the host's own CDP
//! connection ([`super::cdp::Cdp`]), so they never pass through here. Agents
//! connect to the **gated endpoint** ([`super::gate`], printed by
//! `unipi-host browser start`) or to a relay in front of it (UNI-293); every
//! command they send is checked with [`AgentGate::check`].
//!
//! ## The API for relays (small on purpose)
//!
//! One [`AgentGate`] per browser profile, from
//! [`super::Manager::gate`]`(profile)` (an `Arc`, it outlives relaunches):
//!
//! - [`AgentGate::client`] — call once per agent connection; keep the
//!   returned [`AgentClient`] alive while it is connected (drop = gone).
//! - [`AgentGate::check`]`(method)` — call for every agent → browser
//!   command, before forwarding it. `Ok(())` = forward it (and, for
//!   input/navigation, it counts as "driving"). `Err(message)` = the user
//!   has taken over: do NOT forward; answer the agent with a CDP error
//!   `{"id":…,"error":{"code":-32000,"message":message}}`.
//! - [`AgentGate::note_input`] — a relay that synthesises its own input
//!   (e.g. a humanised mouse path for one agent click) may report it here;
//!   `check` already counts the agent's own command.
//!
//! The app side: [`AgentGate::set_takeover`] (`browser_takeover`),
//! [`AgentGate::note_user_input`] (the user's own input keeps a take-over
//! alive), [`AgentGate::snapshot`] (`agent` in status/open data and the
//! `browser_event{agent}` the viewers get).
//!
//! A take-over ends on Give back, or by itself after [`TAKEOVER_IDLE`]
//! without the user's own input.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use unipi_proto::BrowserAgent;

/// Input within this long counts as "driving".
pub const DRIVING_WINDOW: Duration = Duration::from_secs(3);
/// A take-over with no input from the user for this long gives the browser back.
pub const TAKEOVER_IDLE: Duration = Duration::from_secs(10 * 60);
/// What an agent's refused command answers (a CDP error message).
pub const TAKEN_OVER_MESSAGE: &str = "The user has taken over the browser; wait or ask them";
/// The CDP error code of a refused command (JSON-RPC "server error").
pub const TAKEN_OVER_CODE: i64 = -32000;

/// Does `method` drive the page (input or navigation)? Only these are
/// refused while taken over; reads (`DOM.*`, `Runtime.*`, screenshots,
/// `Target.*`) keep working, so an agent can still look and wait.
pub fn is_driving(method: &str) -> bool {
    method.starts_with("Input.")
        || matches!(method, "Page.navigate" | "Page.reload" | "Page.navigateToHistoryEntry" | "Page.stopLoading")
}

#[derive(Debug, Default)]
struct State {
    last_input: Option<Instant>,
    /// When the user took over (None = not taken over).
    takeover: Option<Instant>,
    last_user_input: Option<Instant>,
}

/// Agent activity + take-over of one profile's browser. Cheap to share.
#[derive(Debug, Default)]
pub struct AgentGate {
    state: Mutex<State>,
    clients: AtomicUsize,
    /// Bumped on every change a viewer should hear about at once.
    changed: tokio::sync::watch::Sender<u64>,
}

/// One connected agent; counts as `connected` until dropped.
#[derive(Debug)]
pub struct AgentClient(Arc<AgentGate>);

impl Drop for AgentClient {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::SeqCst);
        self.0.bump();
    }
}

impl AgentGate {
    pub fn new() -> Arc<AgentGate> {
        Arc::new(AgentGate::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Wakes when something changed (connect, take-over, first input).
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// An agent connected (keep the guard while it is).
    pub fn client(self: &Arc<Self>) -> AgentClient {
        self.clients.fetch_add(1, Ordering::SeqCst);
        self.bump();
        AgentClient(self.clone())
    }

    /// Agents connected through the gate (or a relay) now.
    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::SeqCst)
    }

    /// An agent's command: `Err(message)` = refuse it (taken over).
    pub fn check(&self, method: &str) -> Result<(), &'static str> {
        self.check_at(method, Instant::now())
    }

    pub fn check_at(&self, method: &str, now: Instant) -> Result<(), &'static str> {
        if !is_driving(method) {
            return Ok(());
        }
        let mut state = self.lock();
        expire(&mut state, now);
        if state.takeover.is_some() {
            return Err(TAKEN_OVER_MESSAGE);
        }
        let was = driving(&state, now);
        state.last_input = Some(now);
        drop(state);
        if !was {
            self.bump();
        }
        Ok(())
    }

    /// Agent input a relay generated itself (already allowed by `check`).
    pub fn note_input(&self) {
        let now = Instant::now();
        let mut state = self.lock();
        let was = driving(&state, now);
        state.last_input = Some(now);
        drop(state);
        if !was {
            self.bump();
        }
    }

    /// The user's own input in the app (keeps a take-over from expiring).
    pub fn note_user_input(&self) {
        self.lock().last_user_input = Some(Instant::now());
    }

    /// Take over (`true`) or give back (`false`).
    pub fn set_takeover(&self, on: bool) -> BrowserAgent {
        self.set_takeover_at(on, Instant::now())
    }

    pub fn set_takeover_at(&self, on: bool, now: Instant) -> BrowserAgent {
        let mut state = self.lock();
        state.takeover = on.then_some(now);
        if on {
            // The agent's last move is no longer "driving" once you hold it.
            state.last_input = None;
        }
        drop(state);
        self.bump();
        self.snapshot_at(now, 0)
    }

    pub fn taken_over(&self) -> bool {
        let mut state = self.lock();
        expire(&mut state, Instant::now());
        state.takeover.is_some()
    }

    /// The wire view. `direct` = agents seen on the raw DevTools port
    /// (outside the gate; they count as connected but can't be paused).
    pub fn snapshot(&self, direct: usize) -> BrowserAgent {
        self.snapshot_at(Instant::now(), direct)
    }

    pub fn snapshot_at(&self, now: Instant, direct: usize) -> BrowserAgent {
        let mut state = self.lock();
        if expire(&mut state, now) {
            drop(state);
            self.bump();
            state = self.lock();
        }
        BrowserAgent {
            connected: self.clients() > 0 || direct > 0,
            driving: driving(&state, now),
            taken_over: state.takeover.is_some(),
        }
    }
}

fn driving(state: &State, now: Instant) -> bool {
    state.last_input.is_some_and(|t| now.saturating_duration_since(t) < DRIVING_WINDOW)
}

/// Ends a take-over idle for [`TAKEOVER_IDLE`]; true when it just ended.
fn expire(state: &mut State, now: Instant) -> bool {
    let Some(since) = state.takeover else { return false };
    let active = state.last_user_input.map_or(since, |t| t.max(since));
    if now.saturating_duration_since(active) >= TAKEOVER_IDLE {
        state.takeover = None;
        tracing::info!(op = "browser", "take-over ended after 10 min idle: the agent can drive again");
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_input_and_navigation_are_driving() {
        for m in ["Input.dispatchMouseEvent", "Input.insertText", "Input.dispatchKeyEvent", "Page.navigate", "Page.reload", "Page.navigateToHistoryEntry"] {
            assert!(is_driving(m), "{m}");
        }
        for m in ["Runtime.evaluate", "DOM.getDocument", "Page.captureScreenshot", "Target.getTargets", "Page.enable", "Accessibility.getFullAXTree"] {
            assert!(!is_driving(m), "{m}");
        }
    }

    #[test]
    fn input_marks_driving_for_three_seconds() {
        let gate = AgentGate::new();
        let t0 = Instant::now();
        let _agent = gate.client();
        assert_eq!(gate.snapshot_at(t0, 0), BrowserAgent { connected: true, driving: false, taken_over: false });
        // Reads don't drive.
        gate.check_at("DOM.getDocument", t0).unwrap();
        assert!(!gate.snapshot_at(t0, 0).driving);
        gate.check_at("Input.dispatchMouseEvent", t0).unwrap();
        assert!(gate.snapshot_at(t0 + Duration::from_millis(2900), 0).driving);
        assert!(!gate.snapshot_at(t0 + Duration::from_millis(3000), 0).driving);
    }

    #[test]
    fn clients_count_while_connected() {
        let gate = AgentGate::new();
        assert!(!gate.snapshot(0).connected);
        assert!(gate.snapshot(1).connected, "a direct DevTools client counts too");
        let a = gate.client();
        let b = gate.client();
        assert_eq!(gate.clients(), 2);
        drop(a);
        assert!(gate.snapshot(0).connected);
        drop(b);
        assert!(!gate.snapshot(0).connected);
    }

    #[test]
    fn takeover_refuses_driving_but_not_reads_and_give_back_clears_it() {
        let gate = AgentGate::new();
        let t0 = Instant::now();
        gate.check_at("Input.dispatchKeyEvent", t0).unwrap();
        let s = gate.set_takeover_at(true, t0);
        assert!(s.taken_over && !s.driving, "taking over stops showing the agent as driving");
        assert_eq!(gate.check_at("Input.dispatchMouseEvent", t0), Err(TAKEN_OVER_MESSAGE));
        assert_eq!(gate.check_at("Page.navigate", t0), Err(TAKEN_OVER_MESSAGE));
        assert_eq!(gate.check_at("Runtime.evaluate", t0), Ok(()));
        assert!(!gate.snapshot_at(t0, 0).driving, "refused input is not driving");
        let s = gate.set_takeover_at(false, t0);
        assert!(!s.taken_over);
        assert_eq!(gate.check_at("Input.dispatchMouseEvent", t0), Ok(()));
    }

    #[test]
    fn takeover_ends_after_ten_idle_minutes_and_user_input_extends_it() {
        let gate = AgentGate::new();
        let t0 = Instant::now();
        gate.set_takeover_at(true, t0);
        assert!(gate.check_at("Input.insertText", t0 + Duration::from_secs(9 * 60)).is_err());
        // The user's own input (now) moves the idle clock.
        gate.note_user_input();
        let later = Instant::now() + Duration::from_secs(9 * 60);
        assert!(gate.check_at("Input.insertText", later).is_err());
        assert!(gate.snapshot_at(later, 0).taken_over);
        let expired = Instant::now() + TAKEOVER_IDLE;
        assert!(!gate.snapshot_at(expired, 0).taken_over);
        assert_eq!(gate.check_at("Input.insertText", expired), Ok(()));
    }

    #[test]
    fn changes_wake_subscribers() {
        let gate = AgentGate::new();
        let mut rx = gate.subscribe();
        let before = *rx.borrow_and_update();
        let _c = gate.client();
        assert!(rx.has_changed().unwrap());
        assert_ne!(*rx.borrow_and_update(), before);
        gate.set_takeover(true);
        assert!(rx.has_changed().unwrap());
    }
}
