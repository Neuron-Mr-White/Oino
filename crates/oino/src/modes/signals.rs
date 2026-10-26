//! Termination signals of non-interactive modes (pi: SIGHUP → 129, SIGTERM → 143).

/// A signal that ends the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// SIGHUP (not on Windows).
    Hangup,
    /// SIGTERM (Ctrl+Break on Windows).
    Terminate,
}

impl Signal {
    /// pi's exit code for the signal.
    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Hangup => 129,
            Self::Terminate => 143,
        }
    }
}

/// Resolves on the next SIGHUP/SIGTERM; never resolves when handlers cannot be installed.
#[cfg(unix)]
pub async fn next() -> Signal {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut hangup), Ok(mut terminate)) = (
        signal(SignalKind::hangup()),
        signal(SignalKind::terminate()),
    ) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = hangup.recv() => Signal::Hangup,
        _ = terminate.recv() => Signal::Terminate,
    }
}

/// Resolves on the next Ctrl+Break; never resolves when the handler cannot be installed.
#[cfg(windows)]
pub async fn next() -> Signal {
    match tokio::signal::windows::ctrl_break() {
        Ok(mut ctrl_break) => {
            ctrl_break.recv().await;
            Signal::Terminate
        }
        Err(_) => std::future::pending().await,
    }
}
