#![forbid(unsafe_code)]

//! Notify settings schema (`settings.json` `notify` key) and resolution into
//! the effective config the `oino.notify` WASM extension receives as init
//! config. Ported verbatim from the former `notify.rs`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const DEFAULT_NTFY_SERVER: &str = "https://ntfy.sh";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    pub enabled: Option<bool>,
    pub ntfy: NtfySettings,
    pub events: Option<BTreeSet<NotifyEvent>>,
    pub summarizer: NotifySummarizerSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySummarizerSettings {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub prompt: Option<String>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NtfySettings {
    pub server: Option<String>,
    pub topic: Option<String>,
    pub token: Option<String>,
    pub priority: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyEvent {
    AgentEnd,
    ToolError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectiveNotifyConfig {
    pub server: String,
    pub topic: String,
    pub token: Option<String>,
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub events: BTreeSet<NotifyEvent>,
    pub summarizer: EffectiveNotifySummarizerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectiveNotifySummarizerConfig {
    pub enabled: bool,
    pub model: Option<String>,
    pub prompt: String,
    pub max_chars: usize,
}

#[must_use]
pub fn resolve_notify_config(
    global: &NotifySettings,
    project: &NotifySettings,
) -> Option<EffectiveNotifyConfig> {
    if !project.enabled.or(global.enabled).unwrap_or(false) {
        return None;
    }
    let topic = choose_string(project.ntfy.topic.as_deref(), global.ntfy.topic.as_deref())?;
    let server = choose_string(
        project.ntfy.server.as_deref(),
        global.ntfy.server.as_deref(),
    )
    .unwrap_or_else(|| DEFAULT_NTFY_SERVER.into());
    let token = choose_string(project.ntfy.token.as_deref(), global.ntfy.token.as_deref());
    let priority = choose_string(
        project.ntfy.priority.as_deref(),
        global.ntfy.priority.as_deref(),
    );
    let tags = project
        .ntfy
        .tags
        .clone()
        .or_else(|| global.ntfy.tags.clone())
        .unwrap_or_default();
    let events = project
        .events
        .clone()
        .or_else(|| global.events.clone())
        .unwrap_or_else(default_notify_events);
    let summarizer = resolve_summarizer_config(&global.summarizer, &project.summarizer);
    Some(EffectiveNotifyConfig {
        server,
        topic,
        token,
        priority: priority.and_then(|value| normalize_ntfy_priority(&value)),
        tags,
        events,
        summarizer,
    })
}

fn choose_string(project: Option<&str>, global: Option<&str>) -> Option<String> {
    project
        .filter(|value| !value.trim().is_empty())
        .or_else(|| global.filter(|value| !value.trim().is_empty()))
        .map(|value| value.trim().to_string())
}

pub fn normalize_ntfy_priority(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "min" | "1" => Some("min".into()),
        "low" | "2" => Some("low".into()),
        "default" | "3" => Some("default".into()),
        "high" | "4" => Some("high".into()),
        "max" | "urgent" | "5" => Some("max".into()),
        _ => None,
    }
}

fn default_notify_events() -> BTreeSet<NotifyEvent> {
    BTreeSet::from([NotifyEvent::AgentEnd, NotifyEvent::ToolError])
}

fn resolve_summarizer_config(
    global: &NotifySummarizerSettings,
    project: &NotifySummarizerSettings,
) -> EffectiveNotifySummarizerConfig {
    EffectiveNotifySummarizerConfig {
        enabled: project.enabled.or(global.enabled).unwrap_or(true),
        model: choose_string(project.model.as_deref(), global.model.as_deref()),
        prompt: choose_string(project.prompt.as_deref(), global.prompt.as_deref())
            .unwrap_or_else(default_summary_prompt),
        max_chars: project
            .max_chars
            .or(global.max_chars)
            .unwrap_or(280)
            .clamp(80, 2000),
    }
}

fn default_summary_prompt() -> String {
    "Summarize this Oino run for a notification. Keep it concise, factual, and under the configured character limit.".into()
}
