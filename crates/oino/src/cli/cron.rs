//! `oino cron …`: the cron scheduler's CLI (docs/cron.md).
//!
//! Storage ops (add/edit/remove/enable/disable/list/show/runs/log/pause/resume/settings)
//! go straight to `~/.oino/cron/` (no daemon needed); `run`/`stop` need the always-on
//! scheduler (`oino cron serve` or the installed user service) and are routed to it over
//! its control socket. Every op takes `--json`.

use oino_cron::client::ControlClient;
use oino_cron::control::{
    CronControl, EditJob, NewJob, SettingsPatch, handle_control, resolve_schedule_input,
};
use oino_cron::model::{MissedPolicy, NotifySpec, OverlapPolicy};
use oino_cron::paths::CronPaths;
use oino_cron::schedule::{Schedule, next_fires, resolve_tz};
use oino_cron::{install, serve};

use crate::print_stdout;

/// The subcommands.
const SUBCOMMANDS: [&str; 19] = [
    "serve",
    "install",
    "uninstall",
    "status",
    "list",
    "show",
    "add",
    "edit",
    "remove",
    "rm",
    "enable",
    "disable",
    "run",
    "stop",
    "runs",
    "log",
    "pause",
    "resume",
    "settings",
];

/// `oino cron …` entry point: `args` are the words after `cron`.
pub async fn run(args: &[String]) -> i32 {
    match execute(args).await {
        Ok(()) => 0,
        Err(message) => {
            crate::eprintln_line(&format!("Error: {message}"));
            1
        }
    }
}

async fn execute(args: &[String]) -> Result<(), String> {
    let paths = CronPaths::from_agent_dir().map_err(|e| e.to_string())?;
    let (sub, rest): (&str, &[String]) = match args.split_first() {
        Some((first, rest)) if SUBCOMMANDS.contains(&first.as_str()) => (first.as_str(), rest),
        _ => return Err(help_error()),
    };
    let mut flags = Flags::parse(rest)?;
    match sub {
        "serve" => {
            serve::serve(paths).await.map_err(|e| e.to_string())?;
            Ok(())
        }
        "install" => {
            let done = install::install().map_err(|e| e.to_string())?;
            print_stdout(&format!("{done}\n"));
            Ok(())
        }
        "uninstall" => {
            let done = install::uninstall().map_err(|e| e.to_string())?;
            print_stdout(&format!("{done}\n"));
            Ok(())
        }
        _ => {
            let request = build_request(sub, &mut flags)?;
            let needs_scheduler = matches!(
                request,
                CronControl::RunNow { .. } | CronControl::Stop { .. }
            );
            let value = if needs_scheduler {
                // Route to the daemon (its live run table).
                let client = ControlClient::connect(&paths).map_err(|e| e.to_string())?;
                client.send(request).await.map_err(|e| e.to_string())?
            } else {
                handle_control(&paths, None, request)
                    .await
                    .map_err(|e| e.to_string())?
            };
            render(sub, rest, &value);
            Ok(())
        }
    }
}

fn help_error() -> String {
    "usage: oino cron <command>\n\ncommands:\n  serve            run the always-on scheduler (foreground)\n  install          install it as a user service (login-started)\n  uninstall        remove the user service\n  status           scheduler state, next fire per job\n  list             every job\n  show <id>        one job and its recent runs\n  add              add a job (--name, --prompt, --schedule|--at, …)\n  edit <id>        change a job (same flags as add)\n  remove <id>      delete a job and its history\n  enable|disable <id>\n  run <id>         fire a job now\n  stop <id>        kill its running runs\n  runs [<id>] [--limit N]\n  log <run-id>     one run's record and output\n  pause | resume   pause every job / resume\n  settings         show or change scheduler settings\n\nevery command takes --json"
    .to_owned()
}

/// Simple `--flag value` / `--flag=value` parser over the remaining words.
struct Flags {
    values: Vec<(String, String)>,
    positional: Vec<String>,
    json: bool,
}

impl Flags {
    fn parse(words: &[String]) -> Result<Self, String> {
        let mut flags = Self {
            values: Vec::new(),
            positional: Vec::new(),
            json: false,
        };
        let mut iter = words.iter().peekable();
        while let Some(word) = iter.next() {
            if word == "--json" {
                flags.json = true;
            } else if let Some(long) = word.strip_prefix("--") {
                let (key, inline) = match long.split_once('=') {
                    Some((key, value)) => (key, Some(value)),
                    None => (long, None),
                };
                let value = match inline {
                    Some(v) => v.to_owned(),
                    None => iter
                        .next()
                        .cloned()
                        .ok_or_else(|| format!("{word} needs a value"))?,
                };
                flags.values.push((key.to_owned(), value));
            } else {
                flags.positional.push(word.clone());
            }
        }
        Ok(flags)
    }

    fn take(&mut self, key: &str) -> Option<String> {
        self.values
            .iter()
            .position(|(k, _)| k == key)
            .map(|i| self.values.remove(i).1)
    }

    fn take_list(&mut self, key: &str) -> Vec<String> {
        self.take(key)
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn take_parsed<T: std::str::FromStr>(&mut self, key: &str) -> Result<Option<T>, String> {
        self.take(key)
            .map(|v| {
                v.parse::<T>()
                    .map_err(|_| format!("--{key} needs a valid number, got {v:?}"))
            })
            .transpose()
    }

    fn take_u64(&mut self, key: &str) -> Result<Option<u64>, String> {
        self.take_parsed(key)
    }

    fn take_u32(&mut self, key: &str) -> Result<Option<u32>, String> {
        self.take_parsed(key)
    }

    fn take_usize(&mut self, key: &str) -> Result<Option<usize>, String> {
        self.take_parsed(key)
    }

    fn take_f64(&mut self, key: &str) -> Result<Option<f64>, String> {
        self.take(key)
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| format!("--{key} needs a number, got {v:?}"))
            })
            .transpose()
    }

    fn take_bool(&mut self, key: &str) -> Result<Option<bool>, String> {
        self.take(key)
            .map(|v| match v.as_str() {
                "true" | "yes" | "on" => Ok(true),
                "false" | "no" | "off" => Ok(false),
                _ => Err(format!("--{key} needs true or false, got {v:?}")),
            })
            .transpose()
    }
}

fn id_arg(flags: &mut Flags, sub: &str) -> Result<String, String> {
    flags
        .positional
        .first()
        .cloned()
        .ok_or_else(|| format!("cron {sub} needs a job id — `oino cron list` to see them"))
}

fn overlap_of(value: &str) -> Result<OverlapPolicy, String> {
    match value {
        "skip" => Ok(OverlapPolicy::Skip),
        "allow" => Ok(OverlapPolicy::Allow),
        _ => Err(format!("--overlap needs skip or allow, got {value:?}")),
    }
}

fn missed_of(value: &str) -> Result<MissedPolicy, String> {
    match value {
        "run-latest" | "run_latest" | "latest" => Ok(MissedPolicy::RunLatest),
        "skip" => Ok(MissedPolicy::Skip),
        _ => Err(format!("--missed needs run-latest or skip, got {value:?}")),
    }
}

/// `--notify a,b` / `--notify-on done,failed`; `"none"` in either list clears that half.
fn notify_spec(notify: &[String], notify_on: &[String]) -> Option<NotifySpec> {
    let clean = |values: &[String]| {
        values
            .iter()
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty() && v != "none")
            .collect::<Vec<_>>()
    };
    let channels = clean(notify);
    let events = clean(notify_on);
    let channels = (!channels.is_empty()).then_some(channels);
    let events = (!events.is_empty()).then_some(events);
    (channels.is_some() || events.is_some()).then_some(NotifySpec { channels, events })
}

#[allow(
    clippy::too_many_lines,
    reason = "one flat match over the subcommands; each arm is a few flag reads"
)]
fn build_request(sub: &str, flags: &mut Flags) -> Result<CronControl, String> {
    match sub {
        "status" => Ok(CronControl::Status),
        "list" => Ok(CronControl::List),
        "show" => Ok(CronControl::Show {
            id: id_arg(flags, sub)?,
        }),
        "add" => {
            let name = flags
                .take("name")
                .ok_or_else(|| "cron add needs --name".to_owned())?;
            let prompt = flags
                .take("prompt")
                .ok_or_else(|| "cron add needs --prompt".to_owned())?;
            let at = flags.take("at");
            let schedule = flags.take("schedule").or_else(|| at.clone()).ok_or_else(|| {
                r#"cron add needs --schedule (a recurring schedule) or --at (one-time) — example: --schedule "every day at 9:00" or --at "in 2 hours""#.to_owned()
            })?;
            let tz = flags.take("tz");
            let resolved =
                resolve_schedule_input(&schedule, at.is_some(), tz.as_deref().or(Some("")))?;
            let cwd = match flags.take("cwd") {
                Some(cwd) => std::fs::canonicalize(&cwd)
                    .map_err(|e| format!("--cwd {cwd:?}: {e}"))?
                    .to_string_lossy()
                    .into_owned(),
                None => std::env::current_dir()
                    .map_err(|e| {
                        format!("no --cwd given and the current directory is unreadable: {e}")
                    })?
                    .to_string_lossy()
                    .into_owned(),
            };
            let notify_values = flags.take_list("notify");
            let notify_on = flags.take_list("notify-on");
            Ok(CronControl::Add(Box::new(NewJob {
                id: flags.take("id"),
                name,
                prompt,
                schedule: resolved,
                tz,
                cwd: Some(cwd),
                model: flags.take("model"),
                thinking: flags.take("thinking"),
                enabled: flags.take_bool("disabled")?.map(|d| !d),
                time_limit_s: flags.take_u64("time-limit-s")?,
                cost_cap_usd: flags.take_f64("cost-cap")?,
                overlap: flags.take("overlap").deref_map(overlap_of)?,
                missed: flags.take("missed").deref_map(missed_of)?,
                notify: notify_spec(&notify_values, &notify_on),
            })))
        }
        "edit" => {
            let id = id_arg(flags, sub)?;
            let at = flags.take("at");
            let schedule = match flags.take("schedule").or_else(|| at.clone()) {
                Some(raw) => Some(resolve_schedule_input(&raw, at.is_some(), None)?),
                None => None,
            };
            let cwd = match flags.take("cwd") {
                Some(cwd) => Some(
                    std::fs::canonicalize(&cwd)
                        .map_err(|e| format!("--cwd {cwd:?}: {e}"))?
                        .to_string_lossy()
                        .into_owned(),
                ),
                None => None,
            };
            let notify_values = flags.take_list("notify");
            let notify_on = flags.take_list("notify-on");
            let notify_patch = if notify_values.is_empty() && notify_on.is_empty() {
                None
            } else {
                Some(notify_spec(&notify_values, &notify_on))
            };
            let disabled = flags.take_bool("disabled")?;
            Ok(CronControl::Edit {
                id,
                patch: EditJob {
                    name: flags.take("name"),
                    prompt: flags.take("prompt"),
                    schedule,
                    tz: flags.take("tz"),
                    cwd,
                    model: flags.take("model").map(Some),
                    thinking: flags.take("thinking").map(Some),
                    enabled: disabled.map(|d| !d),
                    time_limit_s: flags.take_u64("time-limit-s")?,
                    cost_cap_usd: flags.take_f64("cost-cap")?.map(Some),
                    overlap: flags.take("overlap").deref_map(overlap_of)?,
                    missed: flags.take("missed").deref_map(missed_of)?,
                    notify: notify_patch,
                },
            })
        }
        "remove" | "rm" => Ok(CronControl::Remove {
            id: id_arg(flags, sub)?,
        }),
        "enable" => Ok(CronControl::Enable {
            id: id_arg(flags, sub)?,
        }),
        "disable" => Ok(CronControl::Disable {
            id: id_arg(flags, sub)?,
        }),
        "run" => Ok(CronControl::RunNow {
            id: id_arg(flags, sub)?,
        }),
        "stop" => Ok(CronControl::Stop {
            id: id_arg(flags, sub)?,
        }),
        "runs" => Ok(CronControl::Runs {
            job_id: flags.positional.first().cloned(),
            limit: flags.take_usize("limit")?,
        }),
        "log" => Ok(CronControl::Log {
            run_id: id_arg(flags, sub)?,
        }),
        "pause" => Ok(CronControl::PauseAll),
        "resume" => Ok(CronControl::ResumeAll),
        "settings" => {
            let setting_keys = [
                "concurrency",
                "missed-window-s",
                "retention-runs",
                "retention-days",
                "default-time-limit-s",
                "paused",
                "notify-default-events",
            ];
            let set = setting_keys
                .iter()
                .any(|k| flags.values.iter().any(|(fk, _)| fk == k));
            let patch = SettingsPatch {
                concurrency: flags.take_u32("concurrency")?,
                missed_window_s: flags.take_u64("missed-window-s")?,
                retention_runs: flags.take_u32("retention-runs")?,
                retention_days: flags.take_u32("retention-days")?,
                default_time_limit_s: flags.take_u64("default-time-limit-s")?,
                paused: flags.take_bool("paused")?,
                notify_default_events: {
                    let events = flags.take_list("notify-default-events");
                    (!events.is_empty()).then_some(events)
                },
            };
            Ok(if set {
                CronControl::SettingsSet(patch)
            } else {
                CronControl::SettingsGet
            })
        }
        "test-notify" => Ok(CronControl::TestNotify {
            id: id_arg(flags, sub)?,
        }),
        _ => Err(help_error()),
    }
}

/// The schedule + next-3-fires line shown after `add`/`edit` (human mode).
fn schedule_summary(schedule: &str, tz_input: &str) -> String {
    let Ok(tz) = resolve_tz(tz_input) else {
        return format!("schedule: {schedule}");
    };
    let Ok(parsed) = Schedule::parse(schedule) else {
        return format!("schedule: {schedule}");
    };
    let fires = next_fires(&parsed, tz, oino_cron::schedule::now(), 3);
    if fires.is_empty() {
        return format!("schedule: {schedule} (no upcoming fire — already run, or in the past)");
    }
    let times = fires
        .iter()
        .map(|dt| {
            dt.with_timezone(&tz)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("schedule: {schedule}\nnext: {times}")
}

fn render(sub: &str, words: &[String], value: &serde_json::Value) {
    let json = words.iter().any(|w| w == "--json") || words.contains(&"--json=true".to_owned());
    if json {
        print_stdout(&format!(
            "{}\n",
            serde_json::to_string(value).unwrap_or_default()
        ));
        return;
    }
    let text = match sub {
        "list" => render_list(value),
        "show" => render_show(value),
        "add" | "edit" => {
            let job = value;
            let id = job["id"].as_str().unwrap_or("");
            let what = if sub == "add" { "Added" } else { "Updated" };
            format!(
                "{what} job {id}.\n{}",
                schedule_summary(
                    job["schedule"].as_str().unwrap_or(""),
                    job["tz"].as_str().unwrap_or("UTC")
                )
            )
        }
        "run" => format!("Started run {}", value["run_id"].as_str().unwrap_or("")),
        "stop" => {
            if value["stopped"].as_bool() == Some(true) {
                "Stopped.".to_owned()
            } else {
                "Nothing was running.".to_owned()
            }
        }
        "runs" => render_runs(value),
        "log" => render_log(value),
        "pause" => "Paused: no new runs will start. Running ones finish.".to_owned(),
        "resume" => "Resumed.".to_owned(),
        "status" => render_status(value),
        "settings" => render_settings(value),
        _ => serde_json::to_string_pretty(value).unwrap_or_default(),
    };
    print_stdout(&format!("{text}\n"));
}

fn render_list(value: &serde_json::Value) -> String {
    let jobs = value["jobs"].as_array().cloned().unwrap_or_default();
    if jobs.is_empty() {
        return r#"No cron jobs yet. Add one: oino cron add --name "daily digest" --prompt "Summarize today's inbox" --schedule "every day at 9:00""#.to_owned();
    }
    jobs.iter()
        .map(|job| {
            format!(
                "{:<20} {:<8} {}  {}  {}",
                job["id"].as_str().unwrap_or(""),
                if job["enabled"].as_bool().unwrap_or(false) {
                    "enabled"
                } else {
                    "disabled"
                },
                job["schedule"].as_str().unwrap_or(""),
                job["tz"].as_str().unwrap_or(""),
                job["name"].as_str().unwrap_or(""),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_show(value: &serde_json::Value) -> String {
    let job = &value["job"];
    let tz = job["tz"].as_str().unwrap_or("UTC");
    let mut lines = vec![
        format!(
            "{} — {}",
            job["id"].as_str().unwrap_or(""),
            job["name"].as_str().unwrap_or("")
        ),
        format!(
            "  {}",
            if job["enabled"].as_bool().unwrap_or(false) {
                "enabled"
            } else {
                "disabled"
            }
        ),
        format!("  prompt: {}", job["prompt"].as_str().unwrap_or("")),
        format!("  cwd: {}", job["cwd"].as_str().unwrap_or("")),
        schedule_summary(job["schedule"].as_str().unwrap_or(""), tz)
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    ];
    if let Some(model) = job["model"].as_str() {
        lines.push(format!("  model: {model}"));
    }
    if let Some(cap) = job["cost_cap_usd"].as_f64() {
        lines.push(format!("  cost cap: ${cap:.2}"));
    }
    lines.push(String::new());
    lines.push("runs:".to_owned());
    let runs = value["runs"].as_array().cloned().unwrap_or_default();
    if runs.is_empty() {
        lines.push("  none yet".to_owned());
    }
    for run in runs.iter().take(10) {
        lines.push(format!(
            "  {:<12} {:<8} {}",
            run["id"].as_str().unwrap_or(""),
            run["status"].as_str().unwrap_or(""),
            run["summary"]
                .as_str()
                .or(run["reason"].as_str())
                .unwrap_or(""),
        ));
    }
    lines.join("\n")
}

fn render_runs(value: &serde_json::Value) -> String {
    let runs = value["runs"].as_array().cloned().unwrap_or_default();
    if runs.is_empty() {
        return "No runs yet.".to_owned();
    }
    runs.iter()
        .map(|run| {
            format!(
                "{:<10} {:<12} {:<8} {}",
                run["job_id"].as_str().unwrap_or(""),
                run["id"].as_str().unwrap_or(""),
                run["status"].as_str().unwrap_or(""),
                run["summary"]
                    .as_str()
                    .or(run["reason"].as_str())
                    .unwrap_or(""),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_log(value: &serde_json::Value) -> String {
    let run = &value["run"];
    let header = format!(
        "{} [{}] job {}  exit {}  session {}  cost {}",
        run["id"].as_str().unwrap_or(""),
        run["status"].as_str().unwrap_or(""),
        value["job_id"].as_str().unwrap_or(""),
        run["exit_code"]
            .as_i64()
            .map_or_else(|| "-".to_owned(), |c| c.to_string()),
        run["session_path"].as_str().unwrap_or("-"),
        run["cost_usd"]
            .as_f64()
            .map_or_else(|| "-".to_owned(), |c| format!("${c:.4}")),
    );
    format!("{header}\n{}", value["log"].as_str().unwrap_or(""))
}

fn render_status(value: &serde_json::Value) -> String {
    let mut lines = vec![format!(
        "{}  concurrency {}",
        if value["paused"].as_bool().unwrap_or(false) {
            "paused"
        } else {
            "running"
        },
        value["concurrency"].as_i64().unwrap_or(0),
    )];
    for job in value["jobs"].as_array().cloned().unwrap_or_default() {
        lines.push(format!(
            "  {:<20} {}  next {}  {}",
            job["id"].as_str().unwrap_or(""),
            if job["enabled"].as_bool().unwrap_or(false) {
                "enabled "
            } else {
                "disabled"
            },
            job["next_fire"].as_str().unwrap_or("-"),
            if job["running"].as_bool().unwrap_or(false) {
                "(running)"
            } else {
                ""
            },
        ));
    }
    lines.join("\n")
}

fn render_settings(value: &serde_json::Value) -> String {
    format!(
        "concurrency {}  missed window {}s  keep {} runs / {} days  default limit {}s  {}",
        value["concurrency"].as_i64().unwrap_or(0),
        value["missed_window_s"].as_i64().unwrap_or(0),
        value["retention_runs"].as_i64().unwrap_or(0),
        value["retention_days"].as_i64().unwrap_or(0),
        value["default_time_limit_s"].as_i64().unwrap_or(0),
        if value["paused"].as_bool().unwrap_or(false) {
            "paused"
        } else {
            "running"
        },
    )
}

/// Helper: apply a function to an optional flag value (`Option<&str>`-shaped).
trait DerefMap {
    fn deref_map<T>(self, f: impl Fn(&str) -> Result<T, String>) -> Result<Option<T>, String>;
}

impl DerefMap for Option<String> {
    fn deref_map<T>(self, f: impl Fn(&str) -> Result<T, String>) -> Result<Option<T>, String> {
        self.map(|v| f(&v)).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_parse_values_lists_and_positionals() {
        let words: Vec<String> = [
            "add",
            "--name",
            "x",
            "--notify=ntfy,gotify",
            "--json",
            "pos",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        let mut flags = Flags::parse(&words).unwrap();
        assert!(flags.json);
        assert_eq!(flags.take("name").as_deref(), Some("x"));
        assert_eq!(flags.take_list("notify"), vec!["ntfy", "gotify"]);
        assert_eq!(flags.positional, vec!["add", "pos"]);
        assert!(Flags::parse(&["--name".to_owned()]).is_err());
    }

    #[test]
    fn notify_spec_cleans_and_clears() {
        assert_eq!(notify_spec(&[], &[]), None);
        let spec = notify_spec(
            &["NTFY".into(), "none".into()],
            &["done".into(), "Failed".into()],
        )
        .unwrap();
        assert_eq!(spec.channels.as_deref(), Some(&["ntfy".to_owned()][..]));
        assert_eq!(
            spec.events.as_deref(),
            Some(&["done".to_owned(), "failed".to_owned()][..])
        );
        // "none" alone clears back to defaults.
        assert_eq!(notify_spec(&["none".into()], &[]), None);
    }

    #[test]
    fn build_request_shapes_add_and_runs() {
        let mut flags = Flags::parse(&[
            "--name".to_owned(),
            "d".to_owned(),
            "--prompt".to_owned(),
            "p".to_owned(),
            "--schedule".to_owned(),
            "hourly".to_owned(),
            "--cwd".to_owned(),
            "/tmp".to_owned(),
        ])
        .unwrap();
        match build_request("add", &mut flags).unwrap() {
            CronControl::Add(new) => {
                assert_eq!(new.schedule, "0 0 * * * *"); // "hourly", resolved
                assert!(new.cwd.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        let mut flags =
            Flags::parse(&["job".to_owned(), "--limit".to_owned(), "5".to_owned()]).unwrap();
        match build_request("runs", &mut flags).unwrap() {
            CronControl::Runs { job_id, limit } => {
                assert_eq!(job_id.as_deref(), Some("job"));
                assert_eq!(limit, Some(5));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn settings_patch_is_detected() {
        let mut flags = Flags::parse(&["--concurrency".to_owned(), "3".to_owned()]).unwrap();
        assert!(matches!(
            build_request("settings", &mut flags).unwrap(),
            CronControl::SettingsSet(_)
        ));
        let mut flags = Flags::parse(&[]).unwrap();
        assert!(matches!(
            build_request("settings", &mut flags).unwrap(),
            CronControl::SettingsGet
        ));
    }
}
