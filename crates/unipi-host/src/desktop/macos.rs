//! macOS capture + input (UNI-246 H4): `CGDisplayCreateImage` of the main
//! display (cursor drawn in from `CGEventGetLocation` — the system image has
//! no cursor), input through `CGEvent` posted at the HID tap.
//!
//! Frames go through the same openh264 encoder as Linux/Windows, so the wire
//! format (H.264 Constrained Baseline, Annex-B) is identical and every viewer
//! decodes it unchanged. (VideoToolbox would save CPU; a later step.)
//! `CGDisplayCreateImage` is deprecated in macOS 15 in favour of
//! ScreenCaptureKit but still works there; it needs no Objective-C runtime,
//! run loop or GPU queue, which keeps the host a plain CLI.
//!
//! Permissions (System Settings → Privacy & Security), granted once to the
//! `unipi-host` binary (or the terminal that starts it):
//! - **Screen & System Audio Recording** — for any picture. Without it macOS
//!   hands back only the wallpaper + menu bar; the host detects that with
//!   `CGPreflightScreenCaptureAccess` and refuses with `denied` + a hint
//!   (and asks macOS to show its prompt once, `CGRequestScreenCaptureAccess`).
//! - **Accessibility** — for mouse/keyboard (not needed for view-only).
//!   Without it `CGEventPost` is silently dropped; checked with
//!   `AXIsProcessTrustedWithOptions` (prompting once).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_graphics::display::CGDisplay;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use tokio::sync::{mpsc, watch};
use unipi_proto::{AppErrorDto, PointerButton};

use super::capture::{Capture, Input, StartOptions};
use super::config::unavailable;
use super::encode::Frame;
use super::keys::{mac, KeyAction, KeyPlanner};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const std::ffi::c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDictionaryCreate(
        allocator: *const std::ffi::c_void,
        keys: *const *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        count: isize,
        key_callbacks: *const std::ffi::c_void,
        value_callbacks: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
    fn CFRelease(cf: *const std::ffi::c_void);
    static kCFBooleanTrue: *const std::ffi::c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

pub const SCREEN_HINT: &str = "On the Mac: System Settings → Privacy & Security → Screen & System Audio Recording → turn on unipi-host (or the Terminal that runs it), then restart unipi-host.";
pub const INPUT_HINT: &str = "For mouse and keyboard: System Settings → Privacy & Security → Accessibility → turn on unipi-host (or the Terminal that runs it). Watch-only works without it.";

/// Screen Recording granted? Asks macOS to show its prompt once if not.
pub fn screen_access(prompt: bool) -> bool {
    unsafe {
        if CGPreflightScreenCaptureAccess() {
            return true;
        }
        if prompt {
            CGRequestScreenCaptureAccess();
        }
        false
    }
}

/// Accessibility (input injection) granted? `prompt` shows the system dialog.
pub fn input_access(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrustedWithOptions(std::ptr::null());
        }
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const u8 as *const _,
            &kCFTypeDictionaryValueCallBacks as *const u8 as *const _,
        );
        let trusted = AXIsProcessTrustedWithOptions(dict);
        CFRelease(dict);
        trusted
    }
}

/// Grabs the main display as tight BGRA at its native pixel size, with the
/// cursor drawn as a small arrow (`scale` = pixels per point).
fn grab(display: &CGDisplay) -> Result<Frame, String> {
    let image = display.image().ok_or("CGDisplayCreateImage returned nothing")?;
    let (w, h) = (image.width() as u32, image.height() as u32);
    if image.bits_per_pixel() != 32 {
        return Err(format!("unexpected pixel format ({} bpp)", image.bits_per_pixel()));
    }
    let data = image.data();
    let bytes = data.bytes();
    let mut frame = Frame::from_strided(w, h, image.bytes_per_row(), bytes).ok_or("short image data")?;
    // The cursor (points → pixels).
    let bounds = display.bounds();
    let scale = if bounds.size.width > 0.0 { w as f64 / bounds.size.width } else { 1.0 };
    if let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        if let Ok(event) = CGEvent::new(source) {
            let p = event.location();
            draw_cursor(&mut frame, ((p.x - bounds.origin.x) * scale) as i64, ((p.y - bounds.origin.y) * scale) as i64, scale.max(1.0));
        }
    }
    Ok(frame)
}

/// A plain arrow cursor (white with a black outline), tip at (x, y).
pub fn draw_cursor(frame: &mut Frame, x: i64, y: i64, scale: f64) {
    const ARROW: [&str; 16] = [
        "X..........",
        "XX.........",
        "XOX........",
        "XOOX.......",
        "XOOOX......",
        "XOOOOX.....",
        "XOOOOOX....",
        "XOOOOOOX...",
        "XOOOOOOOX..",
        "XOOOOOOOOX.",
        "XOOOOOXXXXX",
        "XOOXOOX....",
        "XOX.XOOX...",
        "XX..XOOX...",
        "X....XOOX..",
        ".....XXX...",
    ];
    let s = scale.round().max(1.0) as i64;
    let (w, h) = (frame.width as i64, frame.height as i64);
    for (row, line) in ARROW.iter().enumerate() {
        for (col, c) in line.bytes().enumerate() {
            let rgb = match c {
                b'X' => 0u8,
                b'O' => 255u8,
                _ => continue,
            };
            for dy in 0..s {
                for dx in 0..s {
                    let (px, py) = (x + col as i64 * s + dx, y + row as i64 * s + dy);
                    if px >= 0 && py >= 0 && px < w && py < h {
                        let i = ((py * w + px) * 4) as usize;
                        frame.bgra[i] = rgb;
                        frame.bgra[i + 1] = rgb;
                        frame.bgra[i + 2] = rgb;
                    }
                }
            }
        }
    }
}

struct Injector {
    planner: KeyPlanner,
    /// Display bounds in points (global coordinates).
    origin: (f64, f64),
    size: (f64, f64),
    pos: CGPoint,
    buttons: [bool; 3],
    last_click: Option<(Instant, CGPoint, u8)>,
    clicks: i64,
}

impl Injector {
    fn source() -> Option<CGEventSource> {
        CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()
    }

    fn flags(&self) -> CGEventFlags {
        CGEventFlags::from_bits_truncate(self.planner.held().iter().map(|k| mac::flag(*k)).fold(0, |a, b| a | b))
    }

    fn mouse(&self, kind: CGEventType, button: CGMouseButton) {
        let Some(source) = Self::source() else { return };
        if let Ok(event) = CGEvent::new_mouse_event(source, kind, self.pos, button) {
            if matches!(kind, CGEventType::LeftMouseDown | CGEventType::LeftMouseUp | CGEventType::RightMouseDown | CGEventType::RightMouseUp | CGEventType::OtherMouseDown | CGEventType::OtherMouseUp) {
                // kCGMouseEventClickState: double clicks need the count.
                event.set_integer_value_field(1, self.clicks);
            }
            event.set_flags(self.flags());
            event.post(CGEventTapLocation::HID);
        }
    }

    fn key_code(&self, code: u16, down: bool) {
        let Some(source) = Self::source() else { return };
        if let Ok(event) = CGEvent::new_keyboard_event(source, code, down) {
            event.set_flags(self.flags());
            event.post(CGEventTapLocation::HID);
        }
    }

    fn apply(&mut self, input: Input) {
        match input {
            Input::Pointer { x, y } => {
                self.pos = CGPoint::new(self.origin.0 + x.clamp(0.0, 1.0) * self.size.0, self.origin.1 + y.clamp(0.0, 1.0) * self.size.1);
                let (kind, button) = if self.buttons[0] {
                    (CGEventType::LeftMouseDragged, CGMouseButton::Left)
                } else if self.buttons[2] {
                    (CGEventType::RightMouseDragged, CGMouseButton::Right)
                } else if self.buttons[1] {
                    (CGEventType::OtherMouseDragged, CGMouseButton::Center)
                } else {
                    (CGEventType::MouseMoved, CGMouseButton::Left)
                };
                self.mouse(kind, button);
            }
            Input::Button { button, down } => {
                let (i, kind, b) = match (button, down) {
                    (PointerButton::Left, true) => (0, CGEventType::LeftMouseDown, CGMouseButton::Left),
                    (PointerButton::Left, false) => (0, CGEventType::LeftMouseUp, CGMouseButton::Left),
                    (PointerButton::Middle, true) => (1, CGEventType::OtherMouseDown, CGMouseButton::Center),
                    (PointerButton::Middle, false) => (1, CGEventType::OtherMouseUp, CGMouseButton::Center),
                    (PointerButton::Right, true) => (2, CGEventType::RightMouseDown, CGMouseButton::Right),
                    (PointerButton::Right, false) => (2, CGEventType::RightMouseUp, CGMouseButton::Right),
                };
                if down {
                    let now = Instant::now();
                    self.clicks = match self.last_click {
                        Some((at, p, bi)) if bi == i as u8 && now - at < Duration::from_millis(500) && (p.x - self.pos.x).abs() < 5.0 && (p.y - self.pos.y).abs() < 5.0 => self.clicks + 1,
                        _ => 1,
                    };
                    self.last_click = Some((now, self.pos, i as u8));
                }
                self.buttons[i] = down;
                self.mouse(kind, b);
            }
            Input::Scroll { dx, dy } => {
                let Some(source) = Self::source() else { return };
                // Positive = down/right on the wire; CG's wheel1 positive = up.
                if let Ok(event) = CGEvent::new_scroll_event(source, ScrollEventUnit::LINE, 2, -dy * 3, -dx * 3, 0) {
                    event.post(CGEventTapLocation::HID);
                }
            }
            Input::Key { keysym, down } => match self.planner.plan(keysym, down) {
                None => {}
                Some(KeyAction::Special { key, down }) => self.key_code(mac::special(key), down),
                Some(KeyAction::Chord { ch, down }) => {
                    if let Some(code) = mac::char_key(ch) {
                        self.key_code(code, down);
                    }
                }
                Some(KeyAction::Text { ch }) => {
                    // A keyboard event carrying the Unicode string: any
                    // character, independent of the Mac's layout.
                    let Some(source) = Self::source() else { return };
                    let mut buf = [0u16; 2];
                    let units = ch.encode_utf16(&mut buf);
                    for down in [true, false] {
                        if let Ok(event) = CGEvent::new_keyboard_event(source.clone(), 0, down) {
                            event.set_string_from_utf16_unchecked(units);
                            event.post(CGEventTapLocation::HID);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
            },
        }
    }
}

struct Stop(Arc<AtomicBool>);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub async fn start(options: StartOptions) -> Result<Capture, AppErrorDto> {
    if !screen_access(true) {
        return Err(unavailable("denied", "macOS hasn't allowed unipi-host to record the screen").with_hint(SCREEN_HINT));
    }
    let display = CGDisplay::main();
    let probe = tokio::task::spawn_blocking(move || grab(&CGDisplay::main()).map(|f| (f.width, f.height)))
        .await
        .map_err(|e| unavailable("capture_failed", e.to_string()))?;
    let (width, height) = probe.map_err(|e| unavailable("capture_failed", format!("Screen capture failed: {e}")))?;
    let bounds = display.bounds();

    let input = if options.view_only {
        None
    } else {
        if !input_access(true) {
            return Err(unavailable("denied", "macOS hasn't allowed unipi-host to control the mouse and keyboard").with_hint(INPUT_HINT));
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
        let mut injector = Injector {
            planner: KeyPlanner::default(),
            origin: (bounds.origin.x, bounds.origin.y),
            size: (bounds.size.width, bounds.size.height),
            pos: CGPoint::new(bounds.origin.x + bounds.size.width / 2.0, bounds.origin.y + bounds.size.height / 2.0),
            buttons: [false; 3],
            last_click: None,
            clicks: 1,
        };
        std::thread::Builder::new()
            .name("desktop-mac-input".into())
            .spawn(move || {
                while let Some(input) = rx.blocking_recv() {
                    injector.apply(input);
                }
                // Viewer left: release anything still held.
                for key in injector.planner.held().to_vec() {
                    injector.key_code(mac::special(key), false);
                }
            })
            .map_err(|e| unavailable("capture_failed", e.to_string()))?;
        Some(tx)
    };

    let (tx, rx) = watch::channel(None);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let period = Duration::from_millis(1000 / options.fps.clamp(1, 60) as u64);
    std::thread::Builder::new()
        .name("desktop-mac-grab".into())
        .spawn(move || {
            let display = CGDisplay::main();
            let mut failures = 0;
            let mut dedup = super::capture::Dedup::default();
            while !stopped.load(Ordering::SeqCst) {
                let started = Instant::now();
                match grab(&display) {
                    Ok(frame) => {
                        failures = 0;
                        if (frame.width, frame.height) != (width, height) {
                            tracing::info!(op = "desktop", "display size changed; ending capture");
                            break;
                        }
                        if let Some(frame) = dedup.check(frame) {
                            if tx.send(Some(frame)).is_err() {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        failures += 1;
                        if failures == 1 || failures % 50 == 0 {
                            tracing::warn!(%error, failures, "mac grab failed");
                        }
                        if failures > 300 {
                            break;
                        }
                    }
                }
                std::thread::sleep(period.saturating_sub(started.elapsed()));
            }
        })
        .map_err(|e| unavailable("capture_failed", e.to_string()))?;
    Ok(Capture { backend: "macos", width, height, frames: rx, input, display: None, guard: Box::new(Stop(stop)) })
}
