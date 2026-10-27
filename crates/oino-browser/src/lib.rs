//! `oino-browser`: the host browser service (browser-control.md §3–§10, WP2).
//!
//! # Boundary
//!
//! Mechanism only: it launches Chromium-family browsers (system Chrome / Chromium / Edge /
//! Brave, Playwright's Chromium, or an installed `CloakBrowser`), talks CDP over
//! `--remote-debugging-pipe` (Unix) or a loopback WebSocket (Windows), keeps a logical tab
//! registry scoped by `(profile, workspace, session)`, and implements
//! [`oino_extension_api::browser::BrowserHost`]. The model-facing tools live in the bundled
//! `oino.browser` extension; frontends attach screencast views.
//!
//! # API map
//!
//! | Item | Use |
//! |---|---|
//! | [`BrowserService`] | process-wide service: `execute` (agent commands), `status`, `subscribe`, `attach_view`, `frontend`, `shutdown` |
//! | [`BrowserOptions`] | root dir, [`BrowserSettings`], ephemeral mode |
//! | [`BrowserSettings`] | `settings.json` `browser.*` (§12) |
//! | [`engine`] | engine resolution (§4.1) |
//!
//! Internals: `cdp` (client + transports), `launch` (flags, process group), `display` (Xvfb),
//! `profile` (per-profile runtime + CDP event pump), `tab` (state, rings, residency),
//! `snapshot` (AX → text + refs), `page` (actionability, input, eval, screenshots), `keys`,
//! `policy` (URL policy, sensitive heuristic, gate), `screenshot` (compression ladder),
//! `permissions` (site grants), `profiles` (dirs), `commands` (agent ops), `frontend` (viewers).
//!
//! Every browser process runs in its own process group and is killed with the service; with
//! the pipe transport a crashed Oino also ends the browser (the pipe closes).

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

mod cdp;
mod commands;
pub mod config;
mod display;
pub mod engine;
mod frontend;
mod hub;
mod keys;
mod launch;
mod page;
mod permissions;
mod policy;
mod profile;
mod profiles;
mod screenshot;
mod service;
mod snapshot;
mod tab;

pub use config::BrowserSettings;
pub use oino_browser_protocol as protocol;
pub use service::{BrowserOptions, BrowserService};
