//! Host-owned PTY terminals (host.md §9.3, WP-H6): `term_open`,
//! `term_attach`, `term_list`, `term_kill`.
//!
//! The PTY lives in the host, keyed by a [`TermId`] and scoped to a
//! workspace (the cwd it was opened in — the session/project folder), so a
//! terminal survives panel/window close, works from the phone and dies with
//! the host. Multiple tabs are multiple ids.
//!
//! - **Attach/detach**: `term_open` / `term_attach` answer
//!   `ok{data:TermAttachData}` and then the bi-stream carries output
//!   (kind-1 frames), input (kind-2 frames), `Control::Resize`,
//!   `Control::Title` and `Control::Exit{code}`. Dropping the stream
//!   *detaches*: the PTY keeps running and buffering nothing — a later
//!   `term_attach` sees new output only (no scrollback replay; the app keeps
//!   its own xterm buffer, host.md §9.3, risk R7).
//! - **Idle cap**: idle terminals are kept until host quit; `term.max`
//!   (default [`DEFAULT_MAX_TERMS`]) evicts the oldest *idle* one first (a
//!   terminal with no attached stream), the oldest overall if every terminal
//!   is attached.
//! - **Resize throttling**: at most one resize per PTY per
//!   [`RESIZE_INTERVAL`] (last-wins). Apps may throttle too.
//! - Agent bash output stays in the chat's tool rows and is never mirrored
//!   into a terminal (DIRECTION §4.3); tmux/herdr run *inside* a terminal if
//!   the user types them — the host does not model them.
//!
//! The registry is one per host process ([`TermRegistry::shared`]); tests
//! build their own with [`TermRegistry::with_max`].

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, OnceLock,
    },
    time::{Duration, Instant},
};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use unipi_proto::{
    AppErrorCode, AppErrorDto, Control, Request, TermAttachData, TermForeground, TermId, TermInfo,
    TermShell, TERM_FOREGROUND_NAME_MAX,
};

use crate::{
    data_dir::HostPaths,
    protocol::{read_frame, write_control},
};

/// Terminals the host keeps alive before evicting one (`term.max` in
/// `config.json`).
pub const DEFAULT_MAX_TERMS: usize = 16;
/// Hard ceiling for `term.max`; above this the setting is clamped.
pub const MAX_TERMS_LIMIT: usize = 128;
/// The host applies at most one resize per PTY in this window (last-wins).
pub const RESIZE_INTERVAL: Duration = Duration::from_millis(100);
/// Buffered output messages per attached stream; a slower stream is dropped
/// (it can re-attach) rather than letting the host's memory grow.
const STREAM_BACKLOG: usize = 256;
/// `TERM` every terminal runs with.
const TERM: &str = "xterm-256color";

fn err(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    err(AppErrorCode::E_INTERNAL, e.to_string())
}

fn not_found(id: &str) -> AppErrorDto {
    err(
        AppErrorCode::E_NOT_FOUND,
        format!("terminal {id} is not open on the host"),
    )
}

/// Locks a registry/PTY mutex, recovering from a poisoned one: a panic in one
/// terminal's pump must not take the other terminals down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The `term` block of `<host dir>/config.json` (host.md §9.9):
/// `{ "term": { "max": 16 } }`, read fresh on every `term_open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct TermConfig {
    /// Terminals kept alive before the oldest idle one is evicted.
    #[serde(default = "default_max_terms")]
    pub max: usize,
}

fn default_max_terms() -> usize {
    DEFAULT_MAX_TERMS
}

impl Default for TermConfig {
    fn default() -> Self {
        TermConfig {
            max: DEFAULT_MAX_TERMS,
        }
    }
}

impl TermConfig {
    /// Reads the block; a missing/corrupt file is the default (16). `max` is
    /// clamped into `1..=MAX_TERMS_LIMIT`.
    pub fn load(config_path: &Path) -> TermConfig {
        let raw = crate::data_dir::read_json::<serde_json::Value>(config_path)
            .ok()
            .flatten();
        let mut config: TermConfig = raw
            .as_ref()
            .and_then(|value| value.get("term").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        config.max = config.max.clamp(1, MAX_TERMS_LIMIT);
        config
    }
}

/// One output event fanned out to the streams attached to a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermOutput {
    /// Bytes read from the PTY (sent as a kind-1 frame).
    Bytes(Vec<u8>),
    /// The child exited; the stream ends after this.
    Exit(Option<i64>),
}

/// Per-PTY resize throttle state: at most one resize per
/// [`RESIZE_INTERVAL`], last request wins.
#[derive(Debug, Default)]
pub struct ResizeThrottle {
    last: Option<Instant>,
    pending: Option<(u16, u16)>,
}

impl ResizeThrottle {
    /// Records a request. Returns the size to apply *now* when the window is
    /// open (and marks the window); otherwise the request is queued
    /// (last-wins) and [`poll`](Self::poll) releases it when it is due.
    pub fn offer(&mut self, cols: u16, rows: u16, now: Instant) -> Option<(u16, u16)> {
        self.pending = Some((cols, rows));
        let due = match self.last {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= RESIZE_INTERVAL,
        };
        if due {
            self.poll(now)
        } else {
            None
        }
    }

    /// Applies (and clears) the queued resize when the window has elapsed.
    pub fn poll(&mut self, now: Instant) -> Option<(u16, u16)> {
        let size = self.pending?;
        let due = match self.last {
            None => true,
            Some(last) => now.saturating_duration_since(last) >= RESIZE_INTERVAL,
        };
        if !due {
            return None;
        }
        self.pending = None;
        self.last = Some(now);
        Some(size)
    }

    /// How long until a queued resize becomes due (`None`: nothing queued).
    pub fn wait(&self, now: Instant) -> Option<Duration> {
        self.pending?;
        Some(match self.last {
            None => Duration::ZERO,
            Some(last) => RESIZE_INTERVAL.saturating_sub(now.saturating_duration_since(last)),
        })
    }
}

/// One host PTY plus everything attached to it.
struct TermEntry {
    id: String,
    term_id: TermId,
    cwd: PathBuf,
    shell: TermShell,
    created: u64,
    /// The shell's pid (session leader ⇒ pid == pgid), for the foreground
    /// check in [`foreground`](TermEntry::foreground).
    child_pid: Option<u32>,
    last_activity: AtomicU64,
    alive: AtomicBool,
    /// Set once by the waiter/pump threads (or by `kill`).
    exit: Mutex<Option<Option<i64>>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    throttle: Mutex<ResizeThrottle>,
    /// One flusher task at a time applies the throttled trailing resize.
    flushing: AtomicBool,
    /// Applied (not coalesced) resizes; diagnostics and the throttle test.
    resizes: AtomicU64,
    subscribers: Mutex<Vec<(u64, mpsc::Sender<TermOutput>)>>,
    next_subscriber: AtomicU64,
}

impl TermEntry {
    fn touch(&self) {
        self.last_activity
            .store(crate::pairing::now_unix(), Ordering::Relaxed);
    }

    fn applied_resizes(&self) -> u64 {
        self.resizes.load(Ordering::Relaxed)
    }

    fn exit_code(&self) -> Option<i64> {
        lock(&self.exit).flatten()
    }

    /// Has the child been reaped (or the terminal killed)?
    fn exited(&self) -> bool {
        lock(&self.exit).is_some()
    }

    /// The foreground process while it isn't the shell itself: the PTY's
    /// foreground process group vs the shell's own (session-leader) group.
    /// Best effort per OS; `None` = shell/idle (or cannot tell).
    #[cfg(unix)]
    fn foreground(&self) -> Option<TermForeground> {
        if !self.alive.load(Ordering::SeqCst) {
            return None;
        }
        let shell_pid = self.child_pid?;
        let fd = lock(&self.master).as_raw_fd()?;
        // SAFETY: tcgetpgrp only queries the tty's foreground process group.
        let group = unsafe { libc::tcgetpgrp(fd) };
        if group <= 0 {
            return None;
        }
        let group = group as u32;
        if group == shell_pid {
            return None; // the shell itself is in the foreground
        }
        Some(TermForeground {
            name: foreground_name(group),
        })
    }

    /// Windows (ConPTY) has no foreground process group to compare: the
    /// host reports "unknown" rather than guessing.
    #[cfg(windows)]
    fn foreground(&self) -> Option<TermForeground> {
        None
    }

    fn set_exit(&self, code: Option<i64>) {
        let mut exit = lock(&self.exit);
        if exit.is_none() {
            *exit = Some(code);
        }
        self.alive.store(false, Ordering::SeqCst);
    }

    fn attached(&self) -> usize {
        lock(&self.subscribers).len()
    }

    /// Applies one size to the real PTY and counts it.
    fn apply_size(&self, (cols, rows): (u16, u16)) {
        let size = PtySize {
            cols: cols.max(1),
            rows: rows.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        match lock(&self.master).resize(size) {
            Ok(()) => {
                self.resizes.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => tracing::warn!(term = %self.id, %error, "PTY resize failed"),
        }
    }

    /// Writes bytes into the PTY (window 0: input).
    fn write_input(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut writer = lock(&self.writer);
        writer.write_all(bytes)?;
        writer.flush()?;
        self.touch();
        Ok(())
    }

    fn subscribe(self: &Arc<Self>) -> Attached {
        let (tx, rx) = mpsc::channel(STREAM_BACKLOG);
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        lock(&self.subscribers).push((id, tx));
        // A dead terminal answers with its exit code right away.
        if !self.alive.load(Ordering::SeqCst) {
            let _ = self.send_to(id, TermOutput::Exit(self.exit_code()));
        }
        Attached {
            entry: Arc::clone(self),
            id,
            rx,
        }
    }

    fn send_to(&self, id: u64, output: TermOutput) -> bool {
        let mut subscribers = lock(&self.subscribers);
        match subscribers.iter().find(|(sub, _)| *sub == id) {
            Some((_, tx)) => match tx.try_send(output) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(term = %self.id, "terminal stream is too slow; detaching it");
                    subscribers.retain(|(sub, _)| *sub != id);
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    subscribers.retain(|(sub, _)| *sub != id);
                    false
                }
            },
            None => false,
        }
    }

    fn unsubscribe(&self, id: u64) {
        lock(&self.subscribers).retain(|(sub, _)| *sub != id);
    }

    /// Fans one output event out to every attached stream.
    fn fanout(&self, output: TermOutput) {
        let mut subscribers = lock(&self.subscribers);
        subscribers.retain(|(_, tx)| match tx.try_send(output.clone()) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(term = %self.id, "terminal stream is too slow; detaching it");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        });
    }
}

/// One live attach of a client stream to a terminal. Dropping it detaches
/// (the PTY keeps running).
pub struct Attached {
    entry: Arc<TermEntry>,
    id: u64,
    rx: mpsc::Receiver<TermOutput>,
}

impl Attached {
    /// The terminal this stream is attached to.
    pub fn term_id(&self) -> &str {
        &self.entry.id
    }

    /// The next output event (`None`: the stream was dropped as too slow).
    pub async fn recv(&mut self) -> Option<TermOutput> {
        self.rx.recv().await
    }

    /// The terminal's exit code once its child has exited.
    pub fn exit_code(&self) -> Option<i64> {
        self.entry.exit_code()
    }

    /// Writes client input into the PTY (ignored once the child is gone —
    /// the stream ends with its `Exit` anyway).
    pub fn input(&self, bytes: &[u8]) -> Result<(), AppErrorDto> {
        if !self.entry.alive.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.entry.write_input(bytes).map_err(internal)
    }

    /// Requests a resize; the registry applies it under the throttle.
    pub fn resize(&self, cols: u16, rows: u16) {
        if !self.entry.alive.load(Ordering::SeqCst) {
            return;
        }
        self.entry.apply_throttled_resize(cols, rows);
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        self.entry.unsubscribe(self.id);
    }
}

impl TermEntry {
    /// Offers a resize to the throttle and schedules the trailing apply.
    fn apply_throttled_resize(self: &Arc<Self>, cols: u16, rows: u16) {
        let now = Instant::now();
        let immediate = lock(&self.throttle).offer(cols, rows, now);
        if let Some(size) = immediate {
            self.apply_size(size);
        }
        self.ensure_flusher();
    }

    /// Spawns (once) the task that applies a queued trailing resize when the
    /// throttle window elapses. Outside a runtime (unit tests of the pure
    /// throttle, `kill_all`) the queue simply stays queued.
    fn ensure_flusher(self: &Arc<Self>) {
        if self.flushing.swap(true, Ordering::SeqCst) {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            self.flushing.store(false, Ordering::SeqCst);
            return;
        }
        let entry = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let wait = {
                    let throttle = lock(&entry.throttle);
                    match throttle.wait(Instant::now()) {
                        Some(wait) => wait,
                        None => break,
                    }
                };
                tokio::time::sleep(wait).await;
                let due = lock(&entry.throttle).poll(Instant::now());
                if let Some(size) = due {
                    entry.apply_size(size);
                }
            }
            entry.flushing.store(false, Ordering::SeqCst);
        });
    }
}

/// What `term_open` asks the host for.
#[derive(Debug, Clone)]
pub struct OpenRequest {
    /// The workspace to open the shell in (an existing directory).
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// Program to run instead of `$SHELL`; `None` = `$SHELL -l`.
    pub shell: Option<String>,
}

/// The host's terminals: one registry per host process
/// ([`shared`](TermRegistry::shared)), keyed by [`TermId`].
#[derive(Clone, Default)]
pub struct TermRegistry {
    inner: Arc<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    terms: Mutex<BTreeMap<String, Arc<TermEntry>>>,
    created: AtomicU64,
    /// Test override; `None` reads `term.max` from `config.json`.
    max: Option<usize>,
}

impl TermRegistry {
    /// An empty registry that reads `term.max` from `config.json`.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty registry with an explicit cap (tests).
    pub fn with_max(max: usize) -> Self {
        TermRegistry {
            inner: Arc::new(RegistryInner {
                max: Some(max.clamp(1, MAX_TERMS_LIMIT)),
                ..RegistryInner::default()
            }),
        }
    }

    /// The process-wide registry the protocol handler uses (one host per
    /// data dir ⇒ one registry).
    pub fn shared() -> &'static TermRegistry {
        static SHARED: OnceLock<TermRegistry> = OnceLock::new();
        SHARED.get_or_init(TermRegistry::new)
    }

    fn max_terms(&self, paths: &HostPaths) -> usize {
        match self.inner.max {
            Some(max) => max,
            None => TermConfig::load(&paths.config()).max,
        }
    }

    fn get(&self, term_id: &str) -> Result<Arc<TermEntry>, AppErrorDto> {
        lock(&self.inner.terms)
            .get(term_id)
            .cloned()
            .ok_or_else(|| not_found(term_id))
    }

    /// Opens a new PTY in `request.cwd` and returns its id plus an already
    /// attached stream, so no startup output is lost.
    pub fn open(
        &self,
        paths: &HostPaths,
        request: OpenRequest,
    ) -> Result<(TermAttachData, Attached), AppErrorDto> {
        let cwd = request.cwd;
        if !cwd.is_dir() {
            return Err(err(
                AppErrorCode::E_NOT_FOUND,
                format!("workspace {} is not a directory", cwd.display()),
            ));
        }
        self.evict_for(self.max_terms(paths));
        let shell = resolve_shell(request.shell.as_deref());
        let entry = spawn_pty(&self.inner, cwd, shell, request.cols, request.rows)?;
        let data = TermAttachData {
            term_id: entry.term_id.clone(),
            shell: entry.shell.clone(),
        };
        let attached = entry.subscribe();
        lock(&self.inner.terms).insert(entry.id.clone(), Arc::clone(&entry));
        tracing::info!(term = %entry.id, shell = %entry.shell.as_str(), "terminal opened");
        Ok((data, attached))
    }

    /// Attaches a stream to an existing terminal (resizing it through the
    /// throttle first). New output only — the host keeps no scrollback.
    pub fn attach(&self, term_id: &str, cols: u16, rows: u16) -> Result<Attached, AppErrorDto> {
        let entry = self.get(term_id)?;
        if entry.alive.load(Ordering::SeqCst) {
            entry.apply_throttled_resize(cols, rows);
        }
        Ok(entry.subscribe())
    }

    /// Every terminal, oldest first (the order `term.max` evicts in).
    pub fn list(&self) -> Vec<TermInfo> {
        let mut entries: Vec<Arc<TermEntry>> = lock(&self.inner.terms).values().cloned().collect();
        entries.sort_by_key(|entry| entry.created);
        entries
            .into_iter()
            .map(|entry| TermInfo {
                term_id: entry.term_id.clone(),
                cwd: entry.cwd.to_string_lossy().into_owned(),
                shell: entry.shell.clone(),
                alive: entry.alive.load(Ordering::SeqCst),
                last_activity: entry.last_activity.load(Ordering::Relaxed),
                foreground: entry.foreground(),
            })
            .collect()
    }

    /// Closes a terminal: kills its child, ends its streams and forgets the
    /// id. Idempotent (`false` when the id was already gone).
    pub fn kill(&self, term_id: &str) -> Result<bool, AppErrorDto> {
        let entry = lock(&self.inner.terms).remove(term_id);
        match entry {
            Some(entry) => {
                entry.kill();
                tracing::info!(term = %entry.id, "terminal killed");
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Kills every terminal (host shutdown).
    pub fn kill_all(&self) {
        let entries: Vec<Arc<TermEntry>> = std::mem::take(&mut *lock(&self.inner.terms))
            .into_values()
            .collect();
        for entry in entries {
            entry.kill();
        }
    }

    /// Number of attached streams (diagnostics and tests).
    pub fn attached_streams(&self, term_id: &str) -> usize {
        lock(&self.inner.terms)
            .get(term_id)
            .map_or(0, |entry| entry.attached())
    }

    /// Resizes applied to a terminal's PTY (the resize throttle's counter:
    /// a burst inside one [`RESIZE_INTERVAL`] bumps this at most twice).
    pub fn applied_resizes(&self, term_id: &str) -> Option<u64> {
        lock(&self.inner.terms)
            .get(term_id)
            .map(|entry| entry.applied_resizes())
    }

    /// Makes room for one new terminal: evicts the oldest *idle* one (no
    /// attached stream), the oldest overall when every terminal is attached.
    fn evict_for(&self, max: usize) {
        let mut terms = lock(&self.inner.terms);
        while terms.len() >= max {
            let victim = terms
                .values()
                .filter(|entry| entry.attached() == 0)
                .min_by_key(|entry| entry.created)
                .or_else(|| terms.values().min_by_key(|entry| entry.created))
                .map(|entry| entry.id.clone());
            let Some(id) = victim else {
                return;
            };
            if let Some(entry) = terms.remove(&id) {
                tracing::info!(term = %id, max, "evicting terminal (term.max reached)");
                entry.kill();
            }
        }
    }
}

impl TermEntry {
    /// Kills the child and ends every attached stream.
    fn kill(&self) {
        if let Err(error) = lock(&self.killer).kill() {
            tracing::warn!(term = %self.id, %error, "PTY kill failed");
        }
        self.set_exit(None);
        self.fanout(TermOutput::Exit(None));
    }
}

/// Trims and clamps a best-effort process name to the wire limit (never
/// fails; whitespace-only reads as unknown).
fn clamp_name(name: String) -> Option<String> {
    let clamped: String = name.trim().chars().take(TERM_FOREGROUND_NAME_MAX).collect();
    (!clamped.is_empty()).then_some(clamped)
}

/// The name of a foreground process group's leader (best effort).
///
/// Linux: `/proc/<pgid>/comm` — the group leader is the job the shell put
/// in the foreground (e.g. `vim`, or a pipeline's first process).
#[cfg(target_os = "linux")]
fn foreground_name(group: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{group}/comm")).ok()?;
    clamp_name(comm)
}

/// macOS: no `/proc`; a tiny `ps` per reported terminal (only terminals
/// with a non-shell foreground) keeps the lookup dependency-free.
#[cfg(target_os = "macos")]
fn foreground_name(group: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &group.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    clamp_name(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Other unixes: report the foreground group without a name.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn foreground_name(_group: u32) -> Option<String> {
    None
}

/// `$SHELL`, else the platform default; clamped to what the wire accepts.
fn resolve_shell(requested: Option<&str>) -> TermShell {
    let raw = requested
        .filter(|shell| !shell.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("SHELL")
                .ok()
                .filter(|shell| !shell.trim().is_empty())
        })
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "powershell.exe".to_string()
            } else {
                "/bin/sh".to_string()
            }
        });
    let clamped: String = raw.chars().take(unipi_proto::TERM_SHELL_MAX).collect();
    TermShell::try_from(clamped).unwrap_or_default()
}

fn new_term_id(seq: u64) -> TermId {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    let mixed = nanos
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(seq.wrapping_mul(0x85eb_ca6b));
    let id = format!("{mixed:016x}");
    // Hex of a fixed width is always a valid id; the fallback is a constant.
    TermId::try_from(id).unwrap_or_else(|_| TermId::try_from(String::from("0")).unwrap_or_default())
}

/// Builds the `$SHELL -l` command for a workspace (host.md §9.3).
fn pty_command(shell: &str, cwd: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(shell);
    if cfg!(unix) {
        command.arg("-l");
    }
    command.cwd(cwd);
    command.env("TERM", TERM);
    for (key, _) in std::env::vars() {
        // A host started from inside a multiplexer must not leak its socket
        // into the user's own tmux/herdr inside the terminal.
        if key.starts_with("TMUX") || key.starts_with("HERDR_") {
            command.env_remove(key);
        }
    }
    command
}

/// Opens the PTY, spawns the child and starts the pump/waiter threads.
fn spawn_pty(
    inner: &RegistryInner,
    cwd: PathBuf,
    shell: TermShell,
    cols: u16,
    rows: u16,
) -> Result<Arc<TermEntry>, AppErrorDto> {
    let pair = native_pty_system()
        .openpty(PtySize {
            cols: cols.max(1),
            rows: rows.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(internal)?;
    let command = pty_command(shell.as_str(), &cwd);
    let child = pair.slave.spawn_command(command).map_err(|error| {
        err(
            AppErrorCode::E_INTERNAL,
            format!(
                "could not start {} in {}: {error}",
                shell.as_str(),
                cwd.display()
            ),
        )
    })?;
    let child_pid = child.process_id();
    drop(pair.slave);
    let reader = pair.master.try_clone_reader().map_err(internal)?;
    let writer = pair.master.take_writer().map_err(internal)?;
    let killer = child.clone_killer();
    let created = inner.created.fetch_add(1, Ordering::Relaxed);
    let term_id = new_term_id(created);
    let entry = Arc::new(TermEntry {
        id: term_id.as_str().to_owned(),
        term_id,
        cwd,
        shell,
        created,
        child_pid,
        last_activity: AtomicU64::new(crate::pairing::now_unix()),
        alive: AtomicBool::new(true),
        exit: Mutex::new(None),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        killer: Mutex::new(killer),
        throttle: Mutex::new(ResizeThrottle::default()),
        flushing: AtomicBool::new(false),
        resizes: AtomicU64::new(0),
        subscribers: Mutex::new(Vec::new()),
        next_subscriber: AtomicU64::new(0),
    });
    spawn_waiter(Arc::clone(&entry), child);
    spawn_pump(Arc::clone(&entry), reader);
    Ok(entry)
}

/// Waits for the child and records its exit code (and `alive = false`).
fn spawn_waiter(entry: Arc<TermEntry>, mut child: Box<dyn Child + Send + Sync>) {
    let name = format!("term-wait-{}", entry.id);
    let for_error = Arc::clone(&entry);
    let spawned = std::thread::Builder::new().name(name).spawn(move || {
        let code = match child.wait() {
            Ok(status) => Some(i64::from(status.exit_code())),
            Err(error) => {
                tracing::warn!(term = %entry.id, %error, "PTY wait failed");
                None
            }
        };
        entry.set_exit(code);
    });
    if let Err(error) = spawned {
        tracing::warn!(term = %for_error.id, %error, "could not start the PTY waiter");
    }
}

/// Reads the PTY and fans output out to the attached streams; ends them with
/// `Exit{code}` when the child is gone.
fn spawn_pump(entry: Arc<TermEntry>, mut reader: Box<dyn Read + Send>) {
    let name = format!("term-pump-{}", entry.id);
    let for_error = Arc::clone(&entry);
    let spawned = std::thread::Builder::new().name(name).spawn(move || {
        let mut buffer = [0u8; 8192];
        // Windows ConPTY opens with a cursor-position query and renders
        // nothing until it is answered; the host answers it (top-left)
        // and keeps it off the wire (same pump as the 1.x host).
        let mut awaiting_dsr = cfg!(windows);
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let mut bytes = buffer[..read].to_vec();
                    if awaiting_dsr {
                        awaiting_dsr = false;
                        if let Some(at) = bytes.windows(4).position(|window| window == b"\x1b[6n") {
                            bytes.drain(at..at + 4);
                            let _ = entry.write_input(b"\x1b[1;1R");
                        }
                        if bytes.is_empty() {
                            continue;
                        }
                    }
                    entry.touch();
                    entry.fanout(TermOutput::Bytes(bytes));
                }
            }
        }
        // Let the waiter record the code before the last event goes out.
        let mut waited = Duration::ZERO;
        while !entry.exited() && waited < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
            waited += Duration::from_millis(10);
        }
        entry.fanout(TermOutput::Exit(entry.exit_code()));
    });
    if let Err(error) = spawned {
        tracing::warn!(term = %for_error.id, %error, "could not start the PTY pump");
    }
}

/// Serves one `term_*` request on an app/phone bi-stream (host.md §9.3):
/// `term_open` and `term_attach` reply `ok{data:TermAttachData}` and then
/// pump the terminal until the child exits or the client detaches.
pub async fn serve_request<R, W>(
    registry: &TermRegistry,
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
        Request::TermOpen {
            cwd,
            cols,
            rows,
            shell,
        } => {
            let home = crate::fs::home_dir_opt().ok_or_else(|| {
                err(
                    AppErrorCode::E_INTERNAL,
                    "the host has no home directory for this workspace",
                )
            })?;
            let cwd = crate::fs::resolve_request_path(cwd.as_str(), &home)?;
            tracing::info!(op = "term_open", cwd = %cwd.display(), "request");
            let (data, attached) = registry.open(
                paths,
                OpenRequest {
                    cwd,
                    cols: dimension(cols.value()),
                    rows: dimension(rows.value()),
                    shell: shell.map(|shell| shell.as_str().to_owned()),
                },
            )?;
            pump(attached, data, read, write).await
        }
        Request::TermAttach {
            term_id,
            cols,
            rows,
        } => {
            tracing::info!(op = "term_attach", term = term_id.as_str(), "request");
            let attached = registry.attach(
                term_id.as_str(),
                dimension(cols.value()),
                dimension(rows.value()),
            )?;
            let data = TermAttachData {
                term_id: term_id.clone(),
                shell: registry
                    .get(term_id.as_str())
                    .map_or_else(|_| TermShell::default(), |entry| entry.shell.clone()),
            };
            pump(attached, data, read, write).await
        }
        Request::TermList => {
            tracing::info!(op = "term_list", "request");
            let data = serde_json::to_value(registry.list()).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) })
                .await
                .map_err(internal)?;
            Ok(())
        }
        Request::TermKill { term_id } => {
            tracing::info!(op = "term_kill", term = term_id.as_str(), "request");
            registry.kill(term_id.as_str())?;
            write_control(write, &Control::Ok { data: None })
                .await
                .map_err(internal)?;
            Ok(())
        }
        _ => Err(err(AppErrorCode::E_UNSUPPORTED, "not a terminal request")),
    }
}

fn dimension(value: u32) -> u16 {
    u16::try_from(value.clamp(1, 1000)).unwrap_or(24)
}

/// Streams one terminal: output out as kind-1 frames, input in as kind-2
/// frames, `resize`/`title`/`exit` controls both ways. Ends (without
/// killing the PTY) when the client goes away.
async fn pump<R, W>(
    mut attached: Attached,
    data: TermAttachData,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let reply = serde_json::to_value(&data).map_err(internal)?;
    write_control(write, &Control::Ok { data: Some(reply) })
        .await
        .map_err(internal)?;
    loop {
        tokio::select! {
            output = attached.recv() => match output {
                Some(TermOutput::Bytes(bytes)) => {
                    if write
                        .write_all(&unipi_proto::frame::encode(
                            unipi_proto::frame::FrameKind::Output,
                            &bytes,
                        ))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                }
                Some(TermOutput::Exit(code)) => {
                    let _ = write_control(write, &Control::Exit { code }).await;
                    return Ok(());
                }
                // Dropped as too slow: the client can attach again.
                None => {
                    let code = attached.exit_code();
                    let _ = write_control(write, &Control::Exit { code }).await;
                    return Ok(());
                }
            },
            frame = read_frame(read) => {
                let Ok((kind, bytes)) = frame else {
                    // The client detached: the PTY keeps running.
                    return Ok(());
                };
                match kind {
                    2 => attached.input(&bytes)?,
                    0 => {
                        if let Ok(Control::Resize { cols, rows }) =
                            serde_json::from_slice::<Control>(&bytes)
                        {
                            attached.resize(dimension(cols.value()), dimension(rows.value()));
                        }
                    }
                    // kind 1 from a client is meaningless on a terminal stream.
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_throttle_is_last_wins_within_the_window() {
        let start = Instant::now();
        let mut throttle = ResizeThrottle::default();
        // An open window applies at once and starts the 100 ms window.
        assert_eq!(throttle.offer(80, 24, start), Some((80, 24)));
        assert_eq!(throttle.wait(start), None, "nothing queued");
        // Two more inside the window coalesce; the last one wins.
        assert_eq!(throttle.offer(100, 30, start), None);
        assert_eq!(
            throttle.offer(120, 40, start + Duration::from_millis(50)),
            None
        );
        assert_eq!(
            throttle.wait(start + Duration::from_millis(50)),
            Some(Duration::from_millis(50))
        );
        assert_eq!(throttle.poll(start + Duration::from_millis(99)), None);
        assert_eq!(
            throttle.poll(start + Duration::from_millis(100)),
            Some((120, 40))
        );
        assert_eq!(throttle.poll(start + Duration::from_millis(200)), None);
        // A request after the window is applied immediately again.
        assert_eq!(
            throttle.offer(90, 20, start + Duration::from_millis(200)),
            Some((90, 20))
        );
    }

    #[test]
    fn throttle_counts_never_exceed_one_per_window() {
        let start = Instant::now();
        let mut throttle = ResizeThrottle::default();
        let mut applied = 0;
        // 60 requests over 60 ms: the host may apply at most one at t=0.
        for step in 0..60u16 {
            let now = start + Duration::from_millis(u64::from(step));
            if throttle.offer(80 + step, 24 + step, now).is_some() {
                applied += 1;
            }
        }
        assert_eq!(applied, 1, "a 60 ms burst is one resize");
        assert_eq!(
            throttle.poll(start + Duration::from_millis(100)),
            Some((139, 83)),
            "the trailing apply takes the last request"
        );
    }

    #[test]
    fn clamp_name_caps_and_rejects_empty() {
        assert_eq!(clamp_name("vim".into()).as_deref(), Some("vim"));
        assert_eq!(clamp_name("".into()), None);
        assert_eq!(clamp_name(" \n".into()), None);
        let long = "x".repeat(TERM_FOREGROUND_NAME_MAX + 10);
        assert_eq!(
            clamp_name(long).map(|name| name.chars().count()),
            Some(TERM_FOREGROUND_NAME_MAX)
        );
    }

    /// Linux: the leader's `/proc/<pid>/comm` is the reported name. This
    /// test process is its own group leader, so it resolves.
    #[cfg(target_os = "linux")]
    #[test]
    fn foreground_name_reads_proc_comm() {
        let name = foreground_name(std::process::id()).expect("own comm");
        assert!(!name.is_empty());
        assert!(name.chars().count() <= TERM_FOREGROUND_NAME_MAX);
        // A dead group reads as unknown, not as an error.
        assert_eq!(foreground_name(u32::MAX), None);
    }

    #[test]
    fn resolve_shell_prefers_the_request_then_shell_env() {
        let explicit = resolve_shell(Some("/bin/zsh"));
        assert_eq!(explicit.as_str(), "/bin/zsh");
        // An empty request falls back to the environment, never to "".
        assert!(!resolve_shell(Some("")).as_str().is_empty());
        assert!(!resolve_shell(None).as_str().is_empty());
    }

    #[test]
    fn pty_command_is_a_login_shell_in_the_workspace() {
        let cwd = std::path::Path::new("/tmp");
        let command = pty_command("/bin/sh", cwd);
        let debug = format!("{command:?}");
        assert!(debug.contains("/bin/sh"), "{debug}");
        if cfg!(unix) {
            assert!(debug.contains("-l"), "login shell: {debug}");
        }
        assert!(debug.contains("xterm-256color"), "{debug}");
        assert!(debug.contains("/tmp"), "{debug}");
    }
}
