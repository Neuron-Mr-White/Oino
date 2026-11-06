//! Dream state, runs, reports and proposals on disk (unipi `src/{schedule,runs,report}.ts`).
//!
//! Layout (guest paths, v3: one dream across every project): `/ext-global/state.json` holds
//! the schedule state; every run has a staging dir `/ext-global/runs/all/staging-<ms>/` with
//! `run.json`, `digests/`, `memory/before.json` + `memory/after.json` (the memory inventory
//! around the run), `changes.json` (added / updated / merged / pruned memories),
//! `memory-changes.json` (the agent's own account), `proposals/`, `PROMPT.md`,
//! `DREAM_REPORT.md`, `trajectory.log` and the dream agent's session files under
//! `sessions/`. Older per-workspace runs under `/ext-global/runs/<wsid>/` are still listed.
//!
//! Runs are nested agents (D8) in whichever process started them (an interactive session or
//! the headless `oino -p "/dream run --scheduled"` the host cron fires). The owner writes a
//! heartbeat into `run.json` every [`HEARTBEAT_MS`]; any process sees a run as `running` while
//! its heartbeat is fresh, and stops it by setting `stopped` (the owner aborts its agent). A
//! run left without an end and without a heartbeat is resumed once on the next start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::settings::DreamSettings;

/// Stale cap (2 h).
pub const STALE_MS: i64 = 2 * 60 * 60 * 1000;
/// How often the owning process refreshes a running run's heartbeat.
pub const HEARTBEAT_MS: i64 = 20_000;
/// A heartbeat younger than this means the run is alive in some process.
pub const HEARTBEAT_FRESH_MS: i64 = 75_000;

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
    /// The last scheduled/catch-up gate check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<LastCheck>,
}

/// One gate check of a scheduled (or catch-up) dream.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    /// When (unix ms).
    pub at: i64,
    /// `scheduled | catch-up`.
    pub trigger: String,
    /// A dream started.
    pub ran: bool,
    /// Why (not).
    pub reason: String,
}

impl DreamState {
    /// Folds a legacy per-workspace state's decisions and dismissals in (v3 moved state to
    /// the global dir).
    pub fn absorb_legacy(&mut self, legacy: &Self) {
        for (k, v) in &legacy.decisions {
            self.decisions.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for d in &legacy.dismissed {
            if !self.dismissed.contains(d) {
                self.dismissed.push(d.clone());
            }
        }
        self.last_run_at = self.last_run_at.max(legacy.last_run_at);
    }
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
    let fresh = f64::from(new_sessions);
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
    /// `manual | scheduled | catch-up` (absent on old runs: see `manual`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// Memory projects the run covered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
    /// Owner heartbeat (unix ms) while the agent runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heartbeat_at: Option<i64>,
}

impl RunMeta {
    /// The trigger, with old runs mapped from `manual`.
    #[must_use]
    pub fn trigger_name(&self) -> String {
        self.trigger
            .clone()
            .unwrap_or_else(|| if self.manual { "manual" } else { "open" }.to_owned())
    }

    /// The owner's heartbeat is fresh.
    #[must_use]
    pub fn heartbeat_fresh(&self, now: i64) -> bool {
        self.ended_at.is_none()
            && self
                .heartbeat_at
                .is_some_and(|h| now - h < HEARTBEAT_FRESH_MS)
    }
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
    /// `manual | scheduled | catch-up | open`.
    pub trigger: String,
    /// Memory projects covered.
    pub projects: Vec<String>,
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
    let live = ended.is_none()
        && ((now - started_at < STALE_MS && alive(&id))
            || meta.as_ref().is_some_and(|m| m.heartbeat_fresh(now)));
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
        trigger: meta
            .as_ref()
            .map_or_else(|| "open".to_owned(), RunMeta::trigger_name),
        projects: meta.map(|m| m.projects).unwrap_or_default(),
    })
}

/// Every run in every run dir under `runs_root` (`runs/all` + legacy `runs/<wsid>`),
/// newest first, dismissed dropped.
#[must_use]
pub fn list_all_runs(
    runs_root: &Path,
    state: &DreamState,
    now: i64,
    alive: &dyn Fn(&str) -> bool,
) -> Vec<Run> {
    let Ok(rd) = std::fs::read_dir(runs_root) else {
        return Vec::new();
    };
    let mut runs: Vec<Run> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .flat_map(|e| list_runs(&e.path(), state, now, alive))
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    runs
}

/// Ids of runs whose owner heartbeat is fresh (alive in some process).
#[must_use]
pub fn heartbeat_ids(runs_root: &Path, now: i64) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(runs_root) else {
        return out;
    };
    for sub in rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
    {
        let Ok(inner) = std::fs::read_dir(sub.path()) else {
            continue;
        };
        for e in inner.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("staging-")
                && read_meta(&e.path()).is_some_and(|m| m.heartbeat_fresh(now))
            {
                out.push(name);
            }
        }
    }
    out
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
        "trigger": r.trigger, "projects": r.projects,
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
    if let Some(c) = read_changes(&r.staging) {
        v["memoryChanges"] = c.get("counts").cloned().unwrap_or(Value::Null);
    }
    v
}

/// `changes.json` of a run.
#[must_use]
pub fn read_changes(staging: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(staging.join("changes.json")).ok()?).ok()
}

/// The agent's own `memory-changes.json` (an array; `[]` when missing or malformed).
#[must_use]
pub fn read_actions(staging: &Path) -> Vec<Value> {
    std::fs::read(staging.join("memory-changes.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| match v {
            Value::Array(a) => Some(a),
            Value::Object(o) => o.get("changes").and_then(Value::as_array).cloned(),
            _ => None,
        })
        .unwrap_or_default()
}

fn memory_key(m: &Value) -> Option<String> {
    Some(format!(
        "{}/{}",
        m.get("project")?.as_str()?,
        m.get("id")?.as_str()?
    ))
}

fn brief(m: &Value) -> Value {
    json!({
        "project": m.get("project"), "id": m.get("id"), "title": m.get("title"), "type": m.get("type"),
    })
}

/// The memory changes of a run from the inventories before and after it (ground truth;
/// the agent's `memory-changes.json` only explains them):
/// `added` (new, not a merge target), `updated` (rewritten in place), `merged` (a memory
/// whose `mergedFrom` absorbed removed ones), `pruned` (removed, not merged anywhere).
#[must_use]
pub fn diff_inventory(before: &Value, after: &Value) -> Value {
    let rows = |v: &Value| -> BTreeMap<String, Value> {
        v.get("memories")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| memory_key(m).map(|k| (k, m.clone())))
            .collect()
    };
    let (b, a) = (rows(before), rows(after));
    let removed: Vec<&String> = b.keys().filter(|k| !a.contains_key(*k)).collect();
    let mut merged_away: Vec<String> = Vec::new();
    let (mut added, mut updated, mut merged) = (Vec::new(), Vec::new(), Vec::new());
    for (k, m) in &a {
        let from: Vec<&String> = m
            .get("mergedFrom")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|f| removed.iter().copied().find(|r| r.as_str() == f))
            .collect();
        let old = b.get(k);
        if !from.is_empty() {
            let mut e = brief(m);
            e["from"] = json!(from.iter().map(|f| brief(&b[*f])).collect::<Vec<_>>());
            e["new"] = json!(old.is_none());
            merged.push(e);
            merged_away.extend(from.into_iter().cloned());
        } else if old.is_none() {
            added.push(brief(m));
        } else if old.and_then(|o| o.get("updated")) != m.get("updated") {
            updated.push(brief(m));
        }
    }
    let pruned: Vec<Value> = removed
        .iter()
        .filter(|k| !merged_away.contains(k))
        .map(|k| brief(&b[*k]))
        .collect();
    json!({
        "counts": {"added": added.len(), "updated": updated.len(), "merged": merged.len(), "pruned": pruned.len()},
        "added": added, "updated": updated, "merged": merged, "pruned": pruned,
    })
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
            min_sessions: 5.0,
            min_gap_hours: 12.0,
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
        assert_eq!(d.reason, "20 new sessions since last dream");
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

    #[test]
    fn inventory_diff() {
        let m = |p: &str, id: &str, updated: &str, merged: &[&str]| json!({"project": p, "id": id, "title": id.to_uppercase(), "type": "decision", "updated": updated, "mergedFrom": merged});
        let before = json!({"memories": [
            m("p", "a", "1", &[]), m("p", "b", "1", &[]), m("p", "c", "1", &[]),
            m("p", "d", "1", &[]), m("p", "keep", "1", &[]), m("p", "edit", "1", &[]),
        ]});
        let after = json!({"memories": [
            m("p", "ab", "2", &["p/a", "p/b"]),
            m("p", "d", "2", &["p/c", "p/old-gone-long-ago"]),
            m("p", "keep", "1", &[]), m("p", "edit", "2", &[]), m("p", "new", "2", &[]),
        ]});
        let c = diff_inventory(&before, &after);
        assert_eq!(
            c["counts"],
            json!({"added": 1, "updated": 1, "merged": 2, "pruned": 0}),
            "{c}"
        );
        let ab = c["merged"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "ab")
            .unwrap();
        assert_eq!(ab["new"], true);
        assert_eq!(ab["from"].as_array().unwrap().len(), 2);
        let d = c["merged"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "d")
            .unwrap();
        assert_eq!(d["new"], false);
        assert_eq!(d["from"][0]["id"], "c");
        assert_eq!(c["added"][0]["id"], "new");
        assert_eq!(c["updated"][0]["id"], "edit");
        let pruned = diff_inventory(&before, &json!({"memories": [m("p", "keep", "1", &[])]}));
        assert_eq!(pruned["counts"]["pruned"], 5);
        assert_eq!(
            diff_inventory(&Value::Null, &Value::Null)["counts"]["added"],
            0
        );
    }

    #[test]
    fn heartbeat_liveness_and_global_listing() {
        let root = tmp("beat");
        let all = root.join("all");
        let legacy = root.join("abc123");
        let s1 = all.join("staging-1000");
        let s2 = legacy.join("staging-500");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::create_dir_all(&s2).unwrap();
        let now = 1000 + 60_000;
        write_meta(
            &s1,
            &RunMeta {
                started_at: 1000,
                heartbeat_at: Some(now - 10_000),
                trigger: Some("scheduled".into()),
                projects: vec!["oino".into()],
                ..RunMeta::default()
            },
        );
        write_meta(
            &s2,
            &RunMeta {
                started_at: 500,
                manual: true,
                ended_at: Some(900),
                ..RunMeta::default()
            },
        );
        std::fs::write(s2.join("DREAM_REPORT.md"), "## Memory edits\n").unwrap();
        std::fs::write(
            s2.join("changes.json"),
            json!({"counts": {"added": 1, "updated": 0, "merged": 2, "pruned": 3}}).to_string(),
        )
        .unwrap();
        let st = DreamState::default();
        let runs = list_all_runs(&root, &st, now, &|_| false);
        assert_eq!(runs.len(), 2);
        assert_eq!(
            runs[0].status,
            Status::Running,
            "alive by heartbeat in another process"
        );
        assert!(!runs[0].interrupted);
        assert_eq!(runs[0].trigger, "scheduled");
        assert_eq!(runs[0].projects, ["oino"]);
        assert_eq!(runs[1].trigger, "manual", "old runs map from `manual`");
        assert_eq!(heartbeat_ids(&root, now), ["staging-1000"]);
        let info = run_info(&runs[1]);
        assert_eq!(info["memoryChanges"]["pruned"], 3);
        assert_eq!(info["trigger"], "manual");
        // The heartbeat goes stale: the run is interrupted (resumable).
        let later = list_all_runs(&root, &st, now + HEARTBEAT_FRESH_MS + 1, &|_| false);
        assert!(later[0].interrupted);
        assert!(heartbeat_ids(&root, now + HEARTBEAT_FRESH_MS + 1).is_empty());
    }

    #[test]
    fn legacy_state_folds_in() {
        let mut g = DreamState {
            last_run_at: 5,
            ..DreamState::default()
        };
        g.decisions.insert("a".into(), "approved".into());
        let mut l = DreamState {
            last_run_at: 9,
            dismissed: vec!["x".into()],
            ..DreamState::default()
        };
        l.decisions.insert("a".into(), "rejected".into());
        l.decisions.insert("b".into(), "rejected".into());
        g.absorb_legacy(&l);
        assert_eq!(g.decisions["a"], "approved", "global wins");
        assert_eq!(g.decisions["b"], "rejected");
        assert_eq!(g.dismissed, ["x"]);
        assert_eq!(g.last_run_at, 9);
        g.absorb_legacy(&l);
        assert_eq!(g.dismissed.len(), 1, "idempotent");
    }
}
