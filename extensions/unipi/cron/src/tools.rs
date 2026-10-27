//! The `cron_*` agent tools: let the model list, create, update, remove and fire jobs when
//! the user asks. The write tools are subject to the normal permission gate
//! (`unipi.workflow` raises an approval dialog in ask/auto mode); `cron_list` is read-only.

use oino_extension::cron;
use oino_extension::prelude::*;
use oino_extension::schemars;
use oino_extension::schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Cron;

/// One scheduler op, mapped to a tool error message.
async fn op(op: &str, payload: Value) -> Result<Value, String> {
    cron::call(op, &payload).await.map_err(|e| e.to_string())
}

fn brief(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}

// ------------------------------------------------------------ tools

/// `cron_list` args (all optional).
#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Show only this job.
    job_id: Option<String>,
    /// Include the N most recent runs per job (default 3).
    runs: Option<u32>,
}

/// `cron_add` args.
#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    /// Short human name ("daily inbox digest").
    name: String,
    /// The full, self-contained prompt the unattended run gets. Nobody will be there to
    /// answer questions: state the whole task, make choices for it, and say where the
    /// result goes (a file, a notification, …).
    prompt: String,
    /// A recurring schedule: plain language ("every 5 minutes", "every day at 9:00",
    /// "weekdays at 18:30", "every monday 08:00", "hourly", "daily") or a 6-field cron
    /// (seconds first, e.g. "0 30 9 * * MON-FRI").
    schedule: String,
    /// The job's working directory; must exist (default: this project's root).
    cwd: Option<String>,
    /// Model override, `provider/id`.
    model: Option<String>,
    /// IANA time zone for the schedule (default: the user's).
    tz: Option<String>,
    /// Wall-clock limit per run in seconds (default 1800).
    time_limit_s: Option<u64>,
    /// What a fire does while the previous run still goes: "skip" (default) or "allow".
    overlap: Option<String>,
    /// Missed-fire policy while the PC was off: "run-latest" (default) or "skip".
    missed: Option<String>,
    /// Run outcomes that notify: `done`, `failed` or `skipped` (default: failed only).
    notify_events: Option<Vec<String>>,
    /// Notification channels: any of "ntfy", "gotify", "telegram" (default: the user's
    /// enabled channels).
    notify_channels: Option<Vec<String>>,
}

/// `cron_update` args (`id` plus every optional change).
#[derive(Deserialize, JsonSchema)]
struct UpdateArgs {
    /// The job id.
    id: String,
    /// New name.
    name: Option<String>,
    /// New prompt.
    prompt: Option<String>,
    /// New schedule (plain language or 6-field cron).
    schedule: Option<String>,
    /// New working directory (must exist).
    cwd: Option<String>,
    /// New model override ("" clears).
    model: Option<String>,
    /// New thinking level override ("" clears).
    thinking: Option<String>,
    /// New IANA time zone.
    tz: Option<String>,
    /// Enable or disable the job.
    enabled: Option<bool>,
    /// New wall-clock limit per run.
    time_limit_s: Option<u64>,
    /// New overlap policy: "skip" or "allow".
    overlap: Option<String>,
    /// New missed-fire policy: "run-latest" or "skip".
    missed: Option<String>,
    /// New notification events ("[]" clears back to the default).
    notify_events: Option<Vec<String>>,
    /// New notification channels ("[]" clears back to the default).
    notify_channels: Option<Vec<String>>,
}

/// `cron_remove` args.
#[derive(Deserialize, JsonSchema)]
struct RemoveArgs {
    /// The job id.
    id: String,
}

/// `cron_run` args.
#[derive(Deserialize, JsonSchema)]
struct RunArgs {
    /// The job id.
    id: String,
}

/// Validates an overlap/missed policy value ("skip", "allow", "run-latest", "latest").
fn policy(value: Option<&str>) -> Result<Option<&'static str>, String> {
    match value {
        None => Ok(None),
        Some("skip") => Ok(Some("skip")),
        Some("allow") => Ok(Some("allow")),
        Some("run-latest" | "latest") => Ok(Some("run-latest")),
        Some(other) => Err(format!(
            "unknown policy {other:?} — use skip/allow (overlap) or run-latest/skip (missed)"
        )),
    }
}

fn notify_of(events: Option<&Vec<String>>, channels: Option<&Vec<String>>) -> Value {
    match (events, channels) {
        (None, None) => json!(null),
        (events, channels) => json!({
            "events": events,
            "channels": channels,
        }),
    }
}

/// Renders a stored-job reply for a tool result.
fn job_brief(job: &Value) -> String {
    let mut lines = vec![
        format!(
            "job {}",
            job.get("id").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "  name: {}",
            job.get("name").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "  schedule: {} ({})",
            job.get("schedule").and_then(Value::as_str).unwrap_or(""),
            job.get("tz").and_then(Value::as_str).unwrap_or("UTC")
        ),
        format!(
            "  cwd: {}",
            job.get("cwd").and_then(Value::as_str).unwrap_or("")
        ),
    ];
    if let Some(next) = job.get("next_fire").and_then(Value::as_str) {
        lines.push(format!("  next fire: {next}"));
    }
    if let Some(enabled) = job.get("enabled").and_then(Value::as_bool) {
        lines.push(format!("  enabled: {enabled}"));
    }
    lines.join("\n")
}

/// Registers the tools.
#[allow(
    clippy::too_many_lines,
    reason = "five flat tool registrations; each is one description + one payload builder"
)]
pub fn register(cx: &mut Init<Cron>) {
    cx.tool(
        Tool::<Cron>::new("cron_list")
            .description(
                "List the user's scheduled Oino runs (cron jobs): id, name, schedule, \
                 enabled, next fire, and recent run outcomes. Use when the user asks what is \
                 scheduled, when a job next runs, or what a past run did (pass job_id).",
            )
            .args::<ListArgs>()
            .run(|_ext, args: ListArgs, _cx| async move {
                if let Some(id) = args.job_id {
                    let reply = op("show", json!({"id": id})).await?;
                    return Ok(ToolResult::text(brief(&reply)));
                }
                let list = op("list", json!({})).await?;
                let wanted = args.runs.unwrap_or(3) as usize;
                let mut lines = Vec::new();
                for job in list
                    .get("jobs")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    lines.push(job_brief(&job));
                    if wanted > 0
                        && let Some(id) = job.get("id").and_then(Value::as_str)
                    {
                        {
                            let runs = op("runs", json!({"id": id, "limit": wanted})).await?;
                            for run in runs
                                .get("runs")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default()
                            {
                                lines.push(format!(
                                    "  run {} {} {}",
                                    run.get("id").and_then(Value::as_str).unwrap_or(""),
                                    run.get("status").and_then(Value::as_str).unwrap_or(""),
                                    run.get("summary")
                                        .and_then(Value::as_str)
                                        .or(run.get("reason").and_then(Value::as_str))
                                        .unwrap_or(""),
                                ));
                            }
                        }
                    }
                }
                if lines.is_empty() {
                    return Ok(ToolResult::text(
                        "No cron jobs yet. Create one with cron_add (ask the user first).",
                    ));
                }
                Ok(ToolResult::text(lines.join("\n")))
            }),
    );
    cx.tool(
        Tool::<Cron>::new("cron_add")
            .description(
                "Schedule a new unattended Oino run (cron job) — ask the user first and read \
                 back the prompt you are about to schedule: it runs with nobody there to \
                 answer questions. Plain-language schedules work (\"every day at 9:00\", \
                 \"every 5 minutes\", \"weekdays at 18:30\", \"hourly\"). The scheduler must be \
                 running (`oino cron serve` / `oino cron install`).",
            )
            .args::<AddArgs>()
            .run(|_ext, args: AddArgs, cx| async move {
                let overlap = policy(args.overlap.as_deref())?;
                let missed = policy(args.missed.as_deref())?;
                let mut payload = json!({
                    "name": args.name,
                    "prompt": args.prompt,
                    "schedule": args.schedule,
                });
                if let Some(cwd) = args.cwd {
                    payload["cwd"] = Value::String(cwd);
                } else if let Some(root) = cx.cx.cwd().to_str() {
                    payload["cwd"] = Value::String(root.to_owned());
                }
                if let Some(model) = args.model {
                    payload["model"] = Value::String(model);
                }
                if let Some(tz) = args.tz {
                    payload["tz"] = Value::String(tz);
                }
                if let Some(limit) = args.time_limit_s {
                    payload["timeLimitS"] = json!(limit);
                }
                if let Some(overlap) = overlap {
                    payload["overlap"] = Value::String(overlap.to_owned());
                }
                if let Some(missed) = missed {
                    payload["missed"] = Value::String(missed.to_owned());
                }
                let notify = notify_of(args.notify_events.as_ref(), args.notify_channels.as_ref());
                if !notify.is_null() {
                    payload["notify"] = notify;
                }
                let job = op("add", payload).await?;
                Ok(ToolResult::text(job_brief(&job)))
            }),
    );
    cx.tool(
        Tool::<Cron>::new("cron_update")
            .description(
                "Change an existing cron job (schedule, prompt, enabled, model, limits, \
                 notifications…). Ask the user first; only the fields you pass change.",
            )
            .args::<UpdateArgs>()
            .run(|_ext, args: UpdateArgs, _cx| async move {
                let overlap = policy(args.overlap.as_deref())?;
                let missed = policy(args.missed.as_deref())?;
                let mut payload = json!({"id": args.id});
                for (key, value) in [
                    ("name", args.name),
                    ("prompt", args.prompt),
                    ("schedule", args.schedule),
                    ("cwd", args.cwd),
                    ("model", args.model),
                    ("thinking", args.thinking),
                    ("tz", args.tz),
                ] {
                    if let Some(value) = value {
                        payload[key] = Value::String(value);
                    }
                }
                if let Some(enabled) = args.enabled {
                    payload["enabled"] = json!(enabled);
                }
                if let Some(limit) = args.time_limit_s {
                    payload["timeLimitS"] = json!(limit);
                }
                if let Some(overlap) = overlap {
                    payload["overlap"] = Value::String(overlap.to_owned());
                }
                if let Some(missed) = missed {
                    payload["missed"] = Value::String(missed.to_owned());
                }
                let notify = notify_of(args.notify_events.as_ref(), args.notify_channels.as_ref());
                if !notify.is_null() {
                    payload["notify"] = notify;
                }
                let job = op("edit", payload).await?;
                Ok(ToolResult::text(job_brief(&job)))
            }),
    );
    cx.tool(
        Tool::<Cron>::new("cron_remove")
            .description(
                "Delete a cron job and its run history. Ask the user first — this cannot be \
                 undone.",
            )
            .args::<RemoveArgs>()
            .run(|_ext, args: RemoveArgs, _cx| async move {
                op("remove", json!({"id": args.id})).await?;
                Ok(ToolResult::text(format!("Removed job {}.", args.id)))
            }),
    );
    cx.tool(
        Tool::<Cron>::new("cron_run")
            .description(
                "Fire a cron job right now, bypassing its schedule (still subject to overlap \
                 and concurrency). Ask the user first.",
            )
            .args::<RunArgs>()
            .run(|_ext, args: RunArgs, _cx| async move {
                let reply = op("run", json!({"id": args.id})).await?;
                Ok(ToolResult::text(format!(
                    "Started run {} ({}). Check it with cron_list or `/cron show {}`.",
                    reply.get("run_id").and_then(Value::as_str).unwrap_or(""),
                    args.id,
                    args.id
                )))
            }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_parses_known_values() {
        assert_eq!(policy(Some("skip")).unwrap(), Some("skip"));
        assert_eq!(policy(Some("allow")).unwrap(), Some("allow"));
        assert_eq!(policy(Some("run-latest")).unwrap(), Some("run-latest"));
        assert_eq!(policy(None).unwrap(), None);
        assert!(policy(Some("wat")).is_err());
    }

    #[test]
    fn notify_payload_shape() {
        assert!(notify_of(None, None).is_null());
        assert_eq!(
            notify_of(Some(&vec!["done".to_owned()]), None),
            json!({"events": ["done"], "channels": null})
        );
    }
}
