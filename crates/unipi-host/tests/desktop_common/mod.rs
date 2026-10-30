//! Shared harness for the UNI-216 desktop tests: an in-process host
//! `Handler` and a "phone" endpoint over real iroh (no relay, temp data dir),
//! plus a viewer that reads `desktop_event`s, reassembles H.264 access units
//! and decodes them with openh264 like the phone's WebCodecs would.
#![allow(dead_code)]
use std::time::Duration;

use iroh::{
    endpoint::{presets, Connection, RelayMode},
    Endpoint,
};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use unipi_host::{
    data_dir::HostPaths,
    endpoint::ALPN,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, Handler},
};
use unipi_proto::{AppErrorCode, Control, DesktopEvent};

pub fn hello(req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &serde_json::to_vec(&value).unwrap())
}

pub fn cmd(cmd: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"desktop_cmd","cmd":cmd});
    unipi_proto::frame::encode(unipi_proto::FrameKind::Control, &serde_json::to_vec(&value).unwrap())
}

pub async fn request(conn: &Connection, req: serde_json::Value) -> Control {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(req)).await.unwrap();
    send.finish().unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(30), read_frame(&mut recv)).await.unwrap().unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn err_code(control: &Control) -> Option<AppErrorCode> {
    match control {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

pub fn err_reason(control: &Control) -> Option<String> {
    match control {
        Control::Err { error } => error.detail.as_ref()?.get("reason")?.as_str().map(str::to_string),
        _ => None,
    }
}

pub struct Rig {
    pub temp: tempfile::TempDir,
    pub paths: HostPaths,
    pub conn: Connection,
    _host: Endpoint,
    _phone: Endpoint,
    _router: iroh::protocol::Router,
}

impl Rig {
    pub async fn new() -> Rig {
        let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_test_writer().try_init();
        let temp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(temp.path());
        let host = Endpoint::builder(presets::N0)
            .relay_mode(RelayMode::Disabled)
            .clear_address_lookup()
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        let handler = Handler {
            paths: paths.clone(),
            host_id: host.id(),
            host_name: "Desktop test host".into(),
            lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            protected: std::sync::Arc::new(vec![]),
            board: Default::default(),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        };
        let router = iroh::protocol::Router::builder(host.clone()).accept(ALPN, handler).spawn();
        let phone = Endpoint::builder(presets::N0).relay_mode(RelayMode::Disabled).clear_address_lookup().bind().await.unwrap();
        let conn = phone.connect(host.addr(), ALPN).await.unwrap();
        let state = PairingStore::new(paths.pairing()).start(now_unix()).unwrap();
        let reply = request(&conn, serde_json::json!({"op":"pair","code":state.code,"name":"Test phone"})).await;
        assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
        Rig { temp, paths, conn, _host: host, _phone: phone, _router: router }
    }

    pub fn config(&self, desktop: serde_json::Value) {
        std::fs::write(self.paths.config(), serde_json::json!({ "desktop": desktop }).to_string()).unwrap();
    }

    pub async fn open(&self, req: serde_json::Value) -> Result<Viewer, Control> {
        let (mut send, mut recv) = self.conn.open_bi().await.unwrap();
        send.write_all(&hello(req)).await.unwrap();
        let (_, bytes) = tokio::time::timeout(Duration::from_secs(30), read_frame(&mut recv)).await.unwrap().unwrap();
        let control: Control = serde_json::from_slice(&bytes).unwrap();
        match control {
            Control::Ok { .. } => Ok(Viewer { send, recv, decoder: Decoder::new().unwrap(), decoded: Vec::new(), keys: 0, frames: 0, config: None, started: None, last_rgba: None }),
            other => Err(other),
        }
    }
}

pub struct Viewer {
    pub send: iroh::endpoint::SendStream,
    pub recv: iroh::endpoint::RecvStream,
    pub decoder: Decoder,
    /// Sizes of decoded pictures.
    pub decoded: Vec<(usize, usize)>,
    pub keys: usize,
    pub frames: usize,
    pub config: Option<(String, u32, u32)>,
    pub started: Option<DesktopEvent>,
    /// The last decoded picture (RGBA) with its size.
    pub last_rgba: Option<(usize, usize, Vec<u8>)>,
}

pub enum Next {
    Event(DesktopEvent),
    Err(unipi_proto::AppErrorDto),
    Eof,
}

impl Viewer {
    pub async fn send(&mut self, value: serde_json::Value) {
        self.send.write_all(&cmd(value)).await.unwrap();
    }

    /// Reads the next event; a `frame` event also pulls, decodes and acks its bytes.
    pub async fn next(&mut self) -> Next {
        let Ok(Ok((kind, bytes))) = tokio::time::timeout(Duration::from_secs(30), read_frame(&mut self.recv)).await else {
            return Next::Eof;
        };
        assert_eq!(kind, 0, "expected a control frame");
        let control: Control = serde_json::from_slice(&bytes).unwrap();
        let event = match control {
            Control::DesktopEvent { event } => event,
            Control::Err { error } => return Next::Err(error),
            other => panic!("unexpected {other:?}"),
        };
        match &event {
            DesktopEvent::Frame(meta) => {
                let mut au = Vec::with_capacity(meta.bytes as usize);
                while au.len() < meta.bytes as usize {
                    let (kind, chunk) = read_frame(&mut self.recv).await.unwrap();
                    assert_eq!(kind, 1, "frame bytes come as kind-1 frames");
                    au.extend_from_slice(&chunk);
                }
                assert_eq!(au.len(), meta.bytes as usize);
                assert_eq!(&au[..4], &[0, 0, 0, 1], "Annex-B");
                self.frames += 1;
                if meta.key {
                    self.keys += 1;
                }
                if let Some(yuv) = self.decoder.decode(&au).expect("decodes") {
                    let (w, h) = yuv.dimensions();
                    let mut rgba = vec![0u8; w * h * 4];
                    yuv.write_rgba8(&mut rgba);
                    self.decoded.push((w, h));
                    self.last_rgba = Some((w, h, rgba));
                }
                self.send(serde_json::json!({"op":"ack","seq":meta.seq})).await;
            }
            DesktopEvent::Config { codec, width, height } => self.config = Some((codec.clone(), *width, *height)),
            DesktopEvent::Started { .. } => self.started = Some(event.clone()),
            _ => {}
        }
        Next::Event(event)
    }

    /// Reads until `want` matches; panics on error/EOF/timeout.
    pub async fn until(&mut self, what: &str, mut want: impl FnMut(&DesktopEvent, &Self) -> bool) -> DesktopEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
            match self.next().await {
                Next::Event(event) => {
                    if want(&event, self) {
                        return event;
                    }
                }
                Next::Err(error) => panic!("error while waiting for {what}: {error:?}"),
                Next::Eof => panic!("stream ended while waiting for {what}"),
            }
        }
    }

    /// Writes the last decoded picture as a PPM (debugging: `UNIPI_TEST_DUMP=dir`).
    pub fn dump(&self, name: &str) {
        let Ok(dir) = std::env::var("UNIPI_TEST_DUMP") else { return };
        let Some((w, h, rgba)) = self.last_rgba.as_ref() else { return };
        let mut out = format!("P6 {w} {h} 255\n").into_bytes();
        for px in rgba.chunks_exact(4) {
            out.extend_from_slice(&px[..3]);
        }
        let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), out);
    }

    /// RGB of the last decoded picture at normalised (x, y).
    pub fn pixel(&self, x: f64, y: f64) -> [u8; 3] {
        let (w, h, rgba) = self.last_rgba.as_ref().expect("a decoded picture");
        let (px, py) = (((w - 1) as f64 * x) as usize, ((h - 1) as f64 * y) as usize);
        let i = (py * w + px) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }
}
