//! Tokio ↔ GPUI bridge (gpui-app §3.1; the Zed `gpui_tokio` pattern, re-implemented).
//!
//! `oino-core` (wasmtime, reqwest, processes) needs a tokio runtime; GPUI owns the main thread
//! and its own executors. The shell creates a multi-thread runtime at launch and stores its
//! handle in the [`Tokio`] global. [`Tokio::spawn`] runs a future on that runtime and returns a
//! GPUI [`Task`] that resolves with its output; dropping the task aborts the tokio task.

use std::future::Future;

use gpui_kit::{App, AppContext as _, Global, Task};
use tokio::runtime::Handle;
use tokio::task::{AbortHandle, JoinError};

/// The tokio runtime handle global.
#[derive(Clone, Debug)]
pub struct Tokio {
    handle: Handle,
}

impl Global for Tokio {}

struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Tokio {
    /// Installs the runtime handle.
    pub fn init(cx: &mut App, handle: Handle) {
        cx.set_global(Self { handle });
    }

    /// The runtime handle.
    #[must_use]
    pub fn handle(cx: &App) -> Handle {
        cx.global::<Self>().handle.clone()
    }

    /// Runs `future` on the tokio runtime; the returned task resolves with its output and
    /// aborts the tokio task when dropped.
    pub fn spawn<F, R>(cx: &App, future: F) -> Task<Result<R, JoinError>>
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let join = cx.global::<Self>().handle.spawn(future);
        let guard = AbortOnDrop(join.abort_handle());
        cx.background_spawn(async move {
            let result = join.await;
            drop(guard);
            result
        })
    }

    /// Runs `future` on the tokio runtime and forgets it (fire-and-forget commands).
    pub fn detach<F>(cx: &App, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        drop(cx.global::<Self>().handle.spawn(future));
    }
}
