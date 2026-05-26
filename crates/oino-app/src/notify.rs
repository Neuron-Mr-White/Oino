#![forbid(unsafe_code)]

use oino_agent_loop::AgentEvent;
use oino_types::ContentBlock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const DEFAULT_NTFY_SERVER: &str = "https://ntfy.sh";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifySettings {
    pub enabled: Option<bool>,
    pub ntfy: NtfySettings,
    pub events: Option<BTreeSet<NotifyEvent>>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNotifyConfig {
    pub server: String,
    pub topic: String,
    pub token: Option<String>,
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub events: BTreeSet<NotifyEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyMessage {
    pub event: NotifyEvent,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NtfyRequestParts {
    pub url: String,
    pub body: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("ntfy request failed: {0}")]
    Http(String),
    #[error("ntfy rejected notification with status {0}")]
    Status(reqwest::StatusCode),
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
    Some(EffectiveNotifyConfig {
        server,
        topic,
        token,
        priority,
        tags,
        events,
    })
}

#[must_use]
pub fn notify_message_for_event(event: &AgentEvent) -> Option<NotifyMessage> {
    match event {
        AgentEvent::AgentEnd { stop_reason, .. } => Some(NotifyMessage {
            event: NotifyEvent::AgentEnd,
            title: "Oino run finished".into(),
            body: format!("Agent ended with stop reason {stop_reason:?}."),
        }),
        AgentEvent::ToolExecutionEnd { result, .. } if result.is_error => Some(NotifyMessage {
            event: NotifyEvent::ToolError,
            title: format!("Oino tool error: {}", result.tool_name),
            body: first_text_block(&result.content)
                .map_or_else(|| "Tool execution failed.".into(), truncate_body),
        }),
        _ => None,
    }
}

#[must_use]
pub fn ntfy_request_parts(
    config: &EffectiveNotifyConfig,
    message: &NotifyMessage,
) -> NtfyRequestParts {
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
    NtfyRequestParts {
        url: format!(
            "{}/{}",
            config.server.trim_end_matches('/'),
            config.topic.trim_start_matches('/')
        ),
        body: message.body.clone(),
        headers,
    }
}

pub async fn send_ntfy_notification(
    client: &reqwest::Client,
    config: &EffectiveNotifyConfig,
    message: &NotifyMessage,
) -> Result<(), NotifyError> {
    let parts = ntfy_request_parts(config, message);
    let mut request = client.post(parts.url).body(parts.body);
    for (key, value) in parts.headers {
        request = request.header(key, value);
    }
    let response = request
        .send()
        .await
        .map_err(|err| NotifyError::Http(err.to_string()))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(NotifyError::Status(response.status()))
    }
}

fn choose_string(project: Option<&str>, global: Option<&str>) -> Option<String> {
    project
        .filter(|value| !value.trim().is_empty())
        .or_else(|| global.filter(|value| !value.trim().is_empty()))
        .map(|value| value.trim().to_string())
}

fn default_notify_events() -> BTreeSet<NotifyEvent> {
    BTreeSet::from([NotifyEvent::AgentEnd, NotifyEvent::ToolError])
}

fn first_text_block(content: &[ContentBlock]) -> Option<&str> {
    content.iter().find_map(|block| match block {
        ContentBlock::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

fn truncate_body(text: &str) -> String {
    const LIMIT: usize = 500;
    let trimmed = text.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let mut out = trimmed.chars().take(LIMIT).collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::{ToolCall, ToolResult};
    use oino_types::{OinoId, StopReason};
    use serde_json::json;

    #[test]
    fn notify_config_resolves_project_over_global() {
        let global = NotifySettings {
            enabled: Some(true),
            ntfy: NtfySettings {
                server: Some("https://global.invalid/".into()),
                topic: Some("global-topic".into()),
                token: Some("global-token".into()),
                priority: Some("default".into()),
                tags: Some(vec!["global".into()]),
            },
            events: Some(BTreeSet::from([NotifyEvent::AgentEnd])),
        };
        let project = NotifySettings {
            enabled: None,
            ntfy: NtfySettings {
                topic: Some("project-topic".into()),
                priority: Some("high".into()),
                ..NtfySettings::default()
            },
            events: Some(BTreeSet::from([NotifyEvent::ToolError])),
        };

        let resolved = resolve_notify_config(&global, &project)
            .unwrap_or_else(|| panic!("notify should be enabled"));
        assert_eq!(resolved.server, "https://global.invalid/");
        assert_eq!(resolved.topic, "project-topic");
        assert_eq!(resolved.token.as_deref(), Some("global-token"));
        assert_eq!(resolved.priority.as_deref(), Some("high"));
        assert_eq!(resolved.tags, vec!["global"]);
        assert_eq!(resolved.events, BTreeSet::from([NotifyEvent::ToolError]));
    }

    #[test]
    fn notify_config_requires_enabled_and_topic() {
        assert!(
            resolve_notify_config(&NotifySettings::default(), &NotifySettings::default()).is_none()
        );
        let enabled_without_topic = NotifySettings {
            enabled: Some(true),
            ..NotifySettings::default()
        };
        assert!(
            resolve_notify_config(&enabled_without_topic, &NotifySettings::default()).is_none()
        );
        let project_disabled = NotifySettings {
            enabled: Some(false),
            ntfy: NtfySettings {
                topic: Some("topic".into()),
                ..NtfySettings::default()
            },
            ..NotifySettings::default()
        };
        assert!(resolve_notify_config(&enabled_without_topic, &project_disabled).is_none());
    }

    #[test]
    fn ntfy_payload_builds_url_headers_and_body() {
        let config = EffectiveNotifyConfig {
            server: "https://ntfy.example/".into(),
            topic: "/oino".into(),
            token: Some("secret".into()),
            priority: Some("high".into()),
            tags: vec!["oino".into(), "done".into()],
            events: BTreeSet::from([NotifyEvent::AgentEnd]),
        };
        let message = NotifyMessage {
            event: NotifyEvent::AgentEnd,
            title: "Done".into(),
            body: "Finished".into(),
        };
        let parts = ntfy_request_parts(&config, &message);
        assert_eq!(parts.url, "https://ntfy.example/oino");
        assert_eq!(parts.body, "Finished");
        assert_eq!(parts.headers.get("Title").map(String::as_str), Some("Done"));
        assert_eq!(
            parts.headers.get("Priority").map(String::as_str),
            Some("high")
        );
        assert_eq!(
            parts.headers.get("Tags").map(String::as_str),
            Some("oino,done")
        );
        assert_eq!(
            parts.headers.get("Authorization").map(String::as_str),
            Some("Bearer secret")
        );
    }

    #[test]
    fn agent_events_map_to_selected_notification_messages() {
        let agent_end = AgentEvent::AgentEnd {
            run_id: OinoId::nil(),
            stop_reason: StopReason::EndTurn,
        };
        let message = notify_message_for_event(&agent_end)
            .unwrap_or_else(|| panic!("agent end should notify"));
        assert_eq!(message.event, NotifyEvent::AgentEnd);
        assert!(message.body.contains("EndTurn"));

        let call = ToolCall {
            id: OinoId::nil(),
            name: "bash".into(),
            arguments: json!({}),
        };
        let tool_end = AgentEvent::ToolExecutionEnd {
            call_id: OinoId::nil(),
            result: ToolResult::error(&call, "bad command"),
        };
        let message = notify_message_for_event(&tool_end)
            .unwrap_or_else(|| panic!("tool error should notify"));
        assert_eq!(message.event, NotifyEvent::ToolError);
        assert_eq!(message.title, "Oino tool error: bash");
        assert_eq!(message.body, "bad command");
    }
}
