//! `/cron` subcommand handling: argument shapes, completions and the text rendering of
//! scheduler replies (ports unipi's `packages/utility/src/cron/index.ts`, Oino wording,
//! plus `add`/`edit`/`remove`).

use oino_extension::command::CompletionItem;
use oino_extension::ui;
use oino_extension::{context, host};
use serde_json::{Value, json};

use crate::Cron;

/// The subcommands.
pub const SUBCOMMANDS: [&str; 13] = [
    "list", "show", "runs", "run", "stop", "enable", "disable", "pause", "resume", "status", "add",
    "edit", "remove",
];

const SUB_DESCRIPTIONS: [(&str, &str); 13] = [
    ("list", "Every job: schedule, enabled/disabled, name"),
    ("show", "show <id> — one job's fields and recent runs"),
    ("runs", "runs <id> [N] — a job's run history, newest first"),
    ("run", "run <id> — fire it now, bypassing the schedule"),
    ("stop", "stop <id> — kill a running run"),
    ("enable", "enable <id>"),
    ("disable", "disable <id>"),
    ("pause", "Pause every job; running ones finish"),
    ("resume", "Resume after a pause"),
    ("status", "Next fire per job, what's running, paused or not"),
    (
        "add",
        "add name=\"…\" schedule=\"…\" prompt=\"…\" [cwd=…] — schedule a new job",
    ),
    ("edit", "edit <id> key=value… — change fields"),
    ("remove", "remove <id> — delete the job and its history"),
];

/// Subcommands that take a job id as their first argument.
const ID_ARG_SUBS: [&str; 7] = ["show", "runs", "run", "stop", "enable", "disable", "remove"];

/// Splits `/cron <sub> <rest…>` into the subcommand and remaining words.
#[must_use]
pub fn split_cron_args(args: &str) -> (String, Vec<String>) {
    let tokens = args
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let sub = tokens
        .first()
        .cloned()
        .unwrap_or_default()
        .to_ascii_lowercase();
    (sub, tokens.into_iter().skip(1).collect())
}

/// Parses `key="multi word value"` / `key=plain` assignments.
#[must_use]
pub fn parse_key_values(words: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut iter = words.iter().peekable();
    while let Some(word) = iter.next() {
        let Some((key, value)) = word.split_once('=') else {
            continue;
        };
        if let Some(quoted) = value.strip_prefix('"') {
            // Collect words until one closes the quote.
            let mut collected = quoted.to_owned();
            while !collected.ends_with('"') {
                match iter.next() {
                    Some(next) => {
                        collected.push(' ');
                        collected.push_str(next);
                    }
                    None => break,
                }
            }
            let collected = collected.strip_suffix('"').unwrap_or(&collected);
            if !key.is_empty() && !collected.is_empty() {
                out.push((key.to_owned(), collected.to_owned()));
            }
            continue;
        }
        if !key.is_empty() && !value.is_empty() {
            out.push((key.to_owned(), value.to_owned()));
        }
    }
    out
}

/// One of the `add`/`edit` fields the command understands.
fn job_payload(pairs: &[(String, String)], include_optional: bool) -> Value {
    let mut payload = json!({});
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    if include_optional {
        for key in ["name", "prompt"] {
            if let Some(v) = get(key) {
                payload[key] = Value::String(v);
            }
        }
    }
    if let Some(v) = get("schedule").or_else(|| get("at")) {
        payload["schedule"] = Value::String(v);
    }
    for key in ["tz", "cwd", "model", "thinking", "overlap", "missed"] {
        if let Some(v) = get(key) {
            payload[key] = Value::String(v);
        }
    }
    for key in ["id", "timeLimitS"] {
        if let Some(v) = get(key) {
            payload[key] = Value::String(v);
        }
    }
    for (key, target) in [("notify", "notifyChannels"), ("notifyOn", "notifyEvents")] {
        if let Some(v) = get(key).or_else(|| get(&key.to_ascii_lowercase())) {
            let list: Vec<Value> = v
                .split(',')
                .map(|p| Value::String(p.trim().to_owned()))
                .collect();
            payload[target] = Value::Array(list);
        }
    }
    if let Some(v) = get("costCapUsd") {
        payload["costCapUsd"] = Value::String(v);
    }
    payload
}

fn op_of(sub: &str) -> &'static str {
    match sub {
        "list" => "list",
        "show" => "show",
        "add" => "add",
        "edit" => "edit",
        "remove" => "remove",
        "runs" => "runs",
        "run" => "run",
        "stop" => "stop",
        "enable" => "enable",
        "disable" => "disable",
        "pause" => "pause",
        "resume" => "resume",
        _ => "status",
    }
}

/// `/cron <sub> <rest…>` dispatch; `sub` empty = the overview.
pub async fn run_subcommand(ext: &'static Cron, sub: &str, rest: &[String], _cx: &context::Ctx) {
    let notify = |message: &str, kind: &str| {
        let _ = ui::notify(message, kind);
    };
    if sub.is_empty() {
        let status = ext.op("status", json!({})).await;
        let list = ext.op("list", json!({})).await;
        match (status, list) {
            (Ok(status), Ok(list)) => notify(&render_overview(&status, &list), "info"),
            (Err(e), _) | (_, Err(e)) => notify(&unavailable(&e), "warning"),
        }
        return;
    }
    if !SUBCOMMANDS.contains(&sub) {
        notify(&crate::help_text(), "info");
        return;
    }
    if sub == "add" {
        run_add(ext, rest, notify).await;
        return;
    }
    if sub == "edit" {
        run_edit(ext, rest, notify).await;
        return;
    }
    let needs_id = ID_ARG_SUBS.contains(&sub);
    let id = rest.first().cloned();
    if needs_id && id.is_none() {
        notify(
            &format!("cron {sub} needs a job id — /cron list to see them"),
            "warning",
        );
        return;
    }
    let mut payload = json!({});
    if let Some(id) = &id {
        payload["id"] = Value::String(id.clone());
    }
    if sub == "runs"
        && let Some(limit) = rest.get(1)
        && let Ok(n) = limit.parse::<u64>()
    {
        payload["limit"] = json!(n);
    }
    let result = ext.op(op_of(sub), payload).await;
    match result {
        Ok(value) => notify(&render(sub, id.as_deref(), &value), "info"),
        Err(e) => notify(&unavailable(&e), "warning"),
    }
}

async fn run_add(ext: &'static Cron, rest: &[String], notify: impl Fn(&str, &str)) {
    let pairs = parse_key_values(rest);
    let mut payload = job_payload(&pairs, true);
    let missing: Vec<&str> = ["name", "prompt", "schedule"]
        .iter()
        .copied()
        .filter(|key| {
            payload
                .get(*key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
        })
        .collect();
    if !missing.is_empty() {
        notify(
            &format!(
                "/cron add needs {} — example: /cron add name=\"daily digest\" schedule=\"every day at 9:00\" prompt=\"Summarize today's inbox\" cwd=\"{}\"",
                missing
                    .iter()
                    .map(|k| format!("{k}=\"…\""))
                    .collect::<Vec<_>>()
                    .join(" "),
                default_cwd(),
            ),
            "warning",
        );
        return;
    }
    if payload.get("cwd").is_none() {
        payload["cwd"] = Value::String(default_cwd());
    }
    match ext.op("add", payload).await {
        Ok(job) => notify(
            &format!(
                "Added job {}.\n{}",
                job.get("id").and_then(Value::as_str).unwrap_or(""),
                schedule_summary(&job),
            ),
            "info",
        ),
        Err(e) => notify(&unavailable(&e), "warning"),
    }
}

async fn run_edit(ext: &'static Cron, rest: &[String], notify: impl Fn(&str, &str)) {
    let Some(id) = rest.first() else {
        notify(
            "/cron edit needs a job id — /cron list to see them",
            "warning",
        );
        return;
    };
    let pairs = parse_key_values(&rest[1..]);
    if pairs.is_empty() {
        notify(
            "/cron edit changes fields: schedule=\"…\" prompt=\"…\" name=\"…\" cwd=\"…\" model=\"…\" timeLimitS=… notifyOn=done,failed",
            "warning",
        );
        return;
    }
    let payload = job_payload(&pairs, true);
    let mut request = json!({ "id": id });
    if let Some(object) = payload.as_object() {
        for (key, value) in object {
            request[key] = value.clone();
        }
    }
    match ext.op("edit", request).await {
        Ok(job) => notify(
            &format!("Updated job {id}.\n{}", schedule_summary(&job)),
            "info",
        ),
        Err(e) => notify(&unavailable(&e), "warning"),
    }
}

fn default_cwd() -> String {
    host::workspace().cwd
}

fn unavailable(error: &str) -> String {
    if error.contains("not running") || error.contains("couldn't reach") {
        format!(
            "cron: the scheduler isn't running ({error}) — start it with `oino cron serve` or `oino cron install`"
        )
    } else {
        format!("cron: {error}")
    }
}

fn schedule_summary(job: &Value) -> String {
    let schedule = job.get("schedule").and_then(Value::as_str).unwrap_or("");
    let tz = job.get("tz").and_then(Value::as_str).unwrap_or("UTC");
    format!(
        "schedule: {schedule} ({tz})\nnext: {}",
        job.get("next_fire").and_then(Value::as_str).unwrap_or("-")
    )
}

fn render(sub: &str, id: Option<&str>, value: &Value) -> String {
    match sub {
        "list" => render_list(value),
        "show" => render_show(value),
        "runs" => render_runs(value),
        "run" => format!(
            "Started run {}",
            value.get("run_id").and_then(Value::as_str).unwrap_or("")
        ),
        "stop" => {
            if value.get("stopped").and_then(Value::as_bool) == Some(true) {
                "Stopped.".to_owned()
            } else {
                "Nothing was running.".to_owned()
            }
        }
        "enable" => format!("Enabled {}", id.unwrap_or("")),
        "disable" => format!("Disabled {}", id.unwrap_or("")),
        "pause" => "Paused: no new runs will start. Running ones finish.".to_owned(),
        "resume" => "Resumed.".to_owned(),
        "status" => render_status(value),
        "remove" => format!(
            "Removed {}",
            value.get("removed").and_then(Value::as_str).unwrap_or("")
        ),
        _ => serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

/// Bare `/cron`: every job with its next fire and most recent run (status ⊕ list).
pub fn render_overview(status: &Value, list: &Value) -> String {
    let mut lines = vec![format!(
        "{}  concurrency {}",
        if status.get("paused").and_then(Value::as_bool) == Some(true) {
            "paused"
        } else {
            "running"
        },
        status
            .get("concurrency")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )];
    let by_id: Vec<(String, &Value)> = list
        .get("jobs")
        .and_then(Value::as_array)
        .map(|jobs| {
            jobs.iter()
                .filter_map(|j| {
                    j.get("id")
                        .and_then(Value::as_str)
                        .map(|id| (id.to_owned(), j))
                })
                .collect()
        })
        .unwrap_or_default();
    let status_jobs = status
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if status_jobs.is_empty() {
        lines.push(
            r#"No cron jobs yet. Add one: /cron add name="daily digest" schedule="every day at 9:00" prompt="Summarize today's inbox""#
                .to_owned(),
        );
        return lines.join("\n");
    }
    for sj in &status_jobs {
        let sid = sj.get("id").and_then(Value::as_str).unwrap_or("");
        let job = by_id.iter().find(|(id, _)| id == sid).map(|(_, j)| *j);
        lines.push(format!(
            "{:<20} {}  next {}  {}  {}",
            sid,
            if sj.get("enabled").and_then(Value::as_bool) == Some(true) {
                "enabled "
            } else {
                "disabled"
            },
            sj.get("next_fire").and_then(Value::as_str).unwrap_or("-"),
            if sj.get("running").and_then(Value::as_bool) == Some(true) {
                "(running)"
            } else {
                ""
            },
            job.and_then(|j| j.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(""),
        ));
    }
    lines.join("\n")
}

pub(crate) fn render_list(value: &Value) -> String {
    let jobs = value
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if jobs.is_empty() {
        return r#"No cron jobs yet. Add one: /cron add name="daily digest" schedule="every day at 9:00" prompt="Summarize today's inbox""#
            .to_owned();
    }
    jobs.iter()
        .map(|job| {
            format!(
                "{:<20} {:<8} {}  {}  {}",
                job.get("id").and_then(Value::as_str).unwrap_or(""),
                if job.get("enabled").and_then(Value::as_bool) == Some(true) {
                    "enabled"
                } else {
                    "disabled"
                },
                job.get("schedule").and_then(Value::as_str).unwrap_or(""),
                job.get("tz").and_then(Value::as_str).unwrap_or(""),
                job.get("name").and_then(Value::as_str).unwrap_or(""),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_show(value: &Value) -> String {
    let job = value.get("job").cloned().unwrap_or(Value::Null);
    let mut lines = vec![
        format!(
            "{} — {}",
            job.get("id").and_then(Value::as_str).unwrap_or(""),
            job.get("name").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "  {}",
            if job.get("enabled").and_then(Value::as_bool) == Some(true) {
                "enabled"
            } else {
                "disabled"
            }
        ),
        format!(
            "  prompt: {}",
            job.get("prompt").and_then(Value::as_str).unwrap_or("")
        ),
        format!(
            "  cwd: {}",
            job.get("cwd").and_then(Value::as_str).unwrap_or("")
        ),
        format!("  {}", schedule_summary(&job).replace('\n', "\n  ")),
    ];
    if let Some(model) = job.get("model").and_then(Value::as_str) {
        lines.push(format!("  model: {model}"));
    }
    if let Some(cap) = job.get("cost_cap_usd").and_then(Value::as_f64) {
        lines.push(format!("  cost cap: ${cap:.2}"));
    }
    lines.push(String::new());
    lines.push("runs:".to_owned());
    let runs = value
        .get("runs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if runs.is_empty() {
        lines.push("  none yet".to_owned());
    }
    for run in runs.iter().take(10) {
        lines.push(format!(
            "  {:<12} {:<8} {}",
            run.get("id").and_then(Value::as_str).unwrap_or(""),
            run.get("status").and_then(Value::as_str).unwrap_or(""),
            run.get("summary")
                .and_then(Value::as_str)
                .or(run.get("reason").and_then(Value::as_str))
                .unwrap_or(""),
        ));
    }
    lines.join("\n")
}

pub(crate) fn render_runs(value: &Value) -> String {
    let runs = value
        .get("runs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if runs.is_empty() {
        return "No runs yet.".to_owned();
    }
    runs.iter()
        .map(|run| {
            format!(
                "{:<10} {:<12} {:<8} {}",
                run.get("job_id").and_then(Value::as_str).unwrap_or(""),
                run.get("id").and_then(Value::as_str).unwrap_or(""),
                run.get("status").and_then(Value::as_str).unwrap_or(""),
                run.get("summary")
                    .and_then(Value::as_str)
                    .or(run.get("reason").and_then(Value::as_str))
                    .unwrap_or(""),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn render_status(value: &Value) -> String {
    let mut lines = vec![format!(
        "{}  concurrency {}",
        if value.get("paused").and_then(Value::as_bool) == Some(true) {
            "paused"
        } else {
            "running"
        },
        value
            .get("concurrency")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )];
    for job in value
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        lines.push(format!(
            "  {:<20} {}  next {}  {}",
            job.get("id").and_then(Value::as_str).unwrap_or(""),
            if job.get("enabled").and_then(Value::as_bool) == Some(true) {
                "enabled "
            } else {
                "disabled"
            },
            job.get("next_fire").and_then(Value::as_str).unwrap_or("-"),
            if job.get("running").and_then(Value::as_bool) == Some(true) {
                "(running)"
            } else {
                ""
            },
        ));
    }
    lines.join("\n")
}

/// `/cron <TAB>` completions: subcommands, then a job id for the ones that take one.
pub async fn completions(ext: &'static Cron, prefix: &str) -> Vec<CompletionItem> {
    let raw = prefix.trim_start();
    if let Some((sub, _, partial)) = split_first_word(raw) {
        if ID_ARG_SUBS.contains(&sub.as_str()) || ["edit"].contains(&sub.as_str()) {
            let ids = ext.job_ids().await;
            return ids
                .into_iter()
                .filter(|id| id.starts_with(partial))
                .map(|id| CompletionItem {
                    value: format!("{sub} {id}"),
                    label: Some(id.clone()),
                    description: None,
                })
                .collect();
        }
        return Vec::new();
    }
    let needle = raw.to_ascii_lowercase();
    SUB_DESCRIPTIONS
        .iter()
        .filter(|(name, _)| name.starts_with(&needle))
        .map(|(name, description)| CompletionItem {
            value: (*name).to_owned(),
            label: None,
            description: Some((*description).to_owned()),
        })
        .collect()
}

fn split_first_word(text: &str) -> Option<(String, &str, &str)> {
    let trimmed = text.trim_start();
    let (first, rest) = trimmed.split_once(' ')?;
    let partial = rest.trim_start();
    Some((first.to_ascii_lowercase(), " ", partial))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sub_and_rest() {
        assert_eq!(
            split_cron_args("show digest"),
            ("show".to_owned(), vec!["digest".to_owned()])
        );
        assert_eq!(split_cron_args(""), (String::new(), Vec::<String>::new()));
        assert_eq!(
            split_cron_args("RUN digest"),
            ("run".to_owned(), vec!["digest".to_owned()])
        );
    }

    #[test]
    fn parses_quoted_and_plain_key_values() {
        let words: Vec<String> = [
            "name=\"Daily",
            "Digest\"",
            "prompt=\"Summarize",
            "the",
            "inbox\"",
            "schedule=hourly",
            "loose",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let pairs = parse_key_values(&words);
        assert_eq!(
            pairs,
            vec![
                ("name".to_owned(), "Daily Digest".to_owned()),
                ("prompt".to_owned(), "Summarize the inbox".to_owned()),
                ("schedule".to_owned(), "hourly".to_owned()),
            ]
        );
    }

    #[test]
    fn job_payload_maps_camel_and_plain_keys() {
        let words: Vec<String> = [
            "name=digest",
            "schedule=daily",
            "timeLimitS=300",
            "notifyOn=done,failed",
            "cwd=/tmp",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let payload = job_payload(&parse_key_values(&words), true);
        assert_eq!(payload["schedule"], "daily");
        assert_eq!(payload["timeLimitS"], "300");
        assert_eq!(payload["notifyEvents"], json!(["done", "failed"]));
        assert_eq!(payload["cwd"], "/tmp");
    }

    #[test]
    fn overview_renders_jobs_with_next_fires() {
        let status = json!({
            "paused": false,
            "concurrency": 2,
            "jobs": [
                {"id": "digest", "enabled": true, "next_fire": "2026-01-01T09:00:00+00:00", "running": false},
            ],
        });
        let list = json!({"jobs": [{"id": "digest", "name": "Daily Digest"}]});
        let text = render_overview(&status, &list);
        assert!(text.contains("running  concurrency 2"), "{text}");
        assert!(text.contains("digest"));
        assert!(text.contains("Daily Digest"));
    }

    #[test]
    fn unavailable_mentions_how_to_start_the_scheduler() {
        assert!(unavailable("the cron scheduler is not running").contains("oino cron install"));
        assert_eq!(unavailable("no job \"x\""), "cron: no job \"x\"");
    }
}
