#![forbid(unsafe_code)]

//! Read-only Ralph floating-panel support. Loop state is owned by the
//! `oino.ralph_loop` WASM extension; the host only reads `state.json` files
//! and `panel.json` to render the floating panel (ported verbatim from the
//! former `ralph_loop.rs` panel section).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

const RALPH_DIR: &str = "ralph";
const RALPH_ARCHIVE_DIR: &str = "archive";
const RALPH_PANEL_FILE: &str = "panel.json";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RalphPanelSettings {
    pub enabled: bool,
}

/// Subset of `state.json` fields the panel renders.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RalphPanelState {
    pub name: String,
    pub status: RalphLoopStatus,
    pub iteration: u32,
    pub max_iterations: u32,
    #[serde(default)]
    pub reflect_every: u32,
    pub task_file: String,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RalphPanelSummary {
    pub name: String,
    pub status: RalphLoopStatus,
    pub iteration: u32,
    pub max_iterations: u32,
    pub task: String,
    pub next_reflection: Option<u32>,
}

#[derive(Debug, Error)]
pub enum RalphPanelError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Ralph state parse failed at {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

fn read_file(path: &Path) -> Result<String, RalphPanelError> {
    fs::read_to_string(path).map_err(|source| RalphPanelError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn panel_settings_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".oino")
        .join(RALPH_DIR)
        .join(RALPH_PANEL_FILE)
}

pub fn load_panel_settings(project_root: &Path) -> Result<RalphPanelSettings, RalphPanelError> {
    let path = panel_settings_path(project_root);
    if !path.is_file() {
        return Ok(RalphPanelSettings::default());
    }
    let text = read_file(&path)?;
    serde_json::from_str(&text).map_err(|source| RalphPanelError::Json { path, source })
}

fn list_states(project_root: &Path) -> Result<Vec<RalphPanelState>, RalphPanelError> {
    let root = project_root.join(".oino").join(RALPH_DIR);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut states: Vec<RalphPanelState> = Vec::new();
    let entries = fs::read_dir(&root).map_err(|source| RalphPanelError::Io {
        path: root.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| RalphPanelError::Io {
            path: root.clone(),
            source,
        })?;
        let path = entry.path();
        if !path.is_dir()
            || path.file_name().and_then(|value| value.to_str()) == Some(RALPH_ARCHIVE_DIR)
        {
            continue;
        }
        let state_path = path.join("state.json");
        if !state_path.is_file() {
            continue;
        }
        let text = read_file(&state_path)?;
        states.push(
            serde_json::from_str(&text).map_err(|source| RalphPanelError::Json {
                path: state_path,
                source,
            })?,
        );
    }
    states.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(states)
}

pub fn panel_summary(project_root: &Path) -> Result<Option<RalphPanelSummary>, RalphPanelError> {
    let mut states = list_states(project_root)?;
    if states.is_empty() {
        return Ok(None);
    }
    states.sort_by(|left, right| {
        state_rank(&left.status)
            .cmp(&state_rank(&right.status))
            .then_with(|| right.updated_at_unix.cmp(&left.updated_at_unix))
            .then_with(|| left.name.cmp(&right.name))
    });
    let state = states.remove(0);
    let task = first_task_heading_or_file(project_root, &state);
    Ok(Some(RalphPanelSummary {
        name: state.name,
        status: state.status,
        iteration: state.iteration,
        max_iterations: state.max_iterations,
        task,
        next_reflection: next_reflection_iteration(
            state.iteration,
            state.max_iterations,
            state.reflect_every,
        ),
    }))
}

fn state_rank(status: &RalphLoopStatus) -> u8 {
    match status {
        RalphLoopStatus::Active => 0,
        RalphLoopStatus::Paused | RalphLoopStatus::Blocked | RalphLoopStatus::AwaitingDecision => 1,
        RalphLoopStatus::Complete => 2,
        RalphLoopStatus::Cancelled => 3,
        RalphLoopStatus::Archived => 4,
    }
}

fn next_reflection_iteration(
    iteration: u32,
    max_iterations: u32,
    reflect_every: u32,
) -> Option<u32> {
    if reflect_every == 0 || iteration >= max_iterations {
        return None;
    }
    let next = ((iteration / reflect_every) + 1).saturating_mul(reflect_every);
    (next <= max_iterations).then_some(next)
}

fn first_task_heading_or_file(project_root: &Path, state: &RalphPanelState) -> String {
    let path = project_root
        .join(".oino")
        .join(RALPH_DIR)
        .join(&state.name)
        .join("task.md");
    let Ok(text) = read_file(&path) else {
        return state.task_file.clone();
    };
    text.lines()
        .find_map(|line| {
            line.strip_prefix("# ")
                .or_else(|| line.strip_prefix("## Task: "))
        })
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or(state.task_file.as_str())
        .to_string()
}
