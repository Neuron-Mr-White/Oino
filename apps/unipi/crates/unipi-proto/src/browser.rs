//! Remote desktop browser (UNI-215, forwarding phase 3): view and drive a
//! dedicated Chromium-family browser on the host from the phone, via the
//! Chrome DevTools Protocol (the host is the CDP client).
//!
//! Requests (see `crate::control::Request`):
//! - `browser_status` → `ok{data: BrowserStatusData}` (found/enabled/running, tabs).
//! - `browser_open{viewport, url?, target?}` → `ok{data: BrowserOpenData}`, then a
//!   live stream until either side closes:
//!   - host → phone: `browser_event{event}` controls; a `frame` event is
//!     immediately followed by ONE kind-1 (output) frame carrying the JPEG.
//!   - phone → host: `browser_cmd{cmd}` controls (input, navigation, tabs,
//!     viewport, frame acks).
//! - `browser_close` → `ok{}` after the host shut its browser down.
//! - `browser_window{show}` (UNI-268, 1.13.0) → `ok{data: BrowserStatusData}`:
//!   show the browser's window on the PC or hide it. Saved to the host's
//!   config; a running browser relaunches in the new mode with the same tabs
//!   (open viewers get `closed{reason:"relaunch"}` and reconnect).
//! - UNI-292 (1.16.0), all additive: the host's browser is CloakBrowser (a
//!   stealth Chromium build) by default, falling back to Chrome. Status and
//!   open data say which `engine` runs, whether it is `stealth`, and why not
//!   (`stealth_reason`); `browser_open{profile?}` picks a browser profile
//!   (default `"default"`), and status/open data name the `profile`.
//! - UNI-296 (1.17.0), all additive: "Agent is driving" + Take over.
//!   - Status/open data carry `agent: BrowserAgent` (an agent's CDP client
//!     on the host's gated endpoint, whether it sent input in the last 3 s,
//!     whether the user has taken over); viewers get `browser_event{agent}`
//!     whenever it changes.
//!   - `browser_takeover{on, profile?}` → `ok{data: BrowserAgent}`: while on,
//!     the host refuses agents' `Input.*` / navigation commands on that
//!     browser with a CDP error ("The user has taken over the browser; wait
//!     or ask them"). Off = give back; it also ends by itself after 10 min
//!     without the user's own input.
//!   - `browser_profiles` → `ok{data: BrowserProfilesData}`: the host's
//!     browser profiles (read only).
//! - UNI-299 (1.18.0), additive: CloakBrowser runs headed on a hidden,
//!   private display by default instead of the PC's real one. Status/open
//!   data carry `display: "hidden" | "window" | "headless"` (absent =
//!   older host, i.e. `"window"`) and, only for `"headless"`,
//!   `stealth_note` (no hidden display was possible: a milder red flag,
//!   `stealth` itself stays true). The existing `headless`/`browser_window`
//!   toggle now switches hidden ↔ window for CloakBrowser too.
//!
//! - WP-B8 (v2), additive: the element picker. `browser_cmd{op:"inspect", x, y,
//!   pick}` asks what is under a point of the picture; the host answers
//!   `browser_event{kind:"element", pick, element?}` — a box + short label while
//!   hovering (`pick: false`, DOM box model only, no page JS), plus the
//!   details (selector, XPath, role, name, text, safe attributes, HTML excerpt)
//!   computed in an isolated world on the click (`pick: true`). Older hosts
//!   never answer; the app times the picker out.
//!
//! Flow control: the host keeps at most a couple of frames in flight; the
//! phone acks each `frame` (`ack{seq}`) once drawn. Quality/fps adapt to how
//! fast acks come back.

use serde::{Deserialize, Serialize};

/// Largest viewport side the phone may ask for, in CSS pixels.
pub const BROWSER_MAX_SIDE: u32 = 4096;
/// Largest URL accepted in commands.
pub const BROWSER_MAX_URL: usize = 8192;
/// Largest `text` insert per command.
pub const BROWSER_MAX_TEXT: usize = 16 * 1024;

/// The phone's viewport: CSS pixels plus device scale (DPR).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawViewport")]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
    /// Device scale factor, 0.5..=4.
    pub scale: f64,
    /// Emulate a mobile device (touch, mobile viewport meta).
    #[serde(default)]
    pub mobile: bool,
}

#[derive(Deserialize)]
struct RawViewport {
    width: u32,
    height: u32,
    scale: f64,
    #[serde(default)]
    mobile: bool,
}

impl TryFrom<RawViewport> for BrowserViewport {
    type Error = String;
    fn try_from(raw: RawViewport) -> Result<Self, Self::Error> {
        if !(50..=BROWSER_MAX_SIDE).contains(&raw.width)
            || !(50..=BROWSER_MAX_SIDE).contains(&raw.height)
        {
            return Err(format!(
                "viewport must be 50..={BROWSER_MAX_SIDE} px per side"
            ));
        }
        if !(raw.scale.is_finite() && (0.5..=4.0).contains(&raw.scale)) {
            return Err("viewport scale must be 0.5..=4".into());
        }
        Ok(BrowserViewport {
            width: raw.width,
            height: raw.height,
            scale: raw.scale,
            mobile: raw.mobile,
        })
    }
}

/// Longest browser profile name.
pub const BROWSER_PROFILE_MAX: usize = 40;

/// A browser profile name (UNI-292): 1..=40 of `a-z 0-9 - _`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BrowserProfile(String);

impl BrowserProfile {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BrowserProfile {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let ok = !value.is_empty()
            && value.len() <= BROWSER_PROFILE_MAX
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if !ok {
            return Err(format!(
                "profile must be 1..={BROWSER_PROFILE_MAX} of a-z, 0-9, - and _"
            ));
        }
        Ok(BrowserProfile(value))
    }
}

impl<'de> Deserialize<'de> for BrowserProfile {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        BrowserProfile::try_from(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A URL in a command: 1..=8192 chars, no control characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BrowserUrl(String);

impl BrowserUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BrowserUrl {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() || value.len() > BROWSER_MAX_URL {
            return Err(format!("url must be 1..={BROWSER_MAX_URL} bytes"));
        }
        if value.chars().any(|c| c.is_control()) {
            return Err("url must not contain control characters".into());
        }
        Ok(BrowserUrl(value))
    }
}

impl<'de> Deserialize<'de> for BrowserUrl {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        BrowserUrl::try_from(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// One browser tab (a CDP `page` target).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserTab {
    pub id: String,
    pub url: String,
    pub title: String,
}

/// Which browser the host found/uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserInfo {
    /// What to call it: "Google Chrome", "Chromium", "Microsoft Edge" or
    /// "Chromium-based browser" (hosts before UNI-268 sent `chrome`,
    /// `chromium` … — show it as is).
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
}

/// `ok.data` of `browser_status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserStatusData {
    /// The owner switched the feature on (`unipi-host browser enable`).
    pub enabled: bool,
    /// A browser binary was found (or configured).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub browser: Option<BrowserInfo>,
    /// The dedicated browser is running right now.
    pub running: bool,
    pub headless: bool,
    /// Minutes with no viewer before the host shuts the browser down.
    pub idle_minutes: u32,
    #[serde(default)]
    pub tabs: Vec<BrowserTab>,
    /// How to install a browser when none was found.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub install_hint: Option<String>,
    /// UNI-268: the PC has a graphical session, so the window can be shown
    /// (`browser_window`). Absent from older hosts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub can_show_window: Option<bool>,
    /// UNI-292: `"cloak"` (CloakBrowser) or `"chrome"` — the engine running,
    /// or the one that would start. Absent from older hosts (= chrome).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub engine: Option<String>,
    /// UNI-292: the browser has CloakBrowser's fingerprint patches. `false`
    /// = plain Chrome (configured, or CloakBrowser failed and the host fell
    /// back): show "not stealthy". Absent from older hosts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth: Option<bool>,
    /// UNI-292: why it isn't stealthy (the fallback's cause).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth_reason: Option<String>,
    /// UNI-299: set only for `display: "headless"` — stealth patches are
    /// still active (`stealth` stays true for the engine), but running
    /// without any window at all is a separate, milder red flag than
    /// having one on a hidden display. The app shows "Less stealthy: no
    /// hidden display (install Xvfb)".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth_note: Option<String>,
    /// UNI-292: the browser profile this status is about (`"default"`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub profile: Option<String>,
    /// UNI-296: agents on that profile's browser. Absent from older hosts.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent: Option<BrowserAgent>,
    /// UNI-299: `"hidden"` (headed CloakBrowser on a private display nobody
    /// sees — the default), `"window"` (shown on the PC's own desktop) or
    /// `"headless"` (no hidden display was possible: degraded stealth).
    /// Absent from older hosts (= `"window"`, UNI-268's only mode then).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display: Option<String>,
}

/// UNI-296: what agents (CDP clients other than the app's own) are doing
/// with one browser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserAgent {
    /// A CDP client other than the host's own is connected (an agent).
    pub connected: bool,
    /// It sent input or navigated within the last few seconds.
    pub driving: bool,
    /// The user took over: agents' input is refused until given back.
    #[serde(default)]
    pub taken_over: bool,
}

/// UNI-296: one browser profile (`browser_profiles`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserProfileInfo {
    pub name: String,
    /// Unix ms.
    pub created: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_used: Option<u64>,
    /// Its browser is running now.
    pub running: bool,
    /// `cloak` | `chrome` while running.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth: Option<bool>,
    /// Sites whose cookies were imported into it.
    #[serde(default)]
    pub cookie_domains: Vec<String>,
}

/// UNI-296: `ok.data` of `browser_profiles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserProfilesData {
    pub profiles: Vec<BrowserProfileInfo>,
    /// The profile app ops use when none is named (`"default"`).
    pub default: String,
}

/// `ok.data` of `browser_open` (before the stream starts).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserOpenData {
    pub browser: BrowserInfo,
    pub headless: bool,
    /// The tab this stream shows.
    pub target: String,
    pub tabs: Vec<BrowserTab>,
    /// UNI-292: as in [`BrowserStatusData`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth_reason: Option<String>,
    /// UNI-299: as in [`BrowserStatusData`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stealth_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub profile: Option<String>,
    /// UNI-296: as in [`BrowserStatusData`]; changes arrive as `agent` events.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent: Option<BrowserAgent>,
    /// UNI-299: as in [`BrowserStatusData`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub display: Option<String>,
}

/// Mouse event kinds (CDP `Input.dispatchMouseEvent.type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    Pressed,
    Released,
    Moved,
    Wheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    None,
    Left,
    Middle,
    Right,
}

/// Touch event kinds (CDP `Input.dispatchTouchEvent.type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchKind {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TouchPoint {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub id: u32,
}

/// Key event kinds (CDP `Input.dispatchKeyEvent.type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Down,
    Up,
    /// Down + up in one command.
    Press,
}

/// Phone → host commands on a `browser_open` stream. Coordinates are CSS
/// pixels of the page viewport. `modifiers` is the CDP bitmask
/// (Alt=1, Ctrl=2, Meta=4, Shift=8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BrowserCmd {
    Navigate {
        url: BrowserUrl,
    },
    Back,
    Forward,
    Reload,
    Stop,
    TabNew {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        url: Option<BrowserUrl>,
    },
    TabClose {
        id: String,
    },
    TabActivate {
        id: String,
    },
    Viewport {
        viewport: BrowserViewport,
    },
    Mouse {
        kind: MouseKind,
        x: f64,
        y: f64,
        #[serde(default)]
        button: MouseButton,
        #[serde(default)]
        clicks: u32,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
        #[serde(default)]
        modifiers: u32,
    },
    Touch {
        kind: TouchKind,
        #[serde(default)]
        points: Vec<TouchPoint>,
    },
    /// Pinch zoom around (x, y); `scale` > 1 zooms in.
    Pinch {
        x: f64,
        y: f64,
        scale: f64,
    },
    Key {
        kind: KeyKind,
        /// DOM `key` (`Enter`, `Backspace`, `a` …).
        key: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        code: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        text: Option<String>,
        #[serde(default)]
        modifiers: u32,
    },
    /// Type text as if committed by an IME (`Input.insertText`).
    Text {
        text: String,
    },
    /// The phone drew frame `seq`.
    Ack {
        seq: u64,
    },
    /// WP-B8: what is under (x, y)? Answered with [`BrowserEvent::Element`].
    Inspect {
        x: f64,
        y: f64,
        /// The user clicked (details wanted), not just hovering.
        #[serde(default)]
        pick: bool,
    },
}

impl BrowserCmd {
    /// Bounds serde can't express.
    pub fn check(&self) -> Result<(), String> {
        let finite = |v: &[f64]| v.iter().all(|n| n.is_finite());
        match self {
            BrowserCmd::Mouse { x, y, dx, dy, .. } if !finite(&[*x, *y, *dx, *dy]) => {
                Err("mouse coordinates must be finite".into())
            }
            BrowserCmd::Inspect { x, y, .. } if !finite(&[*x, *y]) => {
                Err("inspect coordinates must be finite".into())
            }
            BrowserCmd::Touch { points, .. }
                if points.len() > 10 || !points.iter().all(|p| finite(&[p.x, p.y])) =>
            {
                Err("touch needs ≤ 10 finite points".into())
            }
            BrowserCmd::Pinch { x, y, scale }
                if !finite(&[*x, *y, *scale]) || *scale <= 0.0 || *scale > 10.0 =>
            {
                Err("pinch scale must be 0..10".into())
            }
            BrowserCmd::Text { text } if text.len() > BROWSER_MAX_TEXT => {
                Err("text too long".into())
            }
            BrowserCmd::Key { key, .. } if key.is_empty() || key.len() > 64 => {
                Err("key must be 1..=64 bytes".into())
            }
            _ => Ok(()),
        }
    }
}

/// Metadata of one screencast frame; the JPEG follows as the next kind-1 frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserFrameMeta {
    pub seq: u64,
    /// Page viewport in CSS px the frame shows (map taps with these).
    pub css_width: f64,
    pub css_height: f64,
    /// JPEG quality used (adapts to bandwidth).
    pub quality: u8,
}

/// WP-B8: an element under the picker, in the picture's CSS px (the frame's
/// `css_width` × `css_height` space).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrowserElement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Short label: `button#send.primary`.
    pub label: String,
    /// Only for a pick: what the composer chip carries.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<BrowserElementDetail>,
}

/// WP-B8: the picked element (ZCode web-element context field set).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowserElementDetail {
    pub url: String,
    pub title: String,
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,
    /// Accessible name (aria-label, alt, label text, …).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub xpath: Option<String>,
    /// Visible text (truncated; password fields masked).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
    /// Safe attributes only (no values of inputs, no event handlers).
    #[serde(default)]
    pub attributes: std::collections::BTreeMap<String, String>,
    /// Sanitized outer HTML excerpt.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub html: Option<String>,
}

/// Host → phone events on a `browser_open` stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserEvent {
    /// The tab list changed (or the active tab did).
    Tabs {
        tabs: Vec<BrowserTab>,
        active: String,
    },
    /// The active tab's page state.
    Page {
        url: String,
        title: String,
        loading: bool,
        can_back: bool,
        can_forward: bool,
    },
    Frame(BrowserFrameMeta),
    /// The page's renderer or the whole browser crashed.
    Crashed {
        message: String,
    },
    /// The browser was shut down (`idle`, `closed` = `browser_close`,
    /// `host_exit`, `relaunch` = restarted to show/hide its window: reopen).
    Closed {
        reason: String,
    },
    /// UNI-296: what agents are doing with this browser changed.
    Agent {
        agent: BrowserAgent,
    },
    /// WP-B8: the answer to an `inspect` (`element: None` = nothing there).
    Element {
        pick: bool,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        element: Option<Box<BrowserElement>>,
    },
}
