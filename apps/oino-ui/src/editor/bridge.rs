//! The glue between [`LspClient`] (tokio, `Send`) and the gpui-base editor's
//! provider traits (called on the UI thread, `Rc` handles) (round5 #24).
//!
//! [`EditorLsp`] is the object installed into `EditorState::lsp_mut()`: it
//! implements `HoverProvider` and `CompletionProvider`, owns the server's
//! lifecycle (spawn, ready, failed), buffers the open document until the
//! handshake finishes, debounces `didChange` full-document sync, and relays
//! `publishDiagnostics` to the view over an event channel. Format-on-save and
//! the rustfmt fallback live here too.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use gpui_kit::component::input::{CompletionProvider, HoverProvider, Rope, RopeExt as _};
use gpui_kit::{App, AppContext as _, Task, Window};
use lsp_types::{CompletionContext, CompletionResponse, Hover, TextEdit};
use tokio::io::AsyncWriteExt as _;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::editor::lsp::{LspClient, LspError};
use crate::rt::Tokio;

/// How long typed edits settle before the buffer re-syncs to the server.
const SYNC_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(400);

/// What the status bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LspStatus {
    /// The toggle is off or no server binary exists — plain editing.
    Off(&'static str),
    /// The handshake is running.
    Starting,
    /// Initialized and answering.
    Ready,
    /// Spawn or handshake failed (message for the tooltip).
    Failed(String),
}

/// Events the view's pump task listens for (one `cx.notify` each).
#[derive(Debug)]
pub enum LspEvent {
    /// Status changed (Starting → Ready / Failed).
    Status,
    /// Diagnostics arrived for `uri`.
    Diagnostics,
    /// The server died (reader EOF); editing continues without LSP.
    Stopped,
}

/// The open document handed to the server (there is exactly one buffer this
/// round — the visible file).
struct OpenDoc {
    uri: lsp_types::Uri,
    language_id: String,
    version: i32,
    text: String,
}

enum Server {
    /// Toggle off, or not attempted yet (first Rust file starts it).
    Idle,
    Starting,
    Ready(Arc<LspClient>),
    Failed(String),
}

/// The shared editor↔LSP bridge. `Clone` is cheap (`Arc` inside) — install one
/// `Rc::new(lsp.clone())` per provider slot.
#[derive(Clone)]
pub struct EditorLsp {
    inner: Arc<Inner>,
}

struct Inner {
    server: RwLock<Server>,
    /// The document the editor shows (set on open, updated on save).
    doc: RwLock<Option<OpenDoc>>,
    /// Latest buffer text waiting for the debounce to flush.
    pending_text: Mutex<Option<String>>,
    /// A flush task is already sleeping.
    flush_scheduled: Mutex<bool>,
    /// `uri` → latest published diagnostics (the Problems list reads this).
    diagnostics: RwLock<BTreeMap<String, Vec<lsp_types::Diagnostic>>>,
    /// Bumped per publish so the view can notice a new set cheaply.
    diag_gen: AtomicU64,
    /// Wakes the view's pump on every state change.
    events: mpsc::UnboundedSender<LspEvent>,
    /// Workspace root the server is (will be) started for.
    root: PathBuf,
    /// Whether a server may be started at all (the `editor.json` toggle).
    enabled: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for EditorLsp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorLsp")
            .field("status", &self.status())
            .field("root", &self.inner.root)
            .finish_non_exhaustive()
    }
}

impl EditorLsp {
    /// A bridge for workspace `root`; `events` is the view's wake channel
    /// (paired with the [`DiagnosticsRx`]-style receiver it keeps).
    ///
    /// The caller also keeps `diagnostics_rx`: `EditorLsp` wires it to the
    /// server inside `start` — diagnostics flow file → view without polling.
    #[must_use]
    pub fn new(root: &Path, enabled: bool, events: mpsc::UnboundedSender<LspEvent>) -> Self {
        Self {
            inner: Arc::new(Inner {
                server: RwLock::new(Server::Idle),
                doc: RwLock::new(None),
                pending_text: Mutex::new(None),
                flush_scheduled: Mutex::new(false),
                diagnostics: RwLock::new(BTreeMap::new()),
                diag_gen: AtomicU64::new(0),
                events,
                root: root.to_path_buf(),
                enabled: std::sync::atomic::AtomicBool::new(enabled),
            }),
        }
    }

    /// The current [`LspStatus`] for the status bar.
    #[must_use]
    pub fn status(&self) -> LspStatus {
        if !self.inner.enabled.load(Ordering::Relaxed) {
            return LspStatus::Off("off");
        }
        match &*self
            .inner
            .server
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Server::Idle => {
                if super::lsp::find_server().is_some() {
                    LspStatus::Off("starts with a .rs file")
                } else {
                    LspStatus::Off("no rust-analyzer")
                }
            }
            Server::Starting => LspStatus::Starting,
            Server::Ready(client) => {
                if client.alive() {
                    LspStatus::Ready
                } else {
                    LspStatus::Failed("server stopped".to_owned())
                }
            }
            Server::Failed(err) => LspStatus::Failed(err.clone()),
        }
    }

    /// Flips the toggle: on → drop the server, off → start it on demand.
    pub fn set_enabled(&self, enabled: bool) {
        self.inner.enabled.store(enabled, Ordering::Relaxed);
        if !enabled && let Some(client) = self.take_client() {
            client.shutdown();
        }
        let _ = self.inner.events.send(LspEvent::Status);
    }

    fn take_client(&self) -> Option<Arc<LspClient>> {
        let mut guard = self
            .inner
            .server
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match std::mem::replace(&mut *guard, Server::Idle) {
            Server::Ready(client) => Some(client),
            other => {
                *guard = other;
                None
            }
        }
    }

    fn client(&self) -> Option<Arc<LspClient>> {
        match &*self
            .inner
            .server
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            Server::Ready(client) => Some(Arc::clone(client)),
            _ => None,
        }
    }

    /// Starts `rust-analyzer` if the toggle allows it and `path` is Rust.
    /// Safe to call on every file open — only the first call spawns.
    pub fn ensure_started(&self, path: &Path, cx: &App) {
        if !self.inner.enabled.load(Ordering::Relaxed)
            || path.extension().and_then(|e| e.to_str()) != Some("rs")
        {
            return;
        }
        {
            let mut guard = self
                .inner
                .server
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match &*guard {
                Server::Idle => *guard = Server::Starting,
                // A dead server gets one retry on the next open.
                Server::Ready(c) if !c.alive() => *guard = Server::Starting,
                _ => return,
            }
        }
        let _ = self.inner.events.send(LspEvent::Status);

        let inner = Arc::clone(&self.inner);
        let root = self.inner.root.clone();
        Tokio::detach(cx, async move {
            // Diagnostics published by the server are parked in `diagnostics`
            // and wake the view through `events`.
            let (diag_tx, mut diag_rx) =
                mpsc::unbounded_channel::<lsp_types::PublishDiagnosticsParams>();
            let diag_inner = Arc::clone(&inner);
            let diag_pump = tokio::spawn(async move {
                while let Some(params) = diag_rx.recv().await {
                    if let Ok(mut map) = diag_inner.diagnostics.write() {
                        map.insert(params.uri.as_str().to_owned(), params.diagnostics);
                    }
                    diag_inner.diag_gen.fetch_add(1, Ordering::Relaxed);
                    let _ = diag_inner.events.send(LspEvent::Diagnostics);
                }
            });

            match LspClient::start(&root, diag_tx).await {
                Ok((client, _binary)) if !inner.enabled.load(Ordering::Relaxed) => {
                    // Toggled off while the handshake ran — don't leave it running.
                    client.shutdown();
                }
                Ok((client, _binary)) => {
                    // The doc opened while the handshake ran — catch the server up.
                    let doc = inner
                        .doc
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take();
                    if let Some(doc) = doc {
                        client.did_open(&doc.uri, &doc.language_id, &doc.text, doc.version);
                        let text = inner
                            .pending_text
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .take();
                        if let Some(text) = text {
                            client.did_change(&doc.uri, &text, doc.version + 1);
                        }
                        *inner
                            .doc
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(doc);
                    }
                    *inner
                        .server
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Server::Ready(client);
                }
                Err(err) => {
                    *inner
                        .server
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Server::Failed(err.to_string());
                }
            }
            drop(diag_pump);
            let _ = inner.events.send(LspEvent::Status);
        });
    }

    /// `didOpen` (or a stash for the still-initializing server).
    pub fn open_document(&self, path: &Path, language_id: &str, text: &str) {
        let Some(uri) = super::lsp::file_uri(path) else {
            return;
        };
        let doc = OpenDoc {
            uri,
            language_id: language_id.to_owned(),
            version: 1,
            text: text.to_owned(),
        };
        if let Some(client) = self.client() {
            client.did_open(&doc.uri, &doc.language_id, &doc.text, doc.version);
        }
        *self
            .inner
            .doc
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(doc);
    }

    /// `didClose` for the replaced document, if the server knows it.
    pub fn close_document(&self, path: &Path) {
        let Some(uri) = super::lsp::file_uri(path) else {
            return;
        };
        let mut guard = self
            .inner
            .doc
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref().is_some_and(|d| d.uri == uri) {
            *guard = None;
            if let Some(client) = self.client() {
                client.did_close(&uri);
            }
        }
        if let Ok(mut map) = self.inner.diagnostics.write() {
            map.remove(uri.as_str());
        }
    }

    /// Buffers the newest buffer text; a debounced flush sends `didChange`.
    pub fn document_changed(&self, text: String) {
        *self
            .inner
            .pending_text
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(text);
    }

    /// Runs the debounce pump; the view calls it once per Change event, it is a
    /// no-op while a flush is already scheduled.
    pub fn schedule_sync(&self, cx: &App) {
        {
            let mut scheduled = self
                .inner
                .flush_scheduled
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if *scheduled {
                return;
            }
            *scheduled = true;
        }
        let inner = Arc::clone(&self.inner);
        Tokio::detach(cx, async move {
            tokio::time::sleep(SYNC_DEBOUNCE).await;
            *inner
                .flush_scheduled
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
            let text = inner
                .pending_text
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let (Some(text), Some(client)) = (text, client_of(&inner)) else {
                return;
            };
            let mut guard = inner
                .doc
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(doc) = guard.as_mut() {
                doc.version += 1;
                client.did_change(&doc.uri, &text, doc.version);
                doc.text = text;
            }
        });
    }

    /// Flushes pending text now (before a format request, so the server
    /// formats the buffer the user sees — frames are ordered on the wire).
    fn flush_now(&self) {
        let text = self
            .inner
            .pending_text
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let (Some(text), Some(client)) = (text, self.client()) else {
            return;
        };
        let mut guard = self
            .inner
            .doc
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(doc) = guard.as_mut() {
            doc.version += 1;
            client.did_change(&doc.uri, &text, doc.version);
            doc.text = text;
        }
    }

    /// `textDocument/formatting` for `path` after flushing edits. `Ok(vec![])`
    /// also covers "no server" so the caller can fall back to `rustfmt`.
    pub fn format(&self, path: &Path, cx: &App) -> Task<anyhow::Result<Vec<TextEdit>>> {
        self.flush_now();
        let Some(client) = self.client() else {
            return Task::ready(Ok(Vec::new()));
        };
        let Some(uri) = super::lsp::file_uri(path) else {
            return Task::ready(Ok(Vec::new()));
        };
        spawn_lsp(cx, async move {
            client.formatting(&uri).await.map_err(anyhow::Error::from)
        })
    }

    /// `didSave` for `uri` when the buffer just hit disk (triggers flycheck).
    pub fn did_save(&self, uri: &lsp_types::Uri) {
        if let Some(client) = self.client() {
            client.did_save(uri);
        }
    }

    /// The latest diagnostics for `path` (the Problems list and squiggles).
    #[must_use]
    pub fn diagnostics(&self, path: &Path) -> Vec<lsp_types::Diagnostic> {
        let Some(uri) = super::lsp::file_uri(path) else {
            return Vec::new();
        };
        self.inner
            .diagnostics
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(uri.as_str())
            .cloned()
            .unwrap_or_default()
    }

    /// Generation counter of the diagnostics store (cheap render check).
    #[must_use]
    pub fn diagnostics_gen(&self) -> u64 {
        self.inner.diag_gen.load(Ordering::Relaxed)
    }
}

fn client_of(inner: &Inner) -> Option<Arc<LspClient>> {
    match &*inner
        .server
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        Server::Ready(client) => Some(Arc::clone(client)),
        _ => None,
    }
}

/// Runs `fut` on the tokio runtime and presents it as a GPUI `Task` (the shape
/// the provider traits return).
fn spawn_lsp<F, R>(cx: &App, fut: F) -> Task<anyhow::Result<R>>
where
    F: Future<Output = anyhow::Result<R>> + Send + 'static,
    R: Send + 'static,
{
    let join = Tokio::handle(cx).spawn(fut);
    cx.background_spawn(async move {
        match join.await {
            Ok(result) => result,
            Err(join_err) => Err(anyhow::anyhow!("lsp task aborted: {join_err}")),
        }
    })
}

impl HoverProvider for EditorLsp {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let (Some(client), Some(uri)) = (self.client(), self.current_uri()) else {
            return Task::ready(Ok(None));
        };
        let position = text.offset_to_position(offset);
        spawn_lsp(cx, async move {
            client
                .hover(&uri, position)
                .await
                .map_err(anyhow::Error::from)
        })
    }
}

impl EditorLsp {
    fn current_uri(&self) -> Option<lsp_types::Uri> {
        self.inner
            .doc
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|d| d.uri.clone())
    }
}

impl CompletionProvider for EditorLsp {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        let (Some(client), Some(uri)) = (self.client(), self.current_uri()) else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };
        let position = text.offset_to_position(offset);
        let trigger = if trigger.trigger_character.is_some() {
            Some(trigger)
        } else {
            None
        };
        spawn_lsp(cx, async move {
            client
                .completion(&uri, position, trigger.as_ref())
                .await
                .map_err(anyhow::Error::from)
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        // Word characters and Rust's trigger set; whitespace/never means the
        // menu does not flash on every space.
        new_text
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | ':' | '_'))
    }
}

/// `rustfmt` fallback for save-time formatting when no server answers
/// (LSP off, no binary, non-Rust languages with a formatter on PATH are a
/// later round's problem — `rustfmt` only here).
#[must_use]
pub fn rustfmt_available() -> bool {
    which_on_path("rustfmt") || {
        std::env::var_os("HOME").is_some_and(|h| Path::new(&h).join(".cargo/bin/rustfmt").is_file())
    }
}

fn which_on_path(name: &str) -> bool {
    std::env::var("PATH")
        .is_ok_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// Runs `rustfmt` on `text` (stdin → stdout), returning the formatted buffer.
/// `Err` when `rustfmt` rejects the input (a syntax error must never block a
/// save — the caller keeps the raw buffer then).
///
/// # Errors
/// [`LspError::NotFound`] without a binary, [`LspError::Io`] on spawn,
/// [`LspError::Other`] when `rustfmt` rejects the input or emits non-UTF-8.
pub async fn rustfmt(text: String) -> Result<String, LspError> {
    let bin = if which_on_path("rustfmt") {
        PathBuf::from("rustfmt")
    } else if let Some(home) = std::env::var_os("HOME") {
        Path::new(&home).join(".cargo/bin/rustfmt")
    } else {
        return Err(LspError::NotFound);
    };
    let mut child = Command::new(bin)
        .args(["--emit", "stdout", "--edition", "2024"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "no rustfmt stdin"))?;
    let write = tokio::spawn(async move { stdin.write_all(text.as_bytes()).await });
    let out = child.wait_with_output().await?;
    let _ = write.await;
    if !out.status.success() {
        return Err(LspError::Other(format!("rustfmt exited {}", out.status)));
    }
    String::from_utf8(out.stdout)
        .map_err(|e| LspError::Other(format!("rustfmt output is not UTF-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_without_server_is_off() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let lsp = EditorLsp::new(Path::new("/tmp"), true, tx);
        // No server started — Idle reports a hint, not Starting.
        assert!(matches!(lsp.status(), LspStatus::Off(_)));
    }

    #[test]
    fn disabled_stays_off() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let lsp = EditorLsp::new(Path::new("/tmp"), false, tx);
        assert_eq!(lsp.status(), LspStatus::Off("off"));
    }
}
