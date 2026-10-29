//! UNI-252, the GNOME-native `mutter` backend against a REAL Mutter: a
//! PRIVATE headless `gnome-shell --headless --virtual-monitor 1280x720` on a
//! PRIVATE session bus with a PRIVATE PipeWire + WirePlumber — never the
//! owner's session (nothing appears on their screen).
//!
//! Covers: `org.gnome.Mutter.{RemoteDesktop,ScreenCast}` sessions start with
//! no dialog, the PipeWire node is read by the real gst reader, frames
//! decode at the virtual monitor's size, input goes through `Notify*` and
//! visibly changes the picture (the embedded cursor moves; Escape/Super toggles
//! the overview), view-only makes a plain ScreenCast session and sends no
//! input, and the session ends cleanly when the viewer leaves.
//!
//! Skips without gnome-shell / dbus-daemon / pipewire / wireplumber /
//! gst-launch-1.0, or without the `desktop-mutter` feature.
#![cfg(all(feature = "desktop-mutter", target_os = "linux"))]
mod desktop_common;

use std::path::Path;
use std::time::Duration;

use desktop_common::*;
use unipi_proto::DesktopEvent;

const W: u32 = 1280;
const H: u32 = 720;

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
    command
        .args(args)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env_remove("XDG_SESSION_ID")
        .stdout(std::fs::File::create(log).unwrap())
        .stderr(std::process::Stdio::from(std::fs::File::create(log.with_extension("err")).unwrap()));
    for (k, v) in env {
        command.env(k, v);
    }
    command.spawn().unwrap()
}

/// Mean absolute difference of two decoded pictures (RGBA).
fn diff(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum::<u64>() as f64 / a.len() as f64
}

/// Pixels that differ by more than 40 in a region (x0..x1, y0..y1 in 0..1).
fn changed_in(a: &[u8], b: &[u8], w: usize, h: usize, region: (f64, f64, f64, f64)) -> usize {
    let (x0, x1, y0, y1) = ((region.0 * w as f64) as usize, (region.1 * w as f64) as usize, (region.2 * h as f64) as usize, (region.3 * h as f64) as usize);
    let mut n = 0;
    for y in y0..y1.min(h) {
        for x in x0..x1.min(w) {
            let i = (y * w + x) * 4;
            if (0..3).any(|c| (a[i + c] as i32 - b[i + c] as i32).abs() > 40) {
                n += 1;
            }
        }
    }
    n
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mutter_capture_and_input_on_private_headless_gnome_shell() {
    for bin in ["gnome-shell", "dbus-daemon", "pipewire", "wireplumber", "gst-launch-1.0"] {
        if !have(bin) {
            eprintln!("SKIP: no {bin}");
            return;
        }
    }
    let base = tempfile::Builder::new().prefix("uni252-").tempdir().unwrap();
    let run = base.path().join("run");
    let conf = base.path().join("config/wireplumber/wireplumber.conf.d");
    let home = base.path().join("home");
    for dir in [&run, &conf, &home, &base.path().join("state")] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(
        conf.join("90-uni252.conf"),
        "wireplumber.profiles = { main = { monitor.alsa = disabled\n monitor.alsa-midi = disabled\n monitor.bluez = disabled\n monitor.bluez-midi = disabled\n monitor.v4l2 = disabled\n monitor.libcamera = disabled\n support.dbus = disabled\n support.reserve-device = disabled } }\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let bus_path = run.join("bus");
    let bus_addr = format!("unix:path={}", bus_path.display());
    let config_home = base.path().join("config");
    let state_home = base.path().join("state");
    let bus_env_path = std::path::PathBuf::from(&bus_addr);
    let env: Vec<(&str, &Path)> = vec![
        ("XDG_RUNTIME_DIR", run.as_path()),
        ("PIPEWIRE_RUNTIME_DIR", run.as_path()),
        ("XDG_CONFIG_HOME", config_home.as_path()),
        ("XDG_STATE_HOME", state_home.as_path()),
    ];
    let mut kids = Kill(Vec::new());
    kids.0.push(spawn("dbus-daemon", &["--session", "--nofork", "--nopidfile", &format!("--address={bus_addr}")], &[], &base.path().join("dbus.log")));
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut with_bus = env.clone();
    with_bus.push(("DBUS_SESSION_BUS_ADDRESS", bus_env_path.as_path()));
    kids.0.push(spawn("pipewire", &[], &with_bus, &base.path().join("pw.log")));
    tokio::time::sleep(Duration::from_millis(800)).await;
    kids.0.push(spawn("wireplumber", &[], &with_bus, &base.path().join("wp.log")));
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut shell_env = with_bus.clone();
    shell_env.push(("HOME", home.as_path()));
    shell_env.push(("XDG_CURRENT_DESKTOP", Path::new("GNOME")));
    kids.0.push(spawn(
        "gnome-shell",
        &["--headless", "--wayland", "--no-x11", "--wayland-display", "wl-uni252", "--virtual-monitor", &format!("{W}x{H}")],
        &shell_env,
        &base.path().join("shell.log"),
    ));
    // Wait for Mutter's screen-cast API on the private bus.
    let mut up = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let out = std::process::Command::new("busctl").args(["--address", &bus_addr, "list"]).output();
        if out.map(|o| String::from_utf8_lossy(&o.stdout).contains("org.gnome.Mutter.RemoteDesktop")).unwrap_or(false) {
            up = true;
            break;
        }
    }
    if !up {
        eprintln!("SKIP: the private headless gnome-shell never came up ({})", base.path().display());
        return;
    }
    tokio::time::sleep(Duration::from_secs(2)).await; // let the shell settle (startup animation)

    // This test process (= the host) talks to the private bus/PipeWire only.
    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &bus_addr);
    std::env::set_var("XDG_RUNTIME_DIR", &run);
    std::env::set_var("PIPEWIRE_RUNTIME_DIR", &run);
    std::env::set_var("WAYLAND_DISPLAY", "wl-uni252");
    std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME");

    let rig = Rig::new().await;
    rig.config(serde_json::json!({"enabled":true}));
    // `auto` on GNOME Wayland picks mutter, with no consent step.
    let reply = request(&rig.conn, serde_json::json!({"op":"desktop_status"})).await;
    let unipi_proto::Control::Ok { data: Some(data) } = reply else { panic!("{reply:?}") };
    assert_eq!(data["backend"], "mutter", "{data}");
    assert_eq!(data["consent"], "not_needed", "{data}");

    let open = serde_json::json!({"op":"desktop_open","viewport":{"width":2048,"height":2048},"quality":"high"});
    let mut viewer = rig.open(open.clone()).await.unwrap_or_else(|c| panic!("{c:?}"));
    let mut saw_consent = false;
    let started = viewer
        .until("started", |e, _| {
            saw_consent |= matches!(e, DesktopEvent::Consent { .. });
            matches!(e, DesktopEvent::Started { .. })
        })
        .await;
    assert!(!saw_consent, "Mutter asks nobody");
    assert!(matches!(started, DesktopEvent::Started { width: W, height: H, ref backend, view_only: false, .. } if backend == "mutter"), "{started:?}");
    viewer.until("decoded frames", |_, v| v.decoded.len() >= 3).await;
    assert_eq!(viewer.decoded.last(), Some(&(W as usize, H as usize)));
    viewer.dump("mutter-first");
    let (_, _, first) = viewer.last_rgba.clone().unwrap();
    // A real desktop picture, not black: the shell draws a panel/background.
    let lit = first.chunks_exact(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60).count();
    assert!(lit > (W * H / 50) as usize, "picture looks empty ({lit} lit pixels)");

    // Pointer: park the cursor at the left, then move it far right; the
    // embedded cursor must show up there.
    viewer.send(serde_json::json!({"op":"pointer","x":0.1,"y":0.6})).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    viewer.send(serde_json::json!({"op":"keyframe"})).await;
    let n = viewer.decoded.len();
    viewer.until("frames after move 1", |_, v| v.decoded.len() >= n + 4).await;
    let (_, _, at_left) = viewer.last_rgba.clone().unwrap();
    viewer.send(serde_json::json!({"op":"pointer","x":0.8,"y":0.6})).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    viewer.send(serde_json::json!({"op":"keyframe"})).await;
    let n = viewer.decoded.len();
    viewer.until("frames after move 2", |_, v| v.decoded.len() >= n + 4).await;
    let (_, _, at_right) = viewer.last_rgba.clone().unwrap();
    viewer.dump("mutter-pointer-right");
    let near_right = changed_in(&at_left, &at_right, W as usize, H as usize, (0.78, 0.83, 0.58, 0.66));
    let near_left = changed_in(&at_left, &at_right, W as usize, H as usize, (0.08, 0.13, 0.58, 0.66));
    eprintln!("cursor diff: right {near_right}, left {near_left}");
    // (The first motion creates Mutter's virtual pointer, so the cursor may
    // not be drawn at the left yet; it must be at the right afterwards.)
    assert!(near_right > 20, "the cursor appeared at (0.8, 0.6) (right {near_right}, left {near_left})");

    // Keyboard: Super toggles the overview — a big visible change.
    let before = at_right;
    viewer.send(serde_json::json!({"op":"key","keysym":0xffeb,"down":true})).await;
    viewer.send(serde_json::json!({"op":"key","keysym":0xffeb,"down":false})).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    viewer.send(serde_json::json!({"op":"keyframe"})).await;
    let n = viewer.decoded.len();
    viewer.until("frames after Super", |_, v| v.decoded.len() >= n + 4).await;
    let (_, _, after) = viewer.last_rgba.clone().unwrap();
    viewer.dump("mutter-after-super");
    let d = diff(&before, &after);
    eprintln!("Super key picture change: {d:.2}");
    assert!(d > 3.0, "Super toggled the overview (mean diff {d:.2})");
    // Buttons + scroll go through without Mutter errors (no visible check).
    viewer.send(serde_json::json!({"op":"button","button":"left","down":true})).await;
    viewer.send(serde_json::json!({"op":"button","button":"left","down":false})).await;
    viewer.send(serde_json::json!({"op":"scroll","dy":1})).await;
    let n = viewer.decoded.len();
    viewer.until("still streaming", |_, v| v.decoded.len() >= n + 2).await;
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(800)).await;

    // View-only: a plain ScreenCast session; still frames, input ignored.
    rig.config(serde_json::json!({"enabled":true,"view_only":true}));
    let mut viewer = rig.open(open).await.unwrap_or_else(|c| panic!("{c:?}"));
    viewer.until("started view-only", |e, _| matches!(e, DesktopEvent::Started { view_only: true, .. })).await;
    viewer.until("frames", |_, v| v.decoded.len() >= 3).await;
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let log = std::fs::read_to_string(rig.paths.root.join("desktop.log")).unwrap();
    assert_eq!(log.matches("backend=mutter").count(), 2, "{log}");
    assert!(!log.contains("refused"), "{log}");
    drop(kids);
}

/// On demand, on the REAL login session (run from a terminal inside it):
/// `cargo test -p unipi-host --test desktop_mutter real_session -- --ignored`.
/// View-only (no input device is ever created), ~2 s, nothing written
/// anywhere: GNOME shows no dialog, only its "screen is being shared" icon
/// in the top bar for a few seconds. Proves Mutter accepts a plain user
/// process (no portal, no allow-list) on this machine.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn real_session_view_only_probe() {
    let rig = Rig::new().await;
    rig.config(serde_json::json!({"enabled":true,"view_only":true,"backend":"mutter"}));
    let mut viewer = rig
        .open(serde_json::json!({"op":"desktop_open","viewport":{"width":1280,"height":1280},"quality":"low"}))
        .await
        .unwrap_or_else(|c| panic!("{c:?}"));
    let started = viewer.until("started", |e, _| matches!(e, DesktopEvent::Started { .. })).await;
    eprintln!("real session: {started:?}");
    viewer.until("decoded frames", |_, v| v.decoded.len() >= 3).await;
    eprintln!("decoded {:?} (frames {}, keys {})", viewer.decoded.last(), viewer.frames, viewer.keys);
    drop(viewer);
    tokio::time::sleep(Duration::from_millis(500)).await;
}
