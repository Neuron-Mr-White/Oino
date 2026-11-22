//! app.sock / live.sock client (host.md §7): `[kind u8][len u32 BE][payload]` frames, a
//! `hello` first frame, `ok`/`err` replies, and long-lived session streams whose events are
//! counted (never stored beyond a bounded log of markers the scenarios wait for).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// Protocol version sent in every `hello`.
const PROTO: &str = "2.1.0";

fn encode(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(0);
    out.extend_from_slice(
        &u32::try_from(payload.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(payload);
    out
}

fn read_frame(stream: &mut UnixStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    let mut body = vec![0; len];
    stream.read_exact(&mut body)?;
    Ok((header[0], body))
}

fn hello(req: &Value) -> Vec<u8> {
    encode(
        json!({"type": "hello", "proto": PROTO, "req": req})
            .to_string()
            .as_bytes(),
    )
}

/// One request/response op; returns the reply and its latency.
///
/// # Errors
///
/// Connect/IO failures, timeouts, or an `err` reply.
pub(crate) fn op(sock: &Path, req: &Value, timeout: Duration) -> Result<(Value, Duration)> {
    let started = Instant::now();
    let mut stream =
        UnixStream::connect(sock).with_context(|| format!("connect {}", sock.display()))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.write_all(&hello(req))?;
    loop {
        let (kind, body) = read_frame(&mut stream)?;
        if kind != 0 {
            continue;
        }
        let reply: Value = serde_json::from_slice(&body)?;
        match reply["type"].as_str() {
            Some("ok") => return Ok((reply, started.elapsed())),
            Some("err") => bail!("{} failed: {}", req["op"], reply["error"]),
            _ => {}
        }
    }
}

/// A marker the scenarios wait for: event `type`/`kind`, when it arrived.
#[derive(Clone, Debug)]
pub(crate) struct Marker {
    pub(crate) name: String,
    pub(crate) at: Instant,
}

#[derive(Default)]
struct State {
    replies: HashMap<String, (Instant, Value)>,
    markers: Vec<Marker>,
    closed: bool,
    frames: u64,
    items: u64,
    deltas: u64,
    bytes: u64,
    last_frame: Option<Instant>,
    max_gap: Duration,
}

/// Bounded marker log (oldest dropped).
const MAX_MARKERS: usize = 4096;
/// Event types worth remembering (deltas are only counted).
const MARKED: &[&str] = &[
    "agent_start",
    "agent_end",
    "agent_settled",
    "turn_end",
    "message_end",
    "tool_execution_start",
    "tool_execution_end",
    "woken",
    "wake_failed",
    "crashed",
    "taken_over",
    "error",
];

/// One open session (or events) stream with a reader thread.
pub(crate) struct Stream {
    write: Mutex<UnixStream>,
    state: Arc<(Mutex<State>, Condvar)>,
    /// The `ok` snapshot (`data`) the open answered.
    pub(crate) snapshot: Value,
    /// Open → first reply frame.
    pub(crate) open_latency: Duration,
    /// When the open request was written.
    pub(crate) opened_at: Instant,
    seq: Mutex<u64>,
}

impl Stream {
    /// Opens `req` (`session_open` / `session_new` / `events_open`) and starts reading.
    ///
    /// # Errors
    ///
    /// Connect/IO failures or an `err` reply.
    pub(crate) fn open(sock: &Path, req: &Value, timeout: Duration) -> Result<Self> {
        let mut stream =
            UnixStream::connect(sock).with_context(|| format!("connect {}", sock.display()))?;
        stream.set_read_timeout(Some(timeout))?;
        let opened_at = Instant::now();
        stream.write_all(&hello(req))?;
        let (_, body) = read_frame(&mut stream).with_context(|| format!("open {}", req["op"]))?;
        let open_latency = opened_at.elapsed();
        let reply: Value = serde_json::from_slice(&body)?;
        if reply["type"] != "ok" {
            bail!("open {} failed: {}", req["op"], reply);
        }
        stream.set_read_timeout(None)?;
        let reader = stream.try_clone()?;
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let shared = Arc::clone(&state);
        std::thread::Builder::new()
            .name("bench-stream".into())
            .spawn(move || pump(reader, &shared))?;
        Ok(Self {
            write: Mutex::new(stream),
            state,
            snapshot: reply.get("data").cloned().unwrap_or(Value::Null),
            open_latency,
            opened_at,
            seq: Mutex::new(0),
        })
    }

    /// Session id from the snapshot.
    pub(crate) fn session_id(&self) -> String {
        self.snapshot["session_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Sends a `session_cmd`; returns its id and the send instant.
    ///
    /// # Errors
    ///
    /// Write failures.
    pub(crate) fn send(&self, cmd: &Value) -> Result<(String, Instant)> {
        let id = {
            let mut seq = self.seq.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
            *seq += 1;
            format!("b{}", *seq)
        };
        let frame = encode(
            json!({"type": "session_cmd", "id": id, "cmd": cmd})
                .to_string()
                .as_bytes(),
        );
        let at = Instant::now();
        self.write
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned"))?
            .write_all(&frame)?;
        Ok((id, at))
    }

    /// Sends a command and waits for its reply; returns reply and latency.
    ///
    /// # Errors
    ///
    /// Write failure, timeout, closed stream or an `err` reply.
    pub(crate) fn request(&self, cmd: &Value, timeout: Duration) -> Result<(Value, Duration)> {
        let (id, at) = self.send(cmd)?;
        let (when, reply) = self.wait_reply(&id, timeout)?;
        if reply["type"] == "err" {
            bail!("{} failed: {}", cmd["type"], reply["error"]);
        }
        Ok((reply, when.saturating_duration_since(at)))
    }

    /// Waits for the reply to `id`.
    ///
    /// # Errors
    ///
    /// Timeout or a closed stream.
    pub(crate) fn wait_reply(&self, id: &str, timeout: Duration) -> Result<(Instant, Value)> {
        let (lock, cvar) = &*self.state;
        let deadline = Instant::now() + timeout;
        let mut state = lock.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
        loop {
            if let Some(reply) = state.replies.remove(id) {
                return Ok(reply);
            }
            if state.closed {
                bail!("stream closed before reply {id}");
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                bail!("reply {id} timed out");
            }
            state = cvar
                .wait_timeout(state, left)
                .map_err(|_| anyhow::anyhow!("poisoned"))?
                .0;
        }
    }

    /// Waits for the first marker named one of `names` that arrived at/after `since`.
    pub(crate) fn wait_marker(
        &self,
        names: &[&str],
        since: Instant,
        timeout: Duration,
    ) -> Option<Marker> {
        let (lock, cvar) = &*self.state;
        let deadline = Instant::now() + timeout;
        let mut state = lock.lock().ok()?;
        loop {
            if let Some(found) = state
                .markers
                .iter()
                .find(|m| m.at >= since && names.contains(&m.name.as_str()))
            {
                return Some(found.clone());
            }
            if state.closed {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            state = cvar.wait_timeout(state, left).ok()?.0;
        }
    }

    /// Lifecycle evidence on failure, without retaining message/tool contents.
    pub(crate) fn markers_since(&self, since: Instant) -> Vec<(String, Duration)> {
        self.state
            .0
            .lock()
            .map(|state| {
                state
                    .markers
                    .iter()
                    .filter(|m| m.at >= since)
                    .map(|m| (m.name.clone(), m.at.duration_since(since)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether any marker named `name` arrived at/after `since`.
    pub(crate) fn saw(&self, name: &str, since: Instant) -> bool {
        self.state
            .0
            .lock()
            .is_ok_and(|s| s.markers.iter().any(|m| m.at >= since && m.name == name))
    }

    /// (frames, items, deltas, bytes, max gap between frames).
    pub(crate) fn counters(&self) -> (u64, u64, u64, u64, Duration) {
        self.state
            .0
            .lock()
            .map_or((0, 0, 0, 0, Duration::ZERO), |s| {
                (s.frames, s.items, s.deltas, s.bytes, s.max_gap)
            })
    }

    /// Whether the host ended the stream.
    pub(crate) fn closed(&self) -> bool {
        self.state.0.lock().is_ok_and(|s| s.closed)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Ok(stream) = self.write.lock() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn pump(mut stream: UnixStream, state: &Arc<(Mutex<State>, Condvar)>) {
    let (lock, cvar) = &**state;
    while let Ok((kind, body)) = read_frame(&mut stream) {
        let now = Instant::now();
        let Ok(mut s) = lock.lock() else { break };
        if let Some(last) = s.last_frame {
            s.max_gap = s.max_gap.max(now - last);
        }
        s.last_frame = Some(now);
        s.frames += 1;
        s.bytes += body.len() as u64;
        if kind != 0 {
            continue;
        }
        let Ok(frame) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        if frame["kind"] == "session_changed"
            && frame["card"]["run_state"] == "crashed"
            && let Some(id) = frame["card"]["session_id"].as_str()
        {
            if s.markers.len() >= MAX_MARKERS {
                s.markers.remove(0);
            }
            s.markers.push(Marker {
                name: format!("interrupted:{id}"),
                at: now,
            });
        }
        match frame["type"].as_str() {
            Some("events") => {
                for item in frame["items"].as_array().into_iter().flatten() {
                    s.items += 1;
                    let event = &item["event"];
                    let name = event["type"]
                        .as_str()
                        .or_else(|| event["kind"].as_str())
                        .unwrap_or("");
                    if name == "message_update" {
                        s.deltas += 1;
                    } else if MARKED.contains(&name) {
                        if s.markers.len() >= MAX_MARKERS {
                            s.markers.remove(0);
                        }
                        s.markers.push(Marker {
                            name: name.to_owned(),
                            at: now,
                        });
                    }
                }
            }
            Some("ok" | "err") => {
                if let Some(id) = frame["id"].as_str() {
                    s.replies.insert(id.to_owned(), (now, frame.clone()));
                }
            }
            _ => {}
        }
        drop(s);
        cvar.notify_all();
    }
    if let Ok(mut s) = lock.lock() {
        s.closed = true;
    }
    cvar.notify_all();
}

/// `<host dir>/app.sock` and `<host dir>/live.sock`.
#[derive(Clone, Debug)]
pub(crate) struct Socks {
    pub(crate) app: PathBuf,
    pub(crate) live: PathBuf,
}

impl Socks {
    /// `host_info` over app.sock.
    ///
    /// # Errors
    ///
    /// As [`op`].
    pub(crate) fn ping_app(&self, timeout: Duration) -> Result<Duration> {
        op(&self.app, &json!({"op": "host_info"}), timeout).map(|(_, d)| d)
    }

    /// `host_info` over live.sock (the dedicated liveness thread).
    ///
    /// # Errors
    ///
    /// As [`op`].
    pub(crate) fn ping_live(&self, timeout: Duration) -> Result<Duration> {
        op(&self.live, &json!({"op": "host_info"}), timeout).map(|(_, d)| d)
    }
}
