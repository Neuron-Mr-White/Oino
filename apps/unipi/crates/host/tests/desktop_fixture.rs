//! Records the H.264 fixture used by the design captures
//! (`docs/m7/capture-desktop-stream.py`): a PRIVATE Xvfb (1440×900) with a few real
//! X11 apps (on a private D-Bus, GDK forced to X11 — nothing reaches the
//! owner's session), captured by the real `x11` backend, encoded by the real
//! encoder, streamed over real iroh, and written as received by the phone.
//!
//! `UNIPI_RECORD_FIXTURE=docs/m7/fixtures cargo test -p unipi-host --test desktop_fixture -- --ignored`
//! Output: `desktop-h264.bin` (records: u32be length · u8 key · Annex-B AU)
//! and `desktop-h264.json` (codec, width, height, frames).
#![cfg(feature = "desktop-x11")]
mod desktop_common;

use std::time::Duration;

use desktop_common::*;
use unipi_proto::DesktopEvent;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt as _};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "writes docs fixtures; run on demand"]
async fn record_h264_fixture() {
    let Ok(out_dir) = std::env::var("UNIPI_RECORD_FIXTURE") else {
        eprintln!("SKIP: set UNIPI_RECORD_FIXTURE=<dir>");
        return;
    };
    let xvfb = std::env::var("UNIPI_TEST_XVFB").unwrap_or_else(|_| "Xvfb".into());
    let n = (190..230).find(|n| !std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()).unwrap();
    let display = format!(":{n}");
    struct Kill(Vec<std::process::Child>);
    impl Drop for Kill {
        fn drop(&mut self) {
            for c in self.0.iter_mut().rev() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
    let mut kids = Kill(vec![std::process::Command::new(&xvfb)
        .args([&display, "-screen", "0", "1440x900x24", "-nolisten", "tcp"])
        .spawn()
        .unwrap()]);
    std::thread::sleep(Duration::from_millis(800));
    let (conn, screen) = x11rb::rust_connection::RustConnection::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().background_pixel(0x2b2135)).unwrap();
    conn.clear_area(false, root, 0, 0, 0, 0).unwrap();
    conn.flush().unwrap();
    // Real apps, X11 only, private session bus.
    let app = |cmd: &str| {
        std::process::Command::new("dbus-run-session")
            .args(["--", "sh", "-c", cmd])
            .env("DISPLAY", &display)
            .env("GDK_BACKEND", "x11")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    kids.0.push(app("exec gnome-calculator"));
    kids.0.push(app("exec zenity --list --width=640 --height=420 --title='Deploy' --column=Service --column=Status unipi-host running kanboard running vite 'port 5173' herdr '3 panes'"));
    kids.0.push(app("exec xmessage -geometry +860+520 -center 'Build finished: 0 errors, 2 warnings'"));
    std::thread::sleep(Duration::from_secs(4));
    // No window manager: lay the top-level windows out ourselves.
    let tree = conn.query_tree(root).unwrap().reply().unwrap();
    let mut placed = 0;
    let spots = [(60i32, 70i32), (520, 120), (1000, 640)];
    for &w in &tree.children {
        let Ok(attrs) = conn.get_window_attributes(w).unwrap().reply() else { continue };
        let Ok(geo) = conn.get_geometry(w).unwrap().reply() else { continue };
        if attrs.map_state != x11rb::protocol::xproto::MapState::VIEWABLE || geo.width < 100 {
            continue;
        }
        let (x, y) = if geo.width < 400 && geo.height > 400 { spots[0] } else if geo.height > 200 { spots[1] } else { spots[2] };
        conn.configure_window(w, &x11rb::protocol::xproto::ConfigureWindowAux::new().x(x).y(y)).unwrap();
        placed += 1;
    }
    conn.flush().unwrap();
    eprintln!("placed {placed} windows");
    std::thread::sleep(Duration::from_millis(800));

    std::env::set_var("DISPLAY", &display);
    let rig = Rig::new().await;
    rig.config(serde_json::json!({"enabled":true,"backend":"x11"}));
    let mut viewer = rig
        .open(serde_json::json!({"op":"desktop_open","viewport":{"width":2880,"height":1800},"quality":"high"}))
        .await
        .unwrap_or_else(|c| panic!("{c:?}"));
    let mut records = Vec::new();
    let mut frames = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while frames < 40 && tokio::time::Instant::now() < deadline {
        // Read raw so we can keep the bytes.
        let (kind, bytes) = unipi_host::protocol::read_frame(&mut viewer.recv).await.unwrap();
        assert_eq!(kind, 0);
        let control: unipi_proto::Control = serde_json::from_slice(&bytes).unwrap();
        let unipi_proto::Control::DesktopEvent { event } = control else { panic!("{control:?}") };
        match event {
            DesktopEvent::Config { codec, width, height } => viewer.config = Some((codec, width, height)),
            DesktopEvent::Frame(meta) => {
                let mut au = Vec::new();
                while au.len() < meta.bytes as usize {
                    let (_, chunk) = unipi_host::protocol::read_frame(&mut viewer.recv).await.unwrap();
                    au.extend_from_slice(&chunk);
                }
                if frames == 0 {
                    assert!(meta.key);
                }
                records.extend_from_slice(&(au.len() as u32).to_be_bytes());
                records.push(meta.key as u8);
                records.extend_from_slice(&au);
                frames += 1;
                viewer.send(serde_json::json!({"op":"ack","seq":meta.seq})).await;
            }
            _ => {}
        }
    }
    let (codec, width, height) = viewer.config.clone().unwrap();
    let dir = std::path::Path::new(&out_dir);
    std::fs::write(dir.join("desktop-h264.bin"), &records).unwrap();
    std::fs::write(
        dir.join("desktop-h264.json"),
        serde_json::to_string_pretty(&serde_json::json!({"codec":codec,"width":width,"height":height,"frames":frames,"source":"x11 backend on a private Xvfb 1440x900 (gnome-calculator, zenity, xmessage)"})).unwrap(),
    )
    .unwrap();
    eprintln!("wrote {frames} frames, {} bytes, {codec} {width}x{height}", records.len());
}
