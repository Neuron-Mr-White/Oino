//! Phone layouts (feature `mobile`, design.md §6; owned by design-mobile).
//!
//! The mobile shell composes the shared design system ([`crate::design`] tokens at
//! [`Density::Touch`](crate::design::Density::Touch)) and components ([`crate::components`],
//! the [`SessionPane`](crate::views::session_pane::SessionPane) conversation, docked
//! interaction cards, OUS, kanban) into stacked, thumb-first screens:
//!
//! ```text
//!  M1 Sessions ─tap─▶ M2 Conversation ─◨─▶ M4 right drawer (Review, Plan, Agent, panels)
//!      │ ⌕ ▶ M7 Search    │ ⋯ ▶ M6 Actions sheet ─▶ Status / Model sheets, M9 Board
//!      │ ⚙ ▶ M8 Settings  │ tool row ▶ M5 page (diff, output, overlay panels)
//!      └ hosts ▶ M10      └ blocking card ▶ M3 docked card / bottom sheet
//! ```
//!
//! Back (Android back = `escape`, iOS edge swipe) pops in the order menu → sheet → drawer →
//! page. Safe areas pad every screen and the keyboard inset lifts the dock.
//!
//! | Module | Screen |
//! |---|---|
//! | `shell` | [`MobileShell`]: navigation stack, sheets, effects, toasts, safe areas |
//! | `sessions` | M1 home (needs-you card, sections, swipe actions, crab, New session pill) |
//! | `conversation` | M2 app bar, status capsule row, M3 interaction sheets |
//! | `drawer` | M4 right drawer |
//! | `pages` | M5 full-screen pages with a detail header |
//! | `sheets` | M6 bottom sheets (model, actions, status, rename, confirm, row menus) |
//! | `search` | M7 search |
//! | `settings` | M8 stacked settings pages |
//! | `board` | M9 lane pager |
//! | `hosts` | M10 hosts list and pairing |
//! | `gesture` | swipe rows, long press, pull to refresh |
//!
//! The platform crate (`oino-mobile`) installs a [`MobilePlatform`] global with the soft
//! keyboard, safe-area, notification, in-app browser and storage hooks, so this crate stays
//! free of platform dependencies and keeps building (and testing) on desktop hosts.

#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unused_self,
    clippy::needless_pass_by_value,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::cast_precision_loss,
    reason = "declarative GPUI element trees (as in `components` / `views`): one builder chain per screen part, screen helpers stay methods for uniform call sites, layout code uses short names (`t`, `w`, `h`) and multiplies small counts (lanes, swipe actions) by pixel sizes"
)]

mod board;
mod conversation;
mod drawer;
mod gesture;
mod hosts;
mod pages;
mod search;
mod sessions;
mod settings;
mod sheets;
mod shell;

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

pub use hosts::RemoteHost;
pub use shell::MobileShell;

/// Safe-area insets in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    /// Status bar / notch.
    pub top: f32,
    /// Home indicator / navigation bar.
    pub bottom: f32,
    /// Left cut-out.
    pub left: f32,
    /// Right cut-out.
    pub right: f32,
}

/// Platform hooks the shell calls (installed by the app crate; inert by default).
#[derive(Clone, Copy, Debug)]
pub struct MobilePlatform {
    /// Shows the soft keyboard (a text field gained focus).
    pub show_keyboard: fn(),
    /// Hides the soft keyboard.
    pub hide_keyboard: fn(),
    /// Current safe-area insets.
    pub safe_area: fn() -> Insets,
    /// Current soft keyboard height (0 when hidden), logical pixels.
    pub keyboard_height: fn() -> f32,
    /// Posts a local notification `(id, title, body)` (osNotify, pending interactions).
    pub notify: fn(i32, &str, &str),
    /// Sets the app badge / notification count (needs-you sessions).
    pub set_badge: fn(usize),
    /// Opens a URL in the in-app browser (Android Custom Tabs, iOS `SFSafariViewController`);
    /// returns `false` when unavailable (the shell then opens it externally).
    pub open_browser: fn(&str, bool) -> bool,
    /// A short haptic tick (long press, swipe commit).
    pub haptic: fn(),
    /// Takes the `oino://` deep link that opened or re-activated the app, once.
    pub take_link: fn() -> Option<String>,
    /// Paired desktops (persisted in `mobile.json`).
    pub load_hosts: fn() -> Vec<RemoteHost>,
    /// Saves the paired desktops.
    pub save_hosts: fn(&[RemoteHost]),
    /// Colors the system bars for the theme: `(dark, background 0xRRGGBB)`.
    pub system_bars: fn(bool, u32),
}

fn no_op() {}

fn no_insets() -> Insets {
    Insets::default()
}

fn no_keyboard() -> f32 {
    0.0
}

fn no_notify(_: i32, _: &str, _: &str) {}

fn no_badge(_: usize) {}

fn no_browser(_: &str, _: bool) -> bool {
    false
}

fn no_link() -> Option<String> {
    None
}

fn no_hosts() -> Vec<RemoteHost> {
    Vec::new()
}

fn no_save(_: &[RemoteHost]) {}

fn no_bars(_: bool, _: u32) {}

impl Default for MobilePlatform {
    fn default() -> Self {
        Self {
            show_keyboard: no_op,
            hide_keyboard: no_op,
            safe_area: no_insets,
            keyboard_height: no_keyboard,
            notify: no_notify,
            set_badge: no_badge,
            open_browser: no_browser,
            haptic: no_op,
            take_link: no_link,
            load_hosts: no_hosts,
            save_hosts: no_save,
            system_bars: no_bars,
        }
    }
}

impl Global for MobilePlatform {}

impl MobilePlatform {
    /// Installs the hooks and the touch design density (rows 56, tool rows 32, targets 44).
    pub fn init(self, cx: &mut App) {
        cx.set_global(self);
        crate::design::theme::set_density(crate::design::Density::Touch, cx);
        crate::design::theme::register_fonts(cx);
    }

    /// The installed hooks (defaults when none were installed).
    #[must_use]
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }
}

/// Where the shell starts (screenshots and deep links; `<agent>/mobile-demo` lines in demo
/// builds). Every field is optional; the default is the session list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent launch switches parsed from `key=value` lines"
)]
pub struct Launch {
    /// `sessions` | `new` | `resume` | `settings[:page]` | `search` | `board` | `hosts` |
    /// `pair`.
    pub open: Option<String>,
    /// `model` | `actions` | `status` (over a conversation).
    pub sheet: Option<String>,
    /// Open the right drawer (over a conversation).
    pub drawer: bool,
    /// A long-press menu on the first session row.
    pub menu: bool,
    /// The first session row swiped open.
    pub swipe: bool,
    /// Search query typed into M7.
    pub query: Option<String>,
    /// Return to the session list once the opened session is running (M1 with live rows).
    pub home: bool,
}

impl Launch {
    /// Parses `key=value` lines (`open=new`, `sheet=model`, `drawer=1`); unknown keys and
    /// `scene=` / `theme=` (read by the app crate) are ignored.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut launch = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let on = matches!(value, "1" | "true" | "yes");
            match key.trim() {
                "open" => launch.open = Some(value.to_owned()),
                "sheet" => launch.sheet = Some(value.to_owned()),
                "drawer" => launch.drawer = on,
                "menu" => launch.menu = on,
                "swipe" => launch.swipe = on,
                "query" => launch.query = Some(value.to_owned()),
                "home" => launch.home = on,
                _ => {}
            }
        }
        launch
    }

    /// The launch requested by an `oino://open?...` deep link (`oino://pair?...` opens the
    /// pairing page with the link prefilled by the caller).
    #[must_use]
    pub fn from_link(link: &str) -> Option<Self> {
        let rest = link.strip_prefix("oino://")?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        match path.trim_end_matches('/') {
            "pair" => Some(Self {
                open: Some("pair".to_owned()),
                ..Self::default()
            }),
            "open" => Some(Self::parse(&query.replace('&', "\n"))),
            // `oino://<host>:<port>?token=…` pairing links (modes §5.1).
            p if p.contains(':') => Some(Self {
                open: Some("pair".to_owned()),
                ..Self::default()
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hooks_are_inert() {
        let hooks = MobilePlatform::default();
        (hooks.show_keyboard)();
        (hooks.hide_keyboard)();
        (hooks.notify)(1, "t", "b");
        (hooks.set_badge)(2);
        (hooks.haptic)();
        assert!(!(hooks.open_browser)("https://x", true));
        assert_eq!((hooks.take_link)(), None);
        assert!((hooks.load_hosts)().is_empty());
        assert_eq!((hooks.safe_area)(), Insets::default());
        assert!((hooks.keyboard_height)().abs() < f32::EPSILON);
    }

    #[test]
    fn launch_parses_lines_and_links() {
        let l = Launch::parse("scene=approval\nopen=new\nsheet=model\ndrawer=1\nbogus");
        assert_eq!(l.open.as_deref(), Some("new"));
        assert_eq!(l.sheet.as_deref(), Some("model"));
        assert!(l.drawer);
        assert!(!l.menu);
        let link = Launch::from_link("oino://open?open=settings:remote&drawer=0").unwrap();
        assert_eq!(link.open.as_deref(), Some("settings:remote"));
        assert_eq!(
            Launch::from_link("oino://pair?token=x")
                .unwrap()
                .open
                .as_deref(),
            Some("pair")
        );
        assert_eq!(
            Launch::from_link("oino://studio:7420?token=t")
                .unwrap()
                .open
                .as_deref(),
            Some("pair")
        );
        assert_eq!(Launch::from_link("https://x"), None);
    }
}
