//! The always-on daemon (`oino cron serve`): the single ticking owner plus its control
//! socket.
//!
//! **Single owner:** a lock file (`scheduler.lock`, held for the process's life) makes
//! exactly one scheduler fire jobs — the daemon, or the desktop app's embedded scheduler.
//! A second owner fails fast with [`crate::error::CronError::AlreadyOwned`].
//!
//! **Control socket:** a loopback TCP listener (OS-assigned port, recorded in
//! `serve.json`) speaking one-request/one-reply JSON lines of [`crate::control`].
//!
//! **Startup:** runs left `running`/`queued` by a previous owner whose process is gone
//! are closed as `failed` ("interrupted").
//!
//! **Shutdown:** on Ctrl-C / SIGTERM the daemon stops accepting, waits up to 30 s for
//! running jobs to finish, removes `serve.json` and exits.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::client::{Reply, ServeInfo};
use crate::control::{CronControl, handle_control};
use crate::error::{CronError, Result};
use crate::paths::{CronPaths, atomic_write_json};
use crate::scheduler::Scheduler;

/// How long shutdown waits for running jobs.
const DRAIN_GRACE: Duration = Duration::from_secs(30);

/// The owner lock: held (flock) for the process's life.
///
/// # Errors
/// [`CronError::Io`] on lock errors, [`CronError::AlreadyOwned`] when held elsewhere.
pub fn acquire_owner_lock(paths: &CronPaths) -> Result<OwnerLock> {
    paths.ensure()?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(paths.owner_lock())
        .map_err(|e| CronError::io(e, paths.owner_lock()))?;
    if file.try_lock().is_err() {
        return Err(CronError::AlreadyOwned(holder_hint(paths)));
    }
    Ok(OwnerLock {
        file,
        path: paths.owner_lock(),
    })
}

fn holder_hint(paths: &CronPaths) -> String {
    match crate::client::serve_info(paths) {
        Ok(Some(info)) => format!("pid {}", info.pid),
        _ => "another process holds the lock".to_owned(),
    }
}

/// The held owner lock. Dropping it releases the ownership.
pub struct OwnerLock {
    #[allow(dead_code, reason = "kept alive; the OS releases the flock on drop")]
    file: std::fs::File,
    #[allow(dead_code, reason = "diagnostics")]
    path: std::path::PathBuf,
}

/// Runs the daemon until Ctrl-C / SIGTERM. Intended to be called once from
/// `oino cron serve` with a multi-thread tokio runtime.
///
/// # Errors
/// [`CronError::AlreadyOwned`] when another scheduler owns the lock; I/O errors from the
/// listener or the store.
#[allow(
    clippy::print_stderr,
    reason = "the daemon's console output is its log"
)]
pub async fn serve(paths: CronPaths) -> Result<()> {
    let _owner = acquire_owner_lock(&paths)?;
    let scheduler = Scheduler::with_paths(paths.clone());
    let closed = scheduler.close_stale_runs().unwrap_or(0);
    if closed > 0 {
        eprintln!("closed {closed} run(s) interrupted by a previous scheduler");
    }

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| CronError::io(e, paths.root.join("serve")))?;
    let port = listener
        .local_addr()
        .map_err(|e| CronError::io(e, paths.serve_info()))?
        .port();
    atomic_write_json(
        &paths.serve_info(),
        &ServeInfo {
            pid: std::process::id(),
            port,
        },
    )?;

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let ticker = {
        let scheduler = scheduler.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(scheduler.tick_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => scheduler.tick().await,
                    _ = shutdown.changed() => break,
                }
            }
        })
    };

    eprintln!(
        "oino cron: scheduler running on 127.0.0.1:{port} (pid {}); Ctrl-C to stop",
        std::process::id()
    );

    // Stop on Ctrl-C / SIGTERM: signal the tick task and the accept loop.
    {
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(());
        });
    }

    let mut shutdown = shutdown_rx.clone();
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            _ = shutdown.changed() => break,
        };
        let Ok((stream, _)) = accepted else { continue };
        let scheduler = scheduler.clone();
        let paths = paths.clone();
        tokio::spawn(async move {
            handle_connection(stream, &paths, &scheduler).await;
        });
    }

    drop(shutdown_tx);
    let _ = ticker.await;
    scheduler.wait_idle(DRAIN_GRACE).await;
    let _ = std::fs::remove_file(paths.serve_info());
    eprintln!("oino cron: stopped");
    Ok(())
}

/// Resolves on Ctrl-C (or SIGTERM where it exists): the daemon's stop signal.
async fn shutdown_signal() {
    #[cfg(unix)]
    let sigterm = {
        use tokio::signal::unix::{SignalKind, signal};
        signal(SignalKind::terminate())
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        () = async {
            #[cfg(unix)]
            {
                match sigterm {
                    Ok(mut signal) => { signal.recv().await; },
                    Err(_) => std::future::pending::<()>().await,
                }
            }
            #[cfg(not(unix))]
            std::future::pending::<()>().await
        } => {},
    }
}

async fn handle_connection(stream: TcpStream, paths: &CronPaths, scheduler: &Scheduler) {
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut stream = write;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<CronControl>(trimmed) {
            Ok(request) => request,
            Err(e) => {
                let _ = write_reply(&mut stream, &Reply::Error(format!("bad request: {e}"))).await;
                continue;
            }
        };
        let reply = match handle_control(paths, Some(scheduler), request).await {
            Ok(value) => Reply::Ok(value),
            Err(e) => Reply::Error(e.to_string()),
        };
        if write_reply(&mut stream, &reply).await.is_err() {
            return;
        }
    }
}

async fn write_reply(
    stream: &mut tokio::net::tcp::OwnedWriteHalf,
    reply: &Reply,
) -> std::io::Result<()> {
    let mut line = serde_json::to_string(reply)
        .unwrap_or_else(|_| r#"{"Error":"reply encoding failed"}"#.to_owned());
    line.push('\n');
    stream.write_all(line.as_bytes()).await?;
    stream.flush().await
}

/// A [`Scheduler`] plus its held [`OwnerLock`] — what the desktop embeds to attach as the
/// single owner (it falls back to client mode when the daemon already owns the lock).
pub struct Embedded {
    /// The ticking scheduler.
    pub scheduler: Arc<Scheduler>,
    /// The tick task (aborted on [`Embedded::shutdown`]).
    ticker: tokio::task::JoinHandle<()>,
    shutdown: watch::Sender<()>,
    #[allow(dead_code, reason = "kept alive; the OS releases the flock on drop")]
    owner: OwnerLock,
}

impl Embedded {
    /// Tries to become the single owner on `paths`; `Err(AlreadyOwned)` means the daemon
    /// (or another process) ticks already and the caller should use [`ControlClient`].
    ///
    /// # Errors
    /// [`CronError::AlreadyOwned`] or I/O errors from the lock/store.
    pub fn start(paths: CronPaths) -> Result<Self> {
        let owner = acquire_owner_lock(&paths)?;
        let scheduler = Arc::new(Scheduler::with_paths(paths));
        let _ = scheduler.close_stale_runs();
        let (shutdown, rx) = watch::channel(());
        let sched = Arc::clone(&scheduler);
        let ticker = tokio::spawn(async move {
            let mut shutdown = rx;
            let mut interval = tokio::time::interval(sched.tick_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => sched.tick().await,
                    _ = shutdown.changed() => break,
                }
            }
        });
        Ok(Self {
            scheduler,
            ticker,
            shutdown,
            owner,
        })
    }

    /// Stops ticking (running jobs are not killed; the next owner closes them).
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
        self.ticker.abort();
    }
}
