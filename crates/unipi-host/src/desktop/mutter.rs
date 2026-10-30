//! GNOME-native capture + input: `org.gnome.Mutter.ScreenCast` and
//! `org.gnome.Mutter.RemoteDesktop` on the session bus — the API
//! gnome-remote-desktop uses (UNI-252).
//!
//! Unlike the xdg-desktop-portal path there is **no share dialog**: Mutter
//! (GNOME 46–50 at least) accepts these calls from any process of the
//! logged-in user on the session bus; the only access check is that every
//! later call on a session comes from the D-Bus peer that created it. GNOME
//! still shows its "screen is being shared" indicator in the top bar while a
//! stream runs. Sandboxed (Flatpak) apps can't reach this bus name, which is
//! why the portal exists; unipi-host is a plain user process.
//!
//! 1. Control: `RemoteDesktop.CreateSession` → its `SessionId` →
//!    `ScreenCast.CreateSession{remote-desktop-session-id}`; view-only: a
//!    plain `ScreenCast.CreateSession{}` (no input devices at all).
//! 2. `RecordMonitor("", {cursor-mode: embedded})` — "" = the primary monitor.
//! 3. `Start` (on the RemoteDesktop session for control) → the stream's
//!    `PipeWireStreamAdded(node)` signal; the node is on the session's own
//!    PipeWire daemon, read by `gst-launch-1.0 pipewiresrc path=<node>`.
//! 4. Input: `NotifyPointerMotionAbsolute(stream, x, y)` in stream pixels,
//!    `NotifyPointerButton` (evdev BTN_*), `NotifyPointerAxisDiscrete`,
//!    `NotifyKeyboardKeysym`.
//!
//! The capture uses its own D-Bus connection: when it drops (stream ended,
//! host exits or crashes) Mutter sees the peer vanish and closes the sessions.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::{mpsc, watch};
use unipi_proto::{AppErrorDto, PointerButton};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::capture::{Capture, Input, StartOptions};
use super::config::unavailable;
use super::pipewire::spawn_reader;

const RD_NAME: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_PATH: &str = "/org/gnome/Mutter/RemoteDesktop";
const RD_SESSION: &str = "org.gnome.Mutter.RemoteDesktop.Session";
const SC_NAME: &str = "org.gnome.Mutter.ScreenCast";
const SC_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SC_SESSION: &str = "org.gnome.Mutter.ScreenCast.Session";
const SC_STREAM: &str = "org.gnome.Mutter.ScreenCast.Stream";

const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;
/// `RecordMonitor` cursor-mode: drawn into the frames.
const CURSOR_EMBEDDED: u32 = 1;

/// Maps a zbus error from Mutter to our reasons (pure, tested).
pub fn classify(error: &zbus::Error) -> (&'static str, String) {
    let text = error.to_string();
    match error {
        zbus::Error::MethodError(name, _, _) => match name.as_str() {
            "org.freedesktop.DBus.Error.ServiceUnknown" | "org.freedesktop.DBus.Error.NameHasNoOwner" | "org.freedesktop.DBus.Error.UnknownMethod" => {
                ("no_portal", format!("GNOME's screen-cast service (Mutter) is not on the session bus: {text}"))
            }
            "org.freedesktop.DBus.Error.AccessDenied" => ("denied", format!("GNOME refused screen sharing: {text}")),
            _ => ("capture_failed", format!("GNOME screen sharing failed: {text}")),
        },
        zbus::Error::Address(_) | zbus::Error::InputOutput(_) | zbus::Error::Handshake(_) => {
            ("no_portal", format!("Can't reach the session D-Bus (is unipi-host running inside your login session?): {text}"))
        }
        _ => ("capture_failed", format!("GNOME screen sharing failed: {text}")),
    }
}

fn fail(error: zbus::Error) -> AppErrorDto {
    let (reason, message) = classify(&error);
    unavailable(reason, message)
}

/// The size of the PipeWire stream for a monitor: Mutter streams a logical
/// monitor at `logical × scale` when the stage views are scaled (the
/// "logical" layout mode, GNOME's Wayland default), else at `logical`
/// (pure, tested). Input coordinates are in the same stream pixels.
pub fn stream_size(logical: (i32, i32), scale: f64, layout_mode_logical: bool) -> (u32, u32) {
    let s = if layout_mode_logical && scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let w = ((logical.0.max(1) as f64) * s).round() as u32;
    let h = ((logical.1.max(1) as f64) * s).round() as u32;
    // 4:2:0 + gst caps: even sizes.
    (w.max(2) & !1, h.max(2) & !1)
}

/// `size` / `output-name` from a stream's `Parameters` (pure, tested).
pub fn parse_parameters(params: &HashMap<String, OwnedValue>) -> (Option<(i32, i32)>, Option<String>) {
    let size = params.get("size").and_then(|v| <(i32, i32)>::try_from(v.try_clone().ok()?).ok());
    let output = params.get("output-name").and_then(|v| String::try_from(v.try_clone().ok()?).ok());
    (size, output)
}

/// The scale of the logical monitor showing `connector`, and whether the
/// layout mode is "logical" (`org.gnome.Mutter.DisplayConfig.GetCurrentState`).
/// Best effort: `(1.0, true)` when anything is missing.
async fn monitor_scale(conn: &zbus::Connection, connector: Option<&str>) -> (f64, bool) {
    let Ok(proxy) = zbus::Proxy::new(conn, "org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig", "org.gnome.Mutter.DisplayConfig").await
    else {
        return (1.0, true);
    };
    let Ok((_, _, logical, props)) = proxy.call::<_, _, State>("GetCurrentState", &()).await else {
        return (1.0, true);
    };
    // layout-mode: 1 = logical (scaled stage views), 2 = physical.
    let logical_mode = props.get("layout-mode").and_then(|v| u32::try_from(v.try_clone().ok()?).ok()).map(|m| m == 1).unwrap_or(true);
    let pick = logical
        .iter()
        .find(|l| connector.is_some_and(|c| l.5.iter().any(|m| m.0 == c)))
        .or_else(|| logical.iter().find(|l| l.4))
        .or(logical.first());
    (pick.map(|l| l.2).unwrap_or(1.0), logical_mode)
}

type Mode = (String, i32, i32, f64, f64, Vec<f64>, HashMap<String, OwnedValue>);
type Monitor = ((String, String, String, String), Vec<Mode>, HashMap<String, OwnedValue>);
type Logical = (i32, i32, f64, u32, bool, Vec<(String, String, String, String)>, HashMap<String, OwnedValue>);
type State = (u32, Vec<Monitor>, Vec<Logical>, HashMap<String, OwnedValue>);

/// The active monitors from `DisplayConfig.GetCurrentState` (pure, tested):
/// connector, display name, current mode size, primary.
pub fn parse_displays(state: &State) -> Vec<unipi_proto::DesktopDisplay> {
    let (_, monitors, logical, _) = state;
    let mut out = Vec::new();
    for l in logical {
        for spec in &l.5 {
            let Some(monitor) = monitors.iter().find(|m| m.0 .0 == spec.0) else { continue };
            let current = monitor.1.iter().find(|m| m.6.get("is-current").and_then(|v| bool::try_from(v).ok()).unwrap_or(false));
            let Some(mode) = current else { continue };
            let name = monitor
                .2
                .get("display-name")
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| spec.0.clone());
            out.push(unipi_proto::DesktopDisplay {
                id: spec.0.clone(),
                name,
                width: mode.1.max(1) as u32,
                height: mode.2.max(1) as u32,
                primary: l.4,
            });
        }
    }
    // Primary first, then left-to-right as GetCurrentState lists them.
    out.sort_by_key(|d| !d.primary);
    out
}

/// The monitors on this GNOME session; empty when DisplayConfig is unreachable.
pub async fn displays() -> Vec<unipi_proto::DesktopDisplay> {
    let Ok(conn) = zbus::Connection::session().await else { return Vec::new() };
    let Ok(proxy) = zbus::Proxy::new(&conn, "org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig", "org.gnome.Mutter.DisplayConfig").await
    else {
        return Vec::new();
    };
    match proxy.call::<_, _, State>("GetCurrentState", &()).await {
        Ok(state) => parse_displays(&state),
        Err(_) => Vec::new(),
    }
}

type SharedChild = Arc<std::sync::Mutex<Option<tokio::process::Child>>>;

fn kill(child: &SharedChild) {
    if let Some(mut child) = child.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = child.start_kill();
    }
}

struct MutterGuard {
    conn: zbus::Connection,
    /// The session to `Stop` (the RemoteDesktop one for control).
    stop: Option<(String, &'static str, &'static str)>,
    child: SharedChild,
    /// Ends the "session closed" watcher.
    _watch: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for MutterGuard {
    fn drop(&mut self) {
        kill(&self.child);
        if let (Some((path, name, iface)), Ok(handle)) = (self.stop.take(), tokio::runtime::Handle::try_current()) {
            let conn = self.conn.clone();
            handle.spawn(async move {
                if let Ok(proxy) = zbus::Proxy::new(&conn, name, path.as_str(), iface).await {
                    let _ = proxy.call_method("Stop", &()).await;
                }
                // Dropping the connection closes anything left on Mutter's side.
                drop(conn);
            });
        }
    }
}

pub async fn start(options: StartOptions) -> Result<Capture, AppErrorDto> {
    // A private connection: Mutter ties the sessions to this peer.
    let conn = zbus::connection::Builder::session().map_err(fail)?.build().await.map_err(fail)?;
    let screen_cast = zbus::Proxy::new(&conn, SC_NAME, SC_PATH, SC_NAME).await.map_err(fail)?;

    // 1. Sessions.
    let mut rd_session: Option<zbus::Proxy<'static>> = None;
    let mut sc_props: HashMap<&str, Value> = HashMap::new();
    if !options.view_only {
        let remote = zbus::Proxy::new(&conn, RD_NAME, RD_PATH, RD_NAME).await.map_err(fail)?;
        let path: OwnedObjectPath = remote.call("CreateSession", &()).await.map_err(fail)?;
        let session = zbus::Proxy::new_owned(conn.clone(), RD_NAME.to_string(), path, RD_SESSION.to_string()).await.map_err(fail)?;
        let id: String = session.get_property("SessionId").await.map_err(fail)?;
        sc_props.insert("remote-desktop-session-id", Value::from(id));
        rd_session = Some(session);
    }
    let sc_path: OwnedObjectPath = screen_cast.call("CreateSession", &(sc_props,)).await.map_err(fail)?;
    let sc_session = zbus::Proxy::new_owned(conn.clone(), SC_NAME.to_string(), sc_path.clone(), SC_SESSION.to_string()).await.map_err(fail)?;

    // 2. The chosen monitor ("" = primary), cursor drawn in.
    let mut record: HashMap<&str, Value> = HashMap::new();
    record.insert("cursor-mode", Value::from(CURSOR_EMBEDDED));
    let connector = options.monitor.clone().filter(|m| !m.is_empty() && m.len() <= 64).unwrap_or_default();
    let stream_path: OwnedObjectPath = sc_session.call("RecordMonitor", &(connector.as_str(), record)).await.map_err(fail)?;
    let stream = zbus::Proxy::new_owned(conn.clone(), SC_NAME.to_string(), stream_path.clone(), SC_STREAM.to_string()).await.map_err(fail)?;
    let mut added = stream.receive_signal("PipeWireStreamAdded").await.map_err(fail)?;

    // 3. Start; the node id arrives as a signal.
    let stop = match &rd_session {
        Some(session) => {
            session.call_method("Start", &()).await.map_err(fail)?;
            (session.path().to_string(), RD_NAME, RD_SESSION)
        }
        None => {
            sc_session.call_method("Start", &()).await.map_err(fail)?;
            (sc_path.to_string(), SC_NAME, SC_SESSION)
        }
    };
    let mut guard = MutterGuard { conn: conn.clone(), stop: Some(stop), child: Arc::new(std::sync::Mutex::new(None)), _watch: None };
    let node = match tokio::time::timeout(Duration::from_secs(10), added.next()).await {
        Ok(Some(message)) => message.body().deserialize::<(u32,)>().map_err(fail)?.0,
        _ => return Err(unavailable("capture_failed", "GNOME started the screen cast but never announced its PipeWire stream")),
    };
    let params: HashMap<String, OwnedValue> = stream.get_property("Parameters").await.unwrap_or_default();
    let (logical, output) = parse_parameters(&params);
    let (scale, logical_mode) = monitor_scale(&conn, output.as_deref()).await;
    let (sw, sh) = stream_size(logical.unwrap_or((1920, 1080)), scale, logical_mode);
    tracing::info!(op = "desktop", node, sw, sh, scale, ?output, "mutter screen cast started");

    let (tx, rx) = watch::channel(None);
    *guard.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(spawn_reader(None, node, options.fps.clamp(1, 60), sw, sh, tx)?);

    // Mutter closing the session (the user pressed "stop sharing" in the top
    // bar, monitor unplugged…) ends the capture: the reader is killed, its
    // frame sender drops, the session sends `closed{capture_ended}`.
    let closed_on = rd_session.clone().unwrap_or_else(|| sc_session.clone());
    let mut closed = closed_on.receive_signal("Closed").await.map_err(fail)?;
    let (end_tx, mut end_rx) = tokio::sync::oneshot::channel::<()>();
    let child = guard.child.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = closed.next() => {
                tracing::info!(op = "desktop", "mutter closed the session");
                kill(&child);
            }
            _ = &mut end_rx => {}
        }
    });
    guard._watch = Some(end_tx);

    let input = match rd_session {
        None => None,
        Some(session) => {
            let (itx, mut irx) = mpsc::unbounded_channel::<Input>();
            let stream_path = stream_path.to_string();
            tokio::spawn(async move {
                while let Some(input) = irx.recv().await {
                    let result: zbus::Result<()> = match input {
                        Input::Pointer { x, y } => session
                            .call_method("NotifyPointerMotionAbsolute", &(stream_path.as_str(), x * sw as f64, y * sh as f64))
                            .await
                            .map(drop),
                        Input::Button { button, down } => {
                            let code = match button {
                                PointerButton::Left => BTN_LEFT,
                                PointerButton::Right => BTN_RIGHT,
                                PointerButton::Middle => BTN_MIDDLE,
                            };
                            session.call_method("NotifyPointerButton", &(code, down)).await.map(drop)
                        }
                        Input::Scroll { dx, dy } => {
                            let mut r = Ok(());
                            if dy != 0 {
                                r = session.call_method("NotifyPointerAxisDiscrete", &(0u32, dy)).await.map(drop);
                            }
                            if dx != 0 && r.is_ok() {
                                r = session.call_method("NotifyPointerAxisDiscrete", &(1u32, dx)).await.map(drop);
                            }
                            r
                        }
                        Input::Key { keysym, down } => session.call_method("NotifyKeyboardKeysym", &(keysym, down)).await.map(drop),
                    };
                    if let Err(error) = result {
                        tracing::warn!(%error, "mutter input failed");
                    }
                }
            });
            Some(itx)
        }
    };
    Ok(Capture { backend: "mutter", width: sw, height: sh, frames: rx, input, display: output, guard: Box::new(std::sync::Mutex::new(guard)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_sizes() {
        assert_eq!(stream_size((1920, 1080), 1.0, true), (1920, 1080));
        assert_eq!(stream_size((1280, 800), 2.0, true), (2560, 1600), "HiDPI: physical pixels");
        assert_eq!(stream_size((1280, 800), 2.0, false), (1280, 800), "physical layout mode: unscaled");
        assert_eq!(stream_size((1707, 960), 1.5, true), (2560, 1440));
        assert_eq!(stream_size((1366, 767), f64::NAN, true), (1366, 766), "even, NaN scale ignored");
    }

    #[test]
    fn parameters() {
        let mut params = HashMap::new();
        params.insert("size".to_string(), OwnedValue::try_from(Value::from((1920i32, 1080i32))).unwrap());
        params.insert("output-name".to_string(), OwnedValue::try_from(Value::from("DP-1")).unwrap());
        assert_eq!(parse_parameters(&params), (Some((1920, 1080)), Some("DP-1".into())));
        assert_eq!(parse_parameters(&HashMap::new()), (None, None));
    }

    #[test]
    fn displays_from_current_state() {
        let mode = |w: i32, h: i32, current: bool| -> Mode {
            let mut props = HashMap::new();
            if current {
                props.insert("is-current".to_string(), OwnedValue::from(true));
            }
            (format!("{w}x{h}"), w, h, 60.0, 1.0, vec![1.0], props)
        };
        let spec = |c: &str| (c.to_string(), "DEL".to_string(), "U3419W".to_string(), "x".to_string());
        let mut named = HashMap::new();
        named.insert("display-name".to_string(), OwnedValue::try_from(Value::from("Dell 34\"")).unwrap());
        let monitors = vec![
            (spec("HDMI-1"), vec![mode(1920, 1080, true)], HashMap::new()),
            (spec("DP-1"), vec![mode(3440, 1440, true), mode(2560, 1080, false)], named),
            (spec("DP-2"), vec![mode(1920, 1080, false)], HashMap::new()),
        ];
        let logical: Vec<Logical> = vec![
            (0, 0, 1.0, 0, false, vec![spec("HDMI-1")], HashMap::new()),
            (1920, 0, 1.0, 0, true, vec![spec("DP-1")], HashMap::new()),
        ];
        let displays = parse_displays(&(1, monitors, logical, HashMap::new()));
        assert_eq!(displays.len(), 2, "DP-2 is off (no logical monitor)");
        assert_eq!(displays[0], unipi_proto::DesktopDisplay { id: "DP-1".into(), name: "Dell 34\"".into(), width: 3440, height: 1440, primary: true });
        assert_eq!((displays[1].id.as_str(), displays[1].name.as_str(), displays[1].primary), ("HDMI-1", "HDMI-1", false));
    }

    #[test]
    fn classifies_errors() {
        let denied = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.AccessDenied").unwrap(),
            Some("Permission denied".into()),
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(classify(&denied).0, "denied");
        let missing = zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from("org.freedesktop.DBus.Error.ServiceUnknown").unwrap(),
            None,
            zbus::message::Message::method_call("/", "x").unwrap().build(&()).unwrap(),
        );
        assert_eq!(classify(&missing).0, "no_portal");
        assert_eq!(classify(&zbus::Error::InvalidReply).0, "capture_failed");
    }
}
