//! The `cron_*` agent tools: let the model list, create, update, remove and fire jobs
//! when the user asks. The write tools are subject to the normal permission gate
//! (`unipi.workflow` raises an approval dialog in ask/auto mode); `cron_list` is
//! read-only. Job management goes to the files directly (works under any host);
//! `cron_run` needs the live scheduler (`cron` host service — unipi-host).

use oino_extension::prelude::*;
use oino_extension::schemars;
use oino_extension::schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::jobs::{self, Job, NotifySpec};
use crate::Cron;

fn policy(value: Option<&str>) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some("skip" | "allow" | "run-latest" | "latest") => Ok(Some(
            if value == Some("latest") {
                "run-latest"
            } else {
                value.unwrap_or_default()
            }
            .to_owned(),
        )),
        Some(other) => Err(format!(
            "unknown policy {other:?} — use skip/allow (overlap) or run-latest/skip (missed)"
        )),
    }
}

fn notify_of(events: Option<&Vec<String>>, channels: Option<&Vec<String>>) -> Option<NotifySpec> {
    let has = events.is_some() || channels.is_some();
    has.then(|| NotifySpec {
        channels: channels.filter(|c| !c.is_empty()).cloned(),
        events: events.filter(|e| !e.is_empty()).cloned(),
    })
}

fn job_brief(job: &Job) -> String {
    let value = jobs::job_value(job);
    let mut lines = vec![
        format!("job {}", job.id),
        format!("  name: {}", job.name),
        format!("  schedule: {} ({})", job.schedule, job.tz),
        format!("  cwd: {}", job.cwd),
    ];
    if let Some(next) = value.get("next_fire").and_then(Value::as_str) {
        lines.push(format!("  next fire: {next}"));
    }
    lines.push(format!("  enabled: {}", job.enabled));
    lines.join("\n")
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
    /// "weekdays at 18:30", "every monday 08:00", "hourly", "daily"), a one-time
    /// "in 2 hours" / "tomorrow at 9", a 6-field cron (seconds first, e.g.
    /// "0 30 9 * * MON-FRI") or `at:<RFC3339>`.
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

/// Resolves a schedule as typed against `tz` (the same code the scheduler uses).
fn resolve(input: &str, tz: &str) -> Result<String, String> {
    jobs::resolve_schedule(input, tz)
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
                    let Some(job) = jobs::load(&id) else {
                        return Ok(ToolResult::text(format!("No cron job {id:?}.")));
                    };
                    let mut brief = job_brief(&job);
                    for run in jobs::runs(&id).iter().take(10) {
                        let line = format!(
                            "  run {} {} {}",
                            run.id,
                            run.status,
                            run.summary
                                .as_deref()
                                .or(run.reason.as_deref())
                                .unwrap_or("")
                        );
                        brief.push('\n');
                        brief.push_str(&line);
                    }
                    return Ok(ToolResult::text(brief));
                }
                let wanted = args.runs.unwrap_or(3) as usize;
                let mut lines = Vec::new();
                for job in jobs::list() {
                    lines.push(job_brief(&job));
                    if wanted > 0 {
                        for run in jobs::runs(&job.id).iter().take(wanted) {
                            lines.push(format!(
                                "  run {} {} {}",
                                run.id,
                                run.status,
                                run.summary
                                    .as_deref()
                                    .or(run.reason.as_deref())
                                    .unwrap_or("")
                            ));
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
                 \"every 5 minutes\", \"weekdays at 18:30\", \"hourly\", \"in 2 hours\"). \
                 The job file is written immediately; it fires once the Unipi host runs the \
                 scheduler (under a plain oino TUI nothing fires until then).",
            )
            .args::<AddArgs>()
            .run(|_ext, args: AddArgs, cx| async move {
                let tz = args.tz.unwrap_or_else(unipi_cron_schedule::system_tz_name);
                let schedule = resolve(&args.schedule, &tz)?;
                let cwd = args
                    .cwd
                    .unwrap_or_else(|| cx.cx.cwd().to_string_lossy().into_owned());
                let id = jobs::unique_id(&jobs::slugify(&args.name));
                let mut job = crate::new_job(id, args.name, args.prompt, schedule, tz, cwd);
                if let Some(model) = args.model.filter(|m| !m.is_empty()) {
                    job.model = Some(model);
                }
                if let Some(limit) = args.time_limit_s {
                    job.time_limit_s = limit;
                }
                if let Some(overlap) = policy(args.overlap.as_deref())? {
                    job.overlap = overlap;
                }
                if let Some(missed) = policy(args.missed.as_deref())? {
                    job.missed = missed;
                }
                job.notify = notify_of(args.notify_events.as_ref(), args.notify_channels.as_ref());
                resolve(&job.schedule, &job.tz)?;
                jobs::insert(&job)
                    .await
                    .map_err(|e| Error::msg(format!("cron: {e}")))?;
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
                let notify = notify_of(args.notify_events.as_ref(), args.notify_channels.as_ref());
                let schedule = match args.schedule {
                    Some(raw) => Some(resolve(&raw, "local")?),
                    None => None,
                };
                let result = jobs::update(&args.id, |job| {
                    if let Some(v) = args.name.clone() {
                        job.name = v;
                    }
                    if let Some(v) = args.prompt.clone() {
                        job.prompt = v;
                    }
                    if let Some(v) = schedule.clone() {
                        job.schedule = v;
                    }
                    if let Some(v) = args.cwd.clone() {
                        job.cwd = v;
                    }
                    if let Some(v) = args.model.clone() {
                        job.model = (!v.is_empty()).then_some(v);
                    }
                    if let Some(v) = args.thinking.clone() {
                        job.thinking = (!v.is_empty()).then_some(v);
                    }
                    if let Some(v) = args.tz.clone() {
                        job.tz = v;
                    }
                    if let Some(v) = args.enabled {
                        job.enabled = v;
                    }
                    if let Some(v) = args.time_limit_s {
                        job.time_limit_s = v;
                    }
                    if let Some(v) = overlap.clone() {
                        job.overlap = v;
                    }
                    if let Some(v) = missed.clone() {
                        job.missed = v;
                    }
                    if let Some(v) = notify.clone() {
                        job.notify = Some(v);
                    }
                    resolve(&job.schedule, &job.tz)?;
                    job.updated = jobs::now_ms();
                    Ok::<(), String>(())
                })
                .await;
                match result {
                    Ok(Some(())) => Ok(ToolResult::text(job_brief(
                        &jobs::load(&args.id).unwrap_or_else(|| missing(&args.id)),
                    ))),
                    Ok(None) => Ok(ToolResult::text(format!("No cron job {:?}.", args.id))),
                    Err(e) => Err(Error::msg(format!("cron: {e}"))),
                }
            }),
    );
    cx.tool(
        Tool::<Cron>::new("cron_remove")
            .description(
                "Delete a cron job and its file (its run history ages out with retention). \
                 Ask the user first — this cannot be undone.",
            )
            .args::<RemoveArgs>()
            .run(|_ext, args: RemoveArgs, _cx| async move {
                match jobs::remove(&args.id) {
                    Ok(true) => Ok(ToolResult::text(format!("Removed job {}.", args.id))),
                    Ok(false) => Ok(ToolResult::text(format!("No cron job {:?}.", args.id))),
                    Err(e) => Err(Error::msg(format!("cron: {e}"))),
                }
            }),
    );
    cx.tool(
        Tool::<Cron>::new("cron_run")
            .description(
                "Fire a cron job right now, bypassing its schedule (still subject to overlap \
                 and concurrency). Needs the live scheduler (the Unipi host). Ask the user \
                 first.",
            )
            .args::<RunArgs>()
            .run(|_ext, args: RunArgs, _cx| async move {
                if jobs::load(&args.id).is_none() {
                    return Ok(ToolResult::text(format!("No cron job {:?}.", args.id)));
                }
                let reply = crate::host_op("run", json!({"id": args.id})).await?;
                Ok(ToolResult::text(format!(
                    "Started run {} ({}). Check it with cron_list or `/cron show {}`.",
                    reply.get("run_id").and_then(Value::as_str).unwrap_or(""),
                    args.id,
                    args.id
                )))
            }),
    );
}

fn missing(id: &str) -> Job {
    crate::new_job(
        id.to_owned(),
        String::new(),
        String::new(),
        String::new(),
        "UTC".to_owned(),
        String::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_parses_known_values() {
        assert_eq!(policy(Some("skip")).unwrap().as_deref(), Some("skip"));
        assert_eq!(policy(Some("allow")).unwrap().as_deref(), Some("allow"));
        assert_eq!(
            policy(Some("run-latest")).unwrap().as_deref(),
            Some("run-latest")
        );
        assert_eq!(policy(None).unwrap(), None);
        assert!(policy(Some("wat")).is_err());
    }

    #[test]
    fn notify_payload_shape() {
        assert!(notify_of(None, None).is_none());
        assert_eq!(
            notify_of(Some(&vec!["done".to_owned()]), None),
            Some(NotifySpec {
                channels: None,
                events: Some(vec!["done".to_owned()]),
            })
        );
    }
}
