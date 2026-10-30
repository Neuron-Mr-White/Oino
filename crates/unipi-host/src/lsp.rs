//! Host language servers (app spec §3.12, WP-A21): `lsp_servers`,
//! `lsp_open`.
//!
//! Like Zed, the server is owned by the project side — one process per
//! `(root, language)` — not by the editor view. The files live on the host
//! and so do the servers, so a paired phone gets the same completions a
//! desktop editor would (the alternative, spawning on the app side, would
//! only work on This computer).
//!
//! - **`lsp_servers{path}`** → `LspServersData`: the detected project root
//!   (nearest ancestor with a marker — `Cargo.toml`, `package.json`,
//!   `tsconfig.json`, `pyproject.toml`, `go.mod`, `compile_commands.json`,
//!   `.git` — inside the fs scope) plus every known server's name, command
//!   and `available` flag (`binary on PATH`) with an `install_hint` when
//!   missing. No auto-download in v2.
//! - **`lsp_open{root, language}`** → `ok{data:LspAttachData}`, then a
//!   persistent bi-stream of *raw LSP stdio bytes* (kind 1 server→app,
//!   kind 2 app→server, ≤ 256 KiB chunks, the `Content-Length` framing
//!   untouched) ending with `Control::Exit` when the server dies.
//! - **One attached client**: a second `lsp_open` takes over — the previous
//!   stream ends `Err{code:"E_LSP_TAKEN"}` and the host sends `didClose`
//!   for the documents that client had open. A re-attaching client's
//!   `initialize` is answered from the cached result (its `initialized` is
//!   swallowed); the first client's `initialize` is forwarded and its
//!   reply cached. For that the host *inspects* the byte stream — the
//!   framed bytes themselves always pass through verbatim.
//! - **Commands come only from the host settings**
//!   (`lsp.servers.<language>.command`, else the compiled-in defaults); a
//!   request naming a binary is `E_UNAUTHORIZED`. `lsp.enabled` (default
//!   on) switches the whole feature off; `lsp.idle_minutes` (default 10)
//!   is how long a detached server survives before the host kills it.
//!   Servers die with the host (`kill_all` on shutdown).
//!
//! The registry is one per host process ([`LspRegistry::shared`]); tests
//! build their own.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use unipi_proto::{
    AppErrorCode, AppErrorDto, Control, LspAttachData, LspServerInfo, LspServersData, Request,
    frame::{FrameKind, encode},
};

use crate::{
    data_dir::HostPaths,
    fs_scope::FsScope,
    protocol::{read_frame, write_control},
};

/// Markers that make a directory a project root (nearest ancestor wins).
const ROOT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "tsconfig.json",
    "pyproject.toml",
    "go.mod",
    "compile_commands.json",
    ".git",
];

/// One LSP message frame sent to an attached client at a time (§3.12:
/// "≤ 256 KiB chunks").
const STREAM_CHUNK: usize = 256 * 1024;
/// Largest LSP message the host inspects before giving up on parsing (the
/// bytes still pass through — inspection is an optimization, not a gate).
const INSPECT_CAP: usize = 64 * 1024 * 1024;
/// Buffered server→client bytes per attached stream; a slower client is
/// ended rather than growing the host's memory.
const OUTBOX_CAP: usize = 512;

fn err(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    err(AppErrorCode::E_INTERNAL, e.to_string())
}

/// Locks a registry/entry mutex, recovering from a poisoned one: a panic
/// in one pump must not take the other servers down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The compiled-in server table: `language` id (what `lsp_open` takes),
/// display name, default command, install hint. `lsp.servers.<language>
/// .command` in `config.json` replaces the command, never the language.
const DEFAULT_SERVERS: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "rust",
        "rust-analyzer",
        "rust-analyzer",
        Some("Install with `rustup component add rust-analyzer`"),
    ),
    (
        "typescript",
        "TypeScript",
        "typescript-language-server --stdio",
        Some("Install with `npm i -g typescript typescript-language-server`"),
    ),
    (
        "typescriptreact",
        "TypeScript",
        "typescript-language-server --stdio",
        Some("Install with `npm i -g typescript typescript-language-server`"),
    ),
    (
        "javascript",
        "TypeScript",
        "typescript-language-server --stdio",
        Some("Install with `npm i -g typescript typescript-language-server`"),
    ),
    (
        "javascriptreact",
        "TypeScript",
        "typescript-language-server --stdio",
        Some("Install with `npm i -g typescript typescript-language-server`"),
    ),
    (
        "python",
        "Pyright",
        "pyright-langserver --stdio",
        Some("Install with `npm i -g pyright`"),
    ),
    (
        "go",
        "gopls",
        "gopls",
        Some("Install with `go install golang.org/x/tools/gopls@latest`"),
    ),
    (
        "c",
        "clangd",
        "clangd",
        Some("Install clangd via your package manager"),
    ),
    (
        "cpp",
        "clangd",
        "clangd",
        Some("Install clangd via your package manager"),
    ),
    (
        "yaml",
        "YAML",
        "yaml-language-server --stdio",
        Some("Install with `npm i -g yaml-language-server`"),
    ),
    (
        "json",
        "JSON",
        "vscode-json-language-server --stdio",
        Some("Install with `npm i -g vscode-langservers-extracted`"),
    ),
    (
        "toml",
        "Taplo",
        "taplo lsp stdio",
        Some("Install with `cargo install taplo-cli --locked`"),
    ),
    (
        "markdown",
        "Marksman",
        "marksman server",
        Some("Install with `npm i -g marksman`"),
    ),
];

/// One `lsp.servers.<language>` entry: `"command"` or
/// `{ "command": …, "install": "how to install" }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum LspServerSetting {
    Command(String),
    Object {
        command: String,
        install: Option<String>,
    },
}

impl LspServerSetting {
    pub fn command(&self) -> &str {
        match self {
            LspServerSetting::Command(command) => command,
            LspServerSetting::Object { command, .. } => command,
        }
    }

    pub fn install_hint(&self) -> Option<&str> {
        match self {
            LspServerSetting::Command(_) => None,
            LspServerSetting::Object { install, .. } => install.as_deref(),
        }
    }
}

/// The `lsp` block of `<host dir>/config.json` (app spec §3.12), read
/// fresh on every `lsp_open` / `lsp_servers` like every other block.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LspConfig {
    /// Language servers on or off for this host (R4 opt-out).
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Minutes a detached server survives before the host kills it.
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u64,
    /// `lsp.servers.<language>` — `{ "command": "server --stdio" }` or a
    /// bare `"server --stdio"` string.
    #[serde(default)]
    pub servers: BTreeMap<String, LspServerSetting>,
}

fn default_enabled() -> bool {
    true
}

fn default_idle_minutes() -> u64 {
    10
}

impl Default for LspConfig {
    fn default() -> Self {
        LspConfig {
            enabled: true,
            idle_minutes: 10,
            servers: BTreeMap::new(),
        }
    }
}

impl LspConfig {
    /// Reads the block; a missing/corrupt `lsp` block is the default
    /// (enabled, 10 min idle, built-in server commands).
    pub fn load(config_path: &Path) -> LspConfig {
        let raw = crate::data_dir::read_json::<serde_json::Value>(config_path)
            .ok()
            .flatten();
        raw.as_ref()
            .and_then(|value| value.get("lsp").cloned())
            .and_then(|value| serde_json::from_value::<LspConfig>(value).ok())
            .unwrap_or_default()
    }

    /// `(name, command, install_hint)` for a language: the configured
    /// command first, else the built-in table. `None` = no server known.
    fn spec_for(&self, language: &str) -> Option<(String, String, Option<String>)> {
        if let Some(custom) = self.servers.get(language) {
            let name = DEFAULT_SERVERS
                .iter()
                .find(|(lang, ..)| *lang == language)
                .map(|(_, name, ..)| (*name).to_string())
                .unwrap_or_else(|| language.to_string());
            return Some((
                name,
                custom.command().to_string(),
                custom.install_hint().map(str::to_string),
            ));
        }
        DEFAULT_SERVERS
            .iter()
            .find(|(lang, ..)| *lang == language)
            .map(|(_, name, command, hint)| {
                (
                    (*name).to_string(),
                    (*command).to_string(),
                    hint.map(str::to_string),
                )
            })
    }

    /// Every server `lsp_servers` lists: the built-in table plus
    /// languages the host configured on top.
    fn list(&self) -> Vec<LspServerInfo> {
        let mut languages: Vec<&str> = DEFAULT_SERVERS.iter().map(|(lang, ..)| *lang).collect();
        for language in self.servers.keys() {
            if !languages.contains(&language.as_str()) {
                languages.push(language);
            }
        }
        languages
            .into_iter()
            .filter_map(|language| {
                self.spec_for(language)
                    .map(|(name, command, hint)| LspServerInfo {
                        language: language.to_string(),
                        name,
                        available: on_path(command_bin(&command)),
                        install_hint: if on_path(command_bin(&command)) {
                            None
                        } else {
                            hint
                        },
                        command,
                    })
            })
            .collect()
    }
}

/// The program of a command line (first word; quoting is not supported in
/// settings commands — one simple argv split).
fn command_bin(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or(command)
}

/// Is `bin` runnable from PATH (the binary the first word of `command`
/// names)? No absolute paths from settings either — settings commands
/// may name an absolute path, which is the owner's own choice and fine.
fn on_path(bin: &str) -> bool {
    if bin.is_empty() {
        return false;
    }
    let path = Path::new(bin);
    if path.is_absolute() {
        return is_executable(path);
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&paths) {
        for candidate in candidates(&dir, bin) {
            if is_executable(&candidate) {
                return true;
            }
        }
    }
    false
}

fn candidates(dir: &Path, bin: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            dir.join(format!("{bin}.exe")),
            dir.join(format!("{bin}.cmd")),
            dir.join(format!("{bin}.bat")),
            dir.join(bin),
        ]
    } else {
        vec![dir.join(bin)]
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The project root for `path`: the nearest ancestor carrying a marker,
/// walking up while still inside the fs scope (`None` below the scope's
/// boundary or when nothing marks a project).
pub fn find_root(path: &Path, scope: &FsScope) -> Option<PathBuf> {
    let mut dir = if path.is_dir() { path } else { path.parent()? };
    loop {
        if !scope.allows(dir) {
            return None;
        }
        if ROOT_MARKERS.iter().any(|marker| dir.join(marker).exists()) {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
}

/// What a client→server message means for the host: forward the verbatim
/// framed bytes, answer from the cached `initialize` result instead, or
/// drop the message entirely (`initialized` after a cached initialize).
enum ClientAction {
    Forward(Vec<u8>),
    AnswerFromCache(Vec<u8>),
    Swallow,
}

/// Incremental `Content-Length` splitter over a byte stream that may
/// split a message anywhere. `drain` yields complete `[headers+body]`
/// segments verbatim; a malformed stream is *poisoned* — from then on
/// everything passes through untouched (inspection is never allowed to
/// lose bytes).
#[derive(Default)]
pub struct FrameParser {
    buffer: Vec<u8>,
    poisoned: bool,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `chunk`; returns every complete framed message (headers
    /// included, exactly as received).
    pub fn drain(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        if self.poisoned {
            return Vec::new();
        }
        let mut out = Vec::new();
        while let Some(header_end) = find_subslice(&self.buffer, b"\r\n\r\n") {
            let Some(len) = content_length(&self.buffer[..header_end]) else {
                self.poisoned = true;
                return Vec::new();
            };
            if len > INSPECT_CAP {
                self.poisoned = true;
                return Vec::new();
            }
            let total = header_end + 4 + len;
            if self.buffer.len() < total {
                break;
            }
            out.push(self.buffer.drain(..total).collect());
        }
        // An unterminated buffer that already overflows the cap is poison too.
        if self.buffer.len() > INSPECT_CAP {
            self.poisoned = true;
            return Vec::new();
        }
        out
    }

    /// True once the input looked malformed; callers forward raw bytes.
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }

    /// Whatever partial tail remains (callers flush it on close).
    pub fn rest(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `Content-Length: N` of an LSP header block (other headers may follow;
/// the value line may end with or without `\r\n`).
fn content_length(headers: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(headers).ok()?;
    for line in text.split("\r\n") {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

/// The JSON body of one complete framed message (`[headers+body]`).
fn body_of(framed: &[u8]) -> &[u8] {
    match find_subslice(framed, b"\r\n\r\n") {
        Some(at) => &framed[at + 4..],
        None => &[],
    }
}

/// `{"jsonrpc":"2.0","id":id,"result":result}` with the LSP stdio header.
fn framed_response(id: &serde_json::Value, result: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .unwrap_or_default();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// `{"jsonrpc":"2.0","method":"textDocument/didClose","params":{…}}` framed.
fn framed_did_close(uri: &str) -> Vec<u8> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": uri } },
    }))
    .unwrap_or_default();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// One server→client event inside the attach channel.
enum Out {
    /// Raw LSP stdio bytes from the server (sent as kind-1 frames).
    Bytes(Vec<u8>),
    /// A newer client attached: end this stream with `E_LSP_TAKEN`.
    TakenOver,
    /// The server died; the stream ends with `Exit{code}`.
    Exited(Option<i64>),
}

/// What the registry waits on and kills — a spawned `Child` in the real
/// path, a scripted stand-in in tests (`#[doc(hidden)] testkit`).
#[doc(hidden)]
pub trait Waitable: Send {
    /// `Ok(Some(code))` once exited, `Ok(None)` while running.
    fn try_wait(&mut self) -> std::io::Result<Option<i64>>;
    fn kill(&mut self) -> std::io::Result<()>;
}

impl Waitable for Child {
    fn try_wait(&mut self) -> std::io::Result<Option<i64>> {
        Child::try_wait(self).map(|status| status.and_then(|s| s.code().map(i64::from)))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Child::kill(self)
    }
}

/// One host language server plus the client (at most one) attached to it.
pub struct ServerEntry {
    root: PathBuf,
    language: String,
    /// The resolved command line (diagnostics/tests).
    #[allow(dead_code)]
    command: String,
    stdin: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Waitable + Send>>,
    alive: AtomicBool,
    exit_code: Mutex<Option<i64>>,
    /// The cached `initialize` `result` (filled by the stdout pump when
    /// the forwarded `initialize` is answered).
    init: Mutex<Option<serde_json::Value>>,
    /// The request id of the `initialize` currently forwarded and not
    /// yet answered (`None` once cached or when a cached answer served).
    pending_init: Mutex<Option<serde_json::Value>>,
    /// The one attached stream (`None` while the server idles).
    outbox: Mutex<Option<mpsc::Sender<Out>>>,
    /// Bumped per attach; a detach only counts while its generation is
    /// still the latest (take-over bumps first).
    generation: AtomicU64,
}

impl ServerEntry {
    /// The cached `initialize` result, when the server initialized for an
    /// earlier client already.
    fn initialize_result(&self) -> Option<serde_json::Value> {
        lock(&self.init).clone()
    }

    /// Writes `framed` (headers + body) to the server's stdin.
    fn write_stdin(&self, framed: &[u8]) -> std::io::Result<()> {
        let mut stdin = lock(&self.stdin);
        stdin.write_all(framed)?;
        stdin.flush()
    }

    /// A scripted stand-in for tests: `stdin` receives what the client
    /// pump forwards, `waiter` reports exit/kill. `#[doc(hidden)]`.
    #[doc(hidden)]
    pub fn for_test(
        root: PathBuf,
        language: &str,
        stdin: impl Write + Send + 'static,
        waiter: impl Waitable + 'static,
    ) -> Arc<ServerEntry> {
        Arc::new(ServerEntry {
            root,
            language: language.to_string(),
            command: "test".to_string(),
            stdin: Mutex::new(Box::new(stdin)),
            child: Mutex::new(Box::new(waiter)),
            alive: AtomicBool::new(true),
            exit_code: Mutex::new(None),
            init: Mutex::new(None),
            pending_init: Mutex::new(None),
            outbox: Mutex::new(None),
            generation: AtomicU64::new(0),
        })
    }

    /// Registers a new attached client. When one was already attached it
    /// is ended with `E_LSP_TAKEN` — the evicted attach closes its own
    /// documents on the way out (`AttachState::open_docs` → `didClose`s
    /// through the shared server stdin). The returned generation marks
    /// this attach.
    fn attach(&self) -> (mpsc::Receiver<Out>, u64) {
        let (tx, rx) = mpsc::channel(OUTBOX_CAP);
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let previous = lock(&self.outbox).replace(tx);
        if let Some(previous) = previous {
            let _ = previous.try_send(Out::TakenOver);
        }
        (rx, generation)
    }

    /// This attach's stream ended (EOF, take-over already handled, error):
    /// forget it — unless a newer attach replaced it — and report whether
    /// the server is now truly clientless (the idle killer is ours to arm).
    fn detached(&self, generation: u64) -> bool {
        if self
            .generation
            .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        lock(&self.outbox).take();
        true
    }

    /// Kills the server process (idle shutdown, host quit).
    fn kill(&self) {
        if let Err(error) = lock(&self.child).kill() {
            tracing::warn!(language = %self.language, %error, "LSP server kill failed");
        }
        self.alive.store(false, Ordering::SeqCst);
    }
}

/// The host's language servers, keyed `(root, language)`
/// ([`shared`](LspRegistry::shared) — one registry per host process).
#[derive(Clone, Default)]
pub struct LspRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    servers: Mutex<BTreeMap<(PathBuf, String), Arc<ServerEntry>>>,
}

impl LspRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-wide registry the protocol handler uses.
    pub fn shared() -> &'static LspRegistry {
        static SHARED: OnceLock<LspRegistry> = OnceLock::new();
        SHARED.get_or_init(LspRegistry::new)
    }

    /// The live server for `(root, language)` — spawning it on demand —
    /// plus whether it already ran (`attached` on the wire).
    pub fn open(
        &self,
        paths: &HostPaths,
        root: PathBuf,
        language: &str,
        name: &str,
        command: &str,
    ) -> Result<(Arc<ServerEntry>, bool), AppErrorDto> {
        let key = (root.clone(), language.to_string());
        let mut servers = lock(&self.inner.servers);
        if let Some(entry) = servers.get(&key) {
            if entry.alive.load(Ordering::SeqCst) {
                return Ok((Arc::clone(entry), true));
            }
            servers.remove(&key);
        }
        let entry = spawn_server(paths, root, language, name, command)?;
        servers.insert(key, Arc::clone(&entry));
        Ok((entry, false))
    }

    /// Removes `key` when it still maps to `entry` and nobody is
    /// attached (the idle killer and dead-server cleanup).
    fn evict(&self, key: &(PathBuf, String), entry: &Arc<ServerEntry>) {
        let mut servers = lock(&self.inner.servers);
        let victim = match servers.get(key) {
            Some(current) if Arc::ptr_eq(current, entry) && lock(&entry.outbox).is_none() => {
                servers.remove(key)
            }
            _ => None,
        };
        drop(servers);
        if let Some(victim) = victim {
            victim.kill();
        }
    }

    /// Kills every server (host shutdown, app spec §3.12).
    pub fn kill_all(&self) {
        let entries: Vec<Arc<ServerEntry>> = std::mem::take(&mut *lock(&self.inner.servers))
            .into_values()
            .collect();
        for entry in entries {
            entry.kill();
        }
    }

    /// Live server count (tests).
    pub fn len(&self) -> usize {
        lock(&self.inner.servers).len()
    }

    /// No live servers.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Spawns the server process in `root`, with its stdout pump (raw chunks
/// to the attached stream + `initialize`-result caching), stderr drain
/// and waiter threads. Mirrors `term.rs`'s thread model.
fn spawn_server(
    _paths: &HostPaths,
    root: PathBuf,
    language: &str,
    name: &str,
    command: &str,
) -> Result<Arc<ServerEntry>, AppErrorDto> {
    let mut argv = command.split_whitespace();
    let bin = argv.next().ok_or_else(|| {
        err(
            AppErrorCode::E_UNSUPPORTED,
            format!("no server configured for {language}"),
        )
    })?;
    let mut process = Command::new(bin);
    process
        .args(argv)
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A host started from inside an editor/agent must not leak its env.
    for (key, _) in std::env::vars() {
        if key.starts_with("TMUX") || key.starts_with("HERDR_") {
            process.env_remove(key);
        }
    }
    let mut child = process.spawn().map_err(|error| {
        err(
            AppErrorCode::E_NOT_FOUND,
            format!(
                "could not start {name} ({command}) in {}: {error}",
                root.display()
            ),
        )
    })?;
    let stdin = child.stdin.take().ok_or_else(|| internal("no stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| internal("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| internal("no stderr"))?;
    let entry = Arc::new(ServerEntry {
        root: root.clone(),
        language: language.to_string(),
        command: command.to_string(),
        stdin: Mutex::new(Box::new(stdin)),
        child: Mutex::new(Box::new(child) as Box<dyn Waitable + Send>),
        alive: AtomicBool::new(true),
        exit_code: Mutex::new(None),
        init: Mutex::new(None),
        pending_init: Mutex::new(None),
        outbox: Mutex::new(None),
        generation: AtomicU64::new(0),
    });
    spawn_stdout_pump(Arc::clone(&entry), stdout);
    spawn_stderr_drain(Arc::clone(&entry), stderr);
    spawn_waiter(Arc::clone(&entry));
    tracing::info!(language, root = %entry.root.display(), command, "LSP server spawned");
    Ok(entry)
}

/// Reads the server's stdout: raw chunks to the attached stream (bytes
/// pass through untouched) while a side parser watches for the response
/// to the forwarded `initialize`, whose result is cached for re-attaches.
#[doc(hidden)]
pub fn spawn_stdout_pump(entry: Arc<ServerEntry>, mut stdout: impl Read + Send + 'static) {
    let name = format!("lsp-out-{}-{}", entry.language, std::process::id());
    let for_err = Arc::clone(&entry);
    let spawned = std::thread::Builder::new().name(name).spawn(move || {
        let mut parser = FrameParser::new();
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let read = match stdout.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let chunk = &buffer[..read];
            // Inspect a copy for the initialize cache; forward the raw bytes.
            for framed in parser.drain(chunk) {
                cache_initialize(&entry, body_of(&framed));
            }
            let tx = lock(&entry.outbox).clone();
            if let Some(tx) = tx {
                if tx.blocking_send(Out::Bytes(chunk.to_vec())).is_err() {
                    // The client went away mid-read; keep consuming so the
                    // server doesn't block on a full stdout pipe.
                    continue;
                }
            }
        }
        // EOF: the waiter publishes the exit code to the stream.
    });
    if let Err(error) = spawned {
        tracing::warn!(language = %for_err.language, %error, "could not start the LSP stdout pump");
    }
}

/// `{"id": <pending>, "result": …}` → the initialize cache.
fn cache_initialize(entry: &Arc<ServerEntry>, body: &[u8]) {
    let Ok(message) = serde_json::from_slice::<serde_json::Value>(body) else {
        return;
    };
    // Requests/notifications carry `method`; only bare responses cache.
    if message.get("method").is_some() {
        return;
    }
    let pending = lock(&entry.pending_init).clone();
    let Some(pending) = pending else { return };
    if message.get("id") != Some(&pending) {
        return;
    }
    *lock(&entry.pending_init) = None;
    if let Some(result) = message.get("result").cloned() {
        *lock(&entry.init) = Some(result);
    }
}

/// Drains stderr into tracing so a chatty server can't fill its pipe.
fn spawn_stderr_drain(entry: Arc<ServerEntry>, mut stderr: std::process::ChildStderr) {
    use std::io::Read;
    let name = format!("lsp-err-{}-{}", entry.language, std::process::id());
    let language = entry.language.clone();
    let spawned = std::thread::Builder::new().name(name).spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    tracing::debug!(language = %language, "{}", String::from_utf8_lossy(&buffer[..read]).trim_end());
                }
            }
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(language = %entry.language, %error, "could not start the LSP stderr drain");
    }
}

/// Polls `try_wait` so the child stays killable through `entry.child`
/// (a `wait()` would hold the lock for the server's whole life); on exit
/// the attached stream (if any) gets `Exited` and the entry goes dead.
fn spawn_waiter(entry: Arc<ServerEntry>) {
    let name = format!("lsp-wait-{}-{}", entry.language, std::process::id());
    let for_err = Arc::clone(&entry);
    let spawned = std::thread::Builder::new().name(name).spawn(move || {
        let code = loop {
            match lock(&entry.child).try_wait() {
                Ok(Some(code)) => break Some(code),
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => {
                    tracing::warn!(language = %entry.language, %error, "LSP wait failed");
                    break None;
                }
            }
        };
        *lock(&entry.exit_code) = code;
        entry.alive.store(false, Ordering::SeqCst);
        if let Some(tx) = lock(&entry.outbox).clone() {
            let _ = tx.blocking_send(Out::Exited(code));
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(language = %for_err.language, %error, "could not start the LSP waiter");
    }
}

/// Per-attachment client-message state: whether this attachment's
/// `initialize` was answered from the cache (so its `initialized` is
/// swallowed) and the stream's incremental parser.
struct AttachState {
    parser: FrameParser,
    /// `initialized` follows an `initialize` the host answered itself.
    swallow_initialized: bool,
    /// URIs this attach opened and hasn't closed — flushed with
    /// synthesized `didClose`s when it detaches (also on take-over).
    open_docs: BTreeSet<String>,
}

/// Inspects one complete client→server framed message and decides what
/// the host does with it (§3.12: bytes pass through verbatim — the only
/// synthesized traffic is the cached `initialize` answer and `didClose`s).
fn inspect_client_message(
    entry: &Arc<ServerEntry>,
    state: &mut AttachState,
    framed: Vec<u8>,
) -> ClientAction {
    let Ok(message) = serde_json::from_slice::<serde_json::Value>(body_of(&framed)) else {
        return ClientAction::Forward(framed);
    };
    let method = message.get("method").and_then(|m| m.as_str());
    match method {
        Some("initialize") => {
            if let Some(cached) = entry.initialize_result() {
                // Re-attach: answer from the cache with this client's id;
                // the server never sees the duplicate (§3.12).
                state.swallow_initialized = true;
                let id = message
                    .get("id")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                return ClientAction::AnswerFromCache(framed_response(&id, &cached));
            }
            // First attach: forward and remember the id so the stdout
            // pump knows which response to cache.
            *lock(&entry.pending_init) = message.get("id").cloned();
            ClientAction::Forward(framed)
        }
        Some("initialized") if state.swallow_initialized => {
            state.swallow_initialized = false;
            ClientAction::Swallow
        }
        Some("textDocument/didOpen") => {
            if let Some(uri) = message
                .pointer("/params/textDocument/uri")
                .and_then(|u| u.as_str())
            {
                state.open_docs.insert(uri.to_string());
            }
            ClientAction::Forward(framed)
        }
        Some("textDocument/didClose") => {
            if let Some(uri) = message
                .pointer("/params/textDocument/uri")
                .and_then(|u| u.as_str())
            {
                state.open_docs.remove(uri);
            }
            ClientAction::Forward(framed)
        }
        _ => ClientAction::Forward(framed),
    }
}

/// Serves one `lsp_*` request on an app/phone bi-stream (app spec §3.12):
/// `lsp_servers` answers and returns; `lsp_open` replies
/// `ok{data:LspAttachData}` and then pumps the server until it dies, the
/// client detaches or a newer attach takes over.
pub async fn serve_request<R, W>(
    registry: &LspRegistry,
    paths: &HostPaths,
    req: Request,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    match req {
        Request::LspServers { path, outside } => {
            tracing::info!(op = "lsp_servers", path = path.as_str(), "request");
            let scope = FsScope::load(&paths.config())?;
            let resolved =
                crate::fs::scoped_for_request(path.as_str(), &scope, outside.unwrap_or(false))?;
            let root = find_root(&resolved, &scope);
            let config = LspConfig::load(&paths.config());
            let data = LspServersData {
                root: root.map(|root| root.to_string_lossy().into_owned()),
                servers: config.list(),
            };
            write_control(
                write,
                &Control::Ok {
                    data: Some(serde_json::to_value(data).map_err(internal)?),
                },
            )
            .await
            .map_err(internal)?;
            Ok(())
        }
        Request::LspOpen {
            root,
            language,
            outside,
            command,
        } => {
            tracing::info!(
                op = "lsp_open",
                root = root.as_str(),
                language = language.as_str(),
                "request"
            );
            // §3.12: the app names a language, never a binary — a request
            // carrying a command is refused outright (paired-device safety).
            if command.is_some() {
                return Err(err(
                    AppErrorCode::E_UNAUTHORIZED,
                    "language servers are configured on the host, not by requests",
                ));
            }
            let config = LspConfig::load(&paths.config());
            if !config.enabled {
                return Err(err(
                    AppErrorCode::E_UNSUPPORTED,
                    "language servers are switched off on this host (lsp.enabled)",
                ));
            }
            let scope = FsScope::load(&paths.config())?;
            let resolved =
                crate::fs::scoped_for_request(root.as_str(), &scope, outside.unwrap_or(false))?;
            let language = language.as_str().to_string();
            let Some((name, command, install_hint)) = config.spec_for(&language) else {
                return Err(err(
                    AppErrorCode::E_UNSUPPORTED,
                    format!("no language server configured for {language}"),
                ));
            };
            if !on_path(command_bin(&command)) {
                return Err(err(
                    AppErrorCode::E_NOT_FOUND,
                    match &install_hint {
                        Some(hint) => format!("{name} isn't installed on this PC — {hint}"),
                        None => format!("{name} isn't installed on this PC ({command})"),
                    },
                ));
            }
            let key = (resolved.clone(), language.clone());
            let (entry, existed) = registry.open(paths, resolved, &language, &name, &command)?;
            let data = LspAttachData {
                server: name,
                attached: existed,
                initialize_result: entry.initialize_result(),
            };
            write_control(
                write,
                &Control::Ok {
                    data: Some(serde_json::to_value(&data).map_err(internal)?),
                },
            )
            .await
            .map_err(internal)?;
            let (mut rx, generation) = entry.attach();
            // `idle_minutes: 0` kills the server as soon as it detaches.
            let idle = Duration::from_secs(config.idle_minutes.saturating_mul(60));
            pump(
                registry.clone(),
                entry,
                key,
                generation,
                idle,
                read,
                write,
                &mut rx,
            )
            .await
        }
        _ => Err(err(AppErrorCode::E_UNSUPPORTED, "not an LSP request")),
    }
}

/// Streams one attach: server bytes out as kind-1 frames (≤ 256 KiB),
/// client kind-2 bytes inspected then forwarded verbatim, `E_LSP_TAKEN`
/// on take-over, `Exit{code}` when the server dies. On return the
/// attach's documents are closed on the server and the idle killer is
/// armed (`lsp.idle_minutes`).
#[allow(clippy::too_many_arguments)]
async fn pump<R, W>(
    registry: LspRegistry,
    entry: Arc<ServerEntry>,
    key: (PathBuf, String),
    generation: u64,
    idle: Duration,
    read: &mut R,
    write: &mut W,
    rx: &mut mpsc::Receiver<Out>,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut state = AttachState {
        parser: FrameParser::new(),
        swallow_initialized: false,
        open_docs: BTreeSet::new(),
    };
    let result = pump_loop(&entry, &mut state, read, write, rx).await;

    // Flush any partial client frame to the server (it was already split;
    // dropping the tail mid-message would desync the server worse).
    let rest = state.parser.rest();
    if !rest.is_empty() {
        let _ = entry.write_stdin(&rest);
    }
    // The documents this attach opened and didn't close — the host
    // synthesizes a `didClose` per doc (through the shared server stdin,
    // so this also covers the take-over case where a newer attach now
    // owns the stream).
    for uri in std::mem::take(&mut state.open_docs) {
        let _ = entry.write_stdin(&framed_did_close(&uri));
    }
    // `detached` is a no-op when a newer attach already owns the server;
    // when it detached us, arm the idle killer.

    if entry.detached(generation) {
        let weak = Arc::downgrade(&registry.inner);
        let entry = Arc::clone(&entry);
        let key = key.clone();
        tokio::spawn(async move {
            tokio::time::sleep(idle).await;
            let Some(inner) = weak.upgrade() else { return };
            // `detached` cleared the generation to 0 — a re-attach
            // meanwhile bumped it again.
            if entry.generation.load(Ordering::SeqCst) != 0 {
                return;
            }
            let registry = LspRegistry { inner };
            registry.evict(&key, &entry);
        });
    }
    result
}

async fn pump_loop<R, W>(
    entry: &Arc<ServerEntry>,
    state: &mut AttachState,
    read: &mut R,
    write: &mut W,
    rx: &mut mpsc::Receiver<Out>,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Out::Bytes(bytes)) => {
                    for chunk in bytes.chunks(STREAM_CHUNK) {
                        if write
                            .write_all(&encode(FrameKind::Output, chunk))
                            .await
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                }
                Some(Out::TakenOver) => {
                    let _ = write_control(
                        write,
                        &Control::Err {
                            error: err(
                                AppErrorCode::E_LSP_TAKEN,
                                "language features moved to another device — attach again to take them back",
                            ),
                        },
                    )
                    .await;
                    return Ok(());
                }
                Some(Out::Exited(code)) => {
                    let _ = write_control(write, &Control::Exit { code }).await;
                    return Ok(());
                }
                // The outbox is gone (registry dropped).
                None => return Ok(()),
            },
            frame = read_frame(read) => {
                let Ok((kind, bytes)) = frame else {
                    // The client detached; the server keeps running.
                    return Ok(());
                };
                if kind == 2 {
                    if state.parser.poisoned() {
                        if entry.write_stdin(&bytes).is_err() {
                            return Ok(());
                        }
                        continue;
                    }
                    for framed in state.parser.drain(&bytes) {
                        match inspect_client_message(entry, state, framed) {
                            ClientAction::Forward(raw) => {
                                if entry.write_stdin(&raw).is_err() {
                                    return Ok(());
                                }
                            }
                            ClientAction::AnswerFromCache(reply) => {
                                if write
                                    .write_all(&encode(FrameKind::Output, &reply))
                                    .await
                                    .is_err()
                                {
                                    return Ok(());
                                }
                            }
                            ClientAction::Swallow => {}
                        }
                    }
                }
                // kind 0 controls and stray kind-1 input are
                // meaningless on an LSP stream.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `ServerEntry` over `std::io::sink()` + a never-exiting waiter.
    fn test_entry(
        init: Option<serde_json::Value>,
        pending: Option<serde_json::Value>,
    ) -> Arc<ServerEntry> {
        struct Never;
        impl Waitable for Never {
            fn try_wait(&mut self) -> std::io::Result<Option<i64>> {
                Ok(None)
            }
            fn kill(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let entry = ServerEntry::for_test(PathBuf::from("/tmp"), "rust", std::io::sink(), Never);
        *lock(&entry.init) = init;
        *lock(&entry.pending_init) = pending;
        entry
    }

    #[test]
    fn frame_parser_splits_and_joins_messages() {
        let one = b"Content-Length: 13\r\n\r\n{\"a\":\"first\"}";
        let two = b"Content-Length: 7\r\n\r\n{\"b\":1}";
        let mut parser = FrameParser::new();
        // Split mid-body and mid-header; two messages in one chunk too.
        assert!(parser.drain(&one[..10]).is_empty());
        assert_eq!(parser.drain(&one[10..]), vec![one.to_vec()]);
        assert!(parser.drain(&two[..7]).is_empty());
        assert_eq!(parser.drain(&two[7..]), vec![two.to_vec()]);
        let mut both = Vec::new();
        both.extend_from_slice(one);
        both.extend_from_slice(two);
        let out = parser.drain(&both);
        assert_eq!(out, vec![one.to_vec(), two.to_vec()]);
        assert!(parser.rest().is_empty());
    }

    #[test]
    fn frame_parser_passes_through_after_bad_header() {
        let mut parser = FrameParser::new();
        assert!(parser.drain(b"bogus\r\n\r\n{}").is_empty());
        assert!(parser.poisoned());
        assert!(parser.drain(b"more").is_empty());
        // Everything buffered passes through on the closing flush.
        assert_eq!(parser.rest(), b"bogus\r\n\r\n{}more".to_vec());
    }

    #[test]
    fn content_length_reads_the_header() {
        assert_eq!(content_length(b"Content-Length: 42"), Some(42));
        assert_eq!(
            content_length(b"Content-Type: x\r\nContent-Length: 3"),
            Some(3)
        );
        assert_eq!(content_length(b"content-length: 7\r\n"), Some(7));
        assert_eq!(content_length(b"nope"), None);
    }

    #[test]
    fn did_open_and_close_track_documents() {
        let entry = test_entry(None, None);
        let mut state = AttachState {
            parser: FrameParser::new(),
            swallow_initialized: false,
            open_docs: BTreeSet::new(),
        };
        let open = framed_response(&serde_json::Value::Null, &serde_json::Value::Null);
        let _ = open;
        let msg = |body: &str| {
            let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
            out.extend_from_slice(body.as_bytes());
            out
        };
        match inspect_client_message(
            &entry,
            &mut state,
            msg(
                "{\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///a.rs\"}}}",
            ),
        ) {
            ClientAction::Forward(_) => {}
            _ => panic!("didOpen must forward"),
        }
        assert!(state.open_docs.contains("file:///a.rs"));
        match inspect_client_message(
            &entry,
            &mut state,
            msg(
                "{\"method\":\"textDocument/didClose\",\"params\":{\"textDocument\":{\"uri\":\"file:///a.rs\"}}}",
            ),
        ) {
            ClientAction::Forward(_) => {}
            _ => panic!("didClose must forward"),
        }
        assert!(state.open_docs.is_empty());
        entry.kill();
    }

    #[test]
    fn cached_initialize_answers_and_swallows_initialized() {
        let entry = test_entry(
            Some(serde_json::json!({"capabilities":{"hoverProvider":true}})),
            None,
        );
        let mut state = AttachState {
            parser: FrameParser::new(),
            swallow_initialized: false,
            open_docs: BTreeSet::new(),
        };
        let msg = |body: &str| {
            let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
            out.extend_from_slice(body.as_bytes());
            out
        };
        let reply = match inspect_client_message(
            &entry,
            &mut state,
            msg("{\"id\":9,\"method\":\"initialize\",\"params\":{}}"),
        ) {
            ClientAction::AnswerFromCache(reply) => reply,
            _ => panic!("cached initialize must be answered"),
        };
        let answer: serde_json::Value = serde_json::from_slice(body_of(&reply)).unwrap();
        assert_eq!(answer["id"], 9);
        assert_eq!(answer["result"]["capabilities"]["hoverProvider"], true);
        assert!(state.swallow_initialized);
        match inspect_client_message(
            &entry,
            &mut state,
            msg("{\"method\":\"initialized\",\"params\":{}}"),
        ) {
            ClientAction::Swallow => {}
            _ => panic!("initialized after a cached initialize is swallowed"),
        }
        entry.kill();
    }

    #[test]
    fn first_initialize_is_forwarded_and_its_id_tracked() {
        let entry = test_entry(None, None);
        let mut state = AttachState {
            parser: FrameParser::new(),
            swallow_initialized: false,
            open_docs: BTreeSet::new(),
        };
        let body = "{\"id\":1,\"method\":\"initialize\",\"params\":{}}";
        let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        framed.extend_from_slice(body.as_bytes());
        match inspect_client_message(&entry, &mut state, framed) {
            ClientAction::Forward(_) => {}
            _ => panic!("first initialize must reach the server"),
        }
        assert_eq!(
            lock(&entry.pending_init).clone(),
            Some(serde_json::json!(1))
        );
        // The server's reply (matched by id) fills the cache.
        cache_initialize(&entry, b"{\"id\":2,\"result\":{}}");
        assert!(lock(&entry.init).is_none(), "other ids do not cache");
        cache_initialize(&entry, b"{\"id\":1,\"result\":{\"ok\":true}}");
        assert_eq!(
            entry.initialize_result(),
            Some(serde_json::json!({"ok":true}))
        );
        entry.kill();
    }

    #[test]
    fn find_root_walks_up_to_the_marker_inside_scope() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]").unwrap();
        let scope = FsScope::home_only(temp.path());
        assert_eq!(
            find_root(&root.join("src/deep/file.rs"), &scope).as_deref(),
            Some(dunce::canonicalize(&root).unwrap().as_path())
        );
        // Outside any marker → None.
        let bare = temp.path().join("bare/sub");
        std::fs::create_dir_all(&bare).unwrap();
        assert_eq!(find_root(&bare, &scope), None);
    }

    #[test]
    fn lsp_config_parses_settings_and_defaults() {
        let config: LspConfig = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "idle_minutes": 3,
            "servers": { "rust": { "command": "/opt/ra/bin/rust-analyzer" }, "zig": "zls" }
        }))
        .unwrap();
        assert!(!config.enabled);
        assert_eq!(config.idle_minutes, 3);
        assert_eq!(
            config.spec_for("rust").map(|(_, c, _)| c),
            Some("/opt/ra/bin/rust-analyzer".to_string())
        );
        assert_eq!(
            config.spec_for("zig").map(|(_, c, _)| c),
            Some("zls".to_string())
        );
        assert_eq!(
            LspConfig::default().spec_for("go").map(|(n, ..)| n),
            Some("gopls".to_string())
        );
        assert!(LspConfig::default().spec_for("cobol").is_none());
    }
}
