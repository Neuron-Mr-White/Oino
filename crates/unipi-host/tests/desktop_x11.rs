//! UNI-216, real capture on a PRIVATE X server: starts its own `Xvfb` on a
//! free display number (never the owner's session), paints it, and streams
//! it through the host over real iroh with the `x11` backend — then checks
//! the decoded H.264 shows the paint, and that phone input arrives as real
//! X events (XTest) in a window we own.
//!
//! Skips (passes with a note) without `Xvfb` (`UNIPI_TEST_XVFB=/path/to/Xvfb`
//! or on PATH) or without the `desktop-x11` feature.
#![cfg(feature = "desktop-x11")]
mod desktop_common;

use std::time::Duration;

use desktop_common::*;
use unipi_proto::DesktopEvent;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask, Rectangle, WindowClass,
};
use x11rb::protocol::Event;

struct Xvfb(std::process::Child, String);
impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn xvfb_binary() -> Option<String> {
    if let Ok(path) = std::env::var("UNIPI_TEST_XVFB") {
        return Some(path);
    }
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|d| d.join("Xvfb")).find(|p| p.is_file()))
        .map(|p| p.display().to_string())
}

fn start_xvfb(bin: &str) -> Option<Xvfb> {
    for n in 180..240 {
        if std::path::Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() || std::path::Path::new(&format!("/tmp/.X{n}-lock")).exists() {
            continue;
        }
        let display = format!(":{n}");
        let child = std::process::Command::new(bin)
            .args([&display, "-screen", "0", "1024x768x24", "-nolisten", "tcp", "-noreset"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let x = Xvfb(child, display.clone());
        for _ in 0..50 {
            if x11rb::rust_connection::RustConnection::connect(Some(&display)).is_ok() {
                return Some(x);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        return None;
    }
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn x11_capture_and_input_on_private_xvfb() {
    let Some(bin) = xvfb_binary() else {
        eprintln!("SKIP: no Xvfb (set UNIPI_TEST_XVFB)");
        return;
    };
    let Some(xvfb) = start_xvfb(&bin) else {
        eprintln!("SKIP: Xvfb did not start");
        return;
    };
    eprintln!("private X server {}", xvfb.1);

    // Paint: left half green, right half blue; plus a white input window
    // in the top-left 200×200 that records the events it gets.
    let (conn, screen_num) = x11rb::rust_connection::RustConnection::connect(Some(&xvfb.1)).unwrap();
    let screen = conn.setup().roots[screen_num].clone();
    let root = screen.root;
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, root, &CreateGCAux::new().foreground(0x00c000).subwindow_mode(x11rb::protocol::xproto::SubwindowMode::INCLUDE_INFERIORS)).unwrap();
    conn.change_window_attributes(root, &ChangeWindowAttributesAux::new().background_pixel(0x0000c0)).unwrap();
    conn.clear_area(false, root, 0, 0, 0, 0).unwrap();
    conn.poly_fill_rectangle(root, gc, &[Rectangle { x: 0, y: 0, width: 512, height: 768 }]).unwrap();
    let win = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        win,
        root,
        0,
        300,
        200,
        200,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(0xffffff)
            .override_redirect(1)
            .event_mask(EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::KEY_PRESS | EventMask::POINTER_MOTION),
    )
    .unwrap();
    conn.map_window(win).unwrap();
    conn.set_input_focus(x11rb::protocol::xproto::InputFocus::POINTER_ROOT, win, x11rb::CURRENT_TIME).unwrap();
    conn.flush().unwrap();

    // The host, told to use x11 on that display only.
    std::env::set_var("DISPLAY", &xvfb.1);
    let rig = Rig::new().await;
    rig.config(serde_json::json!({"enabled":true,"backend":"x11"}));
    let mut viewer = rig.open(serde_json::json!({"op":"desktop_open","viewport":{"width":2048,"height":2048},"quality":"high"})).await.unwrap_or_else(|c| panic!("{c:?}"));
    let started = viewer.until("started", |e, _| matches!(e, DesktopEvent::Started { .. })).await;
    assert!(matches!(started, DesktopEvent::Started { width: 1024, height: 768, ref backend, .. } if backend == "x11"), "{started:?}");
    viewer.until("decoded frames", |_, v| v.decoded.len() >= 3).await;
    assert_eq!(viewer.decoded.last(), Some(&(1024, 768)));
    let green = viewer.pixel(0.25, 0.2);
    let blue = viewer.pixel(0.75, 0.5);
    let white = viewer.pixel(100.0 / 1023.0, 400.0 / 767.0);
    assert!(green[1] > 150 && green[0] < 60 && green[2] < 60, "green half {green:?}");
    assert!(blue[2] > 150 && blue[0] < 60 && blue[1] < 60, "blue half {blue:?}");
    assert!(white.iter().all(|&c| c > 200), "white window {white:?}");

    // Input: move into the window, click, type "a".
    viewer.send(serde_json::json!({"op":"pointer","x":100.0/1023.0,"y":400.0/767.0})).await;
    viewer.send(serde_json::json!({"op":"button","button":"left","down":true})).await;
    viewer.send(serde_json::json!({"op":"button","button":"left","down":false})).await;
    viewer.send(serde_json::json!({"op":"text","text":"a"})).await;
    viewer.send(serde_json::json!({"op":"scroll","dy":1})).await;
    viewer.until("more frames", |_, v| v.decoded.len() >= 8).await;

    let mut got = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        while let Some(event) = conn.poll_for_event().unwrap() {
            match event {
                Event::ButtonPress(e) => got.push(format!("press{}@{},{}", e.detail, e.event_x, e.event_y)),
                Event::KeyPress(_) => got.push("key".into()),
                Event::MotionNotify(_) => {
                    if !got.contains(&"motion".to_string()) {
                        got.push("motion".into())
                    }
                }
                _ => {}
            }
        }
        if got.iter().any(|g| g == "key") && got.iter().any(|g| g.starts_with("press5")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!("X events: {got:?}");
    assert!(got.contains(&"motion".to_string()), "{got:?}");
    assert!(got.contains(&"press1@100,100".to_string()), "{got:?}");
    assert!(got.iter().any(|g| g == "key"), "{got:?}");
    assert!(got.iter().any(|g| g.starts_with("press5")), "scroll down = button 5: {got:?}");
    drop(viewer);
    drop(xvfb);
}
