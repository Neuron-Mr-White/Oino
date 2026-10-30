//! The `"cron"` host service (v2 ext-abi §6.6): the [`HostService`] an embedding host
//! (unipi-host, WP-H8) registers under the name `"cron"` so the `unipi.cron` extension's
//! live-scheduler ops (`run`, `stop`, `pause`, `resume`, `status`, `settings-get`,
//! `settings-set`, `test-notify`) reach the embedded scheduler. Payloads and replies are
//! today's `control.rs` shapes; the storage ops (`add`/`edit`/… ) are accepted too (the
//! same handler backs the `unipi-host cron` CLI) even though the extension reads and
//! writes the job files directly.

use std::sync::Arc;

use oino_extension_api::HostError;
use oino_extension_api::RawJson;
use oino_extension_api::host_service::HostService;
use oino_extension_api::runtime::BoxFut;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::control::CronControl;
use crate::paths::CronPaths;
use crate::scheduler::Scheduler;

/// Every op name the service accepts (kebab-case, matching the `CronControl` serde tags;
/// `run`/`pause`/`resume` are shortened forms of `run-now`/`pause-all`/`resume-all`).
pub const OPS: [&str; 18] = [
    "list",
    "show",
    "add",
    "edit",
    "remove",
    "enable",
    "disable",
    "run",
    "stop",
    "runs",
    "log",
    "pause",
    "resume",
    "status",
    "settings-get",
    "settings-set",
    "test-notify",
    "parse",
];

/// Decodes one op + payload into a [`CronControl`] request.
///
/// # Errors
/// A user-facing message for unknown ops or bad payloads.
pub fn control_from_op(op: &str, payload: &Value) -> Result<CronControl, String> {
    let tag = match op {
        "run" => "run-now",
        "pause" => "pause-all",
        "resume" => "resume-all",
        other => OPS
            .iter()
            .find(|o| **o == other)
            .ok_or_else(|| format!("unknown cron op {op:?}"))?,
    };
    let mut value = payload.clone();
    if !value.is_object() {
        value = Value::Object(serde_json::Map::new());
    }
    if let Some(object) = value.as_object_mut() {
        object.insert("op".to_owned(), Value::String(tag.to_owned()));
    }
    serde_json::from_value(value).map_err(|e| format!("bad cron request for {op:?}: {e}"))
}

/// Runs an op directly against the store (and an optional live scheduler).
///
/// # Errors
/// [`HostError::Failed`] carries the scheduler's user-facing message.
pub async fn call_local(
    paths: &CronPaths,
    scheduler: Option<&Scheduler>,
    op: &str,
    payload: RawJson,
) -> Result<RawJson, HostError> {
    let request = control_from_op(op, &payload.to_value()).map_err(failed)?;
    let reply = crate::control::handle_control(paths, scheduler, request)
        .await
        .map_err(|e| failed(e.to_string()))?;
    Ok(RawJson::from_json_value(&reply))
}

fn failed(message: impl Into<String>) -> HostError {
    HostError::Failed(message.into())
}

/// The `"cron"` host service over an embedded (in-process) scheduler: what unipi-host
/// registers (`LocalServices::register_host_service("cron", …)`), and what the
/// `unipi-host cron` CLI's live ops run against.
pub struct CronHostService {
    /// The ticking scheduler (owned by the embedder's [`crate::embedded::Embedded`]).
    pub scheduler: Arc<Scheduler>,
}

impl CronHostService {
    /// A service over `scheduler`.
    #[must_use]
    pub fn new(scheduler: Arc<Scheduler>) -> Self {
        Self { scheduler }
    }

    /// The paths the service operates on.
    #[must_use]
    pub fn paths(&self) -> &CronPaths {
        self.scheduler.paths()
    }
}

impl HostService for CronHostService {
    fn call(
        &self,
        op: &str,
        payload: RawJson,
        _cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        let paths = self.scheduler.paths().clone();
        let scheduler = Arc::clone(&self.scheduler);
        let op = op.to_owned();
        Box::pin(async move { call_local(&paths, Some(&scheduler), &op, payload).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_ops_with_shortened_names() {
        assert!(matches!(
            control_from_op("run", &json!({"id": "j"})).unwrap(),
            CronControl::RunNow { .. }
        ));
        assert!(matches!(
            control_from_op("pause", &json!({})).unwrap(),
            CronControl::PauseAll
        ));
        assert!(matches!(
            control_from_op("show", &json!({"id": "j"})).unwrap(),
            CronControl::Show { .. }
        ));
        assert!(matches!(
            control_from_op("parse", &json!({"schedule": "hourly", "count": 2})).unwrap(),
            CronControl::Parse { .. }
        ));
        assert!(control_from_op("wat", &json!({})).is_err());
        // A non-object payload is treated as empty.
        assert!(matches!(
            control_from_op("status", &json!(null)).unwrap(),
            CronControl::Status
        ));
    }

    #[tokio::test]
    async fn serves_status_and_settings_over_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CronPaths::new(tmp.path());
        let service = CronHostService::new(Arc::new(Scheduler::with_paths(paths.clone())));
        let reply = service
            .call(
                "status",
                RawJson::from_json_value(&json!({})),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let value = reply.to_value();
        assert!(value.get("owner").is_some(), "a live scheduler owns");
        let reply = service
            .call(
                "settings-set",
                RawJson::from_json_value(&json!({"concurrency": 4})),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(reply.to_value()["concurrency"], 4);
    }
}
