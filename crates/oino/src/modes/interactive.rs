//! Interactive mode: [`oino_tui::run`] over an in-process backend.

use std::sync::Arc;

use futures::StreamExt;
use oino_client::{InProcessBackend, SessionBackend};
use oino_core::events::{ClientEvent, OinoEvent};
use oino_tui::TuiOptions;
use oino_tui::host::LocalHost;

use crate::modes::signals;
use crate::startup::{Environment, Started};

/// Runs the terminal UI until the user exits (0), an extension calls `ctx.shutdown()` and the
/// session is idle (0), the session is taken over by another runner (0, with a notice), the
/// UI fails (1) or a signal arrives.
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
    // D-TAKEOVER: when another runner takes the session over, this runtime closes it — show
    // the notice and return to the session list (exit 0) instead of holding a dead session.
    // Boxed: the stream keeps this mode's future small.
    let takeover_notice = Box::pin(async {
        let mut events = backend.events();
        loop {
            match events.next().await {
                Some(ClientEvent::Oino(OinoEvent::SessionTakenOver { by })) => {
                    return format!("Session moved to another window (by {})", by.client);
                }
                Some(_) => {}
                None => std::future::pending().await,
            }
        }
    });
    let mut notice = None;
    let code = tokio::select! {
        () = shutdown => 0,
        result = ui => match result {
            Ok(()) => 0,
            Err(error) => {
                crate::eprintln_line(&format!("Error: {error}"));
                1
            }
        },
        taken = takeover_notice => {
            notice = Some(taken);
            0
        }
        signal = signals::next() => signal.exit_code(),
    };
    backend.shutdown().await;
    if let Some(notice) = notice {
        crate::eprintln_line(&notice);
    }
    code
}
