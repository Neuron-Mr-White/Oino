#![forbid(unsafe_code)]

use oino_types::{ContentBlock, Message, OinoId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageView {
    pub id: OinoId,
    pub role: String,
    pub title: Option<String>,
    pub content: String,
    pub thinking: Option<String>,
    pub thinking_redacted: bool,
    pub is_error: bool,
}

impl MessageView {
    #[must_use]
    pub fn is_user(&self) -> bool {
        self.role == "user"
    }

    #[must_use]
    pub fn is_assistant(&self) -> bool {
        self.role == "assistant"
    }
}

#[must_use]
pub fn project_messages(messages: &[Message]) -> Vec<MessageView> {
    messages.iter().map(project_message).collect()
}

#[must_use]
pub fn project_message(message: &Message) -> MessageView {
    match message {
        Message::User { id, content } => {
            let summary = summarize_content(content);
            MessageView {
                id: *id,
                role: "user".into(),
                title: None,
                content: summary.content,
                thinking: summary.thinking,
                thinking_redacted: summary.thinking_redacted,
                is_error: false,
            }
        }
        Message::Assistant {
            id,
            content,
            provider,
            ..
        } => {
            let summary = summarize_content(content);
            MessageView {
                id: *id,
                role: "assistant".into(),
                title: provider
                    .as_ref()
                    .and_then(|metadata| metadata.model.as_ref())
                    .map(|model| model.name.clone()),
                content: summary.content,
                thinking: summary.thinking,
                thinking_redacted: summary.thinking_redacted,
                is_error: false,
            }
        }
        Message::ToolResult {
            id,
            tool_name,
            content,
            is_error,
            ..
        } => {
            let summary = summarize_content(content);
            MessageView {
                id: *id,
                role: format!("tool:{tool_name}"),
                title: None,
                content: summary.content,
                thinking: summary.thinking,
                thinking_redacted: summary.thinking_redacted,
                is_error: *is_error,
            }
        }
        Message::Custom { id, name, .. } => MessageView {
            id: *id,
            role: format!("custom:{name}"),
            title: None,
            content: "<custom>".into(),
            thinking: None,
            thinking_redacted: false,
            is_error: false,
        },
        Message::CompactionSummary { id, summary } => MessageView {
            id: *id,
            role: "compaction".into(),
            title: None,
            content: summary.clone(),
            thinking: None,
            thinking_redacted: false,
            is_error: false,
        },
        Message::BranchSummary { id, summary } => MessageView {
            id: *id,
            role: "branch".into(),
            title: None,
            content: summary.clone(),
            thinking: None,
            thinking_redacted: false,
            is_error: false,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSummary {
    pub content: String,
    pub thinking: Option<String>,
    pub thinking_redacted: bool,
}

#[must_use]
pub fn project_content_blocks(content: &[ContentBlock]) -> ContentSummary {
    summarize_content(content)
}

fn summarize_content(content: &[ContentBlock]) -> ContentSummary {
    let mut parts = Vec::new();
    let mut thinking_parts = Vec::new();
    let mut thinking_redacted = false;
    for block in content {
        match block {
            ContentBlock::Text { text } => parts.push(text.clone()),
            ContentBlock::Image { media_type, .. } => parts.push(format!("<image:{media_type}>")),
            ContentBlock::Thinking { text, redacted } => {
                thinking_parts.push(text.clone());
                thinking_redacted |= *redacted;
            }
            ContentBlock::ToolCall { name, .. } => parts.push(format!("<tool-call:{name}>")),
        }
    }
    ContentSummary {
        content: if parts.is_empty() {
            "<empty>".into()
        } else {
            parts.join(" ")
        },
        thinking: if thinking_parts.is_empty() {
            None
        } else {
            Some(thinking_parts.join("\n"))
        },
        thinking_redacted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_types::{Model, ProviderMetadata, StopReason};

    #[test]
    fn assistant_title_uses_provider_model_name() {
        let message = Message::Assistant {
            id: OinoId::nil(),
            content: vec![ContentBlock::Text {
                text: "answer".into(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: None,
            provider: Some(ProviderMetadata {
                request_id: None,
                model: Some(Model::new("openrouter", "test/model")),
                values: Default::default(),
            }),
        };
        let view = project_message(&message);
        assert_eq!(view.role, "assistant");
        assert_eq!(view.title.as_deref(), Some("test/model"));
    }

    #[test]
    fn thinking_is_projected_separately_from_content() {
        let message = Message::Assistant {
            id: OinoId::nil(),
            content: vec![
                ContentBlock::Thinking {
                    text: "private chain".into(),
                    redacted: false,
                },
                ContentBlock::Text {
                    text: "public answer".into(),
                },
            ],
            stop_reason: Some(StopReason::EndTurn),
            usage: None,
            provider: None,
        };
        let view = project_message(&message);
        assert_eq!(view.content, "public answer");
        assert_eq!(view.thinking.as_deref(), Some("private chain"));
        assert!(!view.content.contains("<thinking:"));
    }
}
