//! Attached mode (browser.md §2, WP-B7): the embedding host owns the browser process.
//!
//! unipi-host already runs one browser per profile for the app's viewers (screencast pane,
//! phone sheet, take-over gate, `CloakBrowser` install, hidden display). With an [`Attach`]
//! in [`crate::BrowserOptions::attach`], the service never launches anything itself: on
//! the first command for a profile it asks the host for that profile's browser and connects
//! to the CDP websocket it returns — normally the host's gated endpoint, so the agent's
//! commands pass the take-over check and show as "Agent is driving" in the pane. Agent and
//! user then share one browser (one engine, one profile, one set of cookies).
//!
//! Shutting the service's profile down only closes this connection (the gate scopes
//! `Browser.close` to the targets the connection created); the host decides when the
//! process itself ends.

use oino_extension_api::BoxFut;

/// Where an attached browser draws (status wire names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachedDisplay {
    /// Headed on a private display.
    Hidden,
    /// A real window.
    Window,
    /// `--headless`.
    Headless,
}

impl AttachedDisplay {
    /// Wire name (`BrowserStatus.display`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Window => "window",
            Self::Headless => "headless",
        }
    }
}

/// A running browser the host handed over.
#[derive(Clone, Debug)]
pub struct Attached {
    /// The browser-level `DevTools` websocket (`ws://127.0.0.1:<port>/devtools/browser/<id>`).
    pub ws_url: String,
    /// `cloak` / `chrome`.
    pub engine: String,
    /// Where it draws.
    pub display: AttachedDisplay,
}

/// Why the host could not hand a browser over (the reason is a `BrowserResult` reason:
/// `disabled`, `no_engine`, `launch_failed`, `too_many`, …).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachError {
    /// Machine reason.
    pub reason: String,
    /// What the model reads.
    pub message: String,
}

/// The host side of attached mode.
pub trait Attach: Send + Sync {
    /// Starts (or reuses) profile `profile`'s browser and returns how to reach it.
    ///
    /// # Errors
    /// The browser is off for agents on this host, missing, or failed to start.
    fn attach(&self, profile: &str) -> BoxFut<'_, Result<Attached, AttachError>>;
}
