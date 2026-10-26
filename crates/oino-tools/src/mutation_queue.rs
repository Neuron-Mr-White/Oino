//! Per-file serialisation of mutations (pi `tools/file-mutation-queue.ts`).
//!
//! `write` and `edit` calls on the same file run one after another in call order; calls on
//! different files run in parallel. The key is the real path (symlinks resolved), or the
//! resolved path when the file does not exist yet.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::oneshot;

use crate::env::ExecutionEnv;
use crate::node_path;

type Tail = Shared<BoxFuture<'static, ()>>;

/// A mutation queue keyed by file (pi `withFileMutationQueue`).
#[derive(Clone, Debug, Default)]
pub struct FileMutationQueue {
    inner: Arc<QueueInner>,
}

#[derive(Debug, Default)]
struct QueueInner {
    tails: Mutex<HashMap<String, Tail>>,
    /// Orders registrations like pi's `registrationQueue`, so calls queue in call order even
    /// though resolving the key is asynchronous.
    registration: tokio::sync::Mutex<()>,
}

static GLOBAL: LazyLock<FileMutationQueue> = LazyLock::new(FileMutationQueue::default);

impl FileMutationQueue {
    /// The process-wide queue shared by all built-in tools (pi's queue is module-global).
    #[must_use]
    pub fn global() -> Self {
        GLOBAL.clone()
    }

    /// Runs `operation` once every earlier mutation of the same file has settled.
    pub async fn run<T, F>(&self, env: &dyn ExecutionEnv, path: &str, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        let (key, previous, release, ours) = {
            let _registration = self.inner.registration.lock().await;
            let resolved = node_path::resolve("/", path);
            let key = env.realpath(&resolved).await.unwrap_or(resolved);
            let (release, released) = oneshot::channel::<()>();
            let ours: Tail = released.map(|_| ()).boxed().shared();
            let previous = self.lock_tails().insert(key.clone(), ours.clone());
            (key, previous, release, ours)
        };
        if let Some(previous) = previous {
            previous.await;
        }
        let output = operation.await;
        drop(release);
        let mut tails = self.lock_tails();
        if tails.get(&key).is_some_and(|tail| tail.ptr_eq(&ours)) {
            tails.remove(&key);
        }
        output
    }

    fn lock_tails(&self) -> std::sync::MutexGuard<'_, HashMap<String, Tail>> {
        self.inner
            .tails
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    fn pending(&self) -> usize {
        self.lock_tails().len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::env::LocalEnv;

    #[tokio::test]
    async fn same_file_runs_in_order() {
        let queue = FileMutationQueue::default();
        let env = LocalEnv::new();
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = {
            let (queue, log) = (queue.clone(), Arc::clone(&log));
            async move {
                queue
                    .run(&env, "/nonexistent/a", async {
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        log.lock().unwrap().push(1);
                    })
                    .await;
            }
        };
        let second = {
            let (queue, log) = (queue.clone(), Arc::clone(&log));
            async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                queue
                    .run(&env, "/nonexistent/a", async {
                        log.lock().unwrap().push(2);
                    })
                    .await;
            }
        };
        tokio::join!(first, second);
        assert_eq!(*log.lock().unwrap(), vec![1, 2]);
        assert_eq!(queue.pending(), 0);
    }
}
