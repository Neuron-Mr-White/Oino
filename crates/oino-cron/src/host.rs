//! [`oino_extension_api::cron::CronHost`] adapters: the daemon client and an embedded
//! scheduler (the desktop app), plus the op-name → [`CronControl`] decoding shared with
//! the WIT `cron` capability.

use oino_extension_api::HostError;
use oino_extension_api::RawJson;
use oino_extension_api::cron::CronHost;
use oino_extension_api::runtime::BoxFut;
use serde_json::Value;

use crate::client::ControlClient;
use crate::control::CronControl;
use crate::paths::CronPaths;
use crate::scheduler::Scheduler;

/// Every op name the WIT `cron.call` accepts (kebab-case, matching the `CronControl` serde
/// tags; `run`/`pause`/`resume`/`settings-*` are shortened forms of `run-now`, …).
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

/// Decodes one WIT op + payload into a [`CronControl`] request.
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

/// Runs an op directly against the store (and an optional live scheduler): the embedded
/// host's core.
///
/// # Errors
/// [`HostError::Failed`] carries the scheduler's message.
pub async fn call_local(
    paths: &CronPaths,
    scheduler: Option<&Scheduler>,
    op: &str,
    payload: RawJson,
) -> Result<RawJson, HostError> {
    let request = control_from_op(op, &payload.to_value()).map_err(host_error_str)?;
    let reply = crate::control::handle_control(paths, scheduler, request)
        .await
        .map_err(|e| host_error(&e))?;
    Ok(RawJson::from_json_value(&reply))
}

fn host_error_str(message: String) -> HostError {
    HostError::Failed(message)
}

fn host_error(error: &crate::error::CronError) -> HostError {
    HostError::Failed(error.to_string())
}

/// [`CronHost`] over an embedded (in-process) scheduler: what the desktop installs.
pub struct EmbeddedCronHost {
    /// The ticking scheduler (owned by the embedder).
    pub scheduler: std::sync::Arc<Scheduler>,
}

impl CronHost for EmbeddedCronHost {
    fn call(&self, op: &str, payload: RawJson) -> BoxFut<'_, Result<RawJson, HostError>> {
        let paths = self.scheduler.paths().clone();
        let scheduler = self.scheduler.clone();
        let op = op.to_owned();
        Box::pin(async move { call_local(&paths, Some(&scheduler), &op, payload).await })
    }
}

/// [`CronHost`] over the running daemon's control socket: the default fallback.
pub struct DaemonCronHost {
    /// The cron paths (for `serve.json`).
    pub paths: CronPaths,
}

impl CronHost for DaemonCronHost {
    fn call(&self, op: &str, payload: RawJson) -> BoxFut<'_, Result<RawJson, HostError>> {
        let paths = self.paths.clone();
        let op = op.to_owned();
        Box::pin(async move {
            let request = control_from_op(&op, &payload.to_value()).map_err(host_error_str)?;
            // Prefer the running daemon (its live run table); storage-only ops fall back
            // to the store when it isn't running.
            match ControlClient::connect(&paths) {
                Ok(client) => {
                    let reply = client.send(request).await.map_err(|e| host_error(&e))?;
                    Ok(RawJson::from_json_value(&reply))
                }
                Err(not_running) => match request {
                    CronControl::RunNow { .. } | CronControl::Stop { .. } => {
                        Err(host_error(&not_running))
                    }
                    _ => call_local(&paths, None, &op, payload)
                        .await
                        .map_err(|_| host_error(&not_running)),
                },
            }
        })
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
}
