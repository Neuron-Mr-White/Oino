//! Dedicated scheduling resources for component actors and host imports.

use std::sync::mpsc;

/// Synchronous imports can recursively call another component. Hand the current
/// worker to Tokio before waiting so the nested actor can always be polled.
/// Non-runtime callers (e.g. a TUI render thread) need no handoff.
pub(crate) fn block<T>(future: impl Future<Output = T>) -> T {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|handle| handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(|| futures::executor::block_on(future))
    } else {
        futures::executor::block_on(future)
    }
}

/// Owned by the shared engine, not by a session or the application's runtime.
pub(crate) struct Executor {
    handle: tokio::runtime::Handle,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Executor {
    pub(crate) fn new() -> std::io::Result<Self> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop, stopped) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("oino-wasm-runtime".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    // A synchronous import can wait for an actor that itself imports.
                    // A finite pool ceiling creates a dependency cycle when all
                    // replacement workers are waiting on their children. Admission
                    // remains bounded at each actor; nested waits must not be capped.
                    .max_blocking_threads(usize::MAX / 2)
                    .thread_name("oino-wasm-worker")
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        let _ = ready_tx.send(Ok(runtime.handle().clone()));
                        let _ = stopped.recv();
                        // Never drop a runtime on an async worker, or wait forever for a
                        // blocked import during engine teardown.
                        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            })?;
        let handle = ready_rx.recv().map_err(std::io::Error::other)??;
        Ok(Self {
            handle,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    pub(crate) fn handle(&self) -> &tokio::runtime::Handle {
        &self.handle
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            // Wait for `shutdown_timeout` in the executor thread, so no wasm
            // worker is alive when the process runs dynamic-loader finalizers.
            // The last engine owner can be dropped on that runtime's own
            // worker; joining ourselves would deadlock, and the thread is
            // already on its way out.
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::print_stderr, reason = "test diagnostic timing")]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_guest_work_does_not_starve_main_runtime() {
        let executor = Executor::new().unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let guest = executor.handle().spawn(async move {
            let _ = started.send(());
            // Models a synchronous host import blocking an actor's polling thread.
            std::thread::sleep(Duration::from_secs(2));
        });
        ready.await.unwrap();
        let progress = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let progress = progress.clone();
            tasks.push(tokio::spawn(async move {
                for _ in 0..20 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    progress.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        for task in tasks {
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(progress.load(Ordering::Relaxed), 400);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!guest.is_finished());
        guest.await.unwrap();
    }
    #[tokio::test]
    async fn saturated_nested_sync_imports_make_progress() {
        let executor = Executor::new().unwrap();
        let started = Instant::now();
        let barrier = Arc::new(tokio::sync::Barrier::new(141));
        let mut jobs = Vec::new();
        for _ in 0..140 {
            let barrier = barrier.clone();
            let handle = executor.handle().clone();
            jobs.push(executor.handle().spawn(async move {
                block(async move {
                    barrier.wait().await;
                    handle.spawn(async { 42 }).await.unwrap()
                })
            }));
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            barrier.wait().await;
            eprintln!(
                "all_140_sync_imports_admitted_ms={}",
                started.elapsed().as_millis()
            );
            for job in jobs {
                assert_eq!(job.await.unwrap(), 42);
            }
        })
        .await
        .unwrap();
    }
    // QA R4-01: children must not sit above their parents on a stealing stack.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn continuation_inversion_makes_progress() {
        let e = Executor::new().unwrap();
        let entered = Arc::new(AtomicUsize::new(0));
        let nested = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let completed = Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        let roots = Arc::new(std::sync::Barrier::new(3));
        for _ in 0..2 {
            let roots = roots.clone();
            let entered = entered.clone();
            let release = release.clone();
            let completed = completed.clone();
            jobs.push(e.handle().spawn(async move {
                roots.wait();
                block(async {
                    let notified = release.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    entered.fetch_add(1, Ordering::SeqCst);
                    notified.await;
                });
                completed.fetch_add(1, Ordering::SeqCst);
            }));
        }
        roots.wait();
        while entered.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let children = Arc::new(std::sync::Barrier::new(3));
        for _ in 0..2 {
            let children = children.clone();
            let nested = nested.clone();
            let completed = completed.clone();
            jobs.push(e.handle().spawn(async move {
                children.wait();
                block(async {
                    nested.fetch_add(1, Ordering::SeqCst);
                    while completed.load(Ordering::SeqCst) == 0 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                });
            }));
        }
        children.wait();
        while nested.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        release.notify_waiters();
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            for j in jobs {
                j.await.unwrap();
            }
        })
        .await;
        assert!(
            result.is_ok(),
            "ready parent continuations were blocked by children"
        );
    }
}
