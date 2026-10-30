//! Desktop streaming with control (UNI-216, forwarding phase 4): watch and
//! drive the PC's whole desktop from the phone, RustDesk-style.
//! Mirrors `packages/protocol/src/desktop.ts`.
//!
//! Requests (see `crate::control::Request`):
//! - `desktop_status` → `ok{data: DesktopStatusData}`: enabled/view-only,
//!   which capture backends this build has, what display the host sees and
//!   whether the portal consent is already stored.
//! - `desktop_open{viewport, quality?, view_only?}` → `ok{}` as soon as the
//!   gate passes, then a live stream until either side closes:
//!   - host → phone: `desktop_event{event}` controls. `consent{waiting}` while
//!     the PC shows its share dialog, `started{…}` once capture runs,
//!     `config{codec,width,height}` before the first key frame and whenever
//!     the encoded size changes, `frame{seq,key,bytes,…}` immediately followed
//!     by kind-1 (output) frames carrying `bytes` of H.264 Annex-B (one access
//!     unit; split across several kind-1 frames when larger than `MAX_FRAME`),
//!     `stats{…}` about once a second, `closed{reason}` last.
//!     A failure after `ok` is one `err` envelope (then the stream ends).
//!   - phone → host: `desktop_cmd{cmd}` controls (pointer, buttons, scroll,
//!     keys, text, key-frame requests, quality, frame acks). The two
//!     directions of the QUIC stream are independent, so input never waits
//!     behind video.
//!
//! Flow control: the host keeps a few frames un-acked at most and drops
//! captured frames (before encoding) when the phone falls behind; `auto`
//! quality adapts bitrate/fps to how fast acks come back.

use serde::{Deserialize, Serialize};

/// Largest `text` per command.
pub const DESKTOP_MAX_TEXT: usize = 4096;
/// Largest viewport side the phone may report, in device pixels.
pub const DESKTOP_MAX_SIDE: u32 = 8192;

/// Stream quality preset. `auto` adapts to the link.
///
/// UNI-271 (proto 1.13, host feature `presets`): RustDesk-style presets that
/// all adapt to the link — `best` (the screen's own resolution, text-sharp,
/// drops fps before resolution), `balanced` (≤ 2560 px), `speed` (≤ 1600 px
/// and the viewer's size, lowest latency). Older hosts reject these, so the
/// app only sends them when `desktop_status.features` has `presets`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DesktopQuality {
    #[default]
    Auto,
    Low,
    Medium,
    High,
    Best,
    Balanced,
    Speed,
}

impl DesktopQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            DesktopQuality::Auto => "auto",
            DesktopQuality::Low => "low",
            DesktopQuality::Medium => "medium",
            DesktopQuality::High => "high",
            DesktopQuality::Best => "best",
            DesktopQuality::Balanced => "balanced",
            DesktopQuality::Speed => "speed",
        }
    }
}

/// Optional host abilities in `desktop_status.features` (UNI-271).
pub const DESKTOP_FEATURE_PRESETS: &str = "presets";
/// `desktop_open.display` picks a monitor from `desktop_status.displays`.
pub const DESKTOP_FEATURE_DISPLAYS: &str = "displays";

/// One monitor the host can stream (`desktop_status.displays`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopDisplay {
    /// What `desktop_open.display` takes (a connector such as `DP-1`).
    pub id: String,
    /// Human name ("Dell U3419W", or the connector).
    pub name: String,
    /// Pixels (physical, as streamed).
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub primary: bool,
}

/// The phone's canvas in device pixels — a hint for the largest size worth
/// encoding (the host never upscales).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawViewport")]
pub struct DesktopViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize)]
struct RawViewport {
    width: u32,
    height: u32,
}

impl TryFrom<RawViewport> for DesktopViewport {
    type Error = String;
    fn try_from(raw: RawViewport) -> Result<Self, Self::Error> {
        if !(16..=DESKTOP_MAX_SIDE).contains(&raw.width) || !(16..=DESKTOP_MAX_SIDE).contains(&raw.height) {
            return Err(format!("viewport must be 16..={DESKTOP_MAX_SIDE} px per side"));
        }
        Ok(DesktopViewport { width: raw.width, height: raw.height })
    }
}

/// What kind of graphical session the host found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayKind {
    Wayland,
    X11,
    /// A macOS host (proto 1.12): the logged-in user's console session.
    Macos,
    /// A Windows host (proto 1.12): the logged-in user's interactive session.
    Windows,
    /// No graphical session (SSH-only box, headless server, no seat).
    None,
}

/// Is the portal's share consent already stored (restore token)?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopConsent {
    /// A restore token is stored: no dialog expected.
    Granted,
    /// The PC will show its share dialog on the next connect.
    Needed,
    /// The backend has no consent step (X11).
    NotNeeded,
}

/// `ok.data` of `desktop_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopStatusData {
    /// The owner switched it on (`unipi-host desktop enable`) for this device.
    pub enabled: bool,
    /// Input from the phone is ignored (`--view-only`).
    pub view_only: bool,
    /// Capture backends compiled into this unipi-host (`portal`, `x11`).
    pub compiled: Vec<String>,
    pub display: DisplayKind,
    /// The backend a `desktop_open` would use, when one fits.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend: Option<String>,
    pub consent: DesktopConsent,
    /// Why streaming can't work right now (no display, not compiled …).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hint: Option<String>,
    /// UNI-271: optional abilities (`presets`, `displays`); empty on older hosts.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub features: Vec<String>,
    /// UNI-271: the monitors `desktop_open.display` can pick (≥ 2 to matter).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub displays: Vec<DesktopDisplay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// Phone → host commands on a `desktop_open` stream.
///
/// Pointer positions are normalised to the captured screen: `0..=1` on
/// both axes (independent of the encoded size and the phone's zoom).
/// `scroll` is in wheel steps (fractional allowed; `dy > 0` scrolls down).
/// Keys are X11 keysyms (the portal and XTest both speak them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DesktopCmd {
    Pointer { x: f64, y: f64 },
    Button { button: PointerButton, down: bool },
    Scroll {
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
    },
    Key { keysym: u32, down: bool },
    /// Type text (each character as a keysym press).
    Text { text: String },
    /// Ask for a key frame now (decoder reset, after a drop).
    Keyframe,
    Quality { quality: DesktopQuality },
    Viewport { viewport: DesktopViewport },
    /// The phone decoded/drew frame `seq`.
    Ack { seq: u64 },
}

impl DesktopCmd {
    /// Bounds serde can't express.
    pub fn check(&self) -> Result<(), String> {
        match self {
            DesktopCmd::Pointer { x, y } if !(x.is_finite() && y.is_finite()) || !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y) => {
                Err("pointer must be finite and within 0..=1".into())
            }
            DesktopCmd::Scroll { dx, dy } if !(dx.is_finite() && dy.is_finite()) || dx.abs() > 100.0 || dy.abs() > 100.0 => {
                Err("scroll must be finite and at most 100 steps".into())
            }
            DesktopCmd::Key { keysym, .. } if *keysym == 0 || *keysym > 0x1FF_FFFF => Err("keysym out of range".into()),
            DesktopCmd::Text { text } if text.len() > DESKTOP_MAX_TEXT => Err("text too long".into()),
            _ => Ok(()),
        }
    }

    /// Does this command drive the PC (refused in view-only mode)?
    pub fn is_input(&self) -> bool {
        matches!(
            self,
            DesktopCmd::Pointer { .. } | DesktopCmd::Button { .. } | DesktopCmd::Scroll { .. } | DesktopCmd::Key { .. } | DesktopCmd::Text { .. }
        )
    }
}

/// Metadata of one encoded access unit; `bytes` of H.264 follow in kind-1 frames.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopFrameMeta {
    pub seq: u64,
    /// An IDR (decoder can start here).
    pub key: bool,
    /// Capture time, ms since the stream started.
    pub ts_ms: u64,
    pub bytes: u32,
}

/// Host → phone events on a `desktop_open` stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopEvent {
    /// The PC is showing its "share your screen" dialog; waiting for the owner.
    Consent { state: String },
    /// Capture runs.
    Started {
        backend: String,
        /// Captured screen size in pixels.
        width: u32,
        height: u32,
        view_only: bool,
        /// UNI-271: the monitor being streamed (`desktop_status.displays[].id`).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        display: Option<String>,
    },
    /// The decoder configuration for the following frames (H.264 Annex-B,
    /// SPS/PPS in-band with every key frame).
    Config { codec: String, width: u32, height: u32 },
    Frame(DesktopFrameMeta),
    Stats { fps: f32, kbps: u32, quality: DesktopQuality, rtt_ms: u32 },
    /// The stream ended on the host side (`host_exit`, `disabled`, `capture_ended`).
    Closed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_bounds() {
        assert!(DesktopCmd::Pointer { x: 0.5, y: 1.0 }.check().is_ok());
        assert!(DesktopCmd::Pointer { x: 1.1, y: 0.0 }.check().is_err());
        assert!(DesktopCmd::Pointer { x: f64::NAN, y: 0.0 }.check().is_err());
        assert!(DesktopCmd::Scroll { dx: 0.0, dy: 101.0 }.check().is_err());
        assert!(DesktopCmd::Key { keysym: 0, down: true }.check().is_err());
        assert!(DesktopCmd::Text { text: "x".repeat(DESKTOP_MAX_TEXT + 1) }.check().is_err());
        assert!(DesktopCmd::Keyframe.check().is_ok());
        assert!(DesktopCmd::Key { keysym: 0xff0d, down: true }.is_input());
        assert!(!DesktopCmd::Ack { seq: 1 }.is_input());
    }

    #[test]
    fn viewport_bounds() {
        assert!(serde_json::from_str::<DesktopViewport>(r#"{"width":8,"height":800}"#).is_err());
        assert!(serde_json::from_str::<DesktopViewport>(r#"{"width":1080,"height":2400}"#).is_ok());
    }

    #[test]
    fn presets_and_displays_wire() {
        assert_eq!(serde_json::to_value(DesktopQuality::Best).unwrap(), "best");
        assert_eq!(serde_json::from_str::<DesktopQuality>(r#""speed""#).unwrap(), DesktopQuality::Speed);
        // An old host's status (no features/displays) still parses.
        let old = r#"{"enabled":true,"view_only":false,"compiled":["x11"],"display":"x11","consent":"not_needed"}"#;
        let status: DesktopStatusData = serde_json::from_str(old).unwrap();
        assert!(status.features.is_empty() && status.displays.is_empty());
        let json = serde_json::to_value(&status).unwrap();
        assert!(json.get("features").is_none(), "empty lists stay off the wire");
        let started = serde_json::to_value(DesktopEvent::Started { backend: "x11".into(), width: 1, height: 1, view_only: false, display: None }).unwrap();
        assert!(started.get("display").is_none());
    }

    #[test]
    fn frame_event_shape() {
        let event = DesktopEvent::Frame(DesktopFrameMeta { seq: 3, key: true, ts_ms: 40, bytes: 1200 });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json, serde_json::json!({"kind":"frame","seq":3,"key":true,"ts_ms":40,"bytes":1200}));
    }
}
