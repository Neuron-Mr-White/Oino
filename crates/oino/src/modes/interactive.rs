//! pi interactive mode: [`oino_tui::run`] over an in-process backend.

use std::sync::Arc;

use oino_client::{InProcessBackend, SessionBackend};
use oino_tui::TuiOptions;
use oino_tui::host::LocalHost;

use crate::modes::signals;
use crate::startup::{Environment, Started};

/// Runs the terminal UI until the user exits (0), an extension calls `ctx.shutdown()` and the
/// session is idle (0), the UI fails (1) or a signal arrives.
pub async fn run(started: Started, env: &Environment, options: TuiOptions) -> i32 {
    let backend = Arc::new(InProcessBackend::new(started.runtime));
    // The host asks the session for extension render specs (custom messages, entries and the
    // built-in tool rows utility overrides; ui-protocol §6.4).
    let host = Arc::new(
        LocalHost::new(
            &env.cwd,
            &env.agent_dir,
            Some(started.sessions_root),
            started.project_trusted,
        )
        .with_render(Arc::clone(&backend) as Arc<dyn SessionBackend>),
    );
    let ui = oino_tui::run(
        Arc::clone(&backend) as Arc<dyn SessionBackend>,
        host,
        options,
    );
    let shutdown = backend.shutdown_requested();
    let code = tokio::select! {
        () = shutdown => 0,
        result = ui => match result {
            Ok(()) => 0,
            Err(error) => {
                crate::eprintln_line(&format!("Error: {error}"));
                1
            }
        },
        signal = signals::next() => signal.exit_code(),
    };
    backend.shutdown().await;
    code
}
