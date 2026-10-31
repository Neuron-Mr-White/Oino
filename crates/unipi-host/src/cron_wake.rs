//! Wake links for cron runs (WP-H8, host.md §9.8; ext-abi §6.3).
//!
//! A job may link a chat (`sessionLink` in its job file, written by
//! `unipi-host cron add --wake-session/--wake-here` or the extension). When a run finishes,
//! the host **opens that session in the hub** — waking it with the same takeover semantics
//! as `session_open` (§8) instead of poking a live pi over a socket — and prompts the wake
//! message ([`unipi_cron::model::wake_text`]). The outcome (`"sent"`, a failure text) is
//! recorded on the run's `wake` field; a wake never turns a run into a failure.
//!
//! The same sink publishes `unipi:cron:run-finished` — sticky — on the extension bus
//! (ext-abi §6.3's wake/notification hook; the cron extension and anything matching the
//! topic see it).

use std::sync::Arc;

use serde_json::json;

use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_extension_api::{ExtensionId, RawJson};
use oino_wasm_host::LocalServices;

use unipi_cron::model::{wake_text, Job, Run};
use unipi_cron::{store, CronPaths, Scheduler};

use crate::sessions::hub::SessionHub;

/// The bus topic published after every finished run (sticky).
pub const RUN_FINISHED_TOPIC: &str = "unipi:cron:run-finished";

/// The topic's publisher identity (the job files are the cron extension's, so the event
/// carries its id; the host is the one that fires).
const PUBLISHER: &str = "unipi.cron";

/// Installs the wake + bus sink on `scheduler` and returns it for
/// [`unipi_cron::embedded::Embedded::start_with`].
///
/// `hub` is the host's session hub (wake opens the linked session through it); `local` is
/// the process-shared extension hub, when extensions are loaded — without it the event is
/// only skipped, never an error.
#[must_use]
pub fn install(
    scheduler: Scheduler,
    hub: Arc<SessionHub>,
    paths: CronPaths,
    local: Option<Arc<LocalServices>>,
) -> Scheduler {
    // `Handle::current()` outside a runtime would panic; the sink is installed in `serve`
    // (and in tests) inside one. Captured here so the sync sink can spawn the async wake.
    let handle = tokio::runtime::Handle::current();
    scheduler.with_event_sink(Arc::new(move |job: &Job, run: &Run| {
        publish(local.as_deref(), job, run);
        let Some(link) = job.session_link.clone() else {
            return;
        };
        if link.target().is_none() {
            return;
        }
        let hub = Arc::clone(&hub);
        let paths = paths.clone();
        let (job, run) = (job.clone(), run.clone());
        handle.spawn(async move { wake(hub, paths, job, run).await });
    }))
}

/// Opens the job's linked session in the hub and prompts the wake message; records the
/// outcome on the run.
async fn wake(hub: Arc<SessionHub>, paths: CronPaths, job: Job, run: Run) {
    let Some(target) = job
        .session_link
        .as_ref()
        .and_then(|link| link.target())
        .map(str::to_string)
    else {
        return;
    };
    let outcome = match hub.open(&target).await {
        Ok(session) => {
            let message = wake_text(&job, &run);
            match session
                .send(RpcCommand::Agent(AgentCommand::Prompt {
                    message,
                    images: Vec::new(),
                    streaming_behavior: None,
                }))
                .await
            {
                Ok(_) => "sent".to_string(),
                Err(error) => format!("send failed: {error}"),
            }
        }
        Err(error) => format!("session not found: {error}"),
    };
    tracing::info!(
        op = "cron_wake",
        job = job.id,
        run = run.id,
        session = target,
        outcome = outcome,
        "wake"
    );
    // Best effort: the run record is already written; a missing one is not an error.
    if let Ok(Some(mut stored)) = store::load_run(&paths, &job.id, &run.id) {
        stored.wake = Some(outcome);
        let _ = store::save_run(&paths, &stored);
    }
}

/// Publishes the sticky run-finished event on the extension bus (no-op without extensions).
fn publish(local: Option<&LocalServices>, job: &Job, run: &Run) {
    let Some(local) = local else {
        return;
    };
    let Ok(publisher) = ExtensionId::new(PUBLISHER) else {
        return;
    };
    let payload = RawJson::from_json_value(&json!({
        "jobId": job.id,
        "runId": run.id,
        "status": run.status.as_str(),
        "summary": run.summary,
        "missed": run.missed,
    }));
    local.publish_host(&publisher, "", RUN_FINISHED_TOPIC, &payload, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_is_a_valid_extension_id() {
        assert!(ExtensionId::new(PUBLISHER).is_ok());
        assert_eq!(RUN_FINISHED_TOPIC, "unipi:cron:run-finished");
    }
}
