//! The `oino` binary: see the `oino` library for the modes.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::time::Duration;

/// Bound for stopping tokio before process-wide finalizers run.
///
/// RPC and interactive modes read stdin on tokio's blocking pool. A terminal
/// (or a pipe whose writer stays open) can keep that read blocked forever, so
/// the runtime cannot be dropped normally. `shutdown_timeout` stops the worker
/// threads and returns after this bound instead of waiting for the reader.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[allow(
    clippy::print_stderr,
    reason = "reported before the CLI's logging/output setup exists"
)]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("oino: cannot start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(async {
        let code = oino::run(args).await;
        // Stop native engines (zvec/ORT users) while their async handles are
        // still owned by this process and before the runtime disappears. Bound
        // the wait: a background search may still hold the engine lock, and a
        // stuck exit is worse than a skipped teardown.
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, oino::shutdown()).await;
        code
    });
    // Never call `std::process::exit` here: it would run the dynamic loader's
    // finalizers while tokio worker threads are still alive. Stop the runtime
    // first, bounded so a blocked stdin reader cannot keep the process alive.
    runtime.shutdown_timeout(SHUTDOWN_TIMEOUT);
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
