//! MCP client (unipi `src/bridge/client.ts`): stdio child processes through the host
//! `process` interface and streamable HTTP through `http.fetch`. Requests on one connection
//! are serialised; a timed-out or dropped connection is marked broken and reconnected on the
//! next call (pi).

use std::cell::{Cell, RefCell};
use std::time::Duration;

use futures::future::{Either, select};
use futures::lock::Mutex;
use oino_extension::http;
use oino_extension::timers;
use oino_extension::wit::process as wp;
use oino_extension::wit_bindgen::StreamReader;
use serde_json::{Value, json};

use crate::rpc::{self, Incoming, LineBuffer, SseBuffer};

/// Expanded connection parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Child process.
    Stdio {
        /// Executable.
        command: String,
        /// Arguments.
        args: Vec<String>,
        /// Environment additions.
        env: Vec<(String, String)>,
        /// Working directory (host path; relative = session cwd).
        cwd: Option<String>,
    },
    /// Streamable HTTP endpoint.
    Http {
        /// URL.
        url: String,
        /// Headers.
        headers: Vec<(String, String)>,
    },
}

enum Io {
    Stdio {
        child: wp::Child,
        stdout: StreamReader<u8>,
        lines: LineBuffer,
    },
    Http {
        url: String,
        headers: Vec<(String, String)>,
        session: Option<String>,
        protocol: Option<String>,
    },
}

/// A connected server.
pub(crate) struct Client {
    io: Mutex<Io>,
    next_id: Cell<u64>,
    broken: Cell<bool>,
    timeout_ms: u64,
    pid: Option<u32>,
    sensitive: Vec<String>,
    pending: RefCell<Vec<futures::channel::oneshot::Sender<()>>>,
    reloaded: Cell<bool>,
    /// `instructions` from `initialize`.
    pub instructions: RefCell<Option<String>>,
}

fn host_err(e: oino_extension::wit::types::HostError) -> String {
    oino_extension::Error::from(e).to_string()
}

async fn with_timeout<T>(fut: impl Future<Output = T>, ms: u64) -> Option<T> {
    let fut = std::pin::pin!(fut);
    let sleep = std::pin::pin!(timers::sleep(Duration::from_millis(ms)));
    match select(fut, sleep).await {
        Either::Left((v, _)) => Some(v),
        Either::Right(((), _)) => None,
    }
}

impl Client {
    /// Spawns / opens the server and performs the `initialize` handshake.
    pub(crate) async fn connect(
        _name: &str,
        target: &Target,
        timeout_ms: u64,
        version: &str,
    ) -> Result<Self, String> {
        let sensitive = sensitive_values(target);
        let (io, pid) = match target {
            Target::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let spec = wp::SpawnSpec {
                    command: command.clone(),
                    args: args.clone(),
                    cwd: cwd.clone(),
                    env: env.clone(),
                    clear_env: false,
                    stdin: wp::StdioMode::Pipe,
                    stdout: wp::OutputTarget::Pipe,
                    // Servers can echo credentials on stderr. Never persist raw server logs.
                    stderr: wp::OutputTarget::Null,
                    new_process_group: true,
                    detached: false,
                    timeout_ms: None,
                };
                let child = wp::Child::spawn(&spec).map_err(|e| {
                    crate::storage::redact(
                        &format!("Failed to spawn MCP server: {}", host_err(e)),
                        &sensitive,
                    )
                })?;
                let pid = child.pid();
                let stdout = child
                    .stdout()
                    .ok_or_else(|| "Failed to spawn MCP server: no stdout".to_owned())?;
                (
                    Io::Stdio {
                        child,
                        stdout,
                        lines: LineBuffer::default(),
                    },
                    Some(pid),
                )
            }
            Target::Http { url, headers } => (
                Io::Http {
                    url: url.clone(),
                    headers: headers.clone(),
                    session: None,
                    protocol: None,
                },
                None,
            ),
        };
        let client = Self {
            io: Mutex::new(io),
            next_id: Cell::new(1),
            broken: Cell::new(false),
            timeout_ms,
            pid,
            sensitive,
            pending: RefCell::new(Vec::new()),
            reloaded: Cell::new(false),
            instructions: RefCell::new(None),
        };
        match client
            .request("initialize", &rpc::initialize_params(version))
            .await
        {
            Ok(result) => {
                if let Io::Http { protocol, .. } = &mut *client.io.lock().await {
                    *protocol = result
                        .get("protocolVersion")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                }
                *client.instructions.borrow_mut() = result
                    .get("instructions")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                client.notify("notifications/initialized", &json!({})).await;
                Ok(client)
            }
            Err(e) => {
                let exited = client.exit_status().await;
                client.disconnect().await;
                Err(crate::storage::redact(
                    &match exited {
                        Some((code, signal)) => format!(
                            "MCP server exited during startup: code={code}, signal={signal}"
                        ),
                        None => format!("MCP initialize handshake failed: {e}"),
                    },
                    &client.sensitive,
                ))
            }
        }
    }

    async fn exit_status(&self) -> Option<(String, String)> {
        let io = self.io.lock().await;
        let Io::Stdio { child, .. } = &*io else {
            return None;
        };
        let st = child.try_wait()?;
        let code = st.code.map_or_else(|| "null".to_owned(), |c| c.to_string());
        let signal = st
            .signal
            .map_or_else(|| "null".to_owned(), |s| s.to_string());
        Some((code, signal))
    }

    /// Process id (stdio).
    pub(crate) fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The connection timed out or dropped; reconnect before the next call.
    pub(crate) fn is_broken(&self) -> bool {
        self.broken.get()
    }

    /// unipi `listTools` (all pages).
    pub(crate) async fn list_tools(&self) -> Result<Vec<Value>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |c| json!({"cursor": c}));
            let result = self.request("tools/list", &params).await?;
            if let Some(list) = result.get("tools").and_then(Value::as_array) {
                tools.extend(list.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(ToOwned::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// unipi `callTool`.
    pub(crate) async fn call_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        self.request("tools/call", &json!({"name": name, "arguments": args}))
            .await
    }

    /// One request/response.
    pub(crate) async fn request(&self, method: &str, params: &Value) -> Result<Value, String> {
        if self.reloaded.get() {
            return Err("server reloaded".into());
        }
        let (tx, rx) = futures::channel::oneshot::channel();
        {
            let mut pending = self.pending.borrow_mut();
            pending.retain(|tx| !tx.is_canceled());
            pending.push(tx);
        }
        let request = std::pin::pin!(self.request_inner(method, params));
        let canceled = std::pin::pin!(rx);
        match select(request, canceled).await {
            Either::Left((result, _)) => result,
            Either::Right(_) => Err("server reloaded".into()),
        }
    }

    async fn request_inner(&self, method: &str, params: &Value) -> Result<Value, String> {
        if self.broken.get() {
            return Err("MCP server connection was lost".to_owned());
        }
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let mut io = self.io.lock().await;
        let outcome = match &mut *io {
            Io::Stdio {
                child,
                stdout,
                lines,
            } => stdio_request(child, stdout, lines, id, method, params, self.timeout_ms).await,
            Io::Http {
                url,
                headers,
                session,
                protocol,
            } => {
                http_request(
                    url,
                    headers,
                    session,
                    protocol.as_deref(),
                    &rpc::request(id, method, params),
                    Some(id),
                    self.timeout_ms,
                )
                .await
            }
        };
        let result = match outcome {
            Ok(v) => v.ok_or_else(|| format!("MCP server sent no response to {method}"))?,
            Err(Failure::Timeout) => {
                self.broken.set(true);
                Err(format!(
                    "MCP request timed out after {}ms: {method}",
                    self.timeout_ms
                ))
            }
            Err(Failure::Dropped(msg)) => {
                self.broken.set(true);
                Err(msg)
            }
            Err(Failure::Other(msg)) => Err(msg),
        };
        result.map_err(|e| crate::storage::redact(&e, &self.sensitive))
    }

    /// A notification (no response).
    pub(crate) async fn notify(&self, method: &str, params: &Value) {
        let line = rpc::notification(method, params);
        let mut io = self.io.lock().await;
        match &mut *io {
            Io::Stdio { child, .. } => {
                let _ = child.write_stdin(format!("{line}\n").into_bytes()).await;
            }
            Io::Http {
                url,
                headers,
                session,
                protocol,
            } => {
                let p = protocol.clone();
                let _ = http_request(
                    url,
                    headers,
                    session,
                    p.as_deref(),
                    &line,
                    None,
                    self.timeout_ms,
                )
                .await;
            }
        }
    }

    /// unipi `disconnect`: close stdin, SIGTERM the group, wait ≤ 500 ms, SIGKILL; HTTP
    /// sessions are deleted.
    pub(crate) async fn disconnect(&self) {
        self.reloaded.set(true);
        for tx in self.pending.borrow_mut().drain(..) {
            let _ = tx.send(());
        }
        self.broken.set(true);
        let mut io = self.io.lock().await;
        match &mut *io {
            Io::Stdio { child, .. } => {
                child.close_stdin();
                let _ = child.signal(wp::Signal::Term, true);
                if with_timeout(child.wait(), 500).await.is_none() {
                    let _ = child.signal(wp::Signal::Kill, true);
                }
            }
            Io::Http {
                url,
                headers,
                session,
                ..
            } => {
                if let Some(s) = session.take() {
                    let mut b = http::request(http::Method::Delete, url).timeout_ms(2000);
                    for (k, v) in headers.iter() {
                        b = b.header(k, v);
                    }
                    let _ = b.header("mcp-session-id", &s).send().await;
                }
            }
        }
    }
}

enum Failure {
    Timeout,
    Dropped(String),
    Other(String),
}

/// Handles one decoded message; `Some(outcome)` when it answers `want`.
async fn handle_stdio_message(
    child: &wp::Child,
    msg: &Value,
    want: u64,
) -> Option<Result<Value, String>> {
    match rpc::classify(msg)? {
        Incoming::Response { id, outcome } if id == want => Some(outcome),
        Incoming::Request { id, method } => {
            let reply = rpc::reply_to_server(&id, &method);
            let _ = child.write_stdin(format!("{reply}\n").into_bytes()).await;
            None
        }
        _ => None,
    }
}

async fn stdio_request(
    child: &wp::Child,
    stdout: &mut StreamReader<u8>,
    lines: &mut LineBuffer,
    id: u64,
    method: &str,
    params: &Value,
    timeout_ms: u64,
) -> Result<Option<Result<Value, String>>, Failure> {
    let line = rpc::request(id, method, params);
    child
        .write_stdin(format!("{line}\n").into_bytes())
        .await
        .map_err(|e| {
            Failure::Dropped(format!("MCP server process not available: {}", host_err(e)))
        })?;
    let deadline = timers::now_ms() + i64::try_from(timeout_ms).unwrap_or(i64::MAX);
    loop {
        let remaining = deadline - timers::now_ms();
        if remaining <= 0 {
            return Err(Failure::Timeout);
        }
        #[allow(clippy::cast_sign_loss, reason = "remaining > 0")]
        let read = with_timeout(stdout.read(Vec::with_capacity(16 * 1024)), remaining as u64).await;
        let Some((status, buf)) = read else {
            return Err(Failure::Timeout);
        };
        if matches!(status, oino_extension::wit_bindgen::StreamResult::Dropped) && buf.is_empty() {
            let st = child.try_wait();
            let (code, signal) = st.map_or(("null".to_owned(), "null".to_owned()), |s| {
                (
                    s.code.map_or_else(|| "null".to_owned(), |c| c.to_string()),
                    s.signal
                        .map_or_else(|| "null".to_owned(), |c| c.to_string()),
                )
            });
            return Err(Failure::Dropped(format!(
                "MCP server exited unexpectedly: code={code}, signal={signal}"
            )));
        }
        for msg in lines.push(&buf) {
            if let Some(outcome) = handle_stdio_message(child, &msg, id).await {
                return Ok(Some(outcome));
            }
        }
    }
}

async fn http_request(
    url: &str,
    headers: &[(String, String)],
    session: &mut Option<String>,
    protocol: Option<&str>,
    body: &str,
    want: Option<u64>,
    timeout_ms: u64,
) -> Result<Option<Result<Value, String>>, Failure> {
    let mut b = http::post(url).timeout_ms(u32::try_from(timeout_ms).unwrap_or(u32::MAX));
    // A header set in mcp.json replaces the built-in one of the same name (case-insensitive):
    // a duplicate `Content-Type` makes servers like refero answer 415.
    let configured = |name: &str| headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    if !configured("content-type") {
        b = b.header("content-type", "application/json");
    }
    if !configured("accept") {
        b = b.header("accept", "application/json, text/event-stream");
    }
    for (k, v) in headers {
        b = b.header(k, v);
    }
    if let Some(s) = session.as_deref() {
        b = b.header("mcp-session-id", s);
    }
    if let Some(p) = protocol {
        b = b.header("mcp-protocol-version", p);
    }
    let resp = b.body(body.as_bytes().to_vec()).send().await.map_err(|e| {
        let m = e.to_string();
        if m.contains("timed out") {
            Failure::Timeout
        } else {
            Failure::Other(m)
        }
    })?;
    if let Some(s) = resp.header("mcp-session-id") {
        *session = Some(s.to_owned());
    }
    let status = resp.status();
    if status == 404 && session.is_some() {
        *session = None;
        return Err(Failure::Dropped("MCP session expired".to_owned()));
    }
    if !(200..300).contains(&status) {
        let text = resp.text().await;
        let detail = if text.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", text.trim())
        };
        return Err(Failure::Other(format!("HTTP {status}{detail}")));
    }
    let Some(want) = want else { return Ok(None) };
    let is_sse = resp
        .header("content-type")
        .is_some_and(|c| c.to_ascii_lowercase().contains("text/event-stream"));
    let pick = |msgs: Vec<Value>| -> Option<Result<Value, String>> {
        msgs.iter().find_map(|m| match rpc::classify(m)? {
            Incoming::Response { id, outcome } if id == want => Some(outcome),
            _ => None,
        })
    };
    if !is_sse {
        return Ok(pick(rpc::json_body(&resp.bytes().await)));
    }
    let mut reader = resp.body();
    let mut sse = SseBuffer::default();
    loop {
        let (status, buf) = reader.read(Vec::with_capacity(16 * 1024)).await;
        let done =
            matches!(status, oino_extension::wit_bindgen::StreamResult::Dropped) && buf.is_empty();
        let msgs = if done {
            sse.finish()
        } else {
            sse.push(&String::from_utf8_lossy(&buf))
        };
        if let Some(found) = pick(msgs) {
            return Ok(Some(found));
        }
        if done {
            return Ok(None);
        }
    }
}

fn sensitive_values(target: &Target) -> Vec<String> {
    match target {
        Target::Stdio { env, .. } => env.iter().map(|(_, v)| v.clone()).collect(),
        Target::Http { headers, .. } => headers
            .iter()
            .flat_map(|(_, v)| [v.clone(), v.strip_prefix("Bearer ").unwrap_or(v).to_owned()])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_cancels_queued_request_without_waiting_for_io() {
        let mut test = std::pin::pin!(async {
            let c = Client {
                io: Mutex::new(Io::Http {
                    url: "http://unused".into(),
                    headers: Vec::new(),
                    session: None,
                    protocol: None,
                }),
                next_id: Cell::new(1),
                broken: Cell::new(false),
                timeout_ms: 10000,
                pid: None,
                sensitive: Vec::new(),
                pending: RefCell::new(Vec::new()),
                reloaded: Cell::new(false),
                instructions: RefCell::new(None),
            };
            // Keep IO busy, so this test cannot accidentally make a host HTTP request.
            let held = c.io.lock().await;
            let args = json!({});
            let mut request = std::pin::pin!(c.request("tools/call", &args));
            assert!(futures::poll!(request.as_mut()).is_pending());
            let mut disconnect = std::pin::pin!(c.disconnect());
            assert!(futures::poll!(disconnect.as_mut()).is_pending());
            assert_eq!(request.await, Err("server reloaded".into()));
            drop(held);
            disconnect.await;
            assert_eq!(
                c.request("tools/call", &args).await,
                Err("server reloaded".into())
            );
        });
        let mut cx = std::task::Context::from_waker(futures::task::noop_waker_ref());
        assert!(test.as_mut().poll(&mut cx).is_ready());
    }
}
