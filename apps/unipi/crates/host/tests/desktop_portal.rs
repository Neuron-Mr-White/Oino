//! UNI-216, the Wayland portal path against a FAKE xdg-desktop-portal on a
//! PRIVATE session bus, with a PRIVATE PipeWire daemon — never the owner's
//! session (a real portal would pop a share dialog on their screen).
//!
//! The fake implements `org.freedesktop.portal.{RemoteDesktop,ScreenCast}`
//! the way GNOME's does (Request/Response signals, restore tokens, a
//! PipeWire remote fd), serves a `videotestsrc` PipeWire node as "the
//! screen", and records the `Notify*` input calls. The host then runs its
//! real portal backend (ashpd + `gst-launch-1.0 pipewiresrc`) over real iroh.
//!
//! Covers: first connect asks for consent (phone sees `consent{waiting}`),
//! the restore token is stored and sent next time (no consent event),
//! frames decode, input maps to Notify* calls, Cancel → `denied`, view-only
//! uses a plain ScreenCast selection and sends no input.
//!
//! Skips without dbus-daemon / pipewire / wireplumber / gst-launch-1.0 /
//! pw-dump, or without the `desktop-portal` feature.
#![cfg(feature = "desktop-portal")]
mod desktop_common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use desktop_common::*;
use unipi_proto::{AppErrorCode, DesktopEvent};
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedValue, Value};

const NODE_W: u32 = 320;
const NODE_H: u32 = 240;

#[derive(Default, Debug)]
struct Record {
    calls: Vec<String>,
    restore_tokens_seen: Vec<Option<String>>,
    deny: bool,
    /// Act like a portal that refuses any RemoteDesktop persistence.
    no_rd_persist: bool,
    /// Session paths created through ScreenCast.CreateSession.
    screencast_sessions: Vec<String>,
}

#[derive(Clone)]
struct Portal {
    record: Arc<Mutex<Record>>,
    pw_socket: PathBuf,
    pw_env: Vec<(String, PathBuf)>,
    log_dir: PathBuf,
    /// One fresh "screen" node per Start, like GNOME (pre-started).
    sources: Arc<Mutex<Vec<std::process::Child>>>,
    nodes: Arc<Mutex<std::collections::VecDeque<u32>>>,
}

impl Portal {
    /// Starts a `videotestsrc` → PipeWire node and returns its id.
    async fn new_screen_node(&self) -> Option<u32> {
        let n = self.sources.lock().unwrap().len();
        let name = format!("uni216fake{n}");
        let mut cmd = std::process::Command::new("gst-launch-1.0");
        cmd.args([
            "-q",
            "videotestsrc",
            "is-live=true",
            "pattern=smpte",
            "!",
            // BGRx like a compositor's screen stream (pipewiresink mislabels I420).
            &format!("video/x-raw,format=BGRx,width={NODE_W},height={NODE_H},framerate=15/1"),
            "!",
            "pipewiresink",
            &format!("stream-properties=props,media.class=Video/Source,node.name={name}"),
        ])
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(self.log_dir.join(format!("{name}.log"))).unwrap());
        for (k, v) in &self.pw_env {
            cmd.env(k, v);
        }
        self.sources.lock().unwrap().push(cmd.spawn().ok()?);
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut dump = std::process::Command::new("pw-dump");
            for (k, v) in &self.pw_env {
                dump.env(k, v);
            }
            let Ok(out) = dump.output() else { continue };
            let Ok(list) = serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout) else { continue };
            if let Some(id) = list.iter().find(|o| o["info"]["props"]["node.name"] == name.as_str()).and_then(|o| o["id"].as_u64()) {
                return Some(id as u32);
            }
        }
        None
    }
}

fn sender_id(header: &zbus::message::Header<'_>) -> String {
    header.sender().map(|s| s.as_str().trim_start_matches(':').replace('.', "_")).unwrap_or_default()
}

fn opt_str(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

/// Emits `Response` on the request path shortly after the method returns.
fn respond(conn: zbus::Connection, path: String, code: u32, results: HashMap<String, OwnedValue>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let path = ObjectPath::try_from(path).unwrap();
        conn.emit_signal(None::<()>, &path, "org.freedesktop.portal.Request", "Response", &(code, results)).await.unwrap();
    });
}

impl Portal {
    fn request_path(header: &zbus::message::Header<'_>, options: &HashMap<String, OwnedValue>) -> String {
        format!("/org/freedesktop/portal/desktop/request/{}/{}", sender_id(header), opt_str(options, "handle_token").unwrap_or_default())
    }
    fn log(&self, call: impl Into<String>) {
        self.record.lock().unwrap().calls.push(call.into());
    }
}

/// `org.freedesktop.portal.Error.InvalidArgument` like xdg-desktop-portal.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum PortalErr {
    #[zbus(error)]
    ZBus(zbus::Error),
    InvalidArgument(String),
}
trait IntoPortal {
    fn into_portal(self) -> PortalErr;
}
impl IntoPortal for zbus::fdo::Error {
    fn into_portal(self) -> PortalErr {
        match self {
            zbus::fdo::Error::InvalidArgs(m) => PortalErr::InvalidArgument(m),
            other => PortalErr::ZBus(other.into()),
        }
    }
}

/// `Start` (RemoteDesktop or ScreenCast): one pre-started node, a token.
fn start_reply(portal: &Portal, header: &zbus::message::Header<'_>, options: &HashMap<String, OwnedValue>, conn: &zbus::Connection) -> OwnedObjectPathRet {
    let path = Portal::request_path(header, options);
    if portal.record.lock().unwrap().deny {
        respond(conn.clone(), path.clone(), 1, HashMap::new());
        return OwnedObjectPathRet(path);
    }
    let mut props: HashMap<String, OwnedValue> = HashMap::new();
    props.insert("size".into(), OwnedValue::try_from(Value::from((NODE_W as i32, NODE_H as i32))).unwrap());
    props.insert("source_type".into(), OwnedValue::from(1u32));
    let node = portal.nodes.lock().unwrap().pop_front().expect("a pre-started screen node");
    portal.log(format!("Node {node}"));
    let streams: Vec<(u32, HashMap<String, OwnedValue>)> = vec![(node, props)];
    let mut results: HashMap<String, OwnedValue> = HashMap::new();
    results.insert("devices".into(), OwnedValue::from(3u32));
    results.insert("streams".into(), OwnedValue::try_from(Value::from(streams)).unwrap());
    results.insert("restore_token".into(), OwnedValue::try_from(Value::from("fake-token-1")).unwrap());
    respond(conn.clone(), path.clone(), 0, results);
    OwnedObjectPathRet(path)
}

struct RemoteDesktopIface(Portal);
struct ScreenCastIface(Portal);

#[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
impl RemoteDesktopIface {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
    #[zbus(property, name = "AvailableDeviceTypes")]
    fn available_device_types(&self) -> u32 {
        3
    }
    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> OwnedObjectPathRet {
        self.0.log("CreateSession");
        let session = format!("/org/freedesktop/portal/desktop/session/{}/{}", sender_id(&header), opt_str(&options, "session_handle_token").unwrap_or_default());
        let mut results = HashMap::new();
        results.insert("session_handle".to_string(), OwnedValue::try_from(Value::from(session.as_str())).unwrap());
        let path = Portal::request_path(&header, &options);
        respond(conn.clone(), path.clone(), 0, results);
        OwnedObjectPathRet(path)
    }
    async fn select_devices(
        &self,
        _session: ObjectPath<'_>,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<OwnedObjectPathRet, PortalErr> {
        let token = opt_str(&options, "restore_token");
        let persist = options.get("persist_mode").and_then(|v| u32::try_from(v.try_clone().ok()?).ok());
        self.0.log(format!("SelectDevices persist={persist:?}"));
        if persist.is_some() && self.0.record.lock().unwrap().no_rd_persist {
            return Err(zbus::fdo::Error::InvalidArgs("Remote desktop sessions cannot persist".into()).into_portal());
        }
        self.0.record.lock().unwrap().restore_tokens_seen.push(token);
        let path = Portal::request_path(&header, &options);
        respond(conn.clone(), path.clone(), 0, HashMap::new());
        Ok(OwnedObjectPathRet(path))
    }
    async fn start(
        &self,
        _session: ObjectPath<'_>,
        _parent: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> OwnedObjectPathRet {
        self.0.log("Start");
        start_reply(&self.0, &header, &options, conn)
    }
    async fn notify_pointer_motion_absolute(&self, _s: ObjectPath<'_>, _o: HashMap<String, OwnedValue>, stream: u32, x: f64, y: f64) {
        self.0.log(format!("Motion {stream} {x:.0} {y:.0}"));
    }
    async fn notify_pointer_button(&self, _s: ObjectPath<'_>, _o: HashMap<String, OwnedValue>, button: i32, state: u32) {
        self.0.log(format!("Button {button:#x} {state}"));
    }
    async fn notify_pointer_axis_discrete(&self, _s: ObjectPath<'_>, _o: HashMap<String, OwnedValue>, axis: u32, steps: i32) {
        self.0.log(format!("Axis {axis} {steps}"));
    }
    async fn notify_keyboard_keysym(&self, _s: ObjectPath<'_>, _o: HashMap<String, OwnedValue>, keysym: i32, state: u32) {
        self.0.log(format!("Keysym {keysym:#x} {state}"));
    }
}

#[zbus::interface(name = "org.freedesktop.portal.ScreenCast")]
impl ScreenCastIface {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        5
    }
    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> OwnedObjectPathRet {
        self.0.log("ScreenCast.CreateSession");
        let session = format!("/org/freedesktop/portal/desktop/session/{}/{}", sender_id(&header), opt_str(&options, "session_handle_token").unwrap_or_default());
        self.0.record.lock().unwrap().screencast_sessions.push(session.clone());
        let mut results = HashMap::new();
        results.insert("session_handle".to_string(), OwnedValue::try_from(Value::from(session.as_str())).unwrap());
        let path = Portal::request_path(&header, &options);
        respond(conn.clone(), path.clone(), 0, results);
        OwnedObjectPathRet(path)
    }
    async fn start(
        &self,
        _session: ObjectPath<'_>,
        _parent: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> OwnedObjectPathRet {
        self.0.log("ScreenCast.Start");
        start_reply(&self.0, &header, &options, conn)
    }
    async fn select_sources(
        &self,
        session: ObjectPath<'_>,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<OwnedObjectPathRet, PortalErr> {
        let token = opt_str(&options, "restore_token");
        let persist = options.get("persist_mode").and_then(|v| u32::try_from(v.try_clone().ok()?).ok());
        self.0.log(format!("SelectSources persist={persist:?}"));
        // xdg-desktop-portal ≥ 1.19 (GNOME 50): RemoteDesktop sessions can't
        // persist through SelectSources (UNI-252).
        let is_screencast = self.0.record.lock().unwrap().screencast_sessions.iter().any(|s| s == session.as_str());
        if !is_screencast && (persist.is_some() || token.is_some()) {
            return Err(PortalErr::InvalidArgument("Remote desktop sessions cannot persist".into()));
        }
        if persist.is_some() {
            self.0.record.lock().unwrap().restore_tokens_seen.push(token);
        }
        let path = Portal::request_path(&header, &options);
        respond(conn.clone(), path.clone(), 0, HashMap::new());
        Ok(OwnedObjectPathRet(path))
    }
    async fn open_pipe_wire_remote(&self, _session: ObjectPath<'_>, _options: HashMap<String, OwnedValue>) -> zbus::fdo::Result<OwnedFd> {
        self.0.log("OpenPipeWireRemote");
        let stream = std::os::unix::net::UnixStream::connect(&self.0.pw_socket).map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(OwnedFd::from(std::os::fd::OwnedFd::from(stream)))
    }
}

/// Object path return value.
struct OwnedObjectPathRet(String);
impl serde::Serialize for OwnedObjectPathRet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ObjectPath::try_from(self.0.as_str()).unwrap().serialize(s)
    }
}
impl zbus::zvariant::Type for OwnedObjectPathRet {
    const SIGNATURE: &'static zbus::zvariant::Signature = ObjectPath::SIGNATURE;
}

struct Kill(Vec<std::process::Child>);
impl Drop for Kill {
    fn drop(&mut self) {
        for child in self.0.iter_mut().rev() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn have(bin: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file())).unwrap_or(false)
}

fn spawn(cmd: &str, args: &[&str], env: &[(&str, &Path)], log: &Path) -> std::process::Child {
    let mut command = std::process::Command::new(cmd);
    command.args(args).env_remove("DBUS_SESSION_BUS_ADDRESS").stdout(std::fs::File::create(log).unwrap()).stderr(std::process::Stdio::null());
    for (k, v) in env {
        command.env(k, v);
    }
    command.spawn().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn portal_capture_consent_token_and_input_on_private_bus() {
    for bin in ["dbus-daemon", "pipewire", "wireplumber", "gst-launch-1.0", "pw-dump"] {
        if !have(bin) {
            eprintln!("SKIP: no {bin}");
            return;
        }
    }
    // Everything private under one temp dir.
    let base = tempfile::tempdir().unwrap();
    let run = base.path().join("run");
    let conf = base.path().join("config/wireplumber/wireplumber.conf.d");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::create_dir_all(base.path().join("state")).unwrap();
    std::fs::write(
        conf.join("90-uni216.conf"),
        "wireplumber.profiles = { main = { monitor.alsa = disabled\n monitor.alsa-midi = disabled\n monitor.bluez = disabled\n monitor.bluez-midi = disabled\n monitor.v4l2 = disabled\n monitor.libcamera = disabled\n support.dbus = disabled\n support.reserve-device = disabled } }\n",
    )
    .unwrap();
    let pw_env: Vec<(&str, &Path)> = vec![
        ("XDG_RUNTIME_DIR", run.as_path()),
        ("PIPEWIRE_RUNTIME_DIR", run.as_path()),
        ("XDG_CONFIG_HOME", &base.path().join("config")),
        ("XDG_STATE_HOME", &base.path().join("state")),
    ]
    .into_iter()
    .map(|(k, v)| (k, Box::leak(v.to_path_buf().into_boxed_path()) as &Path))
    .collect();
    let bus_path = run.join("bus");
    let mut kids = Kill(Vec::new());
    kids.0.push(spawn(
        "dbus-daemon",
        &["--session", "--nofork", "--nopidfile", &format!("--address=unix:path={}", bus_path.display())],
        &[],
        &base.path().join("dbus.log"),
    ));
    kids.0.push(spawn("pipewire", &[], &pw_env, &base.path().join("pw.log")));
    tokio::time::sleep(Duration::from_millis(800)).await;
    kids.0.push(spawn("wireplumber", &[], &pw_env, &base.path().join("wp.log")));
    tokio::time::sleep(Duration::from_millis(800)).await;
    // The fake portal on the private bus. Point THIS process's session bus
    // there before ashpd connects (it caches the connection).
    let bus_addr = format!("unix:path={}", bus_path.display());
    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &bus_addr);
    std::env::set_var("XDG_RUNTIME_DIR", &run);
    std::env::set_var("WAYLAND_DISPLAY", "wayland-uni216-none");
    let record = Arc::new(Mutex::new(Record::default()));
    let portal = Portal {
        record: record.clone(),
        pw_socket: run.join("pipewire-0"),
        pw_env: pw_env.iter().map(|(k, v)| (k.to_string(), v.to_path_buf())).collect(),
        log_dir: base.path().to_path_buf(),
        sources: Arc::new(Mutex::new(Vec::new())),
        nodes: Arc::new(Mutex::new(std::collections::VecDeque::new())),
    };
    struct KillSources(Arc<Mutex<Vec<std::process::Child>>>);
    impl Drop for KillSources {
        fn drop(&mut self) {
            for child in self.0.lock().unwrap().iter_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    let _sources = KillSources(portal.sources.clone());
    for _ in 0..5 {
        let Some(node) = portal.new_screen_node().await else {
            eprintln!("SKIP: the private PipeWire never showed the test node");
            return;
        };
        portal.nodes.lock().unwrap().push_back(node);
    }
    let _service = zbus::connection::Builder::address(bus_addr.as_str())
        .unwrap()
        .name("org.freedesktop.portal.Desktop")
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", RemoteDesktopIface(portal.clone()))
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", ScreenCastIface(portal.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();

    let rig = Rig::new().await;
    rig.config(serde_json::json!({"enabled":true,"backend":"portal"}));
    let open = serde_json::json!({"op":"desktop_open","viewport":{"width":1080,"height":2400}});

    // 1. First connect: no token → consent event, token stored, frames decode.
    let mut viewer = rig.open(open.clone()).await.unwrap_or_else(|c| panic!("{c:?}"));
    viewer.until("consent", |e, _| matches!(e, DesktopEvent::Consent { .. })).await;
    let started = viewer.until("started", |e, _| matches!(e, DesktopEvent::Started { .. })).await;
    assert!(matches!(started, DesktopEvent::Started { width: NODE_W, height: NODE_H, ref backend, view_only: false, .. } if backend == "portal"), "{started:?}");
    viewer.until("decoded frames", |_, v| v.decoded.len() >= 3).await;
    assert_eq!(viewer.decoded.last(), Some(&(NODE_W as usize, NODE_H as usize)));
    let row: Vec<[u8; 3]> = (0..7).map(|i| viewer.pixel((i as f64 + 0.5) / 7.0, 0.3)).collect();
    eprintln!("top bars: {row:?}");
    viewer.dump("portal-first");
    // SMPTE bars: the 7th of 7 top bars is blue, the 6th red.
    let blue = viewer.pixel(6.5 / 7.0, 0.3);
    let red = viewer.pixel(5.5 / 7.0, 0.3);
    assert!(red[0] > 150 && red[1] < 80 && red[2] < 80, "red bar {red:?}");
    assert!(blue[2] > 150 && blue[0] < 80 && blue[1] < 80, "blue bar {blue:?}");
    assert_eq!(std::fs::read_to_string(rig.paths.root.join("desktop-token-control")).unwrap().trim(), "fake-token-1");

    // Input → portal Notify* calls in screen pixels / evdev codes / keysyms.
    viewer.send(serde_json::json!({"op":"pointer","x":0.5,"y":0.25})).await;
    viewer.send(serde_json::json!({"op":"button","button":"right","down":true})).await;
    viewer.send(serde_json::json!({"op":"button","button":"right","down":false})).await;
    viewer.send(serde_json::json!({"op":"scroll","dy":-1})).await;
    viewer.send(serde_json::json!({"op":"key","keysym":0xff0d,"down":true})).await;
    viewer.send(serde_json::json!({"op":"key","keysym":0xff0d,"down":false})).await;
    viewer.until("more frames", |_, v| v.decoded.len() >= 8).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let calls = record.lock().unwrap().calls.clone();
    eprintln!("portal calls: {calls:?}");
    for want in [
        "CreateSession",
        "SelectDevices persist=Some(2)",
        "SelectSources persist=None",
        "Start",
        "OpenPipeWireRemote",
        &format!("Motion {} 160 60", calls.iter().find_map(|c| c.strip_prefix("Node ")).unwrap()),
        "Button 0x111 1",
        "Button 0x111 0",
        "Axis 0 -1",
        "Keysym 0xff0d 1",
        "Keysym 0xff0d 0",
    ] {
        assert!(calls.iter().any(|c| c == want), "missing {want:?} in {calls:?}");
    }
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 2. Second connect: the stored token is sent, no consent event.
    record.lock().unwrap().calls.clear();
    let mut viewer = rig.open(open.clone()).await.unwrap_or_else(|c| panic!("{c:?}"));
    let mut saw_consent = false;
    viewer
        .until("started", |e, _| {
            saw_consent |= matches!(e, DesktopEvent::Consent { .. });
            matches!(e, DesktopEvent::Started { .. })
        })
        .await;
    assert!(!saw_consent, "a stored token means no consent prompt");
    assert_eq!(record.lock().unwrap().restore_tokens_seen.last().cloned().flatten().as_deref(), Some("fake-token-1"));
    viewer.until("frames", |_, v| v.decoded.len() >= 2).await;
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. View-only: plain ScreenCast selection with persist, no input calls.
    rig.config(serde_json::json!({"enabled":true,"backend":"portal","view_only":true}));
    record.lock().unwrap().calls.clear();
    let mut viewer = rig.open(open.clone()).await.unwrap_or_else(|c| panic!("{c:?}"));
    viewer.until("started", |e, _| matches!(e, DesktopEvent::Started { view_only: true, .. })).await;
    viewer.send(serde_json::json!({"op":"pointer","x":0.1,"y":0.1})).await;
    viewer.until("frames", |_, v| v.decoded.len() >= 3).await;
    let calls = record.lock().unwrap().calls.clone();
    assert!(calls.iter().any(|c| c == "SelectSources persist=Some(2)"), "{calls:?}");
    assert!(calls.iter().any(|c| c == "ScreenCast.CreateSession") && calls.iter().any(|c| c == "ScreenCast.Start"), "a pure ScreenCast session: {calls:?}");
    assert!(!calls.iter().any(|c| c == "CreateSession" || c == "Start"), "no RemoteDesktop session in view-only: {calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("SelectDevices") || c.starts_with("Motion")), "{calls:?}");
    assert_eq!(std::fs::read_to_string(rig.paths.root.join("desktop-token-view")).unwrap().trim(), "fake-token-1");
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3b. A portal that refuses RemoteDesktop persistence altogether:
    // control retries transient (asks every time), no token stored.
    rig.config(serde_json::json!({"enabled":true,"backend":"portal"}));
    let _ = std::fs::remove_file(rig.paths.root.join("desktop-token-control"));
    record.lock().unwrap().no_rd_persist = true;
    record.lock().unwrap().calls.clear();
    let mut viewer = rig.open(open.clone()).await.unwrap_or_else(|c| panic!("{c:?}"));
    viewer.until("consent", |e, _| matches!(e, DesktopEvent::Consent { .. })).await;
    viewer.until("started transient", |e, _| matches!(e, DesktopEvent::Started { view_only: false, .. })).await;
    viewer.until("frames", |_, v| v.decoded.len() >= 2).await;
    let calls = record.lock().unwrap().calls.clone();
    eprintln!("transient calls: {calls:?}");
    assert!(calls.iter().any(|c| c == "SelectDevices persist=Some(2)") && calls.iter().any(|c| c == "SelectDevices persist=None"), "{calls:?}");
    assert!(!rig.paths.root.join("desktop-token-control").exists(), "transient sessions store no token");
    record.lock().unwrap().no_rd_persist = false;
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 4. Cancel in the share dialog → E_DESKTOP_UNAVAILABLE{denied}.
    rig.config(serde_json::json!({"enabled":true,"backend":"portal"}));
    let _ = std::fs::remove_file(rig.paths.root.join("desktop-token-control"));
    record.lock().unwrap().deny = true;
    let mut viewer = rig.open(open).await.unwrap_or_else(|c| panic!("{c:?}"));
    let error = loop {
        match viewer.next().await {
            Next::Err(error) => break error,
            Next::Event(DesktopEvent::Consent { .. }) => {}
            Next::Event(other) => panic!("unexpected {other:?}"),
            Next::Eof => panic!("ended without an error"),
        }
    };
    assert_eq!(error.code, AppErrorCode::E_DESKTOP_UNAVAILABLE);
    assert_eq!(error.detail.as_ref().unwrap()["reason"], "denied", "{error:?}");
    let log = std::fs::read_to_string(rig.paths.root.join("desktop.log")).unwrap();
    assert!(log.contains("refused: denied"), "{log}");
}
