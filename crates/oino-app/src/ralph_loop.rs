#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub const RALPH_DIR: &str = "ralph";
pub const RALPH_ARCHIVE_DIR: &str = "archive";
pub const RALPH_STATE_VERSION: u32 = 1;
pub const PROMISE_COMPLETE: &str = "<promise>COMPLETE</promise>";
pub const PROMISE_CONTINUE: &str = "<promise>CONTINUE</promise>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RalphLoopStart {
    pub name: String,
    pub task: String,
    pub max_iterations: u32,
    pub items_per_iteration: u32,
    pub reflect_every: u32,
}

impl RalphLoopStart {
    #[must_use]
    pub fn new(name: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            task: task.into(),
            max_iterations: 60,
            items_per_iteration: 3,
            reflect_every: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RalphLoopStatus {
    Active,
    Paused,
    Blocked,
    AwaitingDecision,
    Complete,
    Cancelled,
    Archived,
}

impl RalphLoopStatus {
    #[must_use]
    pub const fn can_resume(&self) -> bool {
        matches!(self, Self::Paused | Self::Blocked | Self::AwaitingDecision)
    }

    #[allow(dead_code)]
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled | Self::Archived)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum RalphPromise {
    Continue,
    Complete,
    Blocked(String),
    Decide(String),
    TaskDone(String),
}

impl RalphPromise {
    #[allow(dead_code)]
    #[must_use]
    pub fn parse(output: &str) -> Option<Self> {
        let body = promise_body(output)?.trim();
        if body.eq_ignore_ascii_case("COMPLETE") {
            return Some(Self::Complete);
        }
        if body.eq_ignore_ascii_case("CONTINUE") {
            return Some(Self::Continue);
        }
        if let Some(reason) = body.strip_prefix("BLOCKED:") {
            return Some(Self::Blocked(reason.trim().to_string()));
        }
        if let Some(question) = body.strip_prefix("DECIDE:") {
            return Some(Self::Decide(question.trim().to_string()));
        }
        if let Some(task_id) = body.strip_suffix(":DONE") {
            return Some(Self::TaskDone(task_id.trim().to_string()));
        }
        None
    }

    #[must_use]
    pub fn status_after_record(&self, current: &RalphLoopStatus) -> RalphLoopStatus {
        match self {
            Self::Continue | Self::TaskDone(_) => current.clone(),
            Self::Complete => RalphLoopStatus::Complete,
            Self::Blocked(_) => RalphLoopStatus::Blocked,
            Self::Decide(_) => RalphLoopStatus::AwaitingDecision,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalphLoopState {
    pub schema_version: u32,
    pub name: String,
    pub status: RalphLoopStatus,
    pub iteration: u32,
    pub max_iterations: u32,
    pub items_per_iteration: u32,
    pub reflect_every: u32,
    pub task_file: String,
    pub log_file: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub steering_file: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub history_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_promise: Option<RalphPromise>,
    #[serde(default)]
    pub progress: Vec<RalphLoopProgress>,
}

impl RalphLoopState {
    #[allow(dead_code)]
    #[must_use]
    pub fn next_iteration_prompt(&self) -> String {
        format!(
            "You are in a Ralph loop `{}` (iteration {}/{}). Process approximately {} checklist item{} from the task file, update progress, then emit one promise tag: {}, {}, <promise>BLOCKED:reason</promise>, <promise>DECIDE:question</promise>, or <promise>TASK-ID:DONE</promise>.",
            self.name,
            self.iteration.saturating_add(1).min(self.max_iterations),
            self.max_iterations,
            self.items_per_iteration,
            if self.items_per_iteration == 1 { "" } else { "s" },
            PROMISE_COMPLETE,
            PROMISE_CONTINUE,
        )
    }

    #[allow(dead_code)]
    #[must_use]
    pub const fn should_reflect_next(&self) -> bool {
        self.reflect_every > 0
            && self.iteration > 0
            && self.iteration.is_multiple_of(self.reflect_every)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalphLoopProgress {
    pub iteration: u32,
    pub timestamp_unix: u64,
    pub promise: RalphPromise,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RalphLoopPaths {
    pub root: PathBuf,
    pub task_file: PathBuf,
    pub state_file: PathBuf,
    pub log_file: PathBuf,
    pub steering_file: PathBuf,
    pub history_dir: PathBuf,
    pub archive_dir: PathBuf,
}

impl RalphLoopPaths {
    #[must_use]
    pub fn for_project(project_root: &Path, name: &str) -> Self {
        let safe_name = safe_loop_name(name);
        let root = project_root.join(".oino").join(RALPH_DIR);
        Self {
            task_file: root.join(format!("{safe_name}.md")),
            state_file: root.join(format!("{safe_name}.json")),
            log_file: root.join(format!("{safe_name}.log.md")),
            steering_file: root.join(format!("{safe_name}.steering.md")),
            history_dir: root.join("history").join(&safe_name),
            archive_dir: root.join(RALPH_ARCHIVE_DIR),
            root,
        }
    }
}

#[derive(Debug, Error)]
pub enum RalphLoopError {
    #[error("Ralph loop name is empty after normalization")]
    EmptyName,
    #[error("Ralph loop `{0}` already exists")]
    AlreadyExists(String),
    #[error("Ralph loop `{0}` was not found")]
    NotFound(String),
    #[error("Ralph loop `{name}` is {status:?} and cannot be resumed")]
    CannotResume {
        name: String,
        status: RalphLoopStatus,
    },
    #[error("Ralph loop `{name}` is {status:?} and cannot record an iteration")]
    CannotRecord {
        name: String,
        status: RalphLoopStatus,
    },
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("Ralph loop state parse failed at {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

pub fn start_loop(
    project_root: &Path,
    start: RalphLoopStart,
) -> Result<RalphLoopState, RalphLoopError> {
    let name = safe_loop_name(&start.name);
    if name.is_empty() {
        return Err(RalphLoopError::EmptyName);
    }
    let paths = RalphLoopPaths::for_project(project_root, &name);
    if paths.state_file.exists() || paths.task_file.exists() {
        return Err(RalphLoopError::AlreadyExists(name));
    }
    create_dir_all(&paths.root)?;
    let now = now_unix();
    let state = RalphLoopState {
        schema_version: RALPH_STATE_VERSION,
        name: name.clone(),
        status: RalphLoopStatus::Active,
        iteration: 0,
        max_iterations: start.max_iterations.max(1),
        items_per_iteration: start.items_per_iteration.max(1),
        reflect_every: start.reflect_every,
        task_file: relative_display(project_root, &paths.task_file),
        log_file: relative_display(project_root, &paths.log_file),
        steering_file: relative_display(project_root, &paths.steering_file),
        history_dir: relative_display(project_root, &paths.history_dir),
        created_at_unix: now,
        updated_at_unix: now,
        archived_at_unix: None,
        last_promise: None,
        progress: Vec::new(),
    };
    write_file(
        &paths.task_file,
        &initial_task_document(&state, &start.task),
    )?;
    write_file(&paths.log_file, &format!("# Ralph loop log: {name}\n\n"))?;
    write_file(
        &paths.steering_file,
        &format!(
            "# Ralph steering: {name}\n\nAdd urgent instructions here while the loop is running. Oino includes this file in every Ralph iteration prompt.\n"
        ),
    )?;
    create_dir_all(&paths.history_dir)?;
    save_state(project_root, &state)?;
    Ok(state)
}

pub fn load_state(project_root: &Path, name: &str) -> Result<RalphLoopState, RalphLoopError> {
    let name = safe_loop_name(name);
    let paths = RalphLoopPaths::for_project(project_root, &name);
    if !paths.state_file.is_file() {
        return Err(RalphLoopError::NotFound(name));
    }
    let text = read_file(&paths.state_file)?;
    serde_json::from_str(&text).map_err(|source| RalphLoopError::Json {
        path: paths.state_file,
        source,
    })
}

pub fn save_state(project_root: &Path, state: &RalphLoopState) -> Result<(), RalphLoopError> {
    let paths = RalphLoopPaths::for_project(project_root, &state.name);
    create_dir_all(&paths.root)?;
    let text = serde_json::to_string_pretty(state).map_err(|source| RalphLoopError::Json {
        path: paths.state_file.clone(),
        source,
    })?;
    write_file(&paths.state_file, &format!("{text}\n"))
}

#[must_use]
pub fn status_line(state: &RalphLoopState) -> String {
    format!(
        "{}: {:?} iteration {}/{} (task: {})",
        state.name, state.status, state.iteration, state.max_iterations, state.task_file
    )
}

pub fn list_states(project_root: &Path) -> Result<Vec<RalphLoopState>, RalphLoopError> {
    let root = project_root.join(".oino").join(RALPH_DIR);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut states: Vec<RalphLoopState> = Vec::new();
    let entries = fs::read_dir(&root).map_err(|source| RalphLoopError::Io {
        path: root.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| RalphLoopError::Io {
            path: root.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let text = read_file(&path)?;
        states.push(
            serde_json::from_str(&text).map_err(|source| RalphLoopError::Json { path, source })?,
        );
    }
    states.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(states)
}

pub fn pause_loop(project_root: &Path, name: &str) -> Result<RalphLoopState, RalphLoopError> {
    transition(project_root, name, RalphLoopStatus::Paused)
}

pub fn cancel_loop(project_root: &Path, name: &str) -> Result<RalphLoopState, RalphLoopError> {
    transition(project_root, name, RalphLoopStatus::Cancelled)
}

pub fn resume_loop(project_root: &Path, name: &str) -> Result<RalphLoopState, RalphLoopError> {
    let mut state = load_state(project_root, name)?;
    if state.status == RalphLoopStatus::Active {
        return Ok(state);
    }
    if !state.status.can_resume() {
        return Err(RalphLoopError::CannotResume {
            name: state.name,
            status: state.status,
        });
    }
    state.status = RalphLoopStatus::Active;
    state.updated_at_unix = now_unix();
    save_state(project_root, &state)?;
    Ok(state)
}

pub fn record_iteration(
    project_root: &Path,
    name: &str,
    promise: RalphPromise,
    note: impl Into<String>,
) -> Result<RalphLoopState, RalphLoopError> {
    let mut state = load_state(project_root, name)?;
    if state.status != RalphLoopStatus::Active {
        return Err(RalphLoopError::CannotRecord {
            name: state.name,
            status: state.status,
        });
    }
    state.iteration = state.iteration.saturating_add(1).min(state.max_iterations);
    let timestamp = now_unix();
    let note = note.into();
    state.progress.push(RalphLoopProgress {
        iteration: state.iteration,
        timestamp_unix: timestamp,
        promise: promise.clone(),
        note: note.clone(),
    });
    state.status =
        if state.iteration >= state.max_iterations && matches!(promise, RalphPromise::Continue) {
            RalphLoopStatus::Blocked
        } else {
            promise.status_after_record(&state.status)
        };
    state.last_promise = Some(promise.clone());
    state.updated_at_unix = timestamp;
    append_iteration_notes(project_root, &state, &promise, &note)?;
    save_state(project_root, &state)?;
    Ok(state)
}

pub fn build_iteration_prompt(
    project_root: &Path,
    state: &RalphLoopState,
) -> Result<String, RalphLoopError> {
    let paths = RalphLoopPaths::for_project(project_root, &state.name);
    let task = read_file(&paths.task_file)?;
    let steering = read_file(&paths.steering_file).unwrap_or_default();
    let log = read_file(&paths.log_file).unwrap_or_default();
    let reflection = if state.should_reflect_next() {
        "\n## Reflection checkpoint\n\nBefore implementation, briefly reflect on: what is done, what is blocked, whether the plan should adjust, and the next priorities. Update the task file with that reflection.\n"
    } else {
        ""
    };
    Ok(format!(
        "# Oino Ralph Loop Iteration\n\nYou are running Oino Ralph loop `{name}`. Treat this as a bounded fresh iteration: use the project files and Ralph task files as source of truth, avoid relying on earlier chat context, and finish with exactly one promise tag.\n\n## Loop state\n\n- Iteration: next {next}/{max}\n- Items this iteration: approximately {items}\n- Task file: `{task_file}`\n- Steering file: `{steering_file}`\n- Log file: `{log_file}`\n\n## Required behavior\n\n1. Inspect the task and steering content below.\n2. Complete approximately {items} checklist item(s), unless blocked or complete.\n3. Update the task file and log with concise progress.\n4. Run relevant validation where practical.\n5. End your final response with exactly one promise tag:\n   - `{continue_tag}` if more work remains and Oino should continue automatically.\n   - `{complete_tag}` if all work is done.\n   - `<promise>BLOCKED:reason</promise>` if human help is required.\n   - `<promise>DECIDE:question</promise>` if a human decision is required.\n   - `<promise>TASK-ID:DONE</promise>` if a named task was completed and more work remains.\n{reflection}\n## Task file\n\n```markdown\n{task}\n```\n\n## Steering\n\n```markdown\n{steering}\n```\n\n## Recent log\n\n```markdown\n{log_tail}\n```\n",
        name = state.name,
        next = state.iteration.saturating_add(1).min(state.max_iterations),
        max = state.max_iterations,
        items = state.items_per_iteration,
        task_file = state.task_file,
        steering_file = state.steering_file,
        log_file = state.log_file,
        continue_tag = PROMISE_CONTINUE,
        complete_tag = PROMISE_COMPLETE,
        reflection = reflection,
        task = task,
        steering = steering,
        log_tail = tail_chars(&log, 6000),
    ))
}

pub fn record_iteration_output(
    project_root: &Path,
    name: &str,
    output: &str,
) -> Result<RalphLoopState, RalphLoopError> {
    let promise = match RalphPromise::parse(output) {
        Some(promise) => promise,
        None => RalphPromise::Blocked("missing promise tag in final assistant message".into()),
    };
    let mut state = record_iteration(project_root, name, promise, output_summary(output))?;
    write_history_entry(project_root, &state, output)?;
    if state.iteration >= state.max_iterations && state.status == RalphLoopStatus::Active {
        state.status = RalphLoopStatus::Blocked;
        state.last_promise = Some(RalphPromise::Blocked("max iterations reached".into()));
        state.updated_at_unix = now_unix();
        save_state(project_root, &state)?;
    }
    Ok(state)
}

pub fn append_steering(
    project_root: &Path,
    name: &str,
    text: impl AsRef<str>,
) -> Result<RalphLoopState, RalphLoopError> {
    let state = load_state(project_root, name)?;
    let paths = RalphLoopPaths::for_project(project_root, &state.name);
    let entry = format!(
        "\n## Steering update {}\n\n{}\n",
        now_unix(),
        text.as_ref().trim()
    );
    append_file(&paths.steering_file, &entry)?;
    Ok(state)
}

pub fn clean_archive(project_root: &Path) -> Result<usize, RalphLoopError> {
    let archive_dir = project_root
        .join(".oino")
        .join(RALPH_DIR)
        .join(RALPH_ARCHIVE_DIR);
    if !archive_dir.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in fs::read_dir(&archive_dir).map_err(|source| RalphLoopError::Io {
        path: archive_dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| RalphLoopError::Io {
            path: archive_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.is_file() {
            fs::remove_file(&path).map_err(|source| RalphLoopError::Io {
                path: path.clone(),
                source,
            })?;
            removed = removed.saturating_add(1);
        }
    }
    Ok(removed)
}

pub fn archive_loop(project_root: &Path, name: &str) -> Result<RalphLoopState, RalphLoopError> {
    let mut state = load_state(project_root, name)?;
    let paths = RalphLoopPaths::for_project(project_root, &state.name);
    create_dir_all(&paths.archive_dir)?;
    let stamp = now_unix();
    let archive_prefix = format!("{}-{stamp}", state.name);
    let archive_state = paths.archive_dir.join(format!("{archive_prefix}.json"));
    let archive_task = paths.archive_dir.join(format!("{archive_prefix}.md"));
    let archive_log = paths.archive_dir.join(format!("{archive_prefix}.log.md"));
    let archive_steering = paths
        .archive_dir
        .join(format!("{archive_prefix}.steering.md"));

    state.status = RalphLoopStatus::Archived;
    state.archived_at_unix = Some(stamp);
    state.updated_at_unix = stamp;
    save_state(project_root, &state)?;
    rename_if_exists(&paths.state_file, &archive_state)?;
    rename_if_exists(&paths.task_file, &archive_task)?;
    rename_if_exists(&paths.log_file, &archive_log)?;
    rename_if_exists(&paths.steering_file, &archive_steering)?;
    Ok(state)
}

fn transition(
    project_root: &Path,
    name: &str,
    status: RalphLoopStatus,
) -> Result<RalphLoopState, RalphLoopError> {
    let mut state = load_state(project_root, name)?;
    state.status = status;
    state.updated_at_unix = now_unix();
    save_state(project_root, &state)?;
    Ok(state)
}

fn initial_task_document(state: &RalphLoopState, task: &str) -> String {
    format!(
        "# Ralph Loop: {}\n\n## Task\n\n{}\n\n## Loop controls\n\n- Max iterations: {}\n- Items per iteration: {}\n- Reflect every: {}\n- Completion marker: `{}`\n- Continue marker: `{}`\n- Steering file: `{}`\n- History dir: `{}`\n\n## Checklist\n\n- [ ] Break the task into small, reviewable steps.\n- [ ] Update this file after each iteration.\n- [ ] Emit a promise tag when each iteration finishes.\n\n## Progress Log\n\n",
        state.name,
        task.trim(),
        state.max_iterations,
        state.items_per_iteration,
        state.reflect_every,
        PROMISE_COMPLETE,
        PROMISE_CONTINUE,
        state.steering_file,
        state.history_dir,
    )
}

fn write_history_entry(
    project_root: &Path,
    state: &RalphLoopState,
    output: &str,
) -> Result<(), RalphLoopError> {
    let paths = RalphLoopPaths::for_project(project_root, &state.name);
    create_dir_all(&paths.history_dir)?;
    let path = paths
        .history_dir
        .join(format!("iteration-{:04}.md", state.iteration));
    write_file(
        &path,
        &format!(
            "# Ralph iteration {} output\n\nStatus: {:?}\n\n```text\n{}\n```\n",
            state.iteration, state.status, output
        ),
    )
}

fn output_summary(output: &str) -> String {
    tail_chars(output.trim(), 1000)
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    let mut out = String::from("…");
    out.push_str(
        &text
            .chars()
            .skip(len.saturating_sub(max_chars))
            .collect::<String>(),
    );
    out
}

fn append_iteration_notes(
    project_root: &Path,
    state: &RalphLoopState,
    promise: &RalphPromise,
    note: &str,
) -> Result<(), RalphLoopError> {
    let paths = RalphLoopPaths::for_project(project_root, &state.name);
    let entry = format!(
        "### Iteration {}\n\n- Promise: `{:?}`\n- Note: {}\n\n",
        state.iteration,
        promise,
        note.trim()
    );
    append_file(&paths.task_file, &entry)?;
    append_file(&paths.log_file, &entry)
}

#[allow(dead_code)]
fn promise_body(output: &str) -> Option<&str> {
    let start = output.find("<promise>")? + "<promise>".len();
    let tail = &output[start..];
    let end = tail.find("</promise>")?;
    Some(&tail[..end])
}

#[must_use]
pub fn safe_loop_name(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_' | ' ' | '.') {
            Some('-')
        } else {
            None
        };
        let Some(ch) = mapped else {
            continue;
        };
        if ch == '-' {
            if last_dash || out.is_empty() {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(ch);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn create_dir_all(path: &Path) -> Result<(), RalphLoopError> {
    fs::create_dir_all(path).map_err(|source| RalphLoopError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn read_file(path: &Path) -> Result<String, RalphLoopError> {
    fs::read_to_string(path).map_err(|source| RalphLoopError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_file(path: &Path, content: &str) -> Result<(), RalphLoopError> {
    fs::write(path, content).map_err(|source| RalphLoopError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn append_file(path: &Path, content: &str) -> Result<(), RalphLoopError> {
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| RalphLoopError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    file.write_all(content.as_bytes())
        .map_err(|source| RalphLoopError::Io {
            path: path.to_path_buf(),
            source,
        })
}

fn rename_if_exists(from: &Path, to: &Path) -> Result<(), RalphLoopError> {
    if !from.exists() {
        return Ok(());
    }
    fs::rename(from, to).map_err(|source| RalphLoopError::Io {
        path: from.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_loop_name_normalizes_for_paths() {
        assert_eq!(
            safe_loop_name(" Oino Builtin Extensions! "),
            "oino-builtin-extensions"
        );
        assert_eq!(
            safe_loop_name("...RALPH__Loop---Demo..."),
            "ralph-loop-demo"
        );
    }

    #[test]
    fn parses_reference_promise_tags() {
        assert_eq!(
            RalphPromise::parse("done\n<promise>COMPLETE</promise>"),
            Some(RalphPromise::Complete)
        );
        assert_eq!(
            RalphPromise::parse("<promise>CONTINUE</promise>"),
            Some(RalphPromise::Continue)
        );
        assert_eq!(
            RalphPromise::parse("<promise>BLOCKED:needs API key</promise>"),
            Some(RalphPromise::Blocked("needs API key".into()))
        );
        assert_eq!(
            RalphPromise::parse("<promise>DECIDE:ship now?</promise>"),
            Some(RalphPromise::Decide("ship now?".into()))
        );
        assert_eq!(
            RalphPromise::parse("<promise>TASK-7:DONE</promise>"),
            Some(RalphPromise::TaskDone("TASK-7".into()))
        );
    }

    #[test]
    fn start_record_resume_and_archive_loop() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let mut start = RalphLoopStart::new("Demo Loop", "Build a thing");
        start.max_iterations = 3;
        start.items_per_iteration = 2;
        start.reflect_every = 2;

        let state =
            start_loop(temp.path(), start).unwrap_or_else(|err| panic!("start failed: {err}"));
        assert_eq!(state.name, "demo-loop");
        assert_eq!(state.status, RalphLoopStatus::Active);
        assert!(temp.path().join(".oino/ralph/demo-loop.md").is_file());
        assert!(temp
            .path()
            .join(".oino/ralph/demo-loop.steering.md")
            .is_file());
        assert!(temp.path().join(".oino/ralph/history/demo-loop").is_dir());
        assert!(state.next_iteration_prompt().contains("iteration 1/3"));
        assert!(status_line(&state).contains("demo-loop: Active iteration 0/3"));

        let state = record_iteration(
            temp.path(),
            "demo-loop",
            RalphPromise::TaskDone("TASK-1".into()),
            "created skeleton",
        )
        .unwrap_or_else(|err| panic!("record failed: {err}"));
        assert_eq!(state.iteration, 1);
        assert_eq!(state.status, RalphLoopStatus::Active);
        assert_eq!(
            state.last_promise,
            Some(RalphPromise::TaskDone("TASK-1".into()))
        );
        assert!(!state.should_reflect_next());

        let state = record_iteration(
            temp.path(),
            "demo-loop",
            RalphPromise::Continue,
            "kept going",
        )
        .unwrap_or_else(|err| panic!("record failed: {err}"));
        assert_eq!(state.iteration, 2);
        assert!(state.should_reflect_next());

        let state = pause_loop(temp.path(), "demo-loop")
            .unwrap_or_else(|err| panic!("pause failed: {err}"));
        assert_eq!(state.status, RalphLoopStatus::Paused);
        let state = resume_loop(temp.path(), "demo-loop")
            .unwrap_or_else(|err| panic!("resume failed: {err}"));
        assert_eq!(state.status, RalphLoopStatus::Active);

        let listed = list_states(temp.path()).unwrap_or_else(|err| panic!("list failed: {err}"));
        assert_eq!(
            listed
                .iter()
                .map(|state| state.name.as_str())
                .collect::<Vec<_>>(),
            vec!["demo-loop"]
        );

        let archived = archive_loop(temp.path(), "demo-loop")
            .unwrap_or_else(|err| panic!("archive failed: {err}"));
        assert_eq!(archived.status, RalphLoopStatus::Archived);
        assert!(!temp.path().join(".oino/ralph/demo-loop.json").exists());
        assert!(temp.path().join(".oino/ralph/archive").is_dir());
        let removed =
            clean_archive(temp.path()).unwrap_or_else(|err| panic!("clean failed: {err}"));
        assert!(removed >= 4);
    }

    #[test]
    fn iteration_prompt_and_output_recording_match_controller_contract() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        let mut start = RalphLoopStart::new("Auto Loop", "Ship the feature");
        start.max_iterations = 2;
        let state =
            start_loop(temp.path(), start).unwrap_or_else(|err| panic!("start failed: {err}"));
        append_steering(temp.path(), "auto-loop", "Prioritize tests")
            .unwrap_or_else(|err| panic!("steer failed: {err}"));
        let prompt = build_iteration_prompt(temp.path(), &state)
            .unwrap_or_else(|err| panic!("prompt failed: {err}"));
        assert!(prompt.contains("Ship the feature"));
        assert!(prompt.contains("Prioritize tests"));
        assert!(prompt.contains(PROMISE_CONTINUE));

        let state = record_iteration_output(
            temp.path(),
            "auto-loop",
            "Implemented skeleton.\n<promise>CONTINUE</promise>",
        )
        .unwrap_or_else(|err| panic!("record failed: {err}"));
        assert_eq!(state.status, RalphLoopStatus::Active);
        assert_eq!(state.iteration, 1);
        assert!(temp
            .path()
            .join(".oino/ralph/history/auto-loop/iteration-0001.md")
            .is_file());

        let state = record_iteration_output(temp.path(), "auto-loop", "No promise this time")
            .unwrap_or_else(|err| panic!("record failed: {err}"));
        assert_eq!(state.status, RalphLoopStatus::Blocked);
        assert_eq!(state.iteration, 2);
    }

    #[test]
    fn complete_promise_finishes_loop() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        start_loop(temp.path(), RalphLoopStart::new("Complete", "Finish"))
            .unwrap_or_else(|err| panic!("start failed: {err}"));
        let state = record_iteration(temp.path(), "complete", RalphPromise::Complete, "all done")
            .unwrap_or_else(|err| panic!("record failed: {err}"));
        assert_eq!(state.status, RalphLoopStatus::Complete);
        assert!(state.status.is_terminal());
        assert!(resume_loop(temp.path(), "complete").is_err());
    }

    #[test]
    fn cancelled_loop_is_terminal() {
        let temp = tempfile::tempdir().unwrap_or_else(|err| panic!("tempdir failed: {err}"));
        start_loop(temp.path(), RalphLoopStart::new("Cancel Me", "Stop"))
            .unwrap_or_else(|err| panic!("start failed: {err}"));
        let state = cancel_loop(temp.path(), "cancel-me")
            .unwrap_or_else(|err| panic!("cancel failed: {err}"));
        assert_eq!(state.status, RalphLoopStatus::Cancelled);
        assert!(state.status.is_terminal());
        assert!(resume_loop(temp.path(), "cancel-me").is_err());
    }
}
