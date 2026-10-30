//! Capture backends behind one shape: a running capture publishes its
//! latest frame on a `watch` channel (so a slow phone simply skips frames)
//! and takes input on an `mpsc` channel. Dropping [`Capture`] stops it.
//!
//! - `test_card` — synthetic frames, input recorded in memory. Always
//!   compiled; used by tests and `unipi-host desktop enable --backend test-card`
//!   to check the phone side without touching the real screen.
//! - `x11` (feature `desktop-x11`) — `x11::start`.
//! - `portal` (feature `desktop-portal`) — `portal::start` (Wayland).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use unipi_proto::{AppErrorDto, PointerButton};

use super::encode::Frame;

/// Input for the PC, already validated and in screen terms.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// Absolute, normalised 0..=1.
    Pointer { x: f64, y: f64 },
    Button { button: PointerButton, down: bool },
    /// Whole wheel steps (positive = down/right).
    Scroll { dx: i32, dy: i32 },
    Key { keysym: u32, down: bool },
}

/// A running capture.
pub struct Capture {
    pub backend: &'static str,
    pub width: u32,
    pub height: u32,
    /// The latest frame; the sender is dropped when capture ends.
    pub frames: watch::Receiver<Option<Arc<Frame>>>,
    /// `None` in view-only mode.
    pub input: Option<mpsc::UnboundedSender<Input>>,
    /// The monitor being streamed, when the backend knows (mutter: connector).
    pub display: Option<String>,
    /// Keeps the backend alive (portal session, threads…).
    pub guard: Box<dyn Send + Sync>,
}

/// How to start a capture.
#[derive(Debug, Clone)]
pub struct StartOptions {
    pub view_only: bool,
    /// The capture fps ceiling (the encoder paces further).
    pub fps: u32,
    /// Host data dir (restore tokens).
    pub root: std::path::PathBuf,
    pub x11_display: Option<String>,
    /// UNI-271: the monitor to stream (`desktop_status.displays[].id`); None = primary.
    pub monitor: Option<String>,
}

/// Starts the named backend. `on_consent` fires when the PC shows its
/// share dialog (portal only).
pub async fn start(backend: &str, options: StartOptions, on_consent: Box<dyn FnOnce() + Send>) -> Result<Capture, AppErrorDto> {
    #[cfg(not(all(feature = "desktop-portal", target_os = "linux")))]
    drop(on_consent);
    match backend {
        "test_card" => Ok(test_card::start(&options)),
        #[cfg(all(feature = "desktop-x11", target_os = "linux"))]
        "x11" => super::x11::start(options).await,
        #[cfg(all(feature = "desktop-portal", target_os = "linux"))]
        "portal" => super::portal::start(options, on_consent).await,
        #[cfg(all(feature = "desktop-mutter", target_os = "linux"))]
        "mutter" => super::mutter::start(options).await,
        #[cfg(windows)]
        "windows" => super::windows::start(options).await,
        #[cfg(target_os = "macos")]
        "macos" => super::macos::start(options).await,
        other => Err(super::config::unavailable("not_compiled", format!("The {other} capture backend is not compiled into this unipi-host build"))),
    }
}

/// Unicode text → keysyms (X11 rules: Latin-1 maps 1:1, the rest is
/// `0x0100_0000 + code point`; newline/tab → Return/Tab).
pub fn text_keysyms(text: &str) -> Vec<u32> {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' => Some(0xff0d),
            '\t' => Some(0xff09),
            '\u{8}' => Some(0xff08),
            c if c.is_control() => None,
            c if (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c) => Some(c as u32),
            c => Some(0x0100_0000 + c as u32),
        })
        .collect()
}

/// Same size and pixels as the previous frame?
pub fn same_picture(previous: Option<&Frame>, next: &Frame) -> bool {
    previous.is_some_and(|p| p.width == next.width && p.height == next.height && p.bgra == next.bgra)
}

/// Polling backends (X11, GDI, CoreGraphics) have no damage info: an
/// unchanged picture isn't re-published, so the encoder doesn't burn the CPU
/// on identical frames (UNI-271; coffee managed ~4 fps re-encoding a still
/// 1080p screen). Still sent once a second as a keep-alive.
#[derive(Default)]
pub struct Dedup {
    last: Option<Arc<Frame>>,
    sent_at: Option<std::time::Instant>,
}

impl Dedup {
    /// The frame to publish, or `None` when it repeats the last one.
    pub fn check(&mut self, frame: Frame) -> Option<Arc<Frame>> {
        let fresh = self.sent_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        if !fresh && same_picture(self.last.as_deref(), &frame) {
            return None;
        }
        let frame = Arc::new(frame);
        self.last = Some(frame.clone());
        self.sent_at = Some(std::time::Instant::now());
        Some(frame)
    }
}

/// Accumulates fractional scroll into whole wheel steps.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScrollAccum {
    x: f64,
    y: f64,
}

impl ScrollAccum {
    pub fn push(&mut self, dx: f64, dy: f64) -> Option<(i32, i32)> {
        self.x += dx;
        self.y += dy;
        let (sx, sy) = (self.x.trunc() as i32, self.y.trunc() as i32);
        self.x -= sx as f64;
        self.y -= sy as f64;
        (sx != 0 || sy != 0).then_some((sx, sy))
    }
}

pub mod test_card {
    //! Synthetic capture: colour bars, a moving box, a clock-ish counter and
    //! a crosshair where the "pointer" is. Input is recorded for tests.
    use super::*;

    static LOG: Mutex<Vec<Input>> = Mutex::new(Vec::new());

    /// Input the test card received (drains).
    pub fn take_input_log() -> Vec<Input> {
        std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub const WIDTH: u32 = 1280;
    pub const HEIGHT: u32 = 720;

    /// Renders frame `t` with the pointer at `(px, py)` (normalised).
    pub fn render(width: u32, height: u32, t: u32, pointer: (f64, f64)) -> Frame {
        let colors: [[u8; 3]; 8] = [[200, 200, 200], [0, 200, 200], [200, 200, 0], [0, 200, 0], [200, 0, 200], [0, 0, 200], [200, 0, 0], [30, 30, 30]];
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let c = if y < height * 2 / 3 { colors[(x * 8 / width) as usize] } else { [((x * 255) / width) as u8; 3] };
                let i = ((y * width + x) * 4) as usize;
                bgra[i] = c[2];
                bgra[i + 1] = c[1];
                bgra[i + 2] = c[0];
                bgra[i + 3] = 255;
            }
        }
        let mut fill = |x0: u32, y0: u32, w: u32, h: u32, rgb: [u8; 3]| {
            for y in y0..(y0 + h).min(height) {
                for x in x0..(x0 + w).min(width) {
                    let i = ((y * width + x) * 4) as usize;
                    bgra[i] = rgb[2];
                    bgra[i + 1] = rgb[1];
                    bgra[i + 2] = rgb[0];
                }
            }
        };
        let side = (height / 8).max(4);
        let bx = (t * 9) % (width.saturating_sub(side).max(1));
        fill(bx, height * 2 / 3 + height / 12, side, side, [255, 255, 255]);
        // Binary frame counter (16 cells) along the top.
        for bit in 0..16 {
            let on = (t >> bit) & 1 == 1;
            fill(8 + bit * 18, 8, 14, 14, if on { [255, 255, 255] } else { [0, 0, 0] });
        }
        let (px, py) = ((pointer.0 * (width - 1) as f64) as u32, (pointer.1 * (height - 1) as f64) as u32);
        fill(px.saturating_sub(10), py, 21, 2, [255, 0, 0]);
        fill(px, py.saturating_sub(10), 2, 21, [255, 0, 0]);
        Frame::new(width, height, bgra)
    }

    struct Stop(Option<tokio::sync::oneshot::Sender<()>>);

    pub fn start(options: &StartOptions) -> Capture {
        let (tx, rx) = watch::channel(None);
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Input>();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let fps = options.fps.clamp(1, 60);
        tokio::spawn(async move {
            let mut t = 0u32;
            let mut pointer = (0.5, 0.5);
            let mut tick = tokio::time::interval(Duration::from_millis(1000 / fps as u64));
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    input = input_rx.recv() => {
                        let Some(input) = input else { continue };
                        if let Input::Pointer { x, y } = input { pointer = (x, y); }
                        LOG.lock().unwrap_or_else(|e| e.into_inner()).push(input);
                    }
                    _ = tick.tick() => {
                        let frame = tokio::task::spawn_blocking(move || render(WIDTH, HEIGHT, t, pointer)).await;
                        let Ok(frame) = frame else { break };
                        if tx.send(Some(Arc::new(frame))).is_err() { break; }
                        t = t.wrapping_add(1);
                    }
                }
            }
        });
        Capture {
            backend: "test_card",
            width: WIDTH,
            height: HEIGHT,
            frames: rx,
            input: (!options.view_only).then_some(input_tx),
            display: None,
            guard: Box::new(Mutex::new(Stop(Some(stop_tx)))),
        }
    }

    impl Drop for Stop {
        fn drop(&mut self) {
            if let Some(tx) = self.0.take() {
                let _ = tx.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_for_text() {
        assert_eq!(text_keysyms("aZ 1\n"), vec![0x61, 0x5a, 0x20, 0x31, 0xff0d]);
        assert_eq!(text_keysyms("é"), vec![0xe9]);
        assert_eq!(text_keysyms("€"), vec![0x0100_20ac]);
        assert_eq!(text_keysyms("\u{7}"), Vec::<u32>::new());
    }

    #[test]
    fn same_picture_compares_pixels() {
        let a = Frame::new(2, 1, vec![1; 8]);
        let mut b = a.clone();
        assert!(!same_picture(None, &a));
        assert!(same_picture(Some(&a), &b));
        b.bgra[5] = 9;
        assert!(!same_picture(Some(&a), &b));
        assert!(!same_picture(Some(&a), &Frame::new(1, 2, vec![1; 8])));
        let mut d = Dedup::default();
        assert!(d.check(a.clone()).is_some(), "first frame");
        assert!(d.check(a.clone()).is_none(), "repeat within a second");
        assert!(d.check(b.clone()).is_some(), "changed");
    }

    #[test]
    fn scroll_accumulates() {
        let mut acc = ScrollAccum::default();
        assert_eq!(acc.push(0.0, 0.4), None);
        assert_eq!(acc.push(0.0, 0.7), Some((0, 1)));
        assert_eq!(acc.push(-2.5, 0.0), Some((-2, 0)));
        // y carried +0.1 from before: -2.9 → -2 now, -0.9 kept.
        assert_eq!(acc.push(-0.6, -3.0), Some((-1, -2)));
    }

    #[tokio::test]
    async fn test_card_produces_frames_and_records_input() {
        let options = StartOptions { view_only: false, fps: 30, root: std::env::temp_dir(), x11_display: None, monitor: None };
        let mut capture = start("test_card", options, Box::new(|| {})).await.unwrap();
        capture.frames.changed().await.unwrap();
        let frame = capture.frames.borrow().clone().unwrap();
        assert_eq!((frame.width, frame.height), (test_card::WIDTH, test_card::HEIGHT));
        let _ = test_card::take_input_log();
        capture.input.as_ref().unwrap().send(Input::Key { keysym: 0x61, down: true }).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(test_card::take_input_log().contains(&Input::Key { keysym: 0x61, down: true }));
        drop(capture);
    }

    #[tokio::test]
    async fn view_only_has_no_input_and_unknown_backend_errors() {
        let options = StartOptions { view_only: true, fps: 10, root: std::env::temp_dir(), x11_display: None, monitor: None };
        let capture = start("test_card", options.clone(), Box::new(|| {})).await.unwrap();
        assert!(capture.input.is_none());
        let err = start("nope", options, Box::new(|| {})).await.err().unwrap();
        assert_eq!(err.detail.unwrap()["reason"], "not_compiled");
    }
}
