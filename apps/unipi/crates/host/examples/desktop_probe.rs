//! A headless desktop-stream viewer for cross-OS checks (UNI-246/247/252):
//! pairs with a host from its `unipi://pair?...` link (or reuses a stored
//! identity), opens `desktop_open`, decodes the H.264 with openh264 like the
//! app's WebCodecs path does, saves the last picture as PPM and the raw
//! Annex-B access units (`stream.h264`, playable by ffplay) plus
//! `records.bin` in the app's fixture format (docs/m7/fixtures), and can
//! send input.
//!
//! ```text
//! cargo run -p unipi-host --example desktop_probe -- \
//!     --state /tmp/probe --link 'unipi://pair?t=…&c=…&n=…' \
//!     [--frames 30] [--view-only] [--input "pointer 0.5 0.5; click; text hi; key 0xff0d"]
//! ```
//! Prints one JSON summary line at the end.

use std::time::{Duration, Instant};

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use unipi_host::endpoint::ALPN;
use unipi_host::protocol::read_frame;
use unipi_proto::{Control, DesktopEvent};

fn hello(req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &serde_json::to_vec(&value).unwrap())
}

fn cmd(cmd: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"desktop_cmd","cmd":cmd});
    unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &serde_json::to_vec(&value).unwrap())
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// "pointer 0.5 0.5; click; rclick; scroll 2; text hello; key 0xff0d; down 0xffe3; up 0xffe3"
fn input_script(script: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = step.splitn(2, ' ').collect();
        let rest = parts.get(1).copied().unwrap_or("");
        match parts[0] {
            "pointer" => {
                let v: Vec<f64> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                out.push(serde_json::json!({"op":"pointer","x":v[0],"y":v[1]}));
            }
            "click" | "rclick" => {
                let b = if parts[0] == "click" { "left" } else { "right" };
                out.push(serde_json::json!({"op":"button","button":b,"down":true}));
                out.push(serde_json::json!({"op":"button","button":b,"down":false}));
            }
            "scroll" => out.push(serde_json::json!({"op":"scroll","dy":rest.trim().parse::<f64>().unwrap_or(1.0)})),
            "text" => out.push(serde_json::json!({"op":"text","text":rest})),
            "down" | "up" => {
                let k = u32::from_str_radix(rest.trim().trim_start_matches("0x"), 16).unwrap_or(0xffe3);
                out.push(serde_json::json!({"op":"key","keysym":k,"down":parts[0] == "down"}));
            }
            "key" => {
                let k = u32::from_str_radix(rest.trim().trim_start_matches("0x"), 16).unwrap_or(0xff0d);
                out.push(serde_json::json!({"op":"key","keysym":k,"down":true}));
                out.push(serde_json::json!({"op":"key","keysym":k,"down":false}));
            }
            other => eprintln!("unknown input step {other:?}"),
        }
    }
    out
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = std::path::PathBuf::from(arg("--state").unwrap_or_else(|| "/tmp/desktop-probe".into()));
    std::fs::create_dir_all(&state)?;
    let frames_wanted: usize = arg("--frames").and_then(|f| f.parse().ok()).unwrap_or(20);
    let view_only = std::env::args().any(|a| a == "--view-only");
    let key_path = state.join("secret.key");
    let key = match std::fs::read(&key_path) {
        Ok(bytes) if bytes.len() == 32 => SecretKey::from_bytes(&bytes.try_into().unwrap()),
        _ => {
            let key = SecretKey::generate();
            std::fs::write(&key_path, key.to_bytes())?;
            key
        }
    };
    let endpoint = Endpoint::builder(presets::N0).secret_key(key).bind().await?;
    let addr_path = state.join("host.json");
    let addr: EndpointAddr = if let Some(link) = arg("--link") {
        let url = url::Url::parse(&link)?;
        let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        let ticket: iroh_tickets::endpoint::EndpointTicket = q["t"].parse()?;
        let addr: EndpointAddr = ticket.into();
        let conn = endpoint.connect(addr.clone(), ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&hello(serde_json::json!({"op":"pair","code":q["c"],"name":"desktop-probe"}))).await?;
        send.finish()?;
        let (_, bytes) = read_frame(&mut recv).await?;
        let reply: Control = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(matches!(reply, Control::Ok { .. }), "pairing failed: {reply:?}");
        std::fs::write(&addr_path, serde_json::to_vec(&addr)?)?;
        eprintln!("paired with {}", q.get("n").cloned().unwrap_or_default());
        addr
    } else {
        serde_json::from_slice(&std::fs::read(&addr_path)?)?
    };
    let t0 = Instant::now();
    let conn = endpoint.connect(addr, ALPN).await?;
    eprintln!("connected in {:?} ({:?})", t0.elapsed(), conn.remote_id());

    // Status first.
    {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&hello(serde_json::json!({"op":"desktop_status"}))).await?;
        send.finish()?;
        let (_, bytes) = read_frame(&mut recv).await?;
        eprintln!("status: {}", String::from_utf8_lossy(&bytes));
    }
    let (mut send, mut recv) = conn.open_bi().await?;
    let (vw, vh) = arg("--viewport")
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))))
        .unwrap_or((1920, 1920));
    let quality = arg("--quality").unwrap_or_else(|| "high".into());
    let mut open = serde_json::json!({"op":"desktop_open","viewport":{"width":vw,"height":vh},"quality":quality,"view_only":view_only});
    if let Some(extra) = arg("--open-extra") {
        // Extra desktop_open fields as JSON (newer hosts: display, max_side…).
        let extra: serde_json::Value = serde_json::from_str(&extra)?;
        for (k, v) in extra.as_object().into_iter().flatten() {
            open[k] = v.clone();
        }
    }
    send.write_all(&hello(open)).await?;
    let (_, bytes) = read_frame(&mut recv).await?;
    let reply: Control = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(matches!(reply, Control::Ok { .. }), "desktop_open: {reply:?}");

    let mut decoder = Decoder::new()?;
    let mut h264 = Vec::new();
    // The app's fixture format (docs/m7/fixtures/desktop-h264.bin): per access
    // unit `u32 BE length, u8 key, bytes`.
    let mut records = Vec::new();
    let (mut frames, mut keys, mut decoded) = (0usize, 0usize, 0usize);
    let mut last: Option<(usize, usize, Vec<u8>)> = None;
    let mut started = serde_json::Value::Null;
    let mut codec = String::new();
    let mut first_frame_ms = None;
    let mut script = arg("--input").map(|s| input_script(&s)).unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(arg("--timeout").and_then(|t| t.parse().ok()).unwrap_or(60));
    let mut error = None;
    while decoded < frames_wanted && Instant::now() < deadline {
        let Ok(Ok((kind, bytes))) = tokio::time::timeout(Duration::from_secs(20), read_frame(&mut recv)).await else {
            error = Some("stream ended / timed out".to_string());
            break;
        };
        if kind != 0 {
            continue;
        }
        match serde_json::from_slice::<Control>(&bytes)? {
            Control::DesktopEvent { event } => match event {
                DesktopEvent::Frame(meta) => {
                    let mut au = Vec::with_capacity(meta.bytes as usize);
                    while au.len() < meta.bytes as usize {
                        let (_, chunk) = read_frame(&mut recv).await?;
                        au.extend_from_slice(&chunk);
                    }
                    frames += 1;
                    keys += meta.key as usize;
                    if meta.key {
                        eprintln!("key frame t={:.1}s seq={} {} KB", t0.elapsed().as_secs_f32(), meta.seq, meta.bytes / 1024);
                    }
                    first_frame_ms.get_or_insert(t0.elapsed().as_millis());
                    h264.extend_from_slice(&au);
                    records.extend_from_slice(&(au.len() as u32).to_be_bytes());
                    records.push(meta.key as u8);
                    records.extend_from_slice(&au);
                    if let Some(yuv) = decoder.decode(&au)? {
                        let (w, h) = yuv.dimensions();
                        let mut rgba = vec![0u8; w * h * 4];
                        yuv.write_rgba8(&mut rgba);
                        last = Some((w, h, rgba));
                        decoded += 1;
                    }
                    send.write_all(&cmd(serde_json::json!({"op":"ack","seq":meta.seq}))).await?;
                    // Input after a few frames, then a key frame to see it.
                    if decoded == 5 && !script.is_empty() {
                        for step in script.drain(..) {
                            send.write_all(&cmd(step)).await?;
                            tokio::time::sleep(Duration::from_millis(60)).await;
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        send.write_all(&cmd(serde_json::json!({"op":"keyframe"}))).await?;
                    }
                }
                DesktopEvent::Started { backend, width, height, view_only, display } => {
                    started = serde_json::json!({"backend":backend,"width":width,"height":height,"view_only":view_only,"display":display});
                    eprintln!("started: {started}");
                }
                DesktopEvent::Config { codec: c, width, height } => {
                    eprintln!("config t={:.1}s {c} {width}x{height}", t0.elapsed().as_secs_f32());
                    codec = c;
                }
                DesktopEvent::Consent { state } => eprintln!("consent: {state}"),
                DesktopEvent::Stats { fps, kbps, rtt_ms, .. } => {
                    eprintln!("stats t={:.1}s fps={fps} kbps={kbps} rtt={rtt_ms} size={}", t0.elapsed().as_secs_f32(), last.as_ref().map(|(w, h, _)| format!("{w}x{h}")).unwrap_or_default());
                }
                DesktopEvent::Closed { reason } => {
                    error = Some(format!("closed: {reason}"));
                    break;
                }
            },
            Control::Err { error: e } => {
                error = Some(format!("{:?}: {} ({:?})", e.code, e.message, e.detail));
                break;
            }
            _ => {}
        }
    }
    std::fs::write(state.join("stream.h264"), &h264)?;
    std::fs::write(state.join("records.bin"), &records)?;
    if let Some((w, h, rgba)) = &last {
        let mut out = format!("P6 {w} {h} 255\n").into_bytes();
        for px in rgba.chunks_exact(4) {
            out.extend_from_slice(&px[..3]);
        }
        std::fs::write(state.join("last.ppm"), out)?;
    }
    println!(
        "{}",
        serde_json::json!({
            "started": started, "codec": codec, "frames": frames, "keys": keys, "decoded": decoded,
            "size": last.as_ref().map(|(w, h, _)| [*w, *h]), "first_frame_ms": first_frame_ms,
            "seconds": t0.elapsed().as_secs_f32(), "error": error,
        })
    );
    drop(send);
    endpoint.close().await;
    Ok(())
}
