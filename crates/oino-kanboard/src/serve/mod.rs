//! `serve` — the daemon: single instance, JSON API, embedded UI, SSE, file
//! watch, idle shutdown.

pub(crate) mod api;
pub(crate) mod auth;
pub(crate) mod events;
pub mod settings;
pub(crate) mod undo;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::middleware;
use axum::routing::{get, post, put};
use chrono::Utc;
use notify::{RecursiveMode, Watcher as _};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::daemon::{self, DaemonInfo};
use crate::error::Result;
use crate::store::Layout;

/// Shared, process-global state: the daemon is a singleton by design (the lock
/// enforces it), so handlers reach it through this instead of plumbing it
/// through the framework.
pub(crate) struct AppState {
    pub(crate) layout: Layout,
    /** Bind address this daemon was started with. */
    pub(crate) host: String,
    /** Required when `host` is not loopback (None = open, loopback only). */
    pub(crate) token: Option<String>,
    pub(crate) started: Instant,
    pub(crate) last_activity: Mutex<Instant>,
    pub(crate) sse_clients: AtomicUsize,
    pub(crate) revisions: Mutex<HashMap<String, u64>>,
    pub(crate) senders: Mutex<HashMap<String, broadcast::Sender<u64>>>,
    pub(crate) watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// Flips on shutdown so open SSE streams end — graceful shutdown waits for
    /// every connection, and an event stream never finishes by itself.
    pub(crate) closing: tokio::sync::watch::Sender<bool>,
    /// The daemon-owned model catalog (`pi --list-models`), 10-minute TTL.
    /// Holding the lock across a fetch also serializes refreshes — concurrent
    /// callers share one in-flight run.
    pub(crate) model_cache: tokio::sync::Mutex<Option<(Instant, Vec<String>)>>,
    /// UNI-57 undo tokens: short-lived, daemon-held move snapshots (see
    /// `undo::UndoStore`). Memory only — undo expires on restart.
    pub(crate) undo: undo::UndoStore,
}

impl AppState {
    fn new(layout: Layout, host: String, token: Option<String>) -> Arc<AppState> {
        Arc::new(AppState {
            layout,
            host,
            token,
            started: Instant::now(),
            last_activity: Mutex::new(Instant::now()),
            sse_clients: AtomicUsize::new(0),
            revisions: Mutex::new(HashMap::new()),
            senders: Mutex::new(HashMap::new()),
            watcher: Mutex::new(None),
            closing: tokio::sync::watch::channel(false).0,
            model_cache: tokio::sync::Mutex::new(None),
            undo: undo::UndoStore::new(),
        })
    }

    /// Any HTTP request or SSE connect/disconnect restarts the idle window.
    pub(crate) fn touch(&self) {
        if let Ok(mut last) = self.last_activity.lock() {
            *last = Instant::now();
        }
    }

    pub(crate) fn revision(&self, slug: &str) -> u64 {
        self.revisions
            .lock()
            .map_or(0, |map| map.get(slug).copied().unwrap_or(0))
    }

    /// Bump a project's revision and tell its SSE clients.
    pub(crate) fn bump(&self, slug: &str) {
        let revision = {
            let Ok(mut revisions) = self.revisions.lock() else {
                return;
            };
            let entry = revisions.entry(slug.to_string()).or_insert(0);
            *entry += 1;
            *entry
        };
        if let Ok(senders) = self.senders.lock()
            && let Some(sender) = senders.get(slug)
        {
            let _ = sender.send(revision);
        }
    }

    pub(crate) fn subscribe(&self, slug: &str) -> broadcast::Receiver<u64> {
        let Ok(mut senders) = self.senders.lock() else {
            return broadcast::channel(1).1;
        };
        senders
            .entry(slug.to_string())
            .or_insert_with(|| broadcast::channel(64).0)
            .subscribe()
    }
}

/// How `serve` binds and when it stops.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Bind address: 127.0.0.1 (default) or anything else for remote access.
    pub host: String,
    /// Port (0 = OS-assigned).
    pub port: u16,
    /// Stop after this long with no request and no SSE client.
    pub idle: Duration,
    /// Token-gate loopback binds too (remote binds always require it).
    pub require_auth: bool,
    /// Reuse the token in `<home>/token` so board links survive restarts.
    pub keep_token: bool,
}

impl ServeOptions {
    /// Options for `host:port` with an idle timeout in minutes (at least 1).
    pub fn new(host: impl Into<String>, port: u16, idle_min: u64) -> Self {
        ServeOptions {
            host: host.into(),
            port,
            idle: Duration::from_secs(idle_min.max(1) * 60),
            require_auth: false,
            keep_token: false,
        }
    }
}

/// `<home>/token` — the persistent token --keep-token reads and --rotate-token drops.
#[must_use]
pub fn token_path(layout: &Layout) -> std::path::PathBuf {
    layout.home.join("token")
}

/// Read the persistent token, creating it (mode 0600) on first use.
fn persistent_token(layout: &Layout) -> Result<String> {
    let path = token_path(layout);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let token = text.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let token = auth::generate_token();
    crate::store::write_atomic(&path, &format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}

/// Run the daemon. Returns the JSON payload for the caller to print; when
/// another daemon already holds the lock, returns the existing daemon.json as
/// `{"alreadyRunning": true, "daemon": …}` (exit code 0).
#[allow(
    clippy::too_many_lines,
    reason = "daemon lifecycle in order: lock, bind, watch, serve, clean up"
)]
pub async fn serve(layout: Layout, options: ServeOptions) -> Result<Value> {
    let Some(lock) = daemon::take_lock(&layout)? else {
        let running = daemon::read_info(&layout);
        // Binding changes need a restart; say so instead of silently ignoring.
        let binding_changed = running.as_ref().is_some_and(|info| {
            info.host != options.host || (options.port != 0 && info.port != options.port)
        });
        return Ok(json!({
            "alreadyRunning": true,
            "bindingChanged": binding_changed,
            "requested": { "host": options.host, "port": options.port },
            "daemon": running,
        }));
    };

    let listener = TcpListener::bind((options.host.as_str(), options.port))
        .await
        .map_err(|err| {
            crate::error::Error::Io(format!(
                "cannot bind {}:{}: {err}",
                options.host, options.port
            ))
        })?;
    let port = listener.local_addr()?.port();

    // Remote binds are token-gated; loopback stays open unless --require-auth.
    // --keep-token reuses <home>/token so links survive restarts.
    let needs_token = options.require_auth || !auth::is_loopback(&options.host);
    let token = if !needs_token {
        None
    } else if options.keep_token {
        Some(persistent_token(&layout)?)
    } else {
        Some(auth::generate_token())
    };
    let state = AppState::new(layout.clone(), options.host.clone(), token.clone());

    // File watching feeds the SSE revisions. On machines whose inotify budget
    // is exhausted the daemon died at startup for every caller; it now serves
    // without live updates when OINO_KANBOARD_NO_WATCH=1 (tests on this dev
    // machine set it; production still fails loudly).
    if std::env::var("OINO_KANBOARD_NO_WATCH").as_deref() != Ok("1") {
        let watcher = spawn_watcher(state.clone())?;
        if let Ok(mut slot) = state.watcher.lock() {
            *slot = Some(watcher);
        }
    }

    let info = DaemonInfo {
        pid: std::process::id(),
        port,
        version: env!("CARGO_PKG_VERSION").to_string(),
        started_at: Utc::now(),
        host: options.host.clone(),
        token: token.clone(),
    };
    daemon::write_info(&layout, &info)?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<&'static str>(4);
    spawn_idle_monitor(state.clone(), options.idle, shutdown_tx.clone());

    let router = router(state.clone());
    let signal = async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            match (
                signal(SignalKind::terminate()),
                signal(SignalKind::interrupt()),
            ) {
                (Ok(mut term), Ok(mut int)) => {
                    tokio::select! {
                        _ = term.recv() => {}
                        _ = int.recv() => {}
                        _ = shutdown_rx.recv() => {}
                    }
                }
                // No signal handlers (exotic sandbox): idle/stop still end the daemon.
                _ => {
                    let _ = shutdown_rx.recv().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = shutdown_rx.recv() => {}
            }
        }
    };

    let closing = state.closing.clone();
    let signal = async move {
        signal.await;
        // End the event streams first, or the graceful drain never completes.
        let _ = closing.send(true);
    };
    let server = axum::serve(listener, router).with_graceful_shutdown(signal);
    let mut closing_rx = state.closing.subscribe();
    let result = tokio::select! {
        result = server => result,
        // Hard ceiling on the drain: a stuck request must not keep a stopped daemon alive.
        () = async {
            let _ = closing_rx.wait_for(|closing| *closing).await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        } => Ok(()),
    };

    daemon::remove_info(&layout);
    // Keep the flock until the very end so a racing `serve` sees it held.
    drop(lock);
    if let Ok(mut slot) = state.watcher.lock() {
        *slot = None;
    }

    result.map_err(|err| crate::error::Error::Io(format!("daemon stopped: {err}")))?;

    Ok(json!({
        "alreadyRunning": false,
        "daemon": info,
        "stopped": true,
    }))
}

fn router(state: Arc<AppState>) -> axum::Router {
    axum::Router::new()
        .route("/api/health", get(api::health))
        .route("/api/projects", get(api::projects))
        .route("/api/projects/{slug}", put(api::update_project))
        .route(
            "/api/settings",
            get(api::get_settings).put(api::put_settings),
        )
        .route("/api/projects/{slug}/summarize", post(api::summarize))
        .route(
            "/api/projects/{slug}/archive-summary",
            post(api::archive_summary),
        )
        .route("/api/projects/{slug}/archive-lane", post(api::archive_lane))
        .route("/api/projects/{slug}/done-lane", post(api::done_lane))
        .route("/api/models", get(api::models))
        .route("/api/rules", get(api::rules))
        .route("/api/running", get(api::running))
        .route("/api/dashboard", get(api::dashboard))
        .route("/api/projects/{slug}/tasks", get(api::tasks))
        .route("/api/tasks/{slug}/{id}", get(api::task))
        .route("/api/tasks/{slug}/create", post(api::create))
        .route("/api/tasks/{slug}/{id}/move", post(api::move_task))
        .route("/api/tasks/{slug}/{id}/undo", post(api::undo))
        .route("/api/tasks/{slug}/{id}/note", post(api::note))
        .route("/api/tasks/{slug}/{id}/edit", post(api::edit))
        .route("/api/tasks/{slug}/{id}/link", post(api::link))
        .route("/api/tasks/{slug}/{id}/unlink", post(api::unlink))
        .route("/api/tasks/{slug}/{id}/order", post(api::order))
        .route("/api/tasks/{slug}/{id}/duplicate", post(api::duplicate))
        .route(
            "/api/tasks/{slug}/{id}/attachments",
            post(api::upload).layer(axum::extract::DefaultBodyLimit::max(
                crate::attachments::MAX_BYTES + 1024,
            )),
        )
        .route("/api/files/{slug}/{task}/{name}", get(api::file))
        .route("/events", get(events::events))
        .layer(middleware::from_fn_with_state(state.clone(), auth::guard))
        .with_state(state)
}

fn spawn_idle_monitor(
    state: Arc<AppState>,
    idle: Duration,
    shutdown: tokio::sync::mpsc::Sender<&'static str>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            if state.sse_clients.load(Ordering::SeqCst) > 0 {
                continue;
            }
            let elapsed = state
                .last_activity
                .lock()
                .map(|last| last.elapsed())
                .unwrap_or_default();
            if elapsed >= idle && state.started.elapsed() >= idle {
                let _ = shutdown.send("idle").await;
                return;
            }
        }
    });
}

/// Watch `projects/` and bump the revision of whichever project changed, so CLI
/// and agent writes appear in the UI without polling.
fn spawn_watcher(state: Arc<AppState>) -> Result<notify::RecommendedWatcher> {
    let projects = state.layout.projects_root();
    std::fs::create_dir_all(&projects)?;
    // Strip against the canonical root too: macOS reports FSEvents paths under
    // /private/var while the layout root is the /var symlink (and Linux keeps
    // the symlink), so accept either spelling of the watched directory.
    let canonical_root = std::fs::canonicalize(&projects).unwrap_or_else(|_| projects.clone());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<std::path::PathBuf>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        // Only content changes count. Access events would make the UI refetch on
        // its own reads — an endless revision/refetch loop.
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            let _ = tx.send(path);
        }
    })
    .map_err(|err| {
        crate::error::Error::Io(format!("cannot watch {}: {err}", projects.display()))
    })?;
    watcher
        .watch(&projects, RecursiveMode::Recursive)
        .map_err(|err| {
            crate::error::Error::Io(format!("cannot watch {}: {err}", projects.display()))
        })?;

    tokio::spawn(async move {
        // One write is often several events (tmp file, rename, close): coalesce.
        let mut last_bump: HashMap<String, Instant> = HashMap::new();
        while let Some(path) = rx.recv().await {
            // Ignore lock files and temp files: only real content changes count.
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string());
            let relevant = name.as_deref().is_some_and(|name| {
                std::path::Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                    || name == "project.json"
            });
            if !relevant {
                continue;
            }
            if std::env::var("OINO_KANBOARD_DEBUG_WATCH").as_deref() == Ok("1") {
                #[allow(
                    clippy::print_stderr,
                    reason = "opt-in watcher debug trace (OINO_KANBOARD_DEBUG_WATCH=1)"
                )]
                {
                    eprintln!("watch event: {}", path.display());
                }
            }
            let Some(slug) = path
                .strip_prefix(&canonical_root)
                .or_else(|_| path.strip_prefix(&projects))
                .ok()
                .and_then(|rest| rest.components().next())
                .map(|first| first.as_os_str().to_string_lossy().to_string())
            else {
                continue;
            };
            if let Some(previous) = last_bump.get(&slug)
                && previous.elapsed() < Duration::from_millis(150)
            {
                continue;
            }
            last_bump.insert(slug.clone(), Instant::now());
            state.bump(&slug);
        }
    });

    Ok(watcher)
}
