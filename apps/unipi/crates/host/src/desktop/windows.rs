//! Windows capture + input (UNI-247): GDI `BitBlt` of the virtual primary
//! screen into a DIB section (BGRA, the cursor drawn in with `DrawIconEx`),
//! input through `SendInput`.
//!
//! GDI is used rather than DXGI Desktop Duplication / Windows.Graphics.Capture
//! because it works in every interactive session (console, RDP, a VM without
//! a GPU driver) with no COM/WinRT setup; at the ladder's 10–30 fps it costs a
//! few ms per frame. The host must run in the user's session (not as a
//! service in session 0, which has no desktop). The secure desktop (UAC
//! prompts, Ctrl+Alt+Del, the lock screen) can't be captured or driven from
//! a normal process; frames freeze while it shows.
//!
//! Text is typed as Unicode (`KEYEVENTF_UNICODE`), so any layout works;
//! chords with Ctrl/Alt/Win use virtual-key codes (see `keys.rs`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};
use unipi_proto::{AppErrorDto, PointerButton};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, SRCCOPY,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, VkKeyScanW, MAPVK_VK_TO_VSC, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, GetSystemMetrics, CURSORINFO, CURSOR_SHOWING, DI_NORMAL, ICONINFO, SM_CXSCREEN, SM_CYSCREEN,
};

use super::capture::{Capture, Input, StartOptions};
use super::config::unavailable;
use super::encode::Frame;
use super::keys::{win, KeyAction, KeyPlanner};

const WHEEL_DELTA: i32 = 120;
/// Pause between injected key events of one typed character (see the input thread).
const KEY_PACE_MS: u64 = 12;

/// Per-monitor DPI awareness, so GDI and `GetSystemMetrics` speak physical
/// pixels (else a 150 % screen is captured at 2/3 size, blurred). Once per
/// process; harmless if the manifest already set it.
fn dpi_aware() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
        unsafe {
            windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(-4isize as _);
        }
    });
}

fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// A screen DC + a memory DC with a top-down 32-bit DIB.
struct Grabber {
    screen: HDC,
    mem: HDC,
    bitmap: HBITMAP,
    old: *mut core::ffi::c_void,
    bits: *mut u8,
    width: i32,
    height: i32,
}

// The handles are only used from the grab thread that owns the Grabber.
unsafe impl Send for Grabber {}

impl Grabber {
    fn new(width: i32, height: i32) -> Result<Grabber, String> {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            if screen.is_null() {
                return Err("GetDC failed (no interactive desktop — is unipi-host running in your session, not as a service?)".into());
            }
            let mem = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(mem);
                ReleaseDC(std::ptr::null_mut(), screen);
                return Err("CreateDIBSection failed".into());
            }
            let old = SelectObject(mem, bitmap);
            Ok(Grabber { screen, mem, bitmap, old, bits: bits as *mut u8, width, height })
        }
    }

    fn grab(&mut self) -> Result<Frame, String> {
        unsafe {
            if BitBlt(self.mem, 0, 0, self.width, self.height, self.screen, 0, 0, SRCCOPY | CAPTUREBLT) == 0 {
                return Err(format!("BitBlt failed ({})", std::io::Error::last_os_error()));
            }
            // The cursor isn't part of the screen DC: draw it in.
            let mut ci: CURSORINFO = std::mem::zeroed();
            ci.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci) != 0 && ci.flags & CURSOR_SHOWING != 0 && !ci.hCursor.is_null() {
                let mut ii: ICONINFO = std::mem::zeroed();
                let (mut hx, mut hy) = (0, 0);
                if GetIconInfo(ci.hCursor, &mut ii) != 0 {
                    hx = ii.xHotspot as i32;
                    hy = ii.yHotspot as i32;
                    if !ii.hbmMask.is_null() {
                        DeleteObject(ii.hbmMask);
                    }
                    if !ii.hbmColor.is_null() {
                        DeleteObject(ii.hbmColor);
                    }
                }
                let POINT { x, y } = ci.ptScreenPos;
                DrawIconEx(self.mem, x - hx, y - hy, ci.hCursor, 0, 0, 0, std::ptr::null_mut(), DI_NORMAL);
            }
            let len = (self.width * self.height * 4) as usize;
            let mut bgra = std::slice::from_raw_parts(self.bits, len).to_vec();
            // GDI leaves alpha 0; the encoder ignores it, but keep it opaque.
            for px in bgra.chunks_exact_mut(4) {
                px[3] = 255;
            }
            Ok(Frame::new(self.width as u32, self.height as u32, bgra))
        }
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.mem);
            ReleaseDC(std::ptr::null_mut(), self.screen);
        }
    }
}

fn mouse(flags: u32, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
    // Apps that read the hardware scan code (WinUI, games, Shift state in
    // some text controls) ignore a VK-only event: always add the scan code.
    let scan = if vk != 0 && scan == 0 { unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 } } else { scan };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        // UIPI: a window of a higher integrity level (an elevated app, UAC)
        // has the focus.
        tracing::debug!(op = "desktop", "SendInput blocked ({})", std::io::Error::last_os_error());
    }
}

/// Inputs for one phone input (pure apart from `VkKeyScanW`).
fn inputs_for(planner: &mut KeyPlanner, input: Input) -> Vec<INPUT> {
    match input {
        Input::Pointer { x, y } => {
            let (ax, ay) = win::absolute(x, y);
            vec![mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay, 0)]
        }
        Input::Button { button, down } => {
            let flag = match (button, down) {
                (PointerButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
                (PointerButton::Left, false) => MOUSEEVENTF_LEFTUP,
                (PointerButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                (PointerButton::Right, false) => MOUSEEVENTF_RIGHTUP,
                (PointerButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                (PointerButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            };
            vec![mouse(flag, 0, 0, 0)]
        }
        Input::Scroll { dx, dy } => {
            let mut v = Vec::new();
            if dy != 0 {
                // Positive = down on the wire; Windows' wheel is positive = up.
                v.push(mouse(MOUSEEVENTF_WHEEL, 0, 0, -dy * WHEEL_DELTA));
            }
            if dx != 0 {
                v.push(mouse(MOUSEEVENTF_HWHEEL, 0, 0, dx * WHEEL_DELTA));
            }
            v
        }
        Input::Key { keysym, down } => match planner.plan(keysym, down) {
            None => Vec::new(),
            Some(KeyAction::Special { key: k, down }) => {
                let (vk, ext) = win::special(k);
                let mut flags = if ext { KEYEVENTF_EXTENDEDKEY } else { 0 };
                if !down {
                    flags |= KEYEVENTF_KEYUP;
                }
                vec![key(vk, 0, flags)]
            }
            Some(KeyAction::Text { ch }) => {
                // On the keyboard layout: real key strokes (with Shift/AltGr
                // as the layout needs) — what apps handle best.
                let scan = if (ch as u32) < 0x10000 { unsafe { VkKeyScanW(ch as u16) } } else { -1 };
                if scan != -1 && ch != '\n' {
                    let vk = (scan as u16) & 0xff;
                    let shifts = ((scan as u16) >> 8) & 0xff;
                    let mods: Vec<u16> = [(1u16, 0xa0u16), (2, 0xa2), (4, 0xa4)]
                        .iter()
                        .filter(|(bit, _)| shifts & bit != 0)
                        .map(|(_, vk)| *vk)
                        .collect();
                    let mut v: Vec<INPUT> = mods.iter().map(|m| key(*m, 0, 0)).collect();
                    v.push(key(vk, 0, 0));
                    v.push(key(vk, 0, KEYEVENTF_KEYUP));
                    v.extend(mods.iter().rev().map(|m| key(*m, 0, KEYEVENTF_KEYUP)));
                    return v;
                }
                let mut buf = [0u16; 2];
                let mut v = Vec::new();
                for unit in ch.encode_utf16(&mut buf).iter() {
                    v.push(key(0, *unit, KEYEVENTF_UNICODE));
                    v.push(key(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                }
                v
            }
            Some(KeyAction::Chord { ch, down }) => {
                let vk = win::char_vk(ch).or_else(|| {
                    let scan = unsafe { VkKeyScanW(ch as u16) };
                    (scan != -1).then_some((scan as u16) & 0xff)
                });
                match vk {
                    Some(vk) => vec![key(vk, 0, if down { 0 } else { KEYEVENTF_KEYUP })],
                    None => Vec::new(),
                }
            }
        },
    }
}

struct Stop(Arc<AtomicBool>);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub async fn start(options: StartOptions) -> Result<Capture, AppErrorDto> {
    dpi_aware();
    let (width, height) = screen_size();
    if width <= 0 || height <= 0 {
        return Err(unavailable("no_display", "Windows reports no screen (is unipi-host running in your desktop session?)"));
    }
    // 4:2:0 needs even sizes; the encoder scales anyway, but keep the grab exact.
    let (width, height) = (width & !1, height & !1);
    // Probe one grab now so a broken session fails the open, not the stream.
    let probe = tokio::task::spawn_blocking(move || Grabber::new(width, height).and_then(|mut g| g.grab().map(|_| ())))
        .await
        .map_err(|e| unavailable("capture_failed", e.to_string()))?;
    probe.map_err(|e| unavailable("capture_failed", format!("Screen capture failed: {e}")))?;

    let input = if options.view_only {
        None
    } else {
        let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
        std::thread::Builder::new()
            .name("desktop-win-input".into())
            .spawn(move || {
                let mut planner = KeyPlanner::default();
                while let Some(input) = rx.blocking_recv() {
                    let inputs = inputs_for(&mut planner, input);
                    let keyboard = inputs.len() > 1 && inputs.iter().all(|i| i.r#type == INPUT_KEYBOARD);
                    if keyboard {
                        // Typed text: one event per SendInput, paced. Batched,
                        // Windows 11 Notepad (WinUI) dropped the injected Shift
                        // ("UniPi!" → "unipi1") and every queued VK_PACKET got
                        // the LAST character ("你好" → "好好").
                        for one in &inputs {
                            send(std::slice::from_ref(one));
                            std::thread::sleep(Duration::from_millis(KEY_PACE_MS));
                        }
                    } else {
                        send(&inputs);
                    }
                }
                // Viewer left: never leave a modifier stuck down.
                let held: Vec<_> = planner.held().to_vec();
                for k in held {
                    let (vk, ext) = win::special(k);
                    send(&[key(vk, 0, KEYEVENTF_KEYUP | if ext { KEYEVENTF_EXTENDEDKEY } else { 0 })]);
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
        .name("desktop-win-grab".into())
        .spawn(move || {
            let mut grabber = match Grabber::new(width, height) {
                Ok(g) => g,
                Err(error) => {
                    tracing::warn!(%error, "windows grab setup failed");
                    return;
                }
            };
            let mut failures = 0;
            let mut dedup = super::capture::Dedup::default();
            while !stopped.load(Ordering::SeqCst) {
                let started = Instant::now();
                // Resolution changed: end; the phone reconnects at the new size.
                let (w, h) = screen_size();
                if (w & !1, h & !1) != (grabber.width, grabber.height) {
                    tracing::info!(op = "desktop", "screen size changed; ending capture");
                    break;
                }
                match grabber.grab() {
                    Ok(frame) => {
                        failures = 0;
                        if let Some(frame) = dedup.check(frame) {
                            if tx.send(Some(frame)).is_err() {
                                break;
                            }
                        }
                    }
                    // The secure desktop (UAC, lock screen) makes BitBlt fail
                    // for a while: keep trying, end only if it lasts.
                    Err(error) => {
                        failures += 1;
                        if failures == 1 || failures % 50 == 0 {
                            tracing::warn!(%error, failures, "windows grab failed");
                        }
                        if failures > 600 {
                            break;
                        }
                    }
                }
                std::thread::sleep(period.saturating_sub(started.elapsed()));
            }
        })
        .map_err(|e| unavailable("capture_failed", e.to_string()))?;
    Ok(Capture { backend: "windows", width: width as u32, height: height as u32, frames: rx, input, display: None, guard: Box::new(Stop(stop)) })
}
