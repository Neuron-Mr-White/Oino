//! A hand-rolled stdio JSON-RPC client for `rust-analyzer` (round5 #24).
//!
//! The gpui-base editor engine provides the LSP *traits* (`HoverProvider`,
//! `CompletionProvider`, `apply_lsp_edits`) but no transport: this module is the
//! transport. It is deliberately small — LSP 3.17 request/response routing over
//! `Content-Length`-framed stdio, an initialize handshake, full-document
//! `didOpen`/`didChange`/`didClose`, `publishDiagnostics` delivery on a channel,
//! and the `hover`/`formatting`/`completion` requests the Files page uses.
//!
//! Everything here runs on the app's tokio runtime (`rt::Tokio`); GPUI never
//! blocks. The client is `Send + Sync` so provider closures can move an `Arc`
//! into background futures.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Failures of the language-server transport.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LspError {
    /// `rust-analyzer` was not found on `PATH` or in the usual tool dirs.
    #[error("rust-analyzer not found (checked PATH and ~/.cargo/bin)")]
    NotFound,
    /// The server process could not be spawned or its stdio broke.
    #[error("lsp io: {0}")]
    Io(#[from] io::Error),
    /// The server answered a request with a JSON-RPC error.
    #[error("lsp {method} failed: {code} {message}")]
    Rpc {
        /// The failing method.
        method: &'static str,
        /// JSON-RPC error code.
        code: i64,
        /// JSON-RPC error message.
        message: String,
    },
    /// The server closed stdout (crashed or exited).
    #[error("language server stopped")]
    Stopped,
    /// A response could not be decoded into the expected type.
    #[error("lsp decode {method}: {source}")]
    Decode {
        /// The method whose result failed to decode.
        method: &'static str,
        /// The serde error.
        source: serde_json::Error,
    },
    /// A non-RPC failure (formatter exit status, UTF-8, …).
    #[error("lsp: {0}")]
    Other(String),
}

/// A `textDocument/publishDiagnostics` notification, forwarded by the reader task.
pub type DiagnosticsRx = mpsc::UnboundedReceiver<lsp_types::PublishDiagnosticsParams>;
/// The sending half kept by the reader task.
pub type DiagnosticsTx = mpsc::UnboundedSender<lsp_types::PublishDiagnosticsParams>;

/// Where `rust-analyzer` was found (for the status line).
#[derive(Clone, Debug)]
pub struct ServerBinary {
    /// The resolved executable path.
    pub path: PathBuf,
}

/// Finds a `rust-analyzer` executable. Order matters: the rustup *toolchain*
/// binary first (a version-managing shim on `PATH` — mise, asdf — can be a
/// broken recursion the way `~/.rustup/toolchains/*/bin/rust-analyzer` never
/// is), then `~/.cargo/bin` (rustup proxy), then `PATH`.
#[must_use]
pub fn find_server() -> Option<ServerBinary> {
    if let Some(home) = std::env::var_os("HOME") {
        let toolchains = Path::new(&home).join(".rustup/toolchains");
        if let Ok(mut dirs) = std::fs::read_dir(&toolchains) {
            let mut found: Vec<PathBuf> = dirs
                .by_ref()
                .flatten()
                .map(|e| e.path().join("bin/rust-analyzer"))
                .filter(|p| p.is_file())
                .collect();
            // Newest toolchain name last.
            found.sort();
            if let Some(path) = found.pop() {
                return Some(ServerBinary { path });
            }
        }
        let cargo_bin = Path::new(&home).join(".cargo/bin/rust-analyzer");
        if cargo_bin.is_file() {
            return Some(ServerBinary { path: cargo_bin });
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("rust-analyzer");
            if candidate.is_file() {
                return Some(ServerBinary { path: candidate });
            }
        }
    }
    None
}

/// Percent-encodes the characters in a path that are illegal in a URI
/// (the LSP `Uri` type validates, so spaces must not reach it raw).
#[must_use]
pub fn file_uri(path: &Path) -> Option<lsp_types::Uri> {
    let raw = path.to_string_lossy();
    let mut encoded = String::with_capacity(raw.len() + 8);
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                encoded.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    lsp_types::Uri::from_str(&format!("file://{encoded}")).ok()
}

/// Inverse of [`file_uri`] for the `file://` URIs diagnostics come back with.
#[must_use]
pub fn uri_path(uri: &lsp_types::Uri) -> Option<PathBuf> {
    let text = uri.as_str();
    let rest = text.strip_prefix("file://")?;
    let mut out = Vec::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| -> Option<u8> {
                match b {
                    b'0'..=b'9' => Some(b - b'0'),
                    b'a'..=b'f' => Some(b - b'a' + 10),
                    b'A'..=b'F' => Some(b - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

struct Pending {
    map: Mutex<HashMap<u64, oneshot::Sender<Result<Value, LspError>>>>,
}

impl Pending {
    fn insert(&self, id: u64) -> oneshot::Receiver<Result<Value, LspError>> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut map) = self.map.lock() {
            map.insert(id, tx);
        }
        rx
    }

    fn take(&self, id: u64) -> Option<oneshot::Sender<Result<Value, LspError>>> {
        self.map.lock().ok()?.remove(&id)
    }

    /// Fails every waiter (the server died or the reader loop ended).
    fn fail_all(&self) {
        if let Ok(mut map) = self.map.lock() {
            for (_, tx) in map.drain() {
                let _ = tx.send(Err(LspError::Stopped));
            }
        }
    }
}

/// One running language server: a `rust-analyzer` child and its JSON-RPC plumbing.
///
/// Requests may be issued concurrently; every method takes `&self`. The child is
/// `kill_on_drop`, so dropping the last `Arc` always reaps the server.
pub struct LspClient {
    /// Workspace root this server was started for.
    pub root: PathBuf,
    outgoing: mpsc::UnboundedSender<Value>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    _child: Child,
    _reader: JoinHandle<()>,
    _writer: JoinHandle<()>,
}

impl std::fmt::Debug for LspClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspClient")
            .field("root", &self.root)
            .field("alive", &self.alive.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl LspClient {
    /// Spawns `rust-analyzer` for `root`, performs the `initialize`/`initialized`
    /// handshake and returns the client plus the diagnostics channel.
    ///
    /// Must run inside a tokio runtime (uses `tokio::process` and `tokio::spawn`).
    ///
    /// # Errors
    /// `NotFound` when no server binary exists, `Io` on spawn failure, `Rpc`
    /// when the server rejects `initialize`.
    pub async fn start(
        root: &Path,
        diagnostics: DiagnosticsTx,
    ) -> Result<(Arc<Self>, ServerBinary), LspError> {
        let binary = find_server().ok_or(LspError::NotFound)?;
        let mut child = Command::new(&binary.path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "no server stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "no server stdout"))?;

        let pending = Arc::new(Pending {
            map: Mutex::new(HashMap::new()),
        });
        let (outgoing, mut outgoing_rx) = mpsc::unbounded_channel::<Value>();
        let alive = Arc::new(AtomicBool::new(true));

        // Writer: one task owns stdin so frames can never interleave.
        let writer = tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(msg) = outgoing_rx.recv().await {
                let Ok(body) = serde_json::to_vec(&msg) else {
                    continue;
                };
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                if stdin.write_all(header.as_bytes()).await.is_err()
                    || stdin.write_all(&body).await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
        });

        // Reader: frames stdout, routes responses to waiters and
        // `publishDiagnostics` to the channel. Server→client *requests*
        // (workspace/configuration, window/workDoneProgress/create, …) get a
        // minimal valid response so rust-analyzer does not wedge waiting.
        let reader_pending = Arc::clone(&pending);
        let reader_outgoing = outgoing.clone();
        let alive_flag = Arc::clone(&alive);
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                let Ok(Some(msg)) = read_message(&mut reader).await else {
                    break;
                };
                route_message(msg, &reader_pending, &reader_outgoing, &diagnostics);
            }
            alive_flag.store(false, Ordering::Relaxed);
            reader_pending.fail_all();
        });

        let client = Arc::new(Self {
            root: root.to_path_buf(),
            outgoing,
            pending,
            next_id: AtomicU64::new(1),
            alive,
            _child: child,
            _reader: reader,
            _writer: writer,
        });

        let root_uri = file_uri(&root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
        let capabilities = json!({
            "textDocument": {
                "synchronization": {
                    "didSave": true,
                    "dynamicRegistration": false
                },
                "publishDiagnostics": {},
                "hover": { "contentFormat": ["markdown", "plaintext"] },
                "formatting": { "dynamicRegistration": false },
                "completion": {
                    "completionItem": {
                        "documentationFormat": ["markdown", "plaintext"],
                        "snippetSupport": false
                    }
                }
            },
            "workspace": { "configuration": true }
        });
        let params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri.as_ref().map(|u| u.as_str()),
            "capabilities": capabilities,
            "clientInfo": { "name": "oino", "version": env!("CARGO_PKG_VERSION") },
            "initializationOptions": {
                // Native (no cargo check) diagnostics for open documents — makes
                // diagnostics show for unsaved buffers too.
                "diagnostics": { "experimental": { "enable": true } }
            }
        });
        client.request_value("initialize", params).await?;
        client.notify("initialized", &json!({}))?;
        Ok((client, binary))
    }

    /// Whether the server is still answering (stdout open).
    #[must_use]
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// A JSON-RPC notification; ordering with later requests is preserved
    /// (single writer queue).
    ///
    /// # Errors
    /// [`LspError::Stopped`] when the server already went away.
    pub fn notify(&self, method: &str, params: &Value) -> Result<(), LspError> {
        self.outgoing
            .send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .map_err(|_| LspError::Stopped)
    }

    /// A JSON-RPC notification from serializable `params`.
    ///
    /// # Errors
    /// [`LspError::Stopped`] when the server already went away.
    pub fn notify_ser<P: Serialize>(&self, method: &str, params: P) -> Result<(), LspError> {
        self.notify(method, &serde_json::to_value(params).unwrap_or(Value::Null))
    }

    /// A JSON-RPC request answered with the raw `result` value.
    async fn request_value(&self, method: &'static str, params: Value) -> Result<Value, LspError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let rx = self.pending.insert(id);
        let send = self
            .outgoing
            .send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        if send.is_err() {
            self.pending.take(id);
            return Err(LspError::Stopped);
        }
        let result = rx.await.map_err(|_| LspError::Stopped)??;
        if let Some(error) = result.get("error") {
            return Err(LspError::Rpc {
                method,
                code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
            });
        }
        Ok(result.get("result").cloned().unwrap_or(Value::Null))
    }

    /// A typed JSON-RPC request: the `result` decodes into `R`.
    async fn request<P, R>(&self, method: &'static str, params: P) -> Result<R, LspError>
    where
        P: Serialize,
        R: serde::de::DeserializeOwned,
    {
        let params =
            serde_json::to_value(params).map_err(|e| LspError::Decode { method, source: e })?;
        let value = self.request_value(method, params).await?;
        serde_json::from_value(value).map_err(|e| LspError::Decode { method, source: e })
    }

    /// `textDocument/didOpen` with full-document sync.
    pub fn did_open(&self, uri: &lsp_types::Uri, language_id: &str, text: &str, version: i32) {
        let _ = self.notify(
            "textDocument/didOpen",
            &json!({
                "textDocument": {
                    "uri": uri.as_str(),
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            }),
        );
    }

    /// `textDocument/didChange` with a full-document replacement (the editor
    /// keeps its own rope; diffing for incremental sync is a later polish).
    pub fn did_change(&self, uri: &lsp_types::Uri, text: &str, version: i32) {
        let _ = self.notify(
            "textDocument/didChange",
            &json!({
                "textDocument": { "uri": uri.as_str(), "version": version },
                "contentChanges": [{ "text": text }],
            }),
        );
    }

    /// `textDocument/didClose`.
    pub fn did_close(&self, uri: &lsp_types::Uri) {
        let _ = self.notify(
            "textDocument/didClose",
            &json!({ "textDocument": { "uri": uri.as_str() } }),
        );
    }

    /// `textDocument/didSave` (rust-analyzer uses it to trigger flycheck).
    pub fn did_save(&self, uri: &lsp_types::Uri) {
        let _ = self.notify(
            "textDocument/didSave",
            &json!({ "textDocument": { "uri": uri.as_str() } }),
        );
    }

    /// `textDocument/hover` at `position`.
    ///
    /// # Errors
    /// Propagates [`LspError::Rpc`]/[`LspError::Decode`]; `Ok(None)` = no hover.
    pub async fn hover(
        &self,
        uri: &lsp_types::Uri,
        position: lsp_types::Position,
    ) -> Result<Option<lsp_types::Hover>, LspError> {
        self.request(
            "textDocument/hover",
            json!({
                "textDocument": { "uri": uri.as_str() },
                "position": position,
            }),
        )
        .await
    }

    /// `textDocument/formatting` (whole document, 4-space indent).
    ///
    /// # Errors
    /// Propagates the RPC error; `Ok(vec![])` = server had nothing to change.
    pub async fn formatting(
        &self,
        uri: &lsp_types::Uri,
    ) -> Result<Vec<lsp_types::TextEdit>, LspError> {
        let edits: Option<Vec<lsp_types::TextEdit>> = self
            .request(
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": uri.as_str() },
                    "options": { "tabSize": 4, "insertSpaces": true },
                }),
            )
            .await?;
        Ok(edits.unwrap_or_default())
    }

    /// `textDocument/completion` at `position`.
    ///
    /// # Errors
    /// Propagates the RPC error.
    pub async fn completion(
        &self,
        uri: &lsp_types::Uri,
        position: lsp_types::Position,
        trigger: Option<&lsp_types::CompletionContext>,
    ) -> Result<lsp_types::CompletionResponse, LspError> {
        let context = trigger.map_or(Value::Null, |t| {
            serde_json::to_value(t).unwrap_or(Value::Null)
        });
        self.request(
            "textDocument/completion",
            json!({
                "textDocument": { "uri": uri.as_str() },
                "position": position,
                "context": context,
            }),
        )
        .await
    }

    /// Best-effort orderly shutdown: `shutdown` + `exit`, then `kill_on_drop`
    /// covers whatever refuses to leave.
    pub fn shutdown(&self) {
        let _ = self.outgoing.send(json!({
            "jsonrpc": "2.0",
            "id": self.next_id.fetch_add(1, Ordering::Relaxed),
            "method": "shutdown",
            "params": null,
        }));
        let _ = self.notify("exit", &Value::Null);
    }
}

/// Reads one `Content-Length`-framed message; `Ok(None)` = clean EOF.
async fn read_message(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> io::Result<Option<Value>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            length = rest.trim().parse().ok();
        }
    }
    let Some(len) = length else {
        return Ok(None);
    };
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body).ok())
}

/// Routes one decoded message: response → waiter, server request → minimal
/// valid reply, notification → the diagnostics channel (everything else is
/// progress/log noise this client does not surface).
fn route_message(
    msg: Value,
    pending: &Pending,
    outgoing: &mpsc::UnboundedSender<Value>,
    diagnostics: &DiagnosticsTx,
) {
    let id = msg.get("id").and_then(Value::as_u64);
    let method = msg.get("method").and_then(Value::as_str);
    match (id, method) {
        // A response (has an id, no method). Errors are forwarded as such.
        (Some(id), None) => {
            if let Some(tx) = pending.take(id) {
                let _ = tx.send(Ok(msg));
            }
        }
        // A server→client request; answer so the server never wedges.
        (Some(id), Some(method)) => {
            let result = match method {
                // rust-analyzer asks for `rust-analyzer.*` config sections;
                // `null` per item = "use defaults".
                "workspace/configuration" => {
                    let count = msg
                        .get("params")
                        .and_then(|p| p.get("items"))
                        .and_then(Value::as_array)
                        .map_or(1, Vec::len);
                    json!(vec![Value::Null; count])
                }
                // Nothing else needs a real answer for editing to work.
                _ => Value::Null,
            };
            let _ = outgoing.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
        }
        // A notification.
        (None, Some("textDocument/publishDiagnostics")) => {
            if let Some(params) = msg.get("params")
                && let Ok(parsed) =
                    serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params.clone())
            {
                let _ = diagnostics.send(parsed);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_encodes_spaces() {
        let uri = file_uri(Path::new("/tmp/a b/c.rs")).unwrap();
        assert_eq!(uri.as_str(), "file:///tmp/a%20b/c.rs");
        assert_eq!(uri_path(&uri), Some(PathBuf::from("/tmp/a b/c.rs")));
    }

    #[test]
    fn uri_path_decodes() {
        let uri = lsp_types::Uri::from_str("file:///home/u/x%2By.rs").unwrap();
        assert_eq!(uri_path(&uri), Some(PathBuf::from("/home/u/x+y.rs")));
    }

    /// Spike proof (round5 #24, scout-gpui risk #1): a real rust-analyzer over
    /// this client publishes diagnostics and answers hover/formatting. Runs
    /// only when a server binary exists; skipped otherwise.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rust_analyzer_round_trip() {
        if find_server().is_none() {
            // CI hosts without rust-analyzer keep the unit tests green.
            return;
        }
        // `OINO_LSP_SPIKE_DIR` + `OINO_LSP_SPIKE_FILE` reuse this against a
        // real checkout (debug aid); the default builds a scratch crate.
        // Not `temp_dir()`: the charter points TMPDIR at `target/tmp`.
        let (dir, file, bad) = if let (Ok(d), Ok(f)) = (
            std::env::var("OINO_LSP_SPIKE_DIR"),
            std::env::var("OINO_LSP_SPIKE_FILE"),
        ) {
            let file = PathBuf::from(f);
            let text = std::fs::read_to_string(&file).unwrap();
            (PathBuf::from(d), file, text)
        } else {
            let dir = PathBuf::from(format!("/tmp/oino-lsp-spike-{}", std::process::id()));
            std::fs::create_dir_all(dir.join("src")).unwrap();
            let dir = dir.canonicalize().unwrap_or(dir);
            // `[workspace]` keeps `cargo metadata` from walking up into an
            // ancestor workspace (e.g. the repo's own when TMPDIR is target/).
            std::fs::write(
                dir.join("Cargo.toml"),
                "[package]\nname = \"spike\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
            )
            .unwrap();
            let file = dir.join("src/main.rs");
            let bad = "fn main() {\n    let x: i32 = \"nope\";\n    let _ = x;\n}\n";
            std::fs::write(&file, bad).unwrap();
            (dir, file, bad.to_owned())
        };

        let (tx, mut rx) = mpsc::unbounded_channel();
        let (client, _bin) = LspClient::start(&dir, tx)
            .await
            .expect("rust-analyzer should start and answer initialize");
        let uri = file_uri(&file).unwrap();
        client.did_open(&uri, "rust", &bad, 1);

        // rust-analyzer publishes native diagnostics for the open file; give it
        // a generous budget (first workspace load runs cargo metadata).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut saw = false;
        let mut got = 0_usize;
        while std::time::Instant::now() < deadline && !saw {
            match tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await {
                Ok(Some(params)) => {
                    got += 1;
                    if params.uri == uri && !params.diagnostics.is_empty() {
                        saw = true;
                    }
                }
                Ok(None) => break, // channel closed
                Err(_) => {}       // still waiting
            }
        }
        assert!(
            saw,
            "rust-analyzer should publish diagnostics for a type error (got {got} publishes, uri {uri:?}, alive {})",
            client.alive()
        );
        // `content modified` is r-a's transient error while its initial
        // analysis catches up with the didOpen — retry before failing.
        let mut hover = None;
        for _ in 0..10 {
            match client.hover(&uri, lsp_types::Position::new(1, 8)).await {
                Ok(h) => {
                    hover = h;
                    break;
                }
                Err(LspError::Rpc { code: -32801, .. }) => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => panic!("hover should answer: {e}"),
            }
        }
        assert!(hover.is_some(), "hover over `x` should return content");

        let messy = "fn main() {\nlet   x=1;\nlet _=x;\n}\n";
        client.did_change(&uri, messy, 2);
        let mut edits = Vec::new();
        for _ in 0..10 {
            match client.formatting(&uri).await {
                Ok(e) => {
                    edits = e;
                    break;
                }
                Err(LspError::Rpc { code: -32801, .. }) => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => panic!("format should answer: {e}"),
            }
        }
        assert!(!edits.is_empty(), "rustfmt via LSP should reindent");

        client.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
