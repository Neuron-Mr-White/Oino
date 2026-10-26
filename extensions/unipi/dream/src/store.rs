//! Dream state, runs, reports and proposals on disk (unipi `src/{schedule,runs,report}.ts`).
//!
//! Layout (guest paths): `/ext-workspace/<wsid>/state.json` holds the schedule state; every
//! run has a staging dir `/ext-global/runs/<wsid>/staging-<ms>/` with `run.json`,
//! `digests/`, `proposals/`, `PROMPT.md`, `DREAM_REPORT.md`, `trajectory.log` and the dream
//! agent's session files under `sessions/`. Runs are nested agents in this process (D8): a run
//! is `running` while its agent lives here; a run left without an end by a previous process is
//! resumed once on the next start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::settings::DreamSettings;

/// Stale cap (2 h).
pub const STALE_MS: i64 = 2 * 60 * 60 * 1000;

/// Schedule lock.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    /// Unused in Oino (in-process runs); kept for the unipi shape.
    pub pid: i64,
    /// Lock time.
    pub at: i64,
    /// Run id holding the lock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}

/// `state.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamState {
    /// Last run start.
    #[serde(default)]
    pub last_run_at: i64,
    /// Sessions counted at the last run.
    #[serde(default)]
    pub sessions_seen: u32,
    /// Lock.
    #[serde(default)]
    pub lock: Option<Lock>,
    /// Proposal decisions.
    #[serde(default)]
    pub decisions: BTreeMap<String, String>,
    /// Staging dir already announced.
    #[serde(default)]
    pub shown_report: Option<String>,
    /// Dismissed staging dir names.
    #[serde(default)]
    pub dismissed: Vec<String>,
}

/// Reads `state.json` (defaults on any problem).
#[must_use]
pub fn read_state(path: &Path) -> DreamState {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Writes `state.json` (best-effort).
pub fn write_state(path: &Path, s: &DreamState) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let _ = std::fs::write(path, serde_json::to_vec_pretty(s).unwrap_or_default());
}

/// Due check result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    /// Due now.
    pub due: bool,
    /// Why (not).
    pub reason: String,
    /// New sessions counted.
    pub new_sessions: u32,
}

/// The lock is held by a live run.
#[must_use]
pub fn lock_live(lock: Option<&Lock>, now: i64, alive: &dyn Fn(&str) -> bool) -> bool {
    lock.is_some_and(|l| now - l.at <= STALE_MS && l.run.as_deref().is_some_and(alive))
}

/// unipi `isDue`.
#[must_use]
pub fn is_due(
    state: &DreamState,
    new_sessions: u32,
    cfg: &DreamSettings,
    now: i64,
    alive: &dyn Fn(&str) -> bool,
) -> Due {
    let mk = |due: bool, reason: String| Due {
        due,
        reason,
        new_sessions,
    };
    if !cfg.enabled {
        return mk(false, "disabled in settings".into());
    }
    if lock_live(state.lock.as_ref(), now, alive) {
        let run = state
            .lock
            .as_ref()
            .and_then(|l| l.run.clone())
            .unwrap_or_default();
        return mk(false, format!("dream already running ({run})"));
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "millisecond spans fit f64 exactly"
    )]
    let elapsed_h = (now - state.last_run_at) as f64 / 3_600_000.0;
    if state.last_run_at > 0 && elapsed_h < cfg.min_gap_hours {
        return mk(
            false,
            format!(
                "min gap not reached ({elapsed_h:.1}h of {}h)",
                fmt_num(cfg.min_gap_hours)
            ),
        );
    }
    let fresh = f64::from(new_sessions.saturating_sub(state.sessions_seen));
    if fresh < cfg.min_sessions {
        return mk(
            false,
            format!(
                "needs {} more new sessions",
                fmt_num(cfg.min_sessions - fresh)
            ),
        );
    }
    mk(
        true,
        format!("{} new sessions since last dream", fmt_num(fresh)),
    )
}

/// JS-style number text (`5`, `1.5`).
#[must_use]
pub fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        #[allow(clippy::cast_possible_truncation, reason = "integral and bounded")]
        let i = v as i64;
        i.to_string()
    } else {
        v.to_string()
    }
}

/// `run.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMeta {
    /// Start.
    pub started_at: i64,
    /// Unused (in-process); `-1`.
    #[serde(default)]
    pub pid: i64,
    /// Started by hand.
    #[serde(default)]
    pub manual: bool,
    /// Sessions digested.
    #[serde(default)]
    pub sessions: u32,
    /// Events digested.
    #[serde(default)]
    pub events: u32,
    /// End.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// `0` ok, `124` timed out, `1` failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    /// Abort reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// Stopped by the user.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
    /// Times resumed after an interruption.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub resumes: u32,
    /// Last error text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if passes &T"
)]
fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// Reads `run.json`.
#[must_use]
pub fn read_meta(staging: &Path) -> Option<RunMeta> {
    serde_json::from_slice(&std::fs::read(staging.join("run.json")).ok()?).ok()
}

/// Writes `run.json`.
pub fn write_meta(staging: &Path, m: &RunMeta) {
    let _ = std::fs::write(
        staging.join("run.json"),
        serde_json::to_vec_pretty(m).unwrap_or_default(),
    );
}

/// Patches `run.json`.
pub fn patch_meta(staging: &Path, f: impl FnOnce(&mut RunMeta)) {
    if let Some(mut m) = read_meta(staging) {
        f(&mut m);
        write_meta(staging, &m);
    }
}

/// Proposal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// `<staging>/<entry>`.
    pub id: String,
    /// Name.
    pub name: String,
    /// `skill | check`.
    pub kind: String,
    /// Guest path.
    #[serde(skip)]
    pub path: PathBuf,
    /// `pending | approved | rejected`.
    pub decision: String,
}

/// Proposals left under `<staging>/proposals/`.
#[must_use]
pub fn list_proposals(staging: &Path, decisions: &BTreeMap<String, String>) -> Vec<Proposal> {
    let base = staging
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(staging.join("proposals")) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        let id = format!("{base}/{name}");
        let decision = decisions
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "pending".into());
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            if !p.join("SKILL.md").exists() {
                continue;
            }
            out.push(Proposal {
                id,
                name: name.strip_prefix("skill-").unwrap_or(&name).to_owned(),
                kind: "skill".into(),
                path: p,
                decision,
            });
        } else if let Some(stem) = name.strip_suffix(".md") {
            out.push(Proposal {
                id,
                name: stem.to_owned(),
                kind: "check".into(),
                path: p,
                decision,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Run status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Agent alive in this process.
    Running,
    /// Report written.
    Finished,
    /// Ended without a report.
    Failed,
    /// Stopped by the user.
    Stopped,
}

impl Status {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// One run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// `staging-<ms>`.
    pub id: String,
    /// Guest staging dir.
    pub staging: PathBuf,
    /// Status.
    pub status: Status,
    /// Start.
    pub started_at: i64,
    /// End.
    pub ended_at: Option<i64>,
    /// Manual.
    pub manual: bool,
    /// Sessions digested.
    pub sessions: u32,
    /// Events.
    pub events: u32,
    /// Report exists.
    pub has_report: bool,
    /// Proposals.
    pub proposals: Vec<Proposal>,
    /// Pending proposals.
    pub pending: usize,
    /// Failure detail.
    pub error: Option<String>,
    /// Left unfinished by an earlier process.
    pub interrupted: bool,
}

fn mtime_ms(p: &Path) -> Option<i64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    i64::try_from(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// The last `Error:`-style line of a log (clipped to 240 chars).
#[must_use]
pub fn log_error_line(text: &str) -> Option<String> {
    let start = text.len().saturating_sub(16 * 1024);
    let start = (start..=text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    let hit = text[start..]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .find(|l| {
            ["Error", "TypeError", "SyntaxError", "ReferenceError"]
                .iter()
                .any(|p| {
                    l.strip_prefix(p)
                        .is_some_and(|r| r.chars().next().is_none_or(|c| !c.is_alphanumeric()))
                })
        })?;
    let msg = hit.strip_prefix("Error:").map_or(hit, str::trim_start);
    Some(if msg.chars().count() > 240 {
        format!("{}…", msg.chars().take(239).collect::<String>())
    } else {
        msg.to_owned()
    })
}

/// Status of one staging dir.
#[must_use]
pub fn run_from_staging(
    staging: &Path,
    decisions: &BTreeMap<String, String>,
    now: i64,
    alive: &dyn Fn(&str) -> bool,
) -> Option<Run> {
    let id = staging.file_name()?.to_string_lossy().into_owned();
    let from_name: i64 = id.strip_prefix("staging-")?.parse().unwrap_or(0);
    let meta = read_meta(staging);
    let started_at = meta.as_ref().map_or(from_name, |m| m.started_at);
    if started_at == 0 {
        return None;
    }
    let report = staging.join("DREAM_REPORT.md");
    let has_report = report.exists();
    let ended = meta.as_ref().and_then(|m| m.ended_at);
    let live = ended.is_none() && now - started_at < STALE_MS && alive(&id);
    let stopped = meta.as_ref().is_some_and(|m| m.stopped);
    let status = if live {
        Status::Running
    } else if has_report {
        Status::Finished
    } else if stopped {
        Status::Stopped
    } else {
        Status::Failed
    };
    let interrupted = !live && ended.is_none() && !has_report && !stopped;
    let ended_at = if live {
        None
    } else {
        ended
            .or_else(|| has_report.then(|| mtime_ms(&report)).flatten())
            .or_else(|| mtime_ms(&staging.join("trajectory.log")))
            .or(Some(started_at))
    };
    let proposals = if has_report || staging.join("proposals").exists() {
        list_proposals(staging, decisions)
    } else {
        Vec::new()
    };
    let error = (status == Status::Failed).then(|| {
        let code = meta.as_ref().and_then(|m| m.exit_code);
        if interrupted {
            "interrupted — resumes on the next start".to_owned()
        } else if code == Some(124) {
            "timed out (max runtime)".to_owned()
        } else if let Some(c) = code {
            let why = meta.as_ref().and_then(|m| m.error.clone()).or_else(|| {
                std::fs::read_to_string(staging.join("trajectory.log"))
                    .ok()
                    .and_then(|t| log_error_line(&t))
            });
            why.map_or_else(
                || format!("exit {c} without a report"),
                |w| format!("exit {c}: {w}"),
            )
        } else if let Some(sig) = meta.as_ref().and_then(|m| m.signal.clone()) {
            format!("killed by {sig}")
        } else {
            "ended without a report".to_owned()
        }
    });
    Some(Run {
        pending: proposals.iter().filter(|p| p.decision == "pending").count(),
        id,
        staging: staging.to_path_buf(),
        status,
        started_at,
        ended_at,
        manual: meta.as_ref().is_some_and(|m| m.manual),
        sessions: meta.as_ref().map_or(0, |m| m.sessions),
        events: meta.as_ref().map_or(0, |m| m.events),
        has_report,
        proposals,
        error,
        interrupted,
    })
}

/// Every run under `root`, newest first, dismissed dropped.
#[must_use]
pub fn list_runs(
    root: &Path,
    state: &DreamState,
    now: i64,
    alive: &dyn Fn(&str) -> bool,
) -> Vec<Run> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut runs: Vec<Run> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("staging-") && !state.dismissed.contains(n))
        .filter_map(|n| run_from_staging(&root.join(n), &state.decisions, now, alive))
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    runs
}

/// A trajectory step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// `user | text | tool | error | result | log`.
    pub kind: String,
    /// Text.
    pub text: String,
}

fn one_line(s: &str, max: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max {
        format!("{}…", t.chars().take(max - 1).collect::<String>())
    } else {
        t
    }
}

fn tool_arg(args: &Value) -> String {
    for k in ["command", "path", "file_path", "query", "title", "pattern"] {
        if let Some(s) = args.get(k).and_then(Value::as_str) {
            return s.to_owned();
        }
    }
    if args.is_object() {
        args.to_string()
    } else {
        String::new()
    }
}

/// A session JSONL rendered as one-line steps.
#[must_use]
pub fn trajectory_from_session(text: &str) -> Vec<Step> {
    let mut out = Vec::new();
    let step = |k: &str, t: String| Step {
        kind: k.into(),
        text: t,
    };
    for line in text.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if e.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(m) = e.get("message") else { continue };
        match m.get("role").and_then(Value::as_str) {
            Some("user") => out.push(step("user", "dream prompt sent".into())),
            Some("assistant") => {
                for c in m
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    match c.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let t = c.get("text").and_then(Value::as_str).unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(step("text", one_line(t, 200)));
                            }
                        }
                        Some("toolCall") => out.push(step(
                            "tool",
                            one_line(
                                &format!(
                                    "{} {}",
                                    c.get("name").and_then(Value::as_str).unwrap_or("tool"),
                                    tool_arg(c.get("arguments").unwrap_or(&Value::Null))
                                ),
                                200,
                            ),
                        )),
                        _ => {}
                    }
                }
            }
            Some("toolResult") if m.get("isError") == Some(&Value::Bool(true)) => {
                let t: String = m
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(step(
                    "error",
                    one_line(
                        &format!(
                            "{} failed: {t}",
                            m.get("toolName").and_then(Value::as_str).unwrap_or("tool")
                        ),
                        200,
                    ),
                ));
            }
            _ => {}
        }
    }
    out
}

fn newest_session_file(staging: &Path) -> Option<PathBuf> {
    fn walk(d: &Path, depth: u32, found: &mut Vec<(i64, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) && depth < 3 {
                walk(&p, depth + 1, found);
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                found.push((mtime_ms(&p).unwrap_or(0), p));
            }
        }
    }
    let mut found = Vec::new();
    walk(&staging.join("sessions"), 0, &mut found);
    found.into_iter().max_by_key(|(m, _)| *m).map(|(_, p)| p)
}

/// Newest session file of a run (resume seed).
#[must_use]
pub fn resume_seed(staging: &Path) -> Option<PathBuf> {
    newest_session_file(staging)
}

fn tail_text(p: &Path, max: usize) -> String {
    let Ok(b) = std::fs::read(p) else {
        return String::new();
    };
    let start = b.len().saturating_sub(max);
    let text = String::from_utf8_lossy(&b[start..]).into_owned();
    if start > 0 {
        text.split_once('\n')
            .map_or(text.clone(), |(_, r)| r.to_owned())
    } else {
        text
    }
}

/// Live view: trajectory steps + log tail.
#[must_use]
pub fn run_trajectory(staging: &Path, max_lines: usize) -> (Vec<Step>, Vec<String>) {
    let steps = newest_session_file(staging)
        .map(|f| trajectory_from_session(&tail_text(&f, 512 * 1024)))
        .unwrap_or_default();
    let log: Vec<String> = tail_text(&staging.join("trajectory.log"), 32 * 1024)
        .replace('\r', "")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let skip = steps.len().saturating_sub(max_lines);
    let lskip = log.len().saturating_sub(max_lines.min(80));
    (steps[skip..].to_vec(), log[lskip..].to_vec())
}

/// Report text (`""` when none).
#[must_use]
pub fn read_report(staging: &Path) -> String {
    std::fs::read_to_string(staging.join("DREAM_REPORT.md")).unwrap_or_default()
}

/// Section sizes: `3 memory edits`, `2 proposals`, `1 skipped`.
#[must_use]
pub fn summarize_report(report: &str) -> Vec<String> {
    if report.trim().is_empty() {
        return Vec::new();
    }
    let mut sections: Vec<(&str, u32)> = Vec::new();
    let mut cur: Option<&str> = None;
    for line in report.lines() {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes >= 2 && line[hashes..].starts_with(char::is_whitespace) {
            let name = line[hashes..].trim().to_lowercase();
            cur = if name.starts_with("memory") {
                Some("memory edit")
            } else if name.starts_with("proposal") {
                Some("proposal")
            } else if name.starts_with("skipped") {
                Some("skipped")
            } else {
                None
            };
            if let Some(c) = cur {
                match sections.iter_mut().find(|(k, _)| *k == c) {
                    Some(s) => s.1 = 0,
                    None => sections.push((c, 0)),
                }
            }
            continue;
        }
        let t = line.trim_start();
        let bullet = t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("-\t")
            || t.starts_with("*\t")
            || {
                let d = t.chars().take_while(char::is_ascii_digit).count();
                d > 0 && t[d..].starts_with('.') && t[d + 1..].starts_with(char::is_whitespace)
            };
        if let (Some(c), true) = (cur, bullet)
            && let Some(s) = sections.iter_mut().find(|(k, _)| *k == c)
        {
            s.1 += 1;
        }
    }
    sections
        .into_iter()
        .map(|(k, n)| {
            if k == "skipped" {
                format!("{n} skipped")
            } else {
                format!("{n} {k}{}", if n == 1 { "" } else { "s" })
            }
        })
        .collect()
}

/// Result of craft-skill's `check` (ported from `skills/craft-skill/scripts/check`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Passed.
    pub ok: bool,
    /// Output lines.
    pub output: String,
}

fn secret_like(line: &str) -> bool {
    let c: Vec<char> = line.chars().collect();
    let run =
        |from: usize, ok: &dyn Fn(char) -> bool| c[from..].iter().take_while(|x| ok(**x)).count();
    for i in 0..c.len() {
        let at = |p: &str| {
            p.chars()
                .enumerate()
                .all(|(k, ch)| c.get(i + k) == Some(&ch))
        };
        let wd = |x: char| x.is_ascii_alphanumeric() || x == '_' || x == '-';
        if (at("sk-") && run(i + 3, &wd) >= 16)
            || (at("ghp_") && run(i + 4, &|x| x.is_ascii_alphanumeric()) >= 20)
            || (at("AKIA") && run(i + 4, &|x| x.is_ascii_digit() || x.is_ascii_uppercase()) >= 16)
            || ((at("xoxb-") || at("xoxp-"))
                && run(i + 5, &|x| x.is_ascii_alphanumeric() || x == '-') >= 10)
        {
            return true;
        }
        for key in ["TOKEN", "SECRET", "PASSWORD", "API_KEY"] {
            if !at(key) {
                continue;
            }
            let mut j = i + key.len();
            j += run(j, &|x| x.is_ascii_uppercase() || x == '_');
            if !matches!(c.get(j), Some('=' | ':')) {
                continue;
            }
            j += 1;
            j += run(j, &char::is_whitespace);
            if matches!(c.get(j), Some('"' | '\'')) {
                j += 1;
            }
            if run(j, &wd) >= 12 {
                return true;
            }
        }
    }
    false
}

fn walk_files(d: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            walk_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// craft-skill `check <dir>`: fails on a missing description or secret-like values outside
/// `.env`; `display` is the directory name shown in the output.
#[must_use]
pub fn craft_check(dir: &Path, display: &str) -> Check {
    let mut lines = Vec::new();
    let mut fail = false;
    let Ok(skill) = std::fs::read_to_string(dir.join("SKILL.md")) else {
        return Check {
            ok: false,
            output: "FAIL: no SKILL.md".into(),
        };
    };
    let mut fm = Vec::new();
    let mut inside = false;
    for l in skill.lines() {
        if l == "---" {
            if inside {
                break;
            }
            inside = true;
            continue;
        }
        if inside {
            fm.push(l);
        }
    }
    let desc = fm
        .iter()
        .find_map(|l| l.strip_prefix("description:"))
        .unwrap_or_default();
    if desc.is_empty() {
        lines.push("FAIL: no description".to_owned());
        fail = true;
    }
    if desc.chars().count() > 300 {
        lines.push(format!(
            "WARN: description {} chars; router reads first 200",
            desc.chars().count()
        ));
    }
    if dir.join(".env").exists() {
        let ignored = std::fs::read_to_string(dir.join(".gitignore"))
            .is_ok_and(|g| g.lines().any(|l| l == ".env"));
        if !ignored {
            lines.push(format!("FAIL: .env not in {display}/.gitignore"));
            fail = true;
        }
    }
    let mut files = Vec::new();
    walk_files(dir, &mut files);
    files.sort();
    let mut hits = Vec::new();
    for f in files {
        if f.file_name().is_some_and(|n| n == ".env") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let rel = f.strip_prefix(dir).unwrap_or(&f).display().to_string();
        for (n, l) in text.lines().enumerate() {
            if secret_like(l) {
                hits.push(format!("{display}/{rel}:{}:{l}", n + 1));
            }
        }
    }
    if !hits.is_empty() {
        lines.push("FAIL: secret-like value outside .env:".to_owned());
        lines.extend(hits);
        fail = true;
    }
    if !fail {
        lines.push(format!("PASS {display}"));
    }
    Check {
        ok: !fail,
        output: lines.join("\n"),
    }
}

/// Recursive copy.
///
/// # Errors
///
/// I/O failures.
pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// The first 12 lines of a check proposal (unipi `approveCheck`).
#[must_use]
pub fn check_head(p: &Proposal) -> Option<String> {
    let md = std::fs::read_to_string(&p.path).ok()?;
    Some(md.split('\n').take(12).collect::<Vec<_>>().join("\n"))
}

/// The status wire shape of a run (unipi `DreamRunInfo`).
#[must_use]
pub fn run_info(r: &Run) -> Value {
    let summary = if r.has_report {
        summarize_report(&read_report(&r.staging))
    } else {
        Vec::new()
    };
    let mut v = json!({
        "id": r.id, "status": r.status.as_str(), "startedAt": r.started_at, "manual": r.manual,
        "sessions": r.sessions, "events": r.events, "pending": r.pending, "hasReport": r.has_report,
        "proposals": r.proposals.iter().map(|p| json!({"id": p.id, "name": p.name, "kind": p.kind, "decision": p.decision})).collect::<Vec<_>>(),
    });
    if let Some(e) = r.ended_at {
        v["endedAt"] = json!(e);
    }
    if !summary.is_empty() {
        v["summary"] = json!(summary);
    }
    if let Some(e) = &r.error {
        v["error"] = json!(e);
    }
    v
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("dream-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        let _ = std::fs::create_dir_all(&p);
        p
    }

    fn cfg() -> DreamSettings {
        DreamSettings {
            enabled: true,
            ..DreamSettings::default()
        }
    }

    const H: i64 = 3_600_000;

    #[test]
    fn schedule() {
        let dead = |_: &str| false;
        let live = |_: &str| true;
        let now = 100 * H;
        let st = DreamState::default();
        assert_eq!(
            is_due(&st, 9, &DreamSettings::default(), now, &dead).reason,
            "disabled in settings"
        );
        assert!(is_due(&st, 5, &cfg(), now, &dead).due);
        let d = is_due(&st, 3, &cfg(), now, &dead);
        assert!(!d.due);
        assert_eq!(d.reason, "needs 2 more new sessions");
        let st = DreamState {
            last_run_at: now - 2 * H,
            ..DreamState::default()
        };
        assert_eq!(
            is_due(&st, 50, &cfg(), now, &dead).reason,
            "min gap not reached (2.0h of 12h)"
        );
        let st = DreamState {
            last_run_at: now - 13 * H,
            sessions_seen: 4,
            lock: Some(Lock {
                pid: 0,
                at: now - H,
                run: Some("staging-1".into()),
            }),
            ..DreamState::default()
        };
        assert!(!is_due(&st, 20, &cfg(), now, &live).due);
        let d = is_due(&st, 20, &cfg(), now, &dead);
        assert!(d.due);
        assert_eq!(d.reason, "16 new sessions since last dream");
        assert!(!lock_live(st.lock.as_ref(), now + 3 * H, &live));
        let p = tmp("state").join("state.json");
        write_state(&p, &st);
        assert_eq!(read_state(&p), st);
        assert_eq!(read_state(&p.with_extension("x")), DreamState::default());
    }

    #[test]
    fn runs_and_proposals() {
        let root = tmp("runs");
        let s1 = root.join("staging-1000");
        std::fs::create_dir_all(s1.join("proposals/skill-deploy")).unwrap_or_default();
        std::fs::write(
            s1.join("proposals/skill-deploy/SKILL.md"),
            "---\nname: deploy\ndescription: x\n---\n",
        )
        .unwrap_or_default();
        std::fs::write(s1.join("proposals/lint-check.md"), "# check\nline2").unwrap_or_default();
        std::fs::write(
            s1.join("DREAM_REPORT.md"),
            "## Memory edits\n- a\n- b\n## Proposals\n1. x\n## Skipped\n- y\n",
        )
        .unwrap_or_default();
        write_meta(
            &s1,
            &RunMeta {
                started_at: 1000,
                manual: true,
                sessions: 3,
                ..RunMeta::default()
            },
        );
        let s2 = root.join("staging-2000");
        std::fs::create_dir_all(&s2).unwrap_or_default();
        write_meta(
            &s2,
            &RunMeta {
                started_at: 2000,
                ..RunMeta::default()
            },
        );
        let s3 = root.join("staging-3000");
        std::fs::create_dir_all(&s3).unwrap_or_default();
        write_meta(
            &s3,
            &RunMeta {
                started_at: 3000,
                ended_at: Some(3500),
                exit_code: Some(124),
                ..RunMeta::default()
            },
        );
        let mut st = DreamState::default();
        st.decisions
            .insert("staging-1000/lint-check.md".into(), "rejected".into());
        let runs = list_runs(&root, &st, 5000, &|id| id == "staging-2000");
        assert_eq!(
            runs.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["staging-3000", "staging-2000", "staging-1000"]
        );
        assert_eq!(runs[0].status, Status::Failed);
        assert_eq!(runs[0].error.as_deref(), Some("timed out (max runtime)"));
        assert_eq!(runs[1].status, Status::Running);
        let r = &runs[2];
        assert_eq!(r.status, Status::Finished);
        assert_eq!(r.proposals.len(), 2);
        assert_eq!(r.proposals[0].name, "deploy");
        assert_eq!(r.proposals[0].kind, "skill");
        assert_eq!(r.pending, 1);
        assert_eq!(
            summarize_report(&read_report(&r.staging)),
            ["2 memory edits", "1 proposal", "1 skipped"]
        );
        let info = run_info(r);
        assert_eq!(info["status"], "finished");
        assert_eq!(info["summary"][0], "2 memory edits");
        let after = list_runs(&root, &st, 5000, &|_| false);
        assert!(after[1].interrupted);
        st.dismissed.push("staging-3000".into());
        assert_eq!(list_runs(&root, &st, 5000, &|_| false).len(), 2);
    }

    #[test]
    fn trajectory_and_errors() {
        let text = [
            json!({"type": "message", "message": {"role": "user", "content": "go"}}),
            json!({"type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": "thinking about it"}, {"type": "toolCall", "name": "bash", "arguments": {"command": "ls"}}]}}),
            json!({"type": "message", "message": {"role": "toolResult", "toolName": "bash", "isError": true, "content": [{"type": "text", "text": "boom"}]}}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let steps = trajectory_from_session(&text);
        assert_eq!(
            steps.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
            ["user", "text", "tool", "error"]
        );
        assert_eq!(steps[2].text, "bash ls");
        assert_eq!(steps[3].text, "bash failed: boom");
        assert_eq!(
            log_error_line("ok\nError: Failed to load extension x\nmore").as_deref(),
            Some("Failed to load extension x")
        );
        assert_eq!(
            log_error_line("TypeError: nope").as_deref(),
            Some("TypeError: nope")
        );
        assert_eq!(log_error_line("fine"), None);
    }

    #[test]
    fn craft_skill_check() {
        let d = tmp("check");
        assert_eq!(craft_check(&d, "x").output, "FAIL: no SKILL.md");
        std::fs::write(
            d.join("SKILL.md"),
            "---\nname: a\ndescription: Does a thing.\n---\nbody\n",
        )
        .unwrap_or_default();
        let c = craft_check(&d, "x");
        assert!(c.ok, "{}", c.output);
        assert_eq!(c.output, "PASS x");
        std::fs::write(d.join(".env"), "TOKEN=abcdefghijklmnop").unwrap_or_default();
        let c = craft_check(&d, "x");
        assert!(!c.ok && c.output.contains(".env not in x/.gitignore"));
        std::fs::write(d.join(".gitignore"), ".env\n").unwrap_or_default();
        assert!(craft_check(&d, "x").ok, ".env content is not scanned");
        std::fs::write(d.join("run.sh"), "API_KEY=\"abcdefghijklmnop\"\n").unwrap_or_default();
        let c = craft_check(&d, "x");
        assert!(!c.ok && c.output.contains("secret-like value outside .env"));
        assert!(secret_like("x sk-abcdefghijklmnopqrstu"));
        assert!(!secret_like("TOKEN=short"));
        std::fs::write(d.join("SKILL.md"), "---\nname: a\n---\n").unwrap_or_default();
        assert!(craft_check(&d, "x").output.contains("FAIL: no description"));
    }
}
