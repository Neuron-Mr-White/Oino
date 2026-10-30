//! The embedded single-owner scheduler: what unipi-host runs (v2 ext-abi §6.3).
//!
//! **Single owner:** a lock file (`scheduler.lock`, held for the process's life) makes
//! exactly one scheduler fire jobs — under v2 that is unipi-host. A second owner fails
//! fast with [`crate::error::CronError::AlreadyOwned`]. (The v1 `oino cron serve`
//! daemon, its loopback control socket, `serve.json` and the service installers are
//! gone; the `unipi-host cron` CLI talks to the store and an in-process scheduler
//! directly.)
//!
//! **Startup:** a legacy `jobs.json` is migrated into `jobs/*.json`, runs left
//! `running`/`queued` by a previous owner whose process is gone are closed as `failed`
//! ("interrupted"), and one full scan runs.
//!
//! **Watching:** `jobs/` is watched (notify); any change triggers an immediate rescan
//! tick, so edits by the extension, the CLI or the user land without a restart (the
//! regular tick would pick them up within a second anyway — the watcher makes it fast
//! and covers longer tick intervals; ext-abi risk #3). Edits made while no owner runs
//! are picked up by the startup scan.
//!
//! **Shutdown:** [`Embedded::shutdown`] stops the tick task; running jobs are not
//! killed — the next owner closes them as interrupted.

use std::sync::Arc;
use std::time::Duration;

use notify::Watcher;
use tokio::sync::{mpsc, watch};

use crate::error::{CronError, Result};
use crate::paths::CronPaths;
use crate::scheduler::Scheduler;
use crate::store;

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
        return Err(CronError::AlreadyOwned(
            "another process holds the cron scheduler lock".to_owned(),
        ));
    }
    Ok(OwnerLock {
        file,
        path: paths.owner_lock(),
    })
}

/// The held owner lock. Dropping it releases the ownership.
pub struct OwnerLock {
    #[allow(dead_code, reason = "kept alive; the OS releases the flock on drop")]
    file: std::fs::File,
    #[allow(dead_code, reason = "diagnostics")]
    path: std::path::PathBuf,
}

/// A [`Scheduler`] plus its held [`OwnerLock`] — what unipi-host embeds as the single
/// ticking owner. `unipi-host cron`'s `run`/`stop`/`status` work against its scheduler.
pub struct Embedded {
    /// The ticking scheduler (live run table, fires).
    pub scheduler: Arc<Scheduler>,
    /// The tick task (stopped by [`Embedded::shutdown`]).
    ticker: tokio::task::JoinHandle<()>,
    shutdown: watch::Sender<()>,
    #[allow(dead_code, reason = "kept alive; the OS releases the flock on drop")]
    owner: OwnerLock,
}

impl Embedded {
    /// Tries to become the single owner on `paths`; `Err(AlreadyOwned)` means another
    /// process ticks already.
    ///
    /// # Errors
    /// [`CronError::AlreadyOwned`] or I/O errors from the lock/store.
    pub fn start(paths: CronPaths) -> Result<Self> {
        Self::start_with(Scheduler::with_paths(paths))
    }

    /// [`Embedded::start`] over a caller-built [`Scheduler`] (custom clock or tick
    /// interval; tests).
    ///
    /// # Errors
    /// As [`Embedded::start`].
    pub fn start_with(scheduler: Scheduler) -> Result<Self> {
        let owner = acquire_owner_lock(scheduler.paths())?;
        let scheduler = Arc::new(scheduler);
        // Migrate + a full first scan, then close whatever a previous owner left.
        let _ = store::load_jobs(scheduler.paths())?;
        let closed = scheduler.close_stale_runs().unwrap_or(0);
        let _ = closed; // best effort: already persisted as `failed` on each run record
        let (shutdown, rx) = watch::channel(());
        let sched = Arc::clone(&scheduler);
        let ticker = tokio::spawn(async move {
            let mut shutdown = rx;
            // A rescan tick also runs whenever jobs/ changes (ext-abi §6.2).
            let (tx, mut changes) = mpsc::channel::<()>(8);
            let watcher = notify::recommended_watcher(
                move |event: std::result::Result<notify::Event, notify::Error>| {
                    if event.is_ok_and(|e| {
                        e.kind.is_modify() || e.kind.is_create() || e.kind.is_remove()
                    }) {
                        let _ = tx.try_send(());
                    }
                },
            );
            // A failed watcher degrades to interval-only rescans (ext-abi risk #3).
            let mut watcher = watcher.ok();
            if let Some(watcher) = watcher.as_mut() {
                let _ = watcher.watch(
                    &sched.paths().jobs_dir(),
                    notify::RecursiveMode::NonRecursive,
                );
            }
            let mut interval = tokio::time::interval(sched.tick_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => sched.tick().await,
                    _ = changes.recv() => {
                        // Coalesce bursts (an atomic write is remove+create+modify).
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        while changes.try_recv().is_ok() {}
                        sched.tick().await;
                    }
                    _ = shutdown.changed() => break,
                }
            }
            let _ = tx;
        });
        Ok(Self {
            scheduler,
            ticker,
            shutdown,
            owner,
        })
    }

    /// Stops ticking (running jobs are not killed; the next owner closes them).
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(());
        self.ticker.abort();
    }
}
