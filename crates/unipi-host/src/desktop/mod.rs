//! Desktop streaming with control (UNI-216, forwarding phase 4).
//!
//! The phone watches and drives the PC's whole screen, RustDesk-style:
//! capture → H.264 (openh264) → one iroh stream (video host → phone, input
//! phone → host on the other direction of the same QUIC stream).
//!
//! Capture backends (`capture.rs`):
//! - **portal** (Wayland; GNOME, KDE…): xdg-desktop-portal RemoteDesktop +
//!   ScreenCast via ashpd. The PC shows its share dialog the first time; the
//!   restore token (`persist_mode=2`) is stored in `<host dir>/desktop-token-*`
//!   so later connects don't ask again. Frames come from PipeWire (read with
//!   `gst-launch-1.0 pipewiresrc`, see `portal.rs`); input goes through the
//!   portal's `Notify*` calls.
//! - **mutter** (GNOME Wayland, the `auto` choice there): GNOME's own
//!   `org.gnome.Mutter.{ScreenCast,RemoteDesktop}` D-Bus API — what
//!   gnome-remote-desktop uses; no share dialog for the user's own processes
//!   (`mutter.rs`, UNI-252).
//! - **x11**: root-window `GetImage` + XTest, pure Rust (x11rb).
//! - **test_card**: synthetic frames (tests, and checking the phone side
//!   without sharing anything: `unipi-host desktop enable --backend test-card`).
//!
//! Security: OFF by default; `unipi-host desktop enable [--view-only]
//! [--device X]` writes the `desktop` block of `config.json` (read on every
//! request). One viewer at a time (a new one replaces the old). Every
//! connection is logged to `<host dir>/desktop.log`.

pub mod capture;
pub mod config;
pub mod encode;
pub mod keys;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;
#[cfg(all(feature = "desktop-mutter", target_os = "linux"))]
pub mod mutter;
#[cfg(all(any(feature = "desktop-mutter", feature = "desktop-portal"), target_os = "linux"))]
pub mod pipewire;
#[cfg(all(feature = "desktop-portal", target_os = "linux"))]
pub mod portal;
#[cfg(all(feature = "desktop-x11", target_os = "linux"))]
pub mod x11;

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use unipi_proto::{
    frame::{FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, DesktopCmd, DesktopConsent, DesktopEvent, DesktopFrameMeta, DesktopQuality,
    DesktopStatusData, DesktopViewport, Request,
};

use crate::data_dir::HostPaths;
use capture::{Capture, Input, ScrollAccum, StartOptions};
pub use config::{BackendPref, DesktopConfig};
use encode::{encoded_size, target_for, viewport_limit, Target, Tuning, H264};

/// Frames sent but not yet acked by the phone, at most.
pub const MAX_INFLIGHT: usize = 3;

/// `desktop_status` data for this host process.
pub fn status(paths: &HostPaths, config: &DesktopConfig) -> DesktopStatusData {
    let display = config::detect_here();
    let compiled = config::compiled();
    let chosen = config::choose_backend(config.backend, &display, &compiled);
    let (backend, reason) = match &chosen {
        Ok(name) => (Some(name.to_string()), None),
        Err((reason, _)) => (None, Some(reason.to_string())),
    };
    #[allow(unused_mut)]
    let mut hint: Option<String> = None;
    let consent = match chosen {
        Ok("portal") if config::read_token(&config::restore_token_path(&paths.root, config.view_only)).is_some() => DesktopConsent::Granted,
        Ok("portal") => DesktopConsent::Needed,
        // macOS: Screen Recording (and Accessibility for control) are
        // granted once in System Settings; "needed" until then.
        #[cfg(target_os = "macos")]
        Ok("macos") => {
            let screen = macos::screen_access(false);
            let input = config.view_only || macos::input_access(false);
            if !screen {
                hint = Some(macos::SCREEN_HINT.to_string());
            } else if !input {
                hint = Some(macos::INPUT_HINT.to_string());
            }
            if screen && input { DesktopConsent::Granted } else { DesktopConsent::Needed }
        }
        _ => DesktopConsent::NotNeeded,
    };
    DesktopStatusData {
        enabled: config.enabled,
        view_only: config.view_only,
        compiled,
        display: display.kind,
        backend,
        consent,
        hint: reason.as_deref().and_then(config::hint_for).map(str::to_string).or(hint),
        reason,
        features: vec![unipi_proto::DESKTOP_FEATURE_PRESETS.to_string()],
        displays: Vec::new(),
    }
}

/// `status` plus the monitors the chosen backend can pick between (GNOME:
/// Mutter's DisplayConfig; other backends stream the primary display only).
pub async fn status_with_displays(paths: &HostPaths, config: &DesktopConfig) -> DesktopStatusData {
    #[allow(unused_mut)]
    let mut data = status(paths, config);
    #[cfg(all(feature = "desktop-mutter", target_os = "linux"))]
    if config.enabled && data.backend.as_deref() == Some("mutter") {
        let displays = mutter::displays().await;
        if !displays.is_empty() {
            data.features.push(unipi_proto::DESKTOP_FEATURE_DISPLAYS.to_string());
            data.displays = displays;
        }
    }
    data
}

/// The one live desktop stream per host process (a new viewer replaces it).
struct Slot {
    kick: Option<oneshot::Sender<()>>,
}

fn slot() -> &'static Mutex<Slot> {
    static SLOT: OnceLock<Mutex<Slot>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Slot { kick: None }))
}

fn capture_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Auto-quality decision for one stats period.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Period {
    /// Frames the phone acked this period.
    pub acked: u32,
    /// Frames we wanted to send (target fps × period).
    pub wanted: f32,
    pub rtt_ms: u32,
    /// The in-flight window was full at some point (phone behind).
    pub stalled: bool,
}

/// Next level on `quality`'s ladder: a worse rung at once when the viewer
/// falls behind, a better one after `good` calm periods in a row (3 s for
/// the legacy `auto`, 2 s for the presets).
pub fn adapt(quality: DesktopQuality, level: u8, good: &mut u8, period: Period) -> u8 {
    let rungs = encode::ladder(quality).len() as u8;
    let best_first = encode::best_first(quality);
    let (worse, better) = if best_first {
        ((level + 1).min(rungs - 1), level.checked_sub(1))
    } else {
        (level.saturating_sub(1), (level + 1 < rungs).then_some(level + 1))
    };
    let bad = period.stalled && (period.acked as f32) < period.wanted * 0.6 || period.rtt_ms > 600;
    if bad {
        *good = 0;
        return worse;
    }
    let calm = period.rtt_ms < 150 && !period.stalled;
    if calm {
        *good = good.saturating_add(1);
        let need = if best_first { 2 } else { 3 };
        if *good >= need {
            if let Some(better) = better {
                *good = 0;
                return better;
            }
        }
    } else {
        *good = 0;
    }
    level
}

/// "Sharpen on idle": once the picture has been (nearly) still for a
/// moment, send it again as a key frame with a low QP ceiling — re-encoding a
/// still picture as P frames never sharpens it (measured: a 3440×1440 code
/// screen stays at 35.7 dB luma PSNR; the sharp key frame reaches 42+ dB).
/// "Nearly still" is a rate: a clock or a blinking caret still sends small
/// deltas, so idle = the deltas of the last `IDLE_SHARPEN_MS` stayed under
/// `IDLE_RATE_SHARE` of the rung's bitrate and none was big (scrolling).
/// Back to the normal ceiling when something big moves; each relax doubles
/// the next idle wait (up to 8×) so a busy screen doesn't ping-pong.
/// Pure, so the session loop and tests share it.
#[derive(Debug, Default)]
pub struct Sharpen {
    /// Start of the current quiet window (a big change restarts it).
    pub quiet_since: Option<Instant>,
    /// Small deltas inside the window: (when, bytes).
    recent: VecDeque<(Instant, usize)>,
    /// The sharp key frame was asked for / sent; the encoder uses the sharp QP.
    pub sharp: bool,
    /// Idle-wait multiplier (1, 2, 4, 8).
    backoff: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenStep {
    Keep,
    /// Switch to the sharp QP ceiling (encoder rebuild → key frame).
    Sharpen,
    /// Back to the rung's QP ceiling.
    Relax,
}

impl Sharpen {
    fn wait(&self) -> Duration {
        Duration::from_millis(encode::IDLE_SHARPEN_MS * self.backoff.max(1) as u64)
    }

    /// Called after every encoded frame (`bytes` = its size, `key`).
    pub fn on_frame(&mut self, now: Instant, bytes: usize, key: bool) -> SharpenStep {
        if self.sharp {
            if !key && bytes > encode::SHARP_EXIT_BYTES {
                self.sharp = false;
                self.backoff = (self.backoff.max(1) * 2).min(8);
                self.restart(now);
                return SharpenStep::Relax;
            }
            return SharpenStep::Keep;
        }
        if key || bytes > encode::IDLE_DELTA_BYTES || self.quiet_since.is_none() {
            self.restart(now);
        } else {
            self.recent.push_back((now, bytes));
        }
        SharpenStep::Keep
    }

    fn restart(&mut self, now: Instant) {
        self.quiet_since = Some(now);
        self.recent.clear();
    }

    /// Called on a timer with the rung's bitrate: time for the sharp key frame?
    pub fn on_tick(&mut self, now: Instant, kbps: u32) -> SharpenStep {
        if self.sharp {
            return SharpenStep::Keep;
        }
        let Some(since) = self.quiet_since else { return SharpenStep::Keep };
        let wait = self.wait();
        while self.recent.front().is_some_and(|(at, _)| now.duration_since(*at) > wait) {
            self.recent.pop_front();
        }
        if now.duration_since(since) < wait {
            return SharpenStep::Keep;
        }
        let bits: usize = self.recent.iter().map(|(_, b)| b * 8).sum();
        let rate_kbps = bits as f64 / wait.as_secs_f64() / 1000.0;
        if rate_kbps <= kbps as f64 * encode::IDLE_RATE_SHARE {
            self.sharp = true;
            return SharpenStep::Sharpen;
        }
        SharpenStep::Keep
    }

    /// A rung / preset change: start over (keeps the backoff).
    pub fn reset(&mut self) {
        self.sharp = false;
        self.quiet_since = None;
        self.recent.clear();
    }
}

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false)
}
fn write_err(e: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true)
}

async fn send_event<W: AsyncWrite + Unpin>(write: &mut W, event: DesktopEvent) -> Result<(), AppErrorDto> {
    crate::protocol::write_control(write, &Control::DesktopEvent { event }).await.map_err(write_err)
}

/// Handles `desktop_status` / `desktop_open` after the hello.
pub async fn serve_request<R, W>(
    paths: &HostPaths,
    req: Request,
    device_id: &str,
    device_name: Option<&str>,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let config = DesktopConfig::load(&paths.config()).map_err(internal)?;
    match req {
        Request::DesktopStatus => {
            tracing::info!(op = "desktop_status", "request");
            if config.enabled && !config.device_allowed(device_id, device_name) {
                config.check(device_id, device_name)?;
            }
            let data = status_with_displays(paths, &config).await;
            crate::protocol::write_control(write, &Control::Ok { data: Some(serde_json::to_value(data).map_err(internal)?) })
                .await
                .map_err(write_err)?;
            Ok(())
        }
        Request::DesktopOpen { viewport, quality, view_only, display: monitor } => {
            config.check(device_id, device_name)?;
            let display = config::detect_here();
            let backend = config::choose_backend(config.backend, &display, &config::compiled())
                .map_err(|(reason, message)| config::unavailable(reason, message))?;
            let view_only = config.view_only || view_only.unwrap_or(false);
            let options = StartOptions {
                view_only,
                fps: 30,
                root: paths.root.clone(),
                x11_display: config::detect_here().x11,
                monitor,
            };
            let who = format!(
                "device={} id={}",
                device_name.unwrap_or("?"),
                device_id.chars().take(10).collect::<String>()
            );
            open(write, read, backend, options, viewport, quality.unwrap_or_default(), &who, &paths.root).await
        }
        _ => Err(internal("not a desktop request")),
    }
}

/// Reads phone → host controls until EOF; never cancelled mid-frame (it is
/// polled as one long-lived future).
async fn reader_loop<R: AsyncRead + Unpin>(read: &mut R, tx: mpsc::UnboundedSender<DesktopCmd>) {
    loop {
        let Ok((kind, bytes)) = crate::protocol::read_frame(read).await else { return };
        if kind != 0 {
            continue;
        }
        match serde_json::from_slice::<Control>(&bytes) {
            Ok(Control::DesktopCmd { cmd }) => {
                if cmd.check().is_ok() && tx.send(cmd).is_err() {
                    return;
                }
            }
            _ => continue,
        }
    }
}

/// Starts a capture and streams it until the phone leaves, the capture ends
/// or another viewer takes over.
#[allow(clippy::too_many_arguments)]
pub async fn open<R, W>(
    write: &mut W,
    read: &mut R,
    backend: &'static str,
    options: StartOptions,
    viewport: DesktopViewport,
    quality: DesktopQuality,
    who: &str,
    root: &std::path::Path,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // Replace whoever is watching now.
    let (kick_tx, mut kick_rx) = oneshot::channel();
    if let Some(old) = slot().lock().unwrap_or_else(|e| e.into_inner()).kick.replace(kick_tx) {
        let _ = old.send(());
    }
    let _capture_guard = match tokio::time::timeout(Duration::from_secs(10), capture_lock().lock()).await {
        Ok(guard) => guard,
        Err(_) => return Err(config::unavailable("busy", "The previous desktop viewer is still closing; try again")),
    };
    crate::protocol::write_control(write, &Control::Ok { data: None }).await.map_err(write_err)?;
    let view_only = options.view_only;
    config::log_session(root, &format!("{who} backend={backend} view_only={view_only} connect"));
    let started_at = Instant::now();

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<DesktopCmd>();
    let mut reader = Box::pin(reader_loop(read, cmd_tx));

    // Start the capture; tell the phone when the PC is asking for consent.
    let (consent_tx, mut consent_rx) = mpsc::unbounded_channel::<()>();
    let start = capture::start(backend, options, Box::new(move || {
        let _ = consent_tx.send(());
    }));
    tokio::pin!(start);
    let started = loop {
        tokio::select! {
            result = &mut start => break result,
            Some(()) = consent_rx.recv() => {
                send_event(write, DesktopEvent::Consent { state: "waiting".into() }).await?;
            }
            _ = &mut reader => {
                config::log_session(root, &format!("{who} left while waiting for capture"));
                return Ok(());
            }
            _ = &mut kick_rx => {
                send_event(write, DesktopEvent::Closed { reason: "replaced".into() }).await?;
                return Ok(());
            }
        }
    };
    let capture = match started {
        Ok(capture) => capture,
        Err(error) => {
            let reason = error.detail.as_ref().and_then(|d| d.get("reason")).and_then(|r| r.as_str()).unwrap_or("?").to_string();
            config::log_session(root, &format!("{who} refused: {reason}: {}", error.message));
            return Err(error);
        }
    };
    send_event(
        write,
        DesktopEvent::Started {
            backend: capture.backend.into(),
            width: capture.width,
            height: capture.height,
            view_only,
            display: capture.display.clone(),
        },
    )
    .await?;
    let end = stream(write, &mut reader, &mut cmd_rx, &mut kick_rx, capture, viewport, quality).await;
    let reason = match &end {
        Ok(reason) => reason.to_string(),
        Err(error) => format!("error: {}", error.message),
    };
    config::log_session(root, &format!("{who} ended after {}s ({reason})", started_at.elapsed().as_secs()));
    end.map(|_| ())
}

struct Job {
    handle: tokio::task::JoinHandle<(H264, Result<Option<encode::Encoded>, String>)>,
    captured: Instant,
}

/// The streaming loop. Returns why it ended.
async fn stream<W, F>(
    write: &mut W,
    reader: &mut std::pin::Pin<Box<F>>,
    cmds: &mut mpsc::UnboundedReceiver<DesktopCmd>,
    kick: &mut oneshot::Receiver<()>,
    mut capture: Capture,
    viewport: DesktopViewport,
    mut quality: DesktopQuality,
) -> Result<&'static str, AppErrorDto>
where
    W: AsyncWrite + Unpin,
    F: std::future::Future<Output = ()>,
{
    let t0 = Instant::now();
    let mut viewport_max = Some(viewport.width.max(viewport.height));
    let mut level = encode::start_level(quality);
    let mut good = 0u8;
    let mut target: Target = target_for(quality, level);
    let tuning_for = |target: &Target, sharp: Option<u8>| Tuning { max_qp: sharp.unwrap_or(target.max_qp), ..Tuning::default() };
    let mut encoder = Some(H264::with_tuning(tuning_for(&target, None)));
    if let Some(e) = encoder.as_mut() {
        e.set_rate(target.kbps, target.fps);
    }
    let mut sharpen = Sharpen::default();
    let mut sharpen_tick = tokio::time::interval(Duration::from_millis(250));
    sharpen_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Tuning changes wait for the encoder to come back from a running job.
    let mut pending_tuning: Option<Tuning> = None;
    let mut job: Option<Job> = None;
    let mut dirty = capture.frames.borrow().is_some();
    let mut next_at = tokio::time::Instant::now();
    let mut seq = 0u64;
    let mut inflight: VecDeque<(u64, Instant)> = VecDeque::new();
    let mut config_sent: Option<(u32, u32)> = None;
    let mut want_key = false;
    let mut scroll = ScrollAccum::default();
    let mut stats = tokio::time::interval(Duration::from_secs(1));
    stats.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let (mut period_acked, mut period_bytes, mut period_stalled, mut rtt_ms) = (0u32, 0u64, false, 0u32);
    // Debug counters per stats period: captured frames, rate-control skips, encode time.
    let (mut period_captured, mut period_skipped, mut period_encode_ms) = (0u32, 0u32, 0u64);
    let input = capture.input.clone();
    let send_input = |i: Input| {
        if let Some(tx) = &input {
            let _ = tx.send(i);
        }
    };

    loop {
        let can_start = dirty && job.is_none() && inflight.len() < MAX_INFLIGHT && encoder.is_some();
        if dirty && inflight.len() >= MAX_INFLIGHT {
            period_stalled = true;
        }
        tokio::select! {
            biased;
            _ = &mut *kick => {
                send_event(write, DesktopEvent::Closed { reason: "replaced".into() }).await?;
                return Ok("replaced");
            }
            _ = reader.as_mut() => return Ok("phone left"),
            Some(cmd) = cmds.recv() => {
                match cmd {
                    DesktopCmd::Ack { seq: acked } => {
                        while let Some(&(s, sent)) = inflight.front() {
                            if s > acked { break; }
                            inflight.pop_front();
                            if s == acked {
                                let sample = sent.elapsed().as_millis() as u32;
                                rtt_ms = if rtt_ms == 0 { sample } else { (rtt_ms * 7 + sample) / 8 };
                                period_acked += 1;
                            }
                        }
                    }
                    DesktopCmd::Keyframe => { want_key = true; dirty = capture.frames.borrow().is_some(); inflight.clear(); }
                    DesktopCmd::Quality { quality: q } => {
                        if q != quality {
                            quality = q;
                            level = encode::start_level(quality);
                            good = 0;
                        }
                        target = target_for(quality, level);
                        sharpen.reset();
                        pending_tuning = Some(tuning_for(&target, None));
                        if let Some(e) = encoder.as_mut() { e.set_rate(target.kbps, target.fps); }
                        dirty = capture.frames.borrow().is_some();
                    }
                    DesktopCmd::Viewport { viewport } => {
                        viewport_max = Some(viewport.width.max(viewport.height));
                        dirty = capture.frames.borrow().is_some();
                    }
                    DesktopCmd::Pointer { x, y } => send_input(Input::Pointer { x, y }),
                    DesktopCmd::Button { button, down } => send_input(Input::Button { button, down }),
                    DesktopCmd::Scroll { dx, dy } => {
                        if let Some((sx, sy)) = scroll.push(dx, dy) { send_input(Input::Scroll { dx: sx, dy: sy }); }
                    }
                    DesktopCmd::Key { keysym, down } => send_input(Input::Key { keysym, down }),
                    DesktopCmd::Text { text } => {
                        for keysym in capture::text_keysyms(&text) {
                            send_input(Input::Key { keysym, down: true });
                            send_input(Input::Key { keysym, down: false });
                        }
                    }
                }
            }
            changed = capture.frames.changed() => {
                if changed.is_err() {
                    send_event(write, DesktopEvent::Closed { reason: "capture_ended".into() }).await?;
                    return Ok("capture ended");
                }
                capture.frames.borrow_and_update();
                period_captured += 1;
                dirty = true;
            }
            _ = tokio::time::sleep_until(next_at), if can_start => {
                let Some(frame) = capture.frames.borrow_and_update().clone() else { dirty = false; continue };
                dirty = false;
                next_at = tokio::time::Instant::now() + Duration::from_secs_f32(1.0 / target.fps.max(1.0));
                let (w, h) = encoded_size(frame.width, frame.height, target.max_side, viewport_limit(quality, &target, viewport_max));
                let mut enc = encoder.take().expect("checked in can_start");
                if let Some(tuning) = pending_tuning.take() { enc.set_tuning(tuning); }
                if std::mem::take(&mut want_key) { enc.request_key(); }
                job = Some(Job {
                    captured: Instant::now(),
                    handle: tokio::task::spawn_blocking(move || {
                        let result = enc.encode(&frame, w, h);
                        (enc, result)
                    }),
                });
            }
            joined = async { (&mut job.as_mut().expect("guarded").handle).await }, if job.is_some() => {
                let captured = job.take().map(|j| j.captured).unwrap_or_else(Instant::now);
                period_encode_ms += captured.elapsed().as_millis() as u64;
                let (enc, result) = joined.map_err(internal)?;
                encoder = Some(enc);
                let encoded = match result {
                    Ok(Some(encoded)) => encoded,
                    // Skipped by rate control: try the same picture again next tick.
                    Ok(None) => {
                        period_skipped += 1;
                        // (Skips right after the big sharp key frame are the rate
                        // control paying it off — not motion; see Sharpen.)
                        dirty = capture.frames.borrow().is_some();
                        continue
                    }
                    Err(message) => {
                        return Err(config::unavailable("encoder_failed", format!("The H.264 encoder failed: {message}")));
                    }
                };
                if config_sent != Some((encoded.width, encoded.height)) {
                    if !encoded.key {
                        // A size change must start on a key frame.
                        want_key = true;
                        dirty = true;
                        continue;
                    }
                    config_sent = Some((encoded.width, encoded.height));
                    send_event(write, DesktopEvent::Config {
                        codec: encode::codec_string(encoded.width, encoded.height),
                        width: encoded.width,
                        height: encoded.height,
                    }).await?;
                }
                seq += 1;
                let meta = DesktopFrameMeta {
                    seq,
                    key: encoded.key,
                    ts_ms: captured.duration_since(t0).as_millis() as u64,
                    bytes: encoded.data.len() as u32,
                };
                send_event(write, DesktopEvent::Frame(meta)).await?;
                for chunk in encoded.data.chunks(MAX_FRAME) {
                    write.write_all(&unipi_proto::frame::encode(FrameKind::Output, chunk)).await.map_err(write_err)?;
                }
                write.flush().await.map_err(write_err)?;
                period_bytes += encoded.data.len() as u64;
                inflight.push_back((seq, Instant::now()));
                if encode::sharp_qp(quality).is_some()
                    && sharpen.on_frame(Instant::now(), encoded.data.len(), encoded.key) == SharpenStep::Relax
                {
                    pending_tuning = Some(tuning_for(&target, None));
                }
            }
            _ = sharpen_tick.tick() => {
                if let Some(qp) = encode::sharp_qp(quality) {
                    if qp < target.max_qp && inflight.is_empty() && sharpen.on_tick(Instant::now(), target.kbps) == SharpenStep::Sharpen {
                        tracing::debug!(op = "desktop", qp, "idle: sharp key frame");
                        pending_tuning = Some(tuning_for(&target, Some(qp)));
                        dirty = capture.frames.borrow().is_some();
                    }
                }
            }
            _ = stats.tick() => {
                let wanted = target.fps;
                send_event(write, DesktopEvent::Stats {
                    fps: period_acked as f32,
                    kbps: (period_bytes * 8 / 1000) as u32,
                    quality,
                    rtt_ms,
                }).await?;
                if encode::ladder(quality).len() > 1 && seq > 0 {
                    let next = adapt(quality, level, &mut good, Period { acked: period_acked, wanted, rtt_ms, stalled: period_stalled });
                    if next != level {
                        level = next;
                        target = target_for(quality, level);
                        sharpen.reset();
                        pending_tuning = Some(tuning_for(&target, None));
                        if let Some(e) = encoder.as_mut() { e.set_rate(target.kbps, target.fps); }
                        tracing::debug!(op = "desktop", level, ?target, "auto quality");
                    }
                }
                // A phone that stopped acking (lost frames) gets unstuck.
                if let Some(&(_, sent)) = inflight.front() {
                    if sent.elapsed() > Duration::from_secs(5) {
                        inflight.clear();
                        want_key = true;
                        dirty = capture.frames.borrow().is_some();
                    }
                }
                tracing::debug!(
                    op = "desktop", captured = period_captured, acked = period_acked, skipped = period_skipped,
                    encode_ms = period_encode_ms, stalled = period_stalled, level, sharp = sharpen.sharp, "period"
                );
                period_acked = 0;
                period_bytes = 0;
                period_stalled = false;
                period_captured = 0;
                period_skipped = 0;
                period_encode_ms = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALM: Period = Period { acked: 20, wanted: 20.0, rtt_ms: 40, stalled: false };
    const BAD: Period = Period { acked: 4, wanted: 20.0, rtt_ms: 300, stalled: true };

    #[test]
    fn adapt_walks_the_legacy_auto_ladder() {
        let mut good = 0;
        let q = DesktopQuality::Auto;
        assert_eq!(adapt(q, 2, &mut good, CALM), 2);
        assert_eq!(adapt(q, 2, &mut good, CALM), 2);
        assert_eq!(adapt(q, 2, &mut good, CALM), 3, "up after 3 calm periods");
        assert_eq!(adapt(q, 3, &mut good, BAD), 2, "down at once");
        assert_eq!(adapt(q, 0, &mut good, BAD), 0);
        assert_eq!(adapt(q, 2, &mut good, Period { rtt_ms: 900, ..CALM }), 1, "very slow link");
        let top = (encode::AUTO_LADDER.len() - 1) as u8;
        good = 5;
        assert_eq!(adapt(q, top, &mut good, CALM), top);
    }

    #[test]
    fn presets_start_best_and_fall_back() {
        let q = DesktopQuality::Best;
        assert_eq!(encode::start_level(q), 0, "best starts at the screen's own size");
        let mut good = 0;
        assert_eq!(adapt(q, 0, &mut good, CALM), 0, "already the best rung");
        assert_eq!(adapt(q, 0, &mut good, BAD), 1, "worse rung at once");
        let last = (encode::BEST_LADDER.len() - 1) as u8;
        assert_eq!(adapt(q, last, &mut good, BAD), last);
        assert_eq!(adapt(q, 2, &mut good, CALM), 2);
        assert_eq!(adapt(q, 2, &mut good, CALM), 1, "better after 2 calm seconds");
        // A slow encoder alone (acks < wanted, but nothing stalls) is not "bad":
        // Best keeps full resolution at whatever fps the PC manages.
        assert_eq!(adapt(q, 0, &mut good, Period { acked: 9, wanted: 30.0, rtt_ms: 20, stalled: false }), 0);
        assert_eq!(encode::ladder(DesktopQuality::High).len(), 1, "fixed presets don't adapt");
    }

    #[test]
    fn sharpen_after_idle_and_relax_on_motion() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let idle = encode::IDLE_SHARPEN_MS;
        let kbps = 16_000;
        let mut s = Sharpen::default();
        assert_eq!(s.on_frame(ms(0), 300_000, true), SharpenStep::Keep);
        assert_eq!(s.on_tick(ms(idle / 2), kbps), SharpenStep::Keep, "not idle long enough");
        // A clock / caret keeps sending small deltas: still "idle".
        for i in 1..10 {
            assert_eq!(s.on_frame(ms(i * 100), 4_000, false), SharpenStep::Keep);
        }
        assert_eq!(s.on_frame(ms(1000), 40_000, false), SharpenStep::Keep, "a big change (scroll)");
        assert_eq!(s.on_tick(ms(1000 + idle / 2), kbps), SharpenStep::Keep, "the big change restarted the clock");
        assert_eq!(s.on_tick(ms(1000 + idle), kbps), SharpenStep::Sharpen);
        assert_eq!(s.on_tick(ms(9000), kbps), SharpenStep::Keep, "only once");
        // The sharp key frame itself and small deltas keep it sharp.
        assert_eq!(s.on_frame(ms(9100), 600_000, true), SharpenStep::Keep);
        assert_eq!(s.on_frame(ms(9200), 5_000, false), SharpenStep::Keep);
        assert_eq!(s.on_frame(ms(9300), 50_000, false), SharpenStep::Relax, "real motion relaxes");
        assert!(!s.sharp);
        // Backoff: the next sharpen waits twice as long.
        assert_eq!(s.on_tick(ms(9300 + idle), kbps), SharpenStep::Keep);
        assert_eq!(s.on_tick(ms(9300 + 2 * idle), kbps), SharpenStep::Sharpen);
    }

    #[test]
    fn busy_small_deltas_are_not_idle() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut s = Sharpen::default();
        s.on_frame(ms(0), 100_000, true);
        // 30 fps × 10 KB = 2.4 Mbps of "small" deltas: a video playing.
        for i in 1..60 {
            s.on_frame(ms(i * 33), 10_000, false);
        }
        assert_eq!(s.on_tick(ms(2000), 8_000), SharpenStep::Keep, "busy, below the per-frame limit but over the rate share");
    }

}
