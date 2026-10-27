//! Process-wide bookkeeping shared by the service and every profile: session routing of viewer
//! events, logical id counters, storage paths.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use oino_browser_protocol::BrowserViewEvent;
use tokio::sync::broadcast;

use crate::tab::lock;

/// Per-session browser state.
#[derive(Debug)]
pub(crate) struct SessionRt {
    /// Profile chosen for this session (`None`: settings default).
    pub(crate) profile: Option<String>,
    /// The agent's current tab.
    pub(crate) current: Option<String>,
    /// Viewer events of this session.
    pub(crate) tx: broadcast::Sender<BrowserViewEvent>,
}

/// Storage layout (§12).
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    /// `~/.oino/browser`.
    pub(crate) root: PathBuf,
}

impl Paths {
    pub(crate) fn profile(&self, name: &str) -> PathBuf {
        self.root.join("profiles").join(name)
    }
    pub(crate) fn shots(&self, session: &str) -> PathBuf {
        self.root.join("shots").join(sanitize(session))
    }
    pub(crate) fn downloads(&self, base: Option<&PathBuf>, session: &str) -> PathBuf {
        base.cloned()
            .unwrap_or_else(|| self.root.join("downloads"))
            .join(sanitize(session))
    }
    pub(crate) fn site_permissions(&self) -> PathBuf {
        self.root.join("site-permissions.json")
    }
}

/// File-name-safe version of `s`.
pub(crate) fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let t = t.trim_matches(['.', ' ']).to_owned();
    if t.is_empty() { "file".into() } else { t }
}

/// Shared hub.
#[derive(Debug)]
pub(crate) struct Hub {
    pub(crate) paths: Paths,
    next_tab: AtomicU64,
    next_view: AtomicU64,
    pub(crate) sessions: Mutex<HashMap<String, SessionRt>>,
    /// Events of tabs without a session.
    pub(crate) global: broadcast::Sender<BrowserViewEvent>,
}

impl Hub {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            paths: Paths { root },
            next_tab: AtomicU64::new(1),
            next_view: AtomicU64::new(1),
            sessions: Mutex::new(HashMap::new()),
            global: broadcast::channel(256).0,
        }
    }

    pub(crate) fn tab_id(&self) -> String {
        format!("t{}", self.next_tab.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn view_id(&self) -> u64 {
        self.next_view.fetch_add(1, Ordering::Relaxed)
    }

    /// Ensures a session entry; returns its sender.
    pub(crate) fn session_tx(
        &self,
        session: &str,
        _workspace: &str,
    ) -> broadcast::Sender<BrowserViewEvent> {
        lock(&self.sessions)
            .entry(session.to_owned())
            .or_insert_with(|| SessionRt {
                profile: None,
                current: None,
                tx: broadcast::channel(256).0,
            })
            .tx
            .clone()
    }

    /// Sends to the owning session's viewers (or the global channel).
    pub(crate) fn emit(&self, session: Option<&str>, e: BrowserViewEvent) {
        let tx = session.and_then(|s| lock(&self.sessions).get(s).map(|r| r.tx.clone()));
        let _ = tx.unwrap_or_else(|| self.global.clone()).send(e);
    }

    pub(crate) fn current(&self, session: &str) -> Option<String> {
        lock(&self.sessions)
            .get(session)
            .and_then(|s| s.current.clone())
    }

    pub(crate) fn set_current(&self, session: &str, tab: Option<String>) {
        if let Some(s) = lock(&self.sessions).get_mut(session) {
            s.current = tab;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("../a/b?.pdf"), "_a_b_.pdf");
        assert_eq!(sanitize(".."), "file");
    }
}
