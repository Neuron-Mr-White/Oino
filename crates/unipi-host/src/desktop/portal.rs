//! Wayland capture + input through xdg-desktop-portal (GNOME, KDE, wlroots
//! with a portal backend).
//!
//! 1. View-only: a plain **ScreenCast** session — `SelectSources(monitor,
//!    cursor embedded, persist_mode=2, restore_token?)`, `Start`. No input
//!    devices are ever requested.
//!    Control: a **RemoteDesktop** session — `SelectDevices(keyboard|pointer,
//!    persist_mode=2, restore_token?)`, `SelectSources(monitor)` *without*
//!    persistence (xdg-desktop-portal ≥ 1.19 refuses it there: "Remote
//!    desktop sessions cannot persist", UNI-252), `Start`. If the portal
//!    still rejects persistence (`InvalidArgument`) the session is retried
//!    transient: the PC asks every time.
//!    The PC shows its share dialog unless the stored restore token is valid.
//! 2. The new restore token is stored in `<host dir>/desktop-token-*`.
//! 3. `OpenPipeWireRemote` gives a PipeWire fd; frames are read by a
//!    `gst-launch-1.0 pipewiresrc fd=3 path=<node>` child that converts to
//!    BGRx and writes raw frames to its stdout (no libpipewire link, so the
//!    static musl build keeps the portal).
//! 4. Input: `NotifyPointerMotionAbsolute` (stream coordinates),
//!    `NotifyPointerButton` (evdev BTN_*), `NotifyPointerAxisDiscrete`,
//!    `NotifyKeyboardKeysym`.
//!
//! Never exercised against the owner's real session by tests (it would pop
//! the share dialog on their screen); see docs/m7/DESKTOP.md for what ran.

use std::os::fd::OwnedFd;
use std::sync::Arc;

use ashpd::desktop::remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::{PersistMode, Session};
use tokio::sync::{mpsc, watch};
use unipi_proto::{AppErrorDto, PointerButton};

use super::capture::{Capture, Input, StartOptions};
use super::config::{read_token, restore_token_path, unavailable, write_token};
use super::pipewire::spawn_reader;

const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;

/// Maps an ashpd error to our reasons (pure, tested).
pub fn classify(error: &ashpd::Error) -> (&'static str, String) {
    use ashpd::desktop::ResponseError;
    match error {
        ashpd::Error::Response(ResponseError::Cancelled) => ("denied", "The share request was cancelled on the PC".into()),
        ashpd::Error::Portal(ashpd::PortalError::Cancelled(_)) | ashpd::Error::Portal(ashpd::PortalError::NotAllowed(_)) => {
            ("denied", format!("The PC refused to share its screen: {error}"))
        }
        ashpd::Error::PortalNotFound(_) => ("no_portal", format!("No desktop portal on this PC: {error}")),
        ashpd::Error::Zbus(_) | ashpd::Error::Portal(ashpd::PortalError::ZBus(_)) => {
            ("no_portal", format!("Can't reach the desktop portal over D-Bus (is unipi-host running inside your login session?): {error}"))
        }
        _ => ("capture_failed", format!("Screen sharing failed: {error}")),
    }
}

fn fail(error: ashpd::Error) -> AppErrorDto {
    let (reason, message) = classify(&error);
    unavailable(reason, message)
}

/// One way to ask the portal (pure plan, tested).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// View-only: a ScreenCast session; `persist_mode=2` + restore token on
    /// `SelectSources` (GNOME then skips the dialog next time).
    ScreenCastPersist,
    /// Control: a RemoteDesktop session; `persist_mode=2` + restore token on
    /// `SelectDevices` (RemoteDesktop v2).
    RemoteDesktopPersist,
    /// Control without persistence — the PC asks every time. Used when the
    /// portal refuses persistence (`InvalidArgument`, e.g. "Remote desktop
    /// sessions cannot persist" on older/other backends).
    RemoteDesktopTransient,
}

/// What to try, in order.
pub fn attempts(view_only: bool) -> &'static [Attempt] {
    if view_only {
        &[Attempt::ScreenCastPersist]
    } else {
        &[Attempt::RemoteDesktopPersist, Attempt::RemoteDesktopTransient]
    }
}

/// Should a failed attempt fall through to the next one? Only when the
/// portal rejected the arguments (persistence) — never after Cancel.
pub fn retry_after(error: &ashpd::Error) -> bool {
    matches!(error, ashpd::Error::Portal(ashpd::PortalError::InvalidArgument(_)))
}

/// Closes the portal session and stops the reader.
struct PortalGuard {
    close: Option<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>>,
    child: Option<tokio::process::Child>,
}

impl Drop for PortalGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        if let (Some(close), Ok(handle)) = (self.close.take(), tokio::runtime::Handle::try_current()) {
            handle.spawn(close);
        }
    }
}

/// What a started portal session hands back.
struct Started {
    streams: Vec<ashpd::desktop::screencast::Stream>,
    restore_token: Option<String>,
    fd: OwnedFd,
    close: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
    /// The RemoteDesktop session for input (control only).
    remote: Option<(RemoteDesktop, Arc<Session<RemoteDesktop>>)>,
}

fn monitor_sources(persist: bool, token: Option<&str>) -> SelectSourcesOptions {
    let options = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::Embedded)
        .set_sources(ashpd::enumflags2::BitFlags::from(SourceType::Monitor))
        .set_multiple(false);
    if persist {
        options.set_persist_mode(PersistMode::ExplicitlyRevoked).set_restore_token(token)
    } else {
        options
    }
}

async fn run_attempt(attempt: Attempt, token: Option<&str>, on_consent: &mut Option<Box<dyn FnOnce() + Send>>) -> Result<Started, ashpd::Error> {
    let mut consent = || {
        if token.is_none() || attempt == Attempt::RemoteDesktopTransient {
            if let Some(f) = on_consent.take() {
                f();
            }
        }
    };
    match attempt {
        Attempt::ScreenCastPersist => {
            let cast = Screencast::new().await?;
            let session = cast.create_session(Default::default()).await?;
            cast.select_sources(&session, monitor_sources(true, token)).await?;
            consent();
            let started = cast.start(&session, None, Default::default()).await?;
            let selected = started.response()?;
            let fd: OwnedFd = cast.open_pipe_wire_remote(&session, Default::default()).await?;
            Ok(Started {
                streams: selected.streams().to_vec(),
                restore_token: selected.restore_token().map(str::to_string),
                fd,
                close: Box::pin(async move {
                    let _ = session.close().await;
                }),
                remote: None,
            })
        }
        Attempt::RemoteDesktopPersist | Attempt::RemoteDesktopTransient => {
            let persist = attempt == Attempt::RemoteDesktopPersist;
            let remote = RemoteDesktop::new().await?;
            let cast = Screencast::new().await?;
            let session = remote.create_session(Default::default()).await?;
            let mut devices = SelectDevicesOptions::default().set_devices(DeviceType::Keyboard | DeviceType::Pointer);
            if persist {
                devices = devices.set_persist_mode(PersistMode::ExplicitlyRevoked).set_restore_token(token);
            }
            let result = async {
                remote.select_devices(&session, devices).await?;
                // A RemoteDesktop session never takes persistence on SelectSources.
                cast.select_sources(&session, monitor_sources(false, None)).await?;
                consent();
                let started = remote.start(&session, None, Default::default()).await?;
                let selected = started.response()?;
                let fd: OwnedFd = cast.open_pipe_wire_remote(&session, Default::default()).await?;
                Ok::<_, ashpd::Error>((selected, fd))
            }
            .await;
            let (selected, fd) = match result {
                Ok(ok) => ok,
                Err(error) => {
                    let _ = session.close().await;
                    return Err(error);
                }
            };
            let session = Arc::new(session);
            let closer = session.clone();
            Ok(Started {
                streams: selected.streams().to_vec(),
                restore_token: selected.restore_token().map(str::to_string),
                fd,
                close: Box::pin(async move {
                    let _ = closer.close().await;
                }),
                remote: Some((remote, session)),
            })
        }
    }
}

pub async fn start(options: StartOptions, on_consent: Box<dyn FnOnce() + Send>) -> Result<Capture, AppErrorDto> {
    let token_path = restore_token_path(&options.root, options.view_only);
    let token = read_token(&token_path);
    let mut on_consent = Some(on_consent);
    let mut last = None;
    let mut started = None;
    for &attempt in attempts(options.view_only) {
        match run_attempt(attempt, token.as_deref(), &mut on_consent).await {
            Ok(s) => {
                started = Some((attempt, s));
                break;
            }
            Err(error) if retry_after(&error) => {
                tracing::warn!(op = "desktop", ?attempt, %error, "portal refused; trying the next way");
                last = Some(error);
            }
            Err(error) => return Err(fail(error)),
        }
    }
    let Some((attempt, started)) = started else {
        return Err(fail(last.unwrap_or(ashpd::Error::NoResponse)));
    };
    if attempt != Attempt::RemoteDesktopTransient {
        if let Some(new_token) = started.restore_token.as_deref() {
            write_token(&token_path, new_token);
        }
    }
    let stream = started
        .streams
        .first()
        .cloned()
        .ok_or_else(|| unavailable("capture_failed", "The portal returned no screen to capture"))?;
    let (sw, sh) = stream.size().map(|(w, h)| (w.max(1) as u32, h.max(1) as u32)).unwrap_or((1920, 1080));
    let node = stream.pipe_wire_node_id();

    let (tx, rx) = watch::channel(None);
    let child = spawn_reader(Some(started.fd), node, options.fps.clamp(1, 60), sw, sh, tx)?;

    let input = match started.remote {
        Some((remote, session)) if !options.view_only => {
            let (itx, mut irx) = mpsc::unbounded_channel::<Input>();
            tokio::spawn(async move {
                while let Some(input) = irx.recv().await {
                    let result = match input {
                        Input::Pointer { x, y } => {
                            remote
                                .notify_pointer_motion_absolute(&session, node, x * sw as f64, y * sh as f64, Default::default())
                                .await
                        }
                        Input::Button { button, down } => {
                            let code = match button {
                                PointerButton::Left => BTN_LEFT,
                                PointerButton::Right => BTN_RIGHT,
                                PointerButton::Middle => BTN_MIDDLE,
                            };
                            let state = if down { KeyState::Pressed } else { KeyState::Released };
                            remote.notify_pointer_button(&session, code, state, Default::default()).await
                        }
                        Input::Scroll { dx, dy } => {
                            let mut r = Ok(());
                            if dy != 0 {
                                r = remote.notify_pointer_axis_discrete(&session, Axis::Vertical, dy, Default::default()).await;
                            }
                            if dx != 0 && r.is_ok() {
                                r = remote.notify_pointer_axis_discrete(&session, Axis::Horizontal, dx, Default::default()).await;
                            }
                            r
                        }
                        Input::Key { keysym, down } => {
                            let state = if down { KeyState::Pressed } else { KeyState::Released };
                            remote.notify_keyboard_keysym(&session, keysym as i32, state, Default::default()).await
                        }
                    };
                    if let Err(error) = result {
                        tracing::warn!(%error, "portal input failed");
                    }
                }
            });
            Some(itx)
        }
        _ => None,
    };
    Ok(Capture {
        backend: "portal",
        width: sw,
        height: sh,
        frames: rx,
        input,
        display: None,
        guard: Box::new(std::sync::Mutex::new(PortalGuard { close: Some(started.close), child: Some(child) })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_errors() {
        use ashpd::desktop::ResponseError;
        assert_eq!(classify(&ashpd::Error::Response(ResponseError::Cancelled)).0, "denied");
        assert_eq!(classify(&ashpd::Error::Response(ResponseError::Other)).0, "capture_failed");
        assert_eq!(classify(&ashpd::Error::NoResponse).0, "capture_failed");
    }

    #[test]
    fn view_only_is_a_pure_screencast_and_control_falls_back_to_transient() {
        assert_eq!(attempts(true), &[Attempt::ScreenCastPersist]);
        assert_eq!(attempts(false), &[Attempt::RemoteDesktopPersist, Attempt::RemoteDesktopTransient]);
        // GNOME 50: persistence on a RemoteDesktop session's SelectSources.
        let invalid = ashpd::Error::Portal(ashpd::PortalError::InvalidArgument("Remote desktop sessions cannot persist".into()));
        assert!(retry_after(&invalid));
        assert!(!retry_after(&ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)), "Cancel is final");
        assert!(!retry_after(&ashpd::Error::Portal(ashpd::PortalError::NotAllowed("no".into()))));
        assert_eq!(classify(&invalid).0, "capture_failed");
    }

    #[test]
    fn persistence_only_on_screencast_selections_that_ask_for_it() {
        let with = serde_json::to_value(monitor_sources(true, Some("tok"))).unwrap();
        let without = serde_json::to_value(monitor_sources(false, Some("tok"))).unwrap();
        assert!(with.get("persist_mode").is_some(), "{with}");
        assert!(without.get("persist_mode").is_none() && without.get("restore_token").is_none(), "{without}");
    }
}
