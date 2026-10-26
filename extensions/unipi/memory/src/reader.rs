//! One warm `mempalace-mcp` stdio server (unipi `reader.ts`): JSON-RPC lines over a piped
//! child, started lazily, unloaded after 10 minutes idle (timer `memory-reader-idle`), at most
//! 5 crash respawns per 10 minutes. Calls are serialized (one request in flight).

use std::cell::{Cell, RefCell};
use std::time::Duration;

use futures::FutureExt;
use oino_extension::sync::Mutex;
use oino_extension::wit::process as wp;
use oino_extension::wit_bindgen::rt::async_support::StreamReader;
use serde_json::{Value, json};

/// Idle unload window.
pub const READER_IDLE_MS: u64 = 10 * 60_000;
/// Idle timer id.
pub const IDLE_TIMER: &str = "memory-reader-idle";

/// Splits complete lines off a byte buffer.
#[must_use]
pub fn take_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(i) = buf.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buf.drain(..=i).collect();
        let s = String::from_utf8_lossy(&line).trim().to_owned();
        if !s.is_empty() {
            out.push(s);
        }
    }
    out
}

/// Parses an MCP `tools/call` response into the tool's JSON payload.
#[must_use]
pub fn tool_payload(resp: &Value) -> Value {
    let Some(text) = resp
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
    else {
        let msg = resp
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("empty tool response");
        return json!({"error": msg});
    };
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

struct Live {
    child: wp::Child,
    out: StreamReader<u8>,
    buf: Vec<u8>,
}

/// The reader.
pub struct Reader {
    /// `mempalace-mcp` arguments (palace, `--read-only`).
    args: Vec<String>,
    env: Vec<(String, String)>,
    live: Mutex<Option<Live>>,
    next_id: Cell<u64>,
    respawns: RefCell<Vec<i64>>,
    unloaded_idle: Cell<bool>,
    pid: Cell<Option<u32>>,
    idle_ms: u64,
}

impl Reader {
    /// A reader for `palace` (`None` = `MemPalace` default); `read_only` false = write mode.
    #[must_use]
    pub fn new(palace: Option<&str>, read_only: bool, idle_ms: u64) -> Self {
        let mut args = Vec::new();
        if let Some(p) = palace {
            args.push("--palace".to_owned());
            args.push(p.to_owned());
        }
        if read_only {
            args.push("--read-only".to_owned());
        }
        Self {
            args,
            env: vec![("MALLOC_ARENA_MAX".into(), "2".into())],
            live: Mutex::new(None),
            next_id: Cell::new(1),
            respawns: RefCell::new(Vec::new()),
            unloaded_idle: Cell::new(false),
            pid: Cell::new(None),
            idle_ms,
        }
    }

    /// Live server pid.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid.get()
    }

    fn spawn_child(&self) -> Option<Live> {
        let spec = wp::SpawnSpec {
            command: "mempalace-mcp".into(),
            args: self.args.clone(),
            cwd: None,
            env: self.env.clone(),
            clear_env: false,
            stdin: wp::StdioMode::Pipe,
            stdout: wp::OutputTarget::Pipe,
            stderr: wp::OutputTarget::Null,
            new_process_group: true,
            detached: false,
            timeout_ms: None,
        };
        let child = wp::Child::spawn(&spec).ok()?;
        let out = child.stdout()?;
        Some(Live {
            child,
            out,
            buf: Vec::new(),
        })
    }

    async fn rpc(
        live: &mut Live,
        id: Option<u64>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Value {
        let mut msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        if let Some(i) = id {
            msg["id"] = json!(i);
        }
        let mut bytes = msg.to_string().into_bytes();
        bytes.push(b'\n');
        if live.child.write_stdin(bytes).await.is_err() {
            return json!({"error": {"code": -3, "message": "reader stdin broken"}});
        }
        let Some(want) = id else { return Value::Null };
        let deadline = oino_extension::timers::sleep(timeout).fuse();
        futures::pin_mut!(deadline);
        loop {
            for l in take_lines(&mut live.buf) {
                if let Ok(v) = serde_json::from_str::<Value>(&l)
                    && v.get("id").and_then(Value::as_u64) == Some(want)
                {
                    return v;
                }
            }
            let read = live.out.read(Vec::with_capacity(16 * 1024)).fuse();
            futures::pin_mut!(read);
            futures::select! {
                (status, data) = read => {
                    if matches!(status, oino_extension::wit_bindgen::rt::async_support::StreamResult::Dropped) && data.is_empty() {
                        return json!({"error": {"code": -1, "message": "reader process exited"}});
                    }
                    live.buf.extend(data);
                }
                () = deadline => return json!({"error": {"code": -2, "message": "reader call timed out"}}),
            }
        }
    }

    async fn ensure(&self, slot: &mut Option<Live>) -> bool {
        if let Some(l) = slot.as_ref()
            && l.child.try_wait().is_none()
        {
            return true;
        }
        *slot = None;
        if self.unloaded_idle.replace(false) {
            // Coming back from an idle unload is not a crash.
        } else {
            let now = oino_extension::timers::now_ms();
            let mut r = self.respawns.borrow_mut();
            r.retain(|t| now - t < 10 * 60_000);
            if r.len() >= 5 {
                return false;
            }
            r.push(now);
        }
        let Some(mut live) = self.spawn_child() else {
            return false;
        };
        let id = self.next_id.replace(self.next_id.get() + 1);
        let init = Self::rpc(&mut live, Some(id), "initialize", json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "unipi-memory", "version": "1"}}), Duration::from_secs(15)).await;
        if init.get("result").is_none() {
            let _ = live.child.signal(wp::Signal::Kill, true);
            return false;
        }
        Self::rpc(
            &mut live,
            None,
            "notifications/initialized",
            json!({}),
            Duration::from_secs(1),
        )
        .await;
        self.pid.set(Some(live.child.pid()));
        *slot = Some(live);
        true
    }

    /// `tools/call` → the tool's JSON payload (`{"error": …}` on failure).
    pub async fn call_tool(&self, name: &str, args: Value) -> Value {
        oino_extension::timers::cancel(IDLE_TIMER);
        let mut slot = self.live.lock().await;
        if !self.ensure(&mut slot).await {
            return json!({"error": "reader unavailable"});
        }
        let id = self.next_id.replace(self.next_id.get() + 1);
        let resp = match slot.as_mut() {
            Some(live) => {
                Self::rpc(
                    live,
                    Some(id),
                    "tools/call",
                    json!({"name": name, "arguments": args}),
                    Duration::from_secs(15),
                )
                .await
            }
            None => json!({"error": {"message": "reader unavailable"}}),
        };
        if self.idle_ms > 0 {
            oino_extension::timers::set(IDLE_TIMER, Duration::from_millis(self.idle_ms), false);
        }
        tool_payload(&resp)
    }

    /// Idle unload (timer fired).
    pub async fn unload_idle(&self) {
        let mut slot = self.live.lock().await;
        if let Some(l) = slot.take() {
            l.child.close_stdin();
            let _ = l.child.signal(wp::Signal::Term, true);
            self.unloaded_idle.set(true);
        }
        self.pid.set(None);
    }

    /// Kills the server.
    pub async fn kill(&self) {
        let mut slot = self.live.lock().await;
        if let Some(l) = slot.take() {
            l.child.close_stdin();
            let _ = l.child.signal(wp::Signal::Kill, true);
        }
        self.pid.set(None);
        oino_extension::timers::cancel(IDLE_TIMER);
    }

    /// `mempalace_search`.
    pub async fn search(&self, query: &str, limit: usize, wing: Option<&str>) -> Vec<Value> {
        let mut args = json!({"query": query, "limit": limit});
        if let Some(w) = wing {
            args["wing"] = json!(w);
        }
        self.call_tool("mempalace_search", args)
            .await
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_payloads() {
        let mut buf = b"{\"id\":1}\n\n{\"id\":2".to_vec();
        assert_eq!(take_lines(&mut buf), ["{\"id\":1}"]);
        assert_eq!(buf, b"{\"id\":2");
        let ok = json!({"id": 1, "result": {"content": [{"type": "text", "text": "{\"results\": []}"}]}});
        assert_eq!(tool_payload(&ok), json!({"results": []}));
        let plain = json!({"result": {"content": [{"text": "hi"}]}});
        assert_eq!(tool_payload(&plain), json!("hi"));
        assert_eq!(
            tool_payload(&json!({"error": {"message": "x"}})),
            json!({"error": "x"})
        );
        let r = Reader::new(Some("/p"), true, 0);
        assert_eq!(r.args, ["--palace", "/p", "--read-only"]);
    }
}
