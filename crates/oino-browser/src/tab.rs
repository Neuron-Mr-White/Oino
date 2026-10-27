//! Logical tabs (browser-control.md §6.2–6.3): state, log rings, viewers, residency order.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use oino_browser_protocol::{
    BrowserViewEvent, ControlledBy, TabInfo, TabLifecycle, TabOwner, ToolEvent, Viewport,
};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::snapshot::RefMap;

/// Ring buffer sizes (§5.2 `browser_logs`).
pub(crate) const CONSOLE_CAP: usize = 500;
/// Errors kept.
pub(crate) const ERRORS_CAP: usize = 200;
/// Network requests kept.
pub(crate) const NETWORK_CAP: usize = 300;

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One log line.
#[derive(Clone, Debug)]
pub(crate) struct LogLine {
    pub(crate) seq: u64,
    pub(crate) text: String,
}

/// One network request.
#[derive(Clone, Debug, Default)]
pub(crate) struct NetEntry {
    pub(crate) seq: u64,
    pub(crate) id: String,
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) status: Option<i64>,
    pub(crate) kind: String,
    pub(crate) size: Option<i64>,
    pub(crate) started: Option<f64>,
    pub(crate) ms: Option<f64>,
    pub(crate) failed: Option<String>,
}

/// Open JS dialog.
#[derive(Clone, Debug)]
pub(crate) struct DialogInfo {
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) default_prompt: Option<String>,
}

/// An OOPIF / worker session attached under this tab.
#[derive(Clone, Debug)]
pub(crate) struct ChildSession {
    pub(crate) session: String,
    pub(crate) target_id: String,
    pub(crate) parent_session: String,
    pub(crate) kind: String,
}

/// One viewer of a tab.
#[derive(Debug)]
pub(crate) struct View {
    pub(crate) id: u64,
    pub(crate) tx: mpsc::Sender<BrowserViewEvent>,
    pub(crate) unacked: u32,
    pub(crate) last_seq: u64,
}

/// Mutable tab state.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent CDP-derived flags"
)]
#[derive(Debug)]
pub(crate) struct TabState {
    pub(crate) owner: TabOwner,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) loading: bool,
    pub(crate) lifecycle: TabLifecycle,
    pub(crate) controlled_by: ControlledBy,
    pub(crate) opened_at: Instant,
    pub(crate) last_activity: Instant,
    pub(crate) last_selected: Option<Instant>,
    pub(crate) last_agent_cmd: Option<Instant>,
    pub(crate) last_user_input: Option<Instant>,
    pub(crate) driving: bool,
    pub(crate) refs: RefMap,
    pub(crate) snapshot_seq: u64,
    pub(crate) dialog: Option<DialogInfo>,
    pub(crate) console: VecDeque<LogLine>,
    pub(crate) errors: VecDeque<LogLine>,
    pub(crate) network: VecDeque<NetEntry>,
    pub(crate) inflight: HashMap<String, Instant>,
    pub(crate) last_network: Instant,
    pub(crate) log_seq: u64,
    pub(crate) collecting: Option<Vec<ToolEvent>>,
    pub(crate) children: Vec<ChildSession>,
    pub(crate) history: (bool, bool),
    pub(crate) viewport: Viewport,
    pub(crate) fit: bool,
    pub(crate) handoff: Option<(String, oneshot::Sender<bool>)>,
    pub(crate) file_chooser: Option<oneshot::Sender<i64>>,
    pub(crate) closing: bool,
    pub(crate) crash_reload_at: Option<Instant>,
    pub(crate) downloads_active: u32,
    pub(crate) screencast: bool,
    pub(crate) pointer: (f64, f64),
}

/// A logical tab bound to a CDP page target.
#[derive(Debug)]
pub(crate) struct Tab {
    pub(crate) id: String,
    pub(crate) profile: String,
    pub(crate) target_id: String,
    pub(crate) session: String,
    pub(crate) state: Mutex<TabState>,
    pub(crate) changed: Notify,
    pub(crate) cmd: tokio::sync::Mutex<()>,
    pub(crate) views: Mutex<Vec<View>>,
}

impl Tab {
    /// New tab state.
    pub(crate) fn new(
        id: String,
        profile: String,
        target_id: String,
        session: String,
        owner: TabOwner,
    ) -> Arc<Self> {
        let now = Instant::now();
        let lifecycle = if owner.is_agent() {
            TabLifecycle::Active
        } else {
            TabLifecycle::User
        };
        Arc::new(Self {
            id,
            profile,
            target_id,
            session,
            state: Mutex::new(TabState {
                owner,
                url: "about:blank".into(),
                title: String::new(),
                loading: false,
                lifecycle,
                controlled_by: ControlledBy::Agent,
                opened_at: now,
                last_activity: now,
                last_selected: None,
                last_agent_cmd: None,
                last_user_input: None,
                driving: false,
                refs: RefMap::default(),
                snapshot_seq: 0,
                dialog: None,
                console: VecDeque::new(),
                errors: VecDeque::new(),
                network: VecDeque::new(),
                inflight: HashMap::new(),
                last_network: now,
                log_seq: 0,
                collecting: None,
                children: Vec::new(),
                history: (false, false),
                viewport: Viewport::default(),
                fit: false,
                handoff: None,
                file_chooser: None,
                closing: false,
                crash_reload_at: None,
                downloads_active: 0,
                screencast: false,
                pointer: (640.0, 400.0),
            }),
            changed: Notify::new(),
            cmd: tokio::sync::Mutex::new(()),
            views: Mutex::new(Vec::new()),
        })
    }

    /// Public description.
    pub(crate) fn info(&self) -> TabInfo {
        let s = lock(&self.state);
        TabInfo {
            id: self.id.clone(),
            url: s.url.clone(),
            title: s.title.clone(),
            owner: s.owner.clone(),
            lifecycle: s.lifecycle,
            controlled_by: s.controlled_by,
            loading: s.loading,
            can_back: s.history.0,
            can_forward: s.history.1,
            profile: self.profile.clone(),
            driving: s.driving,
        }
    }

    /// Records a caused event while a command collects.
    pub(crate) fn note(&self, e: ToolEvent) {
        if let Some(c) = lock(&self.state).collecting.as_mut() {
            c.push(e);
        }
    }

    /// Appends to a log ring.
    pub(crate) fn log(&self, ring: Ring, text: String) {
        let mut s = lock(&self.state);
        s.log_seq += 1;
        let line = LogLine {
            seq: s.log_seq,
            text,
        };
        let (buf, cap) = match ring {
            Ring::Console => (&mut s.console, CONSOLE_CAP),
            Ring::Errors => (&mut s.errors, ERRORS_CAP),
        };
        buf.push_back(line);
        while buf.len() > cap {
            buf.pop_front();
        }
    }

    /// Whether this CDP session belongs to the tab (page or child).
    pub(crate) fn owns_session(&self, session: &str) -> bool {
        self.session == session
            || lock(&self.state)
                .children
                .iter()
                .any(|c| c.session == session)
    }

    /// Sends to every viewer (drops slow ones' frames, removes closed ones).
    pub(crate) fn broadcast_view(&self, e: &BrowserViewEvent) {
        lock(&self.views).retain(|v| match v.tx.try_send(e.clone()) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => true,
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
    }
}

/// Log ring selector.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Ring {
    /// Console.
    Console,
    /// Errors.
    Errors,
}

/// Residency candidate facts (pure ordering, §6.3).
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) id: String,
    pub(crate) protected: bool,
    pub(crate) last_activity: Instant,
    pub(crate) last_selected: Option<Instant>,
    pub(crate) opened_at: Instant,
    pub(crate) seq: u64,
}

impl Candidate {
    /// Facts of `tab`.
    pub(crate) fn of(tab: &Tab) -> Self {
        let visible = lock(&tab.views).len();
        let s = lock(&tab.state);
        let protected = visible > 0
            || s.driving
            || s.loading
            || s.downloads_active > 0
            || s.controlled_by == ControlledBy::User
            || matches!(s.lifecycle, TabLifecycle::Handoff)
            || s.dialog.is_some();
        Self {
            id: tab.id.clone(),
            protected,
            last_activity: s.last_activity,
            last_selected: s.last_selected,
            opened_at: s.opened_at,
            seq: tab.id.trim_start_matches('t').parse().unwrap_or(0),
        }
    }
}

/// The tab to close when the profile is over `limit` (oldest safe candidate), if any.
pub(crate) fn residency_victim(cands: &[Candidate], limit: usize) -> Option<String> {
    if cands.len() < limit {
        return None;
    }
    cands
        .iter()
        .filter(|c| !c.protected)
        .min_by(|a, b| {
            a.last_activity
                .cmp(&b.last_activity)
                .then(a.last_selected.cmp(&b.last_selected))
                .then(a.opened_at.cmp(&b.opened_at))
                .then(a.seq.cmp(&b.seq))
        })
        .map(|c| c.id.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn residency_picks_oldest_safe() {
        let t0 = Instant::now();
        let c = |id: &str, act: u64, protected: bool| Candidate {
            id: id.into(),
            protected,
            last_activity: t0 + Duration::from_secs(act),
            last_selected: None,
            opened_at: t0,
            seq: id[1..].parse().unwrap(),
        };
        let cands = vec![c("t1", 5, true), c("t2", 9, false), c("t3", 7, false)];
        assert_eq!(residency_victim(&cands, 4), None);
        assert_eq!(residency_victim(&cands, 3).as_deref(), Some("t3"));
        let all = vec![c("t1", 5, true), c("t2", 9, true)];
        assert_eq!(residency_victim(&all, 2), None);
    }
}
