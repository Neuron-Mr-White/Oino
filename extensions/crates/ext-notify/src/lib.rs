//! `oino.notify` — ntfy.sh notifications for agent events.
//!
//! Ported from the native `notify.rs`: the host injects the resolved
//! `EffectiveNotifyConfig` (same keys as `resolve_notify_config`) as init
//! config; `server`'s URL host is granted via `hosts_from_config`. The LLM
//! summary path uses `host.complete` (`model.complete` capability).

use oino_extension_guest::{export_extension, host, Extension, Registration};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EffectiveNotifyConfig {
    pub server: String,
    pub topic: String,
    pub token: Option<String>,
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub events: BTreeSet<NotifyEvent>,
    pub summarizer: EffectiveNotifySummarizerConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EffectiveNotifySummarizerConfig {
    pub enabled: bool,
    pub model: Option<String>,
    pub prompt: String,
    pub max_chars: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyEvent {
    AgentEnd,
    ToolError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyMessage {
    pub event: NotifyEvent,
    pub title: String,
    pub body: String,
}

static CONFIG: Mutex<Option<EffectiveNotifyConfig>> = Mutex::new(None);
static LAST_ASSISTANT_TEXT: Mutex<Option<String>> = Mutex::new(None);

/// Agent event → notification (port of `notify_message_for_event`). The
/// payload is the serialized `AgentEvent` JSON.
pub fn notify_message_for_event(event: &Value) -> Option<NotifyMessage> {
    match event["kind"].as_str()? {
        "agent_end" => Some(NotifyMessage {
            event: NotifyEvent::AgentEnd,
            title: "Oino run finished".into(),
            body: match event["stop_reason"].as_str() {
                Some("end_turn") => "Agent run finished.".into(),
                other => format!("Agent run finished ({}).", other.unwrap_or("unknown")),
            },
        }),
        "tool_execution_end" if event["result"]["is_error"].as_bool().unwrap_or(false) => {
            let tool = event["result"]["tool_name"]
                .as_str()
                .unwrap_or("tool")
                .to_string();
            Some(NotifyMessage {
                event: NotifyEvent::ToolError,
                title: format!("Oino tool error: {tool}"),
                body: event["result"]["content"]
                    .as_array()
                    .and_then(|content| content.iter().find_map(|block| block["text"].as_str()))
                    .map_or_else(
                        || "Tool execution failed.".into(),
                        |text| truncate_to(text, 500),
                    ),
            })
        }
        _ => None,
    }
}

/// Port of `ntfy_request_parts`.
pub fn ntfy_request_parts(
    config: &EffectiveNotifyConfig,
    message: &NotifyMessage,
) -> (String, String, BTreeMap<String, String>) {
    let mut headers = BTreeMap::new();
    headers.insert("Title".into(), message.title.clone());
    if let Some(priority) = config
        .priority
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        headers.insert("Priority".into(), priority.trim().to_string());
    }
    if !config.tags.is_empty() {
        headers.insert("Tags".into(), config.tags.join(","));
    }
    if let Some(token) = config
        .token
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        headers.insert("Authorization".into(), format!("Bearer {}", token.trim()));
    }
    (
        format!(
            "{}/{}",
            config.server.trim_end_matches('/'),
            config.topic.trim_start_matches('/')
        ),
        message.body.clone(),
        headers,
    )
}

/// Port of `notification_summary_from_text`.
pub fn notification_summary_from_text(text: &str, max_chars: usize) -> String {
    let cleaned = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return "Agent run finished.".into();
    }
    truncate_to(cleaned.as_str(), max_chars.clamp(80, 2000))
}

fn truncate_to(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(limit).collect::<String>();
    out.push('…');
    out
}

/// LLM summary path (native `summarize_notify_body`) via `host.complete`.
fn summarize_body(config: &EffectiveNotifyConfig, source: &str) -> String {
    let fallback = notification_summary_from_text(source, config.summarizer.max_chars);
    let Some(model_id) = config.summarizer.model.as_deref() else {
        return fallback;
    };
    let prompt = format!(
        "{}\n\nReturn only the notification body. Maximum characters: {}.\n\nRun output:\n{}",
        config.summarizer.prompt, config.summarizer.max_chars, source
    );
    let Ok(result) = host::complete(&json!({
        "model": model_id,
        "system": "You write concise desktop/mobile notification summaries.",
        "messages": [{"role": "user", "text": prompt}],
    })) else {
        return fallback;
    };
    let text = result["text"].as_str().unwrap_or_default();
    if text.trim().is_empty() {
        fallback
    } else {
        notification_summary_from_text(text, config.summarizer.max_chars)
    }
}

fn send_if_subscribed(event: &Value) {
    let Some(config) = CONFIG.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let Some(mut message) = notify_message_for_event(event) else {
        return;
    };
    if !config.events.contains(&message.event) {
        return;
    }
    if message.event == NotifyEvent::AgentEnd {
        let source = LAST_ASSISTANT_TEXT
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .filter(|source| !source.trim().is_empty());
        if let Some(source) = source {
            if config.summarizer.enabled {
                message.body = summarize_body(&config, &source);
            }
        }
    }
    let (url, body, headers) = ntfy_request_parts(&config, &message);
    let headers: Vec<(String, String)> = headers.into_iter().collect();
    let _ = oino_extension_guest::http::request("POST", &url, &headers, Some(body.as_bytes()));
}

struct Notify;

impl Extension for Notify {
    fn init(config: Value) -> Result<Registration, String> {
        if let Ok(effective) = serde_json::from_value::<EffectiveNotifyConfig>(config) {
            if !effective.server.is_empty() && !effective.topic.is_empty() {
                *CONFIG.lock().unwrap_or_else(|e| e.into_inner()) = Some(effective);
            }
        }
        Ok(Registration {
            commands: vec![(
                "notify.settings".into(),
                "Open the Notify settings page".into(),
            )],
            hooks: vec![
                "message_end".into(),
                "agent_end".into(),
                "tool_execution_end".into(),
            ],
            ..Registration::default()
        })
    }

    fn handle_event(kind: &str, payload: Value) -> Result<Value, String> {
        match kind {
            "message_end" => {
                let text = payload["event"]["message"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|block| block["text"].as_str().map(str::to_string))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty()
                    && payload["event"]["kind"].as_str() == Some("message_end")
                {
                    *LAST_ASSISTANT_TEXT
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(text);
                }
                Ok(Value::Null)
            }
            "agent_end" | "tool_execution_end" => {
                send_if_subscribed(&payload["event"]);
                Ok(Value::Null)
            }
            _ => Ok(Value::Null),
        }
    }

    fn run_command(name: &str, _args: &str) -> Result<Value, String> {
        match name {
            "notify.settings" => Ok(json!({
                "message": "Notify settings live under `/settings notify`"
            })),
            _ => Err(format!("unknown command `{name}`")),
        }
    }
}

export_extension!(Notify);
