//! X11 capture (`GetImage` of the root window) and input (XTest), pure Rust
//! via x11rb — no libX11, so it builds static/musl. Used on X11 sessions
//! (and with `--backend x11` against any `$DISPLAY`, e.g. an Xvfb in tests).

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};
use unipi_proto::{AppErrorDto, PointerButton};
use x11rb::connection::Connection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, ImageOrder};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::capture::{Capture, Input, StartOptions};
use super::config::unavailable;
use super::encode::Frame;

const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;
const BUTTON_PRESS: u8 = 4;
const BUTTON_RELEASE: u8 = 5;
const MOTION: u8 = 6;
const SHIFT_L: u32 = 0xffe1;

struct Stop(Arc<std::sync::atomic::AtomicBool>);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn connect(display: Option<&str>) -> Result<(RustConnection, usize), AppErrorDto> {
    RustConnection::connect(display).map_err(|e| unavailable("no_display", format!("Can't open the X11 display {}: {e}", display.unwrap_or("$DISPLAY"))))
}

/// Grabs the root window as tight BGRA.
fn grab(conn: &RustConnection, root: u32, width: u16, height: u16) -> Result<Frame, String> {
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, width, height, !0)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    let expected = width as usize * height as usize * 4;
    if reply.data.len() < expected {
        return Err(format!("unsupported X11 pixel format (depth {}, {} bytes)", reply.depth, reply.data.len()));
    }
    let mut bgra = reply.data;
    bgra.truncate(expected);
    for px in bgra.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Frame::new(width as u32, height as u32, bgra))
}

/// keysym → (keycode, needs shift), from the server's keymap.
struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
    /// A keycode with no symbols we can borrow for unmapped keysyms.
    spare: Option<u8>,
}

impl Keymap {
    fn load(conn: &RustConnection) -> Result<Keymap, String> {
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let per = reply.keysyms_per_keycode as usize;
        let syms = reply.keysyms;
        let spare = (0..(max - min + 1) as usize).rev().find(|i| syms[i * per..(i + 1) * per].iter().all(|&s| s == 0)).map(|i| min + i as u8);
        Ok(Keymap { min, per, syms, spare })
    }

    fn find(&self, keysym: u32) -> Option<(u8, bool)> {
        for (i, chunk) in self.syms.chunks(self.per).enumerate() {
            if chunk.first() == Some(&keysym) {
                return Some((self.min + i as u8, false));
            }
            if chunk.get(1) == Some(&keysym) {
                return Some((self.min + i as u8, true));
            }
        }
        None
    }
}

struct Injector {
    conn: RustConnection,
    root: u32,
    width: u16,
    height: u16,
    keymap: Keymap,
}

impl Injector {
    fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        let _ = self.conn.xtest_fake_input(kind, detail, 0, self.root, x, y, 0);
    }

    fn key(&mut self, keysym: u32, down: bool) {
        let (code, shift) = match self.keymap.find(keysym) {
            Some(found) => found,
            None => {
                // Borrow a spare keycode for this keysym (e.g. "€").
                let Some(spare) = self.keymap.spare else { return };
                let mut row = vec![0u32; self.keymap.per];
                row[0] = keysym;
                if row.len() > 1 {
                    row[1] = keysym;
                }
                let _ = self.conn.change_keyboard_mapping(1, spare, self.keymap.per as u8, &row);
                let _ = self.conn.sync();
                let i = (spare - self.keymap.min) as usize * self.keymap.per;
                self.keymap.syms[i..i + self.keymap.per].copy_from_slice(&row);
                (spare, false)
            }
        };
        let shift_code = if shift { self.keymap.find(SHIFT_L).map(|(c, _)| c) } else { None };
        if down {
            if let Some(s) = shift_code {
                self.fake(KEY_PRESS, s, 0, 0);
            }
            self.fake(KEY_PRESS, code, 0, 0);
        } else {
            self.fake(KEY_RELEASE, code, 0, 0);
            if let Some(s) = shift_code {
                self.fake(KEY_RELEASE, s, 0, 0);
            }
        }
    }

    fn apply(&mut self, input: Input) {
        match input {
            Input::Pointer { x, y } => {
                let px = (x * (self.width as f64 - 1.0)).round() as i16;
                let py = (y * (self.height as f64 - 1.0)).round() as i16;
                self.fake(MOTION, 0, px, py);
            }
            Input::Button { button, down } => {
                let detail = match button {
                    PointerButton::Left => 1,
                    PointerButton::Middle => 2,
                    PointerButton::Right => 3,
                };
                self.fake(if down { BUTTON_PRESS } else { BUTTON_RELEASE }, detail, 0, 0);
            }
            Input::Scroll { dx, dy } => {
                let click = |button: u8, n: i32| {
                    for _ in 0..n.unsigned_abs().min(50) {
                        self.fake(BUTTON_PRESS, button, 0, 0);
                        self.fake(BUTTON_RELEASE, button, 0, 0);
                    }
                };
                click(if dy > 0 { 5 } else { 4 }, dy);
                click(if dx > 0 { 7 } else { 6 }, dx);
            }
            Input::Key { keysym, down } => self.key(keysym, down),
        }
        let _ = self.conn.flush();
    }
}

pub async fn start(options: StartOptions) -> Result<Capture, AppErrorDto> {
    let display = options.x11_display.clone();
    let (conn, screen) = tokio::task::spawn_blocking({
        let display = display.clone();
        move || connect(display.as_deref())
    })
    .await
    .map_err(|e| unavailable("capture_failed", e.to_string()))??;
    let screen = &conn.setup().roots[screen];
    let (root, width, height) = (screen.root, screen.width_in_pixels, screen.height_in_pixels);
    if conn.setup().image_byte_order != ImageOrder::LSB_FIRST {
        return Err(unavailable("capture_failed", "Big-endian X servers are not supported"));
    }
    // Probe one grab now so a broken display fails the open, not the stream.
    grab(&conn, root, width, height).map_err(|e| unavailable("capture_failed", format!("X11 screen grab failed: {e}")))?;

    let input = if options.view_only {
        None
    } else {
        let (iconn, _) = connect(display.as_deref())?;
        iconn
            .xtest_get_version(2, 2)
            .ok()
            .and_then(|c| c.reply().ok())
            .ok_or_else(|| unavailable("capture_failed", "The X server has no XTest extension (needed for input)"))?;
        let keymap = Keymap::load(&iconn).map_err(|e| unavailable("capture_failed", e))?;
        let mut injector = Injector { conn: iconn, root, width, height, keymap };
        let (tx, mut rx) = mpsc::unbounded_channel::<Input>();
        std::thread::Builder::new()
            .name("desktop-x11-input".into())
            .spawn(move || {
                while let Some(input) = rx.blocking_recv() {
                    injector.apply(input);
                }
            })
            .map_err(|e| unavailable("capture_failed", e.to_string()))?;
        Some(tx)
    };

    let (tx, rx) = watch::channel(None);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    let period = Duration::from_millis(1000 / options.fps.clamp(1, 60) as u64);
    std::thread::Builder::new()
        .name("desktop-x11-grab".into())
        .spawn(move || {
            let mut dedup = super::capture::Dedup::default();
            while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                let started = Instant::now();
                match grab(&conn, root, width, height) {
                    Ok(frame) => {
                        if let Some(frame) = dedup.check(frame) {
                            if tx.send(Some(frame)).is_err() {
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "x11 grab failed; ending capture");
                        break;
                    }
                }
                std::thread::sleep(period.saturating_sub(started.elapsed()));
            }
            // Dropping `tx` tells the session the capture ended.
        })
        .map_err(|e| unavailable("capture_failed", e.to_string()))?;
    Ok(Capture { backend: "x11", width: width as u32, height: height as u32, frames: rx, input, display: None, guard: Box::new(Stop(stop)) })
}
