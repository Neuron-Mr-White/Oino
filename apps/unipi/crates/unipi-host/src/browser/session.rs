//! One `browser_open` stream: a viewer of one tab of the dedicated browser.
//!
//! host → phone: `browser_event{tabs|page|frame|crashed|closed}`; each
//! `frame` event is followed by one kind-1 frame with the JPEG bytes.
//! phone → host: `browser_cmd{…}` controls.
//!
//! Flow control: at most `MAX_IN_FLIGHT` frames un-acked by the phone. While
//! the phone is behind, the newest CDP frame is held back (and CDP isn't
//! acked, so Chrome stops producing more). Quality and frame skipping adapt
//! to how long the phone takes to ack.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, BrowserCmd, BrowserEvent, BrowserFrameMeta, BrowserOpenData,
    BrowserTab, BrowserViewport, Control,
};

use super::cdp::{Cdp, CdpEvent};
use super::{Instance, Running, ViewerGuard};
use crate::protocol::{read_frame, write_control};

/// Frames sent to the phone but not acked yet.
pub const MAX_IN_FLIGHT: usize = 2;

/// Adaptive JPEG quality/frame-rate from the phone's ack latency.
#[derive(Debug, Clone, PartialEq)]
pub struct Adapt {
    pub quality: u8,
    /// Chrome's `everyNthFrame`.
    pub every_nth: u32,
    /// Smoothed send→ack latency, ms.
    pub rtt_ms: f64,
    last_change: Option<Instant>,
}

impl Default for Adapt {
    fn default() -> Self {
        Adapt {
            quality: 70,
            every_nth: 1,
            rtt_ms: 0.0,
            last_change: None,
        }
    }
}

impl Adapt {
    pub const MIN_QUALITY: u8 = 25;
    pub const MAX_QUALITY: u8 = 80;

    /// Feeds one ack latency; returns true when quality/fps changed (the
    /// screencast must be restarted with the new settings).
    pub fn sample(&mut self, latency: Duration, now: Instant) -> bool {
        let ms = latency.as_secs_f64() * 1000.0;
        self.rtt_ms = if self.rtt_ms == 0.0 {
            ms
        } else {
            self.rtt_ms * 0.8 + ms * 0.2
        };
        if self
            .last_change
            .is_some_and(|t| now.duration_since(t) < Duration::from_secs(3))
        {
            return false;
        }
        let (quality, every_nth) = if self.rtt_ms > 700.0 {
            (
                self.quality.saturating_sub(15).max(Self::MIN_QUALITY),
                (self.every_nth + 1).min(4),
            )
        } else if self.rtt_ms > 350.0 {
            (
                self.quality.saturating_sub(10).max(Self::MIN_QUALITY),
                self.every_nth,
            )
        } else if self.rtt_ms < 120.0 {
            (
                (self.quality + 5).min(Self::MAX_QUALITY),
                self.every_nth.saturating_sub(1).max(1),
            )
        } else {
            (self.quality, self.every_nth)
        };
        if (quality, every_nth) == (self.quality, self.every_nth) {
            return false;
        }
        self.quality = quality;
        self.every_nth = every_nth;
        self.last_change = Some(now);
        true
    }

    /// A frame was too big for one wire frame: drop quality hard.
    pub fn oversize(&mut self, now: Instant) {
        self.quality = self.quality.saturating_sub(20).max(Self::MIN_QUALITY);
        self.last_change = Some(now);
    }
}

/// Windows virtual key codes for the special keys the phone sends.
pub fn key_code(key: &str) -> Option<u32> {
    Some(match key {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" => 13,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Delete" => 46,
        k if k.len() == 1 => {
            let c = k.chars().next()?.to_ascii_uppercase();
            if c.is_ascii_alphanumeric() {
                c as u32
            } else {
                return None;
            }
        }
        k if k.starts_with('F') && k[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) => {
            111 + k[1..].parse::<u32>().ok()?
        }
        _ => return None,
    })
}

/// The CDP calls for one input command (method, params). Pure, for tests.
pub fn input_calls(cmd: &BrowserCmd) -> Vec<(&'static str, Value)> {
    use unipi_proto::browser::{KeyKind, MouseButton, MouseKind, TouchKind};
    match cmd {
        BrowserCmd::Mouse {
            kind,
            x,
            y,
            button,
            clicks,
            dx,
            dy,
            modifiers,
        } => {
            let kind = match kind {
                MouseKind::Pressed => "mousePressed",
                MouseKind::Released => "mouseReleased",
                MouseKind::Moved => "mouseMoved",
                MouseKind::Wheel => "mouseWheel",
            };
            let button = match button {
                MouseButton::None => "none",
                MouseButton::Left => "left",
                MouseButton::Middle => "middle",
                MouseButton::Right => "right",
            };
            vec![(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": clicks,
                        "deltaX": dx, "deltaY": dy, "modifiers": modifiers, "pointerType": "mouse" }),
            )]
        }
        BrowserCmd::Touch { kind, points } => {
            let kind = match kind {
                TouchKind::Start => "touchStart",
                TouchKind::Move => "touchMove",
                TouchKind::End => "touchEnd",
                TouchKind::Cancel => "touchCancel",
            };
            let points: Vec<Value> = points
                .iter()
                .map(|p| json!({ "x": p.x, "y": p.y, "id": p.id, "radiusX": 4, "radiusY": 4, "force": 1 }))
                .collect();
            vec![(
                "Input.dispatchTouchEvent",
                json!({ "type": kind, "touchPoints": points }),
            )]
        }
        BrowserCmd::Pinch { x, y, scale } => vec![(
            "Input.synthesizePinchGesture",
            json!({ "x": x, "y": y, "scaleFactor": scale, "relativeSpeed": 1200, "gestureSourceType": "touch" }),
        )],
        BrowserCmd::Key {
            kind,
            key,
            code,
            text,
            modifiers,
        } => {
            let text = text.clone().or_else(|| match key.as_str() {
                "Enter" => Some("\r".into()),
                k if k.chars().count() == 1 => Some(k.to_string()),
                _ => None,
            });
            let vk = key_code(key);
            let base = |kind: &str, with_text: bool| {
                let mut params = json!({ "type": kind, "key": key, "modifiers": modifiers });
                if let Some(code) = code {
                    params["code"] = json!(code);
                }
                if let Some(vk) = vk {
                    params["windowsVirtualKeyCode"] = json!(vk);
                    params["nativeVirtualKeyCode"] = json!(vk);
                }
                if with_text {
                    if let Some(text) = &text {
                        params["text"] = json!(text);
                        params["unmodifiedText"] = json!(text);
                    }
                }
                params
            };
            // Ctrl/Alt/Meta combos must not type text.
            let types = text.is_some() && modifiers & 0b0111 == 0;
            let down = base(if types { "keyDown" } else { "rawKeyDown" }, types);
            let up = base("keyUp", false);
            match kind {
                KeyKind::Down => vec![("Input.dispatchKeyEvent", down)],
                KeyKind::Up => vec![("Input.dispatchKeyEvent", up)],
                KeyKind::Press => vec![
                    ("Input.dispatchKeyEvent", down),
                    ("Input.dispatchKeyEvent", up),
                ],
            }
        }
        BrowserCmd::Text { text } => vec![("Input.insertText", json!({ "text": text }))],
        _ => Vec::new(),
    }
}

/// `Emulation.setDeviceMetricsOverride` params for the phone's viewport.
pub fn metrics(viewport: &BrowserViewport) -> Value {
    json!({
        "width": viewport.width,
        "height": viewport.height,
        "deviceScaleFactor": viewport.scale,
        "mobile": viewport.mobile,
        "screenWidth": viewport.width,
        "screenHeight": viewport.height,
    })
}

/// Normalises what the user typed in the URL bar.
pub fn normalize_url(raw: &str) -> String {
    let text = raw.trim();
    let lower = text.to_ascii_lowercase();
    if lower.contains("://")
        || [
            "about:",
            "data:",
            "chrome:",
            "edge:",
            "view-source:",
            "file:",
        ]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return text.to_string();
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or("");
    let local = host.starts_with("localhost")
        || host.starts_with("127.")
        || host.starts_with("[::1]")
        || host.starts_with("0.0.0.0")
        || host.ends_with(".local");
    if local {
        return format!("http://{text}");
    }
    let looks_like_host = !text.contains(' ') && (host.contains('.') || host.contains(':'));
    if looks_like_host {
        format!("https://{text}")
    } else {
        let query: String = text
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                b' ' => "+".into(),
                other => format!("%{other:02X}"),
            })
            .collect();
        format!("https://duckduckgo.com/?q={query}")
    }
}

/// The `closed{reason}` a viewer gets for a deliberate shutdown.
pub fn close_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("idle") => "idle",
        Some("relaunch") => "relaunch",
        Some("host_exit") => "host_exit",
        _ => "closed",
    }
}

fn transport(e: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true)
}

fn cdp_error(e: String) -> AppErrorDto {
    super::unavailable("crashed", format!("The browser did not answer: {e}"))
}

struct Tab {
    target: String,
    session: String,
}

struct State {
    cdp: Cdp,
    tab: Tab,
    viewport: BrowserViewport,
    adapt: Adapt,
    seq: u64,
    in_flight: std::collections::VecDeque<(u64, Instant)>,
    /// Newest CDP frame held back while the phone is behind.
    held: Option<Arc<CdpEvent>>,
    url: String,
    title: String,
    loading: bool,
    /// Screencast (re)started and no frame since: kick a screenshot after a moment
    /// (a static page produces no screencast frames).
    kick_at: Option<Instant>,
    /// `Page.startScreencast` failed (seen right after `Page.navigate`: "Not attached to an
    /// active page"); retried when the page finishes loading.
    screencast_failed: bool,
    /// The visual viewport (offset in layout px, page scale) for mapping phone points.
    visual: VisualViewport,
}

/// Where the phone's picture sits in the page: a page without a mobile `<meta viewport>` is laid
/// out 980px wide and shown at ~0.4 scale, and pinch-zoom changes scale/offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualViewport {
    pub offset_x: f64,
    pub offset_y: f64,
    pub scale: f64,
}

impl Default for VisualViewport {
    fn default() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
        }
    }
}

impl VisualViewport {
    /// From `Page.getLayoutMetrics`.
    pub fn from_metrics(metrics: &Value) -> Option<Self> {
        let v = metrics.get("cssVisualViewport")?;
        let scale = v
            .get("scale")
            .and_then(Value::as_f64)
            .filter(|s| s.is_finite() && *s > 0.0)?;
        Some(Self {
            offset_x: v.get("offsetX").and_then(Value::as_f64).unwrap_or(0.0),
            offset_y: v.get("offsetY").and_then(Value::as_f64).unwrap_or(0.0),
            scale,
        })
    }

    /// A point on the phone's picture (CSS px of the emulated screen) → the layout-viewport
    /// point `Input.dispatchMouseEvent` hit-tests (it ignores the page scale: on a 0.4-scale
    /// page a tap on a link landed 2.5× too far up-left and missed).
    pub fn to_layout(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.offset_x + x / self.scale,
            self.offset_y + y / self.scale,
        )
    }
}

/// Maps a mouse command's point through the visual viewport (other commands unchanged).
pub fn map_input(cmd: &BrowserCmd, visual: &VisualViewport) -> BrowserCmd {
    match cmd {
        BrowserCmd::Mouse {
            kind,
            x,
            y,
            button,
            clicks,
            dx,
            dy,
            modifiers,
        } => {
            let (x, y) = visual.to_layout(*x, *y);
            BrowserCmd::Mouse {
                kind: *kind,
                x,
                y,
                button: *button,
                clicks: *clicks,
                dx: *dx,
                dy: *dy,
                modifiers: *modifiers,
            }
        }
        other => other.clone(),
    }
}

async fn send_event<W: AsyncWrite + Unpin>(
    write: &mut W,
    event: BrowserEvent,
) -> Result<(), AppErrorDto> {
    write_control(write, &Control::BrowserEvent { event })
        .await
        .map_err(transport)
}

impl State {
    async fn attach(
        cdp: &Cdp,
        target: &str,
        viewport: &BrowserViewport,
    ) -> Result<Tab, AppErrorDto> {
        let result = cdp
            .call(
                "Target.attachToTarget",
                json!({ "targetId": target, "flatten": true }),
                None,
            )
            .await
            .map_err(cdp_error)?;
        let session = result
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| cdp_error("no session id".into()))?
            .to_string();
        let s = Some(session.as_str());
        for method in ["Page.enable", "Inspector.enable"] {
            cdp.call(method, json!({}), s).await.map_err(cdp_error)?;
        }
        Self::apply_viewport(cdp, &session, viewport).await?;
        let _ = cdp
            .call("Target.activateTarget", json!({ "targetId": target }), None)
            .await;
        Ok(Tab {
            target: target.to_string(),
            session,
        })
    }

    async fn apply_viewport(
        cdp: &Cdp,
        session: &str,
        viewport: &BrowserViewport,
    ) -> Result<(), AppErrorDto> {
        let s = Some(session);
        cdp.call("Emulation.setDeviceMetricsOverride", metrics(viewport), s)
            .await
            .map_err(cdp_error)?;
        let _ = cdp
            .call(
                "Emulation.setTouchEmulationEnabled",
                json!({ "enabled": viewport.mobile, "maxTouchPoints": 5 }),
                s,
            )
            .await;
        Ok(())
    }

    async fn start_screencast(&mut self) {
        let v = &self.viewport;
        let _ = self
            .cdp
            .call("Page.bringToFront", json!({}), Some(&self.tab.session))
            .await;
        let result = self
            .cdp
            .call(
                "Page.startScreencast",
                json!({
                    "format": "jpeg",
                    "quality": self.adapt.quality,
                    "maxWidth": (v.width as f64 * v.scale).round() as u32,
                    "maxHeight": (v.height as f64 * v.scale).round() as u32,
                    "everyNthFrame": self.adapt.every_nth,
                }),
                Some(&self.tab.session),
            )
            .await;
        if let Err(error) = &result {
            tracing::warn!(op = "browser", %error, "startScreencast failed");
        }
        self.screencast_failed = result.is_err();
    }

    async fn refresh_visual(&mut self) {
        if let Ok(metrics) = self
            .cdp
            .call("Page.getLayoutMetrics", json!({}), Some(&self.tab.session))
            .await
        {
            if let Some(v) = VisualViewport::from_metrics(&metrics) {
                self.visual = v;
            }
        }
    }

    async fn restart_screencast(&mut self) {
        let _ = self
            .cdp
            .call("Page.stopScreencast", json!({}), Some(&self.tab.session))
            .await;
        self.held = None;
        self.start_screencast().await;
        self.kick_at = Some(Instant::now());
    }

    /// One frame via `Page.captureScreenshot` (initial frame of a static page).
    async fn kick<W: AsyncWrite + Unpin>(&mut self, write: &mut W) -> Result<(), AppErrorDto> {
        self.kick_at = None;
        let Ok(shot) = self
            .cdp
            .call(
                "Page.captureScreenshot",
                json!({ "format": "jpeg", "quality": self.adapt.quality, "optimizeForSpeed": true }),
                Some(&self.tab.session),
            )
            .await
        else {
            return Ok(());
        };
        let Some(data) = shot.get("data").and_then(Value::as_str) else {
            return Ok(());
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
            return Ok(());
        };
        let (w, h) = (self.viewport.width as f64, self.viewport.height as f64);
        self.send_jpeg(write, bytes, w, h).await
    }

    async fn send_jpeg<W: AsyncWrite + Unpin>(
        &mut self,
        write: &mut W,
        bytes: Vec<u8>,
        css_width: f64,
        css_height: f64,
    ) -> Result<(), AppErrorDto> {
        if bytes.len() > MAX_FRAME {
            self.adapt.oversize(Instant::now());
            self.restart_screencast().await;
            return Ok(());
        }
        self.seq += 1;
        let frame = BrowserFrameMeta {
            seq: self.seq,
            css_width,
            css_height,
            quality: self.adapt.quality,
        };
        write_control(
            write,
            &Control::BrowserEvent {
                event: BrowserEvent::Frame(frame),
            },
        )
        .await
        .map_err(transport)?;
        write
            .write_all(&encode(FrameKind::Output, &bytes))
            .await
            .map_err(transport)?;
        write.flush().await.map_err(transport)?;
        self.in_flight.push_back((self.seq, Instant::now()));
        // A kick scheduled for later (after input) still runs: the frame we
        // just sent may predate the input's effect.
        if self.kick_at.is_some_and(|t| t <= Instant::now()) {
            self.kick_at = None;
        }
        Ok(())
    }

    async fn page_event(&mut self) -> BrowserEvent {
        let (mut can_back, mut can_forward) = (false, false);
        if let Ok(history) = self
            .cdp
            .call(
                "Page.getNavigationHistory",
                json!({}),
                Some(&self.tab.session),
            )
            .await
        {
            let index = history
                .get("currentIndex")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let count = history
                .get("entries")
                .and_then(Value::as_array)
                .map(|e| e.len() as i64)
                .unwrap_or(0);
            can_back = index > 0;
            can_forward = index + 1 < count;
            if let Some(entry) = history.get("entries").and_then(|e| e.get(index as usize)) {
                if let Some(url) = entry.get("url").and_then(Value::as_str) {
                    self.url = url.to_string();
                }
                if let Some(title) = entry.get("title").and_then(Value::as_str) {
                    self.title = title.to_string();
                }
            }
        }
        // The target's title follows `document.title` (history entries don't).
        if let Ok(info) = self
            .cdp
            .call(
                "Target.getTargetInfo",
                json!({ "targetId": self.tab.target }),
                None,
            )
            .await
        {
            if let Some(title) = info.pointer("/targetInfo/title").and_then(Value::as_str) {
                self.title = title.to_string();
            }
        }
        BrowserEvent::Page {
            url: self.url.clone(),
            title: self.title.clone(),
            loading: self.loading,
            can_back,
            can_forward,
        }
    }

    async fn history_step(&self, delta: i64) {
        let s = Some(self.tab.session.as_str());
        let Ok(history) = self
            .cdp
            .call("Page.getNavigationHistory", json!({}), s)
            .await
        else {
            return;
        };
        let index = history
            .get("currentIndex")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            + delta;
        if let Some(id) = history
            .get("entries")
            .and_then(|e| e.get(index.max(0) as usize))
            .and_then(|e| e.get("id"))
        {
            if index >= 0 {
                let _ = self
                    .cdp
                    .call("Page.navigateToHistoryEntry", json!({ "entryId": id }), s)
                    .await;
            }
        }
    }

    async fn switch_to(&mut self, target: &str) -> Result<(), AppErrorDto> {
        let _ = self
            .cdp
            .call("Page.stopScreencast", json!({}), Some(&self.tab.session))
            .await;
        let _ = self
            .cdp
            .call(
                "Target.detachFromTarget",
                json!({ "sessionId": self.tab.session }),
                None,
            )
            .await;
        self.tab = Self::attach(&self.cdp, target, &self.viewport).await?;
        self.in_flight.clear();
        self.held = None;
        self.loading = false;
        self.start_screencast().await;
        self.kick_at = Some(Instant::now());
        Ok(())
    }

    /// Sends one CDP screencast frame to the phone (and acks CDP).
    async fn push_frame<W: AsyncWrite + Unpin>(
        &mut self,
        write: &mut W,
        event: &CdpEvent,
    ) -> Result<(), AppErrorDto> {
        let params = &event.params;
        if let Some(ack) = params.get("sessionId") {
            self.cdp.fire(
                "Page.screencastFrameAck",
                json!({ "sessionId": ack }),
                Some(&self.tab.session),
            );
        }
        let Some(data) = params.get("data").and_then(Value::as_str) else {
            return Ok(());
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
            return Ok(());
        };
        let meta = params.get("metadata").cloned().unwrap_or(Value::Null);
        let css_width = meta
            .get("deviceWidth")
            .and_then(Value::as_f64)
            .unwrap_or(self.viewport.width as f64);
        let css_height = meta
            .get("deviceHeight")
            .and_then(Value::as_f64)
            .unwrap_or(self.viewport.height as f64);
        // A frame rendered before the viewport override took effect: skip it.
        if (css_width - self.viewport.width as f64).abs() > 1.0 {
            self.kick_at.get_or_insert_with(Instant::now);
            return Ok(());
        }
        self.send_jpeg(write, bytes, css_width, css_height).await
    }
}

async fn tabs_event(running: &Running, active: &str) -> (Vec<BrowserTab>, BrowserEvent) {
    let tabs = running.tabs().await;
    (
        tabs.clone(),
        BrowserEvent::Tabs {
            tabs,
            active: active.to_string(),
        },
    )
}

/// Picks the tab to show: requested → last used → first page → a new one.
async fn pick_target(
    running: &Running,
    manager: &Instance,
    requested: Option<&str>,
) -> Result<String, AppErrorDto> {
    let tabs = running.tabs().await;
    let last = manager
        .last_target
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for wanted in [requested.map(str::to_string), last].into_iter().flatten() {
        if tabs.iter().any(|t| t.id == wanted) {
            return Ok(wanted);
        }
    }
    if let Some(first) = tabs.first() {
        return Ok(first.id.clone());
    }
    let created = running
        .cdp
        .call("Target.createTarget", json!({ "url": "about:blank" }), None)
        .await
        .map_err(cdp_error)?;
    created
        .get("targetId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| cdp_error("no target id".into()))
}

/// Serves one `browser_open` stream until the phone hangs up, the browser
/// quits or the host shuts it down.
#[allow(clippy::too_many_arguments)]
pub async fn serve<R, W>(
    manager: Arc<Instance>,
    running: Arc<Running>,
    _viewer: ViewerGuard,
    read: &mut R,
    write: &mut W,
    viewport: BrowserViewport,
    url: Option<String>,
    target: Option<String>,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // After the start: its own relaunch announcement is for the other viewers.
    let mut shutdown = manager.subscribe_shutdown();
    let cdp = running.cdp.clone();
    let mut events = cdp.subscribe();
    let target = pick_target(&running, &manager, target.as_deref()).await?;
    let tab = State::attach(&cdp, &target, &viewport).await?;
    *manager
        .last_target
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(target.clone());
    let mut state = State {
        cdp: cdp.clone(),
        tab,
        viewport,
        adapt: Adapt::default(),
        seq: 0,
        in_flight: Default::default(),
        held: None,
        url: String::new(),
        title: String::new(),
        loading: false,
        kick_at: None,
        screencast_failed: false,
        visual: VisualViewport::default(),
    };
    if let Some(url) = url {
        let _ = cdp
            .call(
                "Page.navigate",
                json!({ "url": normalize_url(&url) }),
                Some(&state.tab.session),
            )
            .await;
    }
    let (tabs, tabs_ev) = tabs_event(&running, &state.tab.target).await;
    let open = BrowserOpenData {
        browser: running.info.clone(),
        headless: running.headless,
        target: state.tab.target.clone(),
        tabs,
        engine: Some(running.engine.as_str().into()),
        stealth: Some(running.stealth),
        stealth_reason: running.stealth_reason.clone(),
        stealth_note: running.stealth_note(),
        profile: Some(manager.name.clone()),
        agent: Some(manager.agent(Some(&running))),
        display: Some(running.placement.as_str().into()),
    };
    write_control(
        write,
        &Control::Ok {
            data: Some(serde_json::to_value(&open).map_err(transport)?),
        },
    )
    .await
    .map_err(transport)?;
    send_event(write, tabs_ev).await?;
    let page = state.page_event().await;
    send_event(write, page.clone()).await?;
    state.start_screencast().await;
    state.kick_at = Some(Instant::now());

    let mut phone = Box::pin(futures_util::stream::unfold(read, |read| async move {
        read_frame(read).await.ok().map(|frame| (frame, read))
    }));
    let mut tabs_dirty: Option<Instant> = None;
    let mut page_dirty = false;
    // A cheap "did the title/url change?" probe, scheduled after input and on frames.
    let mut page_check: Option<Instant> = None;
    let mut last_page_check = Instant::now();
    let mut last_sent_page: Option<BrowserEvent> = Some(page);
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    // UNI-296: "Agent is driving" / taken over → `agent` events on change.
    let mut agent_changed = manager.gate.subscribe();
    agent_changed.mark_unchanged();
    let mut last_agent = open.agent;
    let mut agent_check = Instant::now();

    let outcome = loop {
        tokio::select! {
            frame = phone.next() => {
                let Some((kind, bytes)) = frame else { break "phone" };
                if kind != 0 { continue; }
                let Ok(Control::BrowserCmd { cmd }) = serde_json::from_slice::<Control>(&bytes) else { continue };
                if let Err(why) = cmd.check() {
                    tracing::debug!(op = "browser", %why, "bad command");
                    continue;
                }
                let s = state.tab.session.clone();
                let s = Some(s.as_str());
                if !matches!(cmd, BrowserCmd::Ack { .. } | BrowserCmd::Viewport { .. }) {
                    // The user's own input keeps a take-over alive.
                    manager.gate.note_user_input();
                }
                match &cmd {
                    BrowserCmd::Ack { seq } => {
                        while let Some(&(sent, at)) = state.in_flight.front() {
                            if sent > *seq { break; }
                            state.in_flight.pop_front();
                            if sent == *seq && state.adapt.sample(at.elapsed(), Instant::now()) {
                                state.restart_screencast().await;
                            }
                        }
                        if state.in_flight.len() < MAX_IN_FLIGHT {
                            if let Some(held) = state.held.take() {
                                state.push_frame(write, &held).await?;
                            }
                        }
                    }
                    BrowserCmd::Navigate { url } => {
                        let _ = cdp.call("Page.navigate", json!({ "url": normalize_url(url.as_str()) }), s).await;
                        page_dirty = true;
                    }
                    BrowserCmd::Back => { state.history_step(-1).await; page_dirty = true; }
                    BrowserCmd::Forward => { state.history_step(1).await; page_dirty = true; }
                    BrowserCmd::Reload => { let _ = cdp.call("Page.reload", json!({}), s).await; }
                    BrowserCmd::Stop => { let _ = cdp.call("Page.stopLoading", json!({}), s).await; }
                    BrowserCmd::TabNew { url } => {
                        let url = url.as_ref().map(|u| normalize_url(u.as_str())).unwrap_or_else(|| "about:blank".into());
                        if let Ok(created) = cdp.call("Target.createTarget", json!({ "url": url }), None).await {
                            if let Some(id) = created.get("targetId").and_then(Value::as_str) {
                                state.switch_to(id).await?;
                                *manager.last_target.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.to_string());
                                page_dirty = true;
                            }
                        }
                        tabs_dirty = Some(Instant::now());
                    }
                    BrowserCmd::TabClose { id } => {
                        let _ = cdp.call("Target.closeTarget", json!({ "targetId": id }), None).await;
                        tabs_dirty = Some(Instant::now());
                    }
                    BrowserCmd::TabActivate { id } => {
                        if *id != state.tab.target {
                            state.switch_to(id).await?;
                            *manager.last_target.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
                            page_dirty = true;
                            tabs_dirty = Some(Instant::now());
                        }
                    }
                    BrowserCmd::Viewport { viewport } => {
                        state.viewport = *viewport;
                        State::apply_viewport(&cdp, &state.tab.session, viewport).await?;
                        state.restart_screencast().await;
                    }
                    other => {
                        // A press (and a scroll) starts a gesture: re-read the page scale then, so
                        // later moves/releases of the same gesture map the same way.
                        if matches!(other, BrowserCmd::Mouse { kind: unipi_proto::browser::MouseKind::Pressed | unipi_proto::browser::MouseKind::Wheel, .. }) {
                            state.refresh_visual().await;
                        }
                        let mapped = map_input(other, &state.visual);
                        for (method, params) in input_calls(&mapped) {
                            cdp.fire(method, params, s);
                        }
                        // Input often retitles/navigates without a CDP event.
                        page_check = Some(Instant::now() + Duration::from_millis(300));
                        // Some builds stall the screencast until the next paint; make sure
                        // the phone sees the result of its input.
                        state.kick_at = Some(Instant::now() + Duration::from_millis(200));
                    }
                }
            }
            event = events.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break "browser",
                };
                let mine = event.session_id.as_deref() == Some(state.tab.session.as_str());
                match event.method.as_str() {
                    "Page.screencastFrame" if mine => {
                        if page_check.is_none() && last_page_check.elapsed() > Duration::from_secs(1) {
                            page_check = Some(Instant::now());
                        }
                        if state.in_flight.len() < MAX_IN_FLIGHT {
                            state.push_frame(write, &event).await?;
                        } else {
                            // Phone is behind: keep only the newest, un-acked to CDP so
                            // Chrome pauses until the phone catches up.
                            state.held = Some(event.clone());
                        }
                    }
                    "Page.frameStartedLoading" if mine && event.params.get("frameId").and_then(Value::as_str) == Some(state.tab.target.as_str()) => {
                        state.loading = true;
                        page_dirty = true;
                        // Tell the phone right away (a fast load may finish before the next tick).
                        if let Some(BrowserEvent::Page { url, title, can_back, can_forward, .. }) = last_sent_page.clone() {
                            let ev = BrowserEvent::Page { url, title, loading: true, can_back, can_forward };
                            last_sent_page = Some(ev.clone());
                            send_event(write, ev).await?;
                        }
                    }
                    "Page.frameStoppedLoading" if mine && event.params.get("frameId").and_then(Value::as_str) == Some(state.tab.target.as_str()) => {
                        state.loading = false;
                        page_dirty = true;
                        if state.screencast_failed {
                            state.restart_screencast().await;
                        }
                        // Tell the phone right away (a fast load may finish before the next tick).
                        if let Some(BrowserEvent::Page { url, title, can_back, can_forward, .. }) = last_sent_page.clone() {
                            let ev = BrowserEvent::Page { url, title, loading: false, can_back, can_forward };
                            last_sent_page = Some(ev.clone());
                            send_event(write, ev).await?;
                        }
                    }
                    "Page.frameNavigated" | "Page.navigatedWithinDocument" | "Page.loadEventFired" if mine => {
                        page_dirty = true;
                        if state.screencast_failed && event.method == "Page.loadEventFired" {
                            state.restart_screencast().await;
                        }
                    }
                    "Inspector.targetCrashed" if mine => {
                        send_event(write, BrowserEvent::Crashed { message: "The page crashed. Reload to try again.".into() }).await?;
                    }
                    "Target.targetCreated" | "Target.targetDestroyed" | "Target.targetInfoChanged" => {
                        let info = event.params.get("targetInfo");
                        let id = info.and_then(|i| i.get("targetId")).or(event.params.get("targetId")).and_then(Value::as_str);
                        if id == Some(state.tab.target.as_str()) {
                            if event.method == "Target.targetDestroyed" {
                                // Our tab closed: show another (or a fresh one).
                                let next = pick_target(&running, &manager, None).await?;
                                state.switch_to(&next).await?;
                                *manager.last_target.lock().unwrap_or_else(|e| e.into_inner()) = Some(next);
                            }
                            page_dirty = true;
                        }
                        tabs_dirty = Some(Instant::now());
                    }
                    _ => {}
                }
            }
            reason = shutdown.recv() => {
                break close_reason(reason.ok().as_deref());
            }
            _ = cdp.closed() => {
                // A relaunch closes the old browser right after announcing it.
                match shutdown.try_recv() {
                    Ok(reason) => break close_reason(Some(&reason)),
                    Err(_) => break "browser",
                }
            }
            changed = agent_changed.changed() => {
                if changed.is_ok() {
                    agent_check = Instant::now() - Duration::from_secs(1);
                }
            }
            _ = tick.tick() => {
                // Driving fades after a few seconds without a change event: re-read twice a second.
                if agent_check.elapsed() >= Duration::from_millis(500) {
                    agent_check = Instant::now();
                    let agent = manager.agent(Some(&running));
                    if last_agent != Some(agent) {
                        last_agent = Some(agent);
                        send_event(write, BrowserEvent::Agent { agent }).await?;
                    }
                }
                if state.kick_at.is_some_and(|t| t.elapsed() > Duration::from_millis(400)) && state.in_flight.len() < MAX_IN_FLIGHT {
                    state.kick(write).await?;
                }
                if tabs_dirty.is_some_and(|t| t.elapsed() > Duration::from_millis(150)) {
                    tabs_dirty = None;
                    let (_, ev) = tabs_event(&running, &state.tab.target).await;
                    send_event(write, ev).await?;
                }
                let probe = page_check.is_some_and(|t| Instant::now() >= t);
                if page_dirty || probe {
                    let forced = page_dirty;
                    page_dirty = false;
                    page_check = None;
                    last_page_check = Instant::now();
                    let ev = state.page_event().await;
                    if forced || last_sent_page.as_ref() != Some(&ev) {
                        last_sent_page = Some(ev.clone());
                        send_event(write, ev).await?;
                    }
                }
            }
        }
    };
    tracing::info!(op = "browser_open", outcome, frames = state.seq, "closed");
    match outcome {
        "phone" => {
            let _ = cdp
                .call("Page.stopScreencast", json!({}), Some(&state.tab.session))
                .await;
            let _ = cdp
                .call(
                    "Target.detachFromTarget",
                    json!({ "sessionId": state.tab.session }),
                    None,
                )
                .await;
        }
        "browser" => {
            let _ = send_event(
                write,
                BrowserEvent::Crashed {
                    message: "The browser on the PC quit.".into(),
                },
            )
            .await;
        }
        reason => {
            let _ = send_event(
                write,
                BrowserEvent::Closed {
                    reason: reason.to_string(),
                },
            )
            .await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unipi_proto::browser::{KeyKind, MouseButton, MouseKind, TouchKind, TouchPoint};

    #[test]
    fn adapt_lowers_on_slow_acks_and_recovers() {
        let mut a = Adapt::default();
        let t0 = Instant::now();
        assert!(a.sample(Duration::from_millis(900), t0));
        assert!(a.quality < 70 && a.every_nth == 2);
        // Within 3 s: no change.
        assert!(!a.sample(Duration::from_millis(900), t0 + Duration::from_secs(1)));
        let mut t = t0;
        for _ in 0..40 {
            t += Duration::from_secs(4);
            a.sample(Duration::from_millis(1500), t);
        }
        assert_eq!(a.quality, Adapt::MIN_QUALITY);
        assert_eq!(a.every_nth, 4);
        for _ in 0..80 {
            t += Duration::from_secs(4);
            a.sample(Duration::from_millis(30), t);
        }
        assert_eq!(a.quality, Adapt::MAX_QUALITY);
        assert_eq!(a.every_nth, 1);
        a.oversize(t);
        assert_eq!(a.quality, 60);
    }

    #[test]
    fn mouse_and_touch_map_to_cdp() {
        let calls = input_calls(&BrowserCmd::Mouse {
            kind: MouseKind::Pressed,
            x: 10.0,
            y: 20.0,
            button: MouseButton::Left,
            clicks: 1,
            dx: 0.0,
            dy: 0.0,
            modifiers: 0,
        });
        assert_eq!(calls[0].0, "Input.dispatchMouseEvent");
        assert_eq!(calls[0].1["type"], "mousePressed");
        assert_eq!(calls[0].1["button"], "left");
        assert_eq!(calls[0].1["clickCount"], 1);
        let calls = input_calls(&BrowserCmd::Touch {
            kind: TouchKind::Move,
            points: vec![TouchPoint {
                x: 1.0,
                y: 2.0,
                id: 3,
            }],
        });
        assert_eq!(calls[0].1["type"], "touchMove");
        assert_eq!(calls[0].1["touchPoints"][0]["id"], 3);
        let calls = input_calls(&BrowserCmd::Touch {
            kind: TouchKind::End,
            points: vec![],
        });
        assert_eq!(calls[0].1["touchPoints"], json!([]));
        let calls = input_calls(&BrowserCmd::Pinch {
            x: 1.0,
            y: 2.0,
            scale: 2.0,
        });
        assert_eq!(calls[0].0, "Input.synthesizePinchGesture");
    }

    #[test]
    fn mouse_points_map_through_the_visual_viewport() {
        // Seen on the Redmi: a desktop-only page (no meta viewport) laid out 980px wide at 0.401
        // scale; a tap on a link at (25,72) on the picture reached the page at (25,72) layout px
        // and missed — the link sits at layout (8..119, 153..198).
        let v = VisualViewport::from_metrics(&json!({
            "cssVisualViewport": { "offsetX": 0, "offsetY": 0, "pageX": 0, "pageY": 0, "clientWidth": 980, "clientHeight": 1665.75, "scale": 0.4010204, "zoom": 1 }
        }))
        .unwrap();
        let tap = BrowserCmd::Mouse {
            kind: MouseKind::Pressed,
            x: 25.0,
            y: 72.0,
            button: MouseButton::Left,
            clicks: 1,
            dx: 0.0,
            dy: 0.0,
            modifiers: 0,
        };
        let BrowserCmd::Mouse { x, y, .. } = map_input(&tap, &v) else {
            panic!()
        };
        assert!(
            (8.0..119.0).contains(&x) && (153.0..198.0).contains(&y),
            "({x},{y})"
        );
        // Pinch-zoomed and panned.
        let zoomed = VisualViewport {
            offset_x: 100.0,
            offset_y: 50.0,
            scale: 2.0,
        };
        assert_eq!(zoomed.to_layout(10.0, 20.0), (105.0, 60.0));
        // Scale 1 (a mobile page): unchanged.
        assert_eq!(
            VisualViewport::default().to_layout(10.0, 20.0),
            (10.0, 20.0)
        );
        // Bad metrics are ignored.
        assert!(
            VisualViewport::from_metrics(&json!({ "cssVisualViewport": { "scale": 0 } })).is_none()
        );
        assert!(VisualViewport::from_metrics(&json!({})).is_none());
        // Non-mouse commands pass through.
        let text = BrowserCmd::Text { text: "hi".into() };
        assert_eq!(map_input(&text, &zoomed), text);
    }

    #[test]
    fn keys_map_to_cdp() {
        let key = |key: &str, modifiers| BrowserCmd::Key {
            kind: KeyKind::Press,
            key: key.into(),
            code: None,
            text: None,
            modifiers,
        };
        let enter = input_calls(&key("Enter", 0));
        assert_eq!(enter.len(), 2);
        assert_eq!(enter[0].1["type"], "keyDown");
        assert_eq!(enter[0].1["text"], "\r");
        assert_eq!(enter[0].1["windowsVirtualKeyCode"], 13);
        assert_eq!(enter[1].1["type"], "keyUp");
        let back = input_calls(&key("Backspace", 0));
        assert_eq!(back[0].1["type"], "rawKeyDown");
        assert!(back[0].1.get("text").is_none());
        assert_eq!(back[0].1["windowsVirtualKeyCode"], 8);
        let ctrl_a = input_calls(&key("a", 2));
        assert_eq!(ctrl_a[0].1["type"], "rawKeyDown");
        assert_eq!(ctrl_a[0].1["windowsVirtualKeyCode"], 65);
        assert_eq!(ctrl_a[0].1["modifiers"], 2);
        let text = input_calls(&BrowserCmd::Text {
            text: "héllo".into(),
        });
        assert_eq!(text, vec![("Input.insertText", json!({ "text": "héllo" }))]);
        assert_eq!(key_code("F5"), Some(116));
        assert_eq!(key_code("ArrowDown"), Some(40));
        assert_eq!(key_code("Unidentified"), None);
    }

    #[test]
    fn urls_are_normalised() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("localhost:5173/x"), "http://localhost:5173/x");
        assert_eq!(normalize_url("127.0.0.1:8000"), "http://127.0.0.1:8000");
        assert_eq!(normalize_url("https://a.b/c"), "https://a.b/c");
        assert_eq!(normalize_url("about:blank"), "about:blank");
        assert_eq!(
            normalize_url("rust async book"),
            "https://duckduckgo.com/?q=rust+async+book"
        );
    }

    #[test]
    fn metrics_carry_scale_and_mobile() {
        let m = metrics(&BrowserViewport {
            width: 412,
            height: 780,
            scale: 2.625,
            mobile: true,
        });
        assert_eq!(m["width"], 412);
        assert_eq!(m["deviceScaleFactor"], 2.625);
        assert_eq!(m["mobile"], true);
    }
}
