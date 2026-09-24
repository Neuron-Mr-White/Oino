//! Transcript replay for system prompt and tool declarations.
//!
//! Port of pi `packages/ai/src/utils/transcript.ts`. [`Message::System`] entries
//! carry the prompt and tool declarations inside the transcript; these helpers
//! replay every system message in order to compute the current prompt and tools
//! and to collapse the transcript for providers without mid-conversation system
//! message support.

use crate::{Message, OinoId, PromptSection, ToolDeclaration};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A transcript normalized for provider requests. Constructed by [`normalize`]
/// (or [`Transcript::new`] for already-normalized message lists).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Transcript(Vec<Message>);

impl Transcript {
    #[must_use]
    pub fn new(messages: Vec<Message>) -> Self {
        Self(messages)
    }

    #[must_use]
    pub fn messages(&self) -> &[Message] {
        &self.0
    }

    #[must_use]
    pub fn into_messages(self) -> Vec<Message> {
        self.0
    }

    pub fn push(&mut self, message: Message) {
        self.0.push(message);
    }
}

impl From<Vec<Message>> for Transcript {
    fn from(messages: Vec<Message>) -> Self {
        Self::new(messages)
    }
}

impl From<Transcript> for Vec<Message> {
    fn from(transcript: Transcript) -> Self {
        transcript.into_messages()
    }
}

/// Fold a system prompt and tool declarations into a leading [`Message::System`].
/// The transcript stays unmodified when both are empty.
#[must_use]
pub fn normalize(
    system_prompt: Option<String>,
    tools: Vec<ToolDeclaration>,
    messages: Vec<Message>,
) -> Transcript {
    let has_prompt = system_prompt
        .as_deref()
        .is_some_and(|prompt| !prompt.is_empty());
    if !has_prompt && tools.is_empty() {
        return Transcript(messages);
    }
    let initial = Message::System {
        id: OinoId::new_v4(),
        content: system_prompt.unwrap_or_default(),
        sections: Vec::new(),
        tools_added: tools,
        tools_removed: Vec::new(),
    };
    let mut normalized = Vec::with_capacity(messages.len() + 1);
    normalized.push(initial);
    normalized.extend(messages);
    Transcript(normalized)
}

/// Resolve the tools available after applying every system message delta in
/// order. Removes apply before adds within each message; later adds replace by
/// name. Output order matches JavaScript `Map` semantics: a declaration keeps
/// its first position when replaced, while a removed-then-re-added tool moves
/// to the end.
#[must_use]
pub fn current_tools(messages: &[Message]) -> Vec<ToolDeclaration> {
    let mut tools: Vec<ToolDeclaration> = Vec::new();
    for message in messages {
        let Message::System {
            tools_added,
            tools_removed,
            ..
        } = message
        else {
            continue;
        };
        for name in tools_removed {
            tools.retain(|tool| tool.name != *name);
        }
        for tool in tools_added {
            if let Some(existing) = tools.iter_mut().find(|existing| existing.name == tool.name) {
                *existing = tool.clone();
            } else {
                tools.push(tool.clone());
            }
        }
    }
    tools
}

/// Every definition referenced by transcript tool state, in first-declaration
/// order. Unlike [`current_tools`], removals do not drop earlier declarations.
#[must_use]
pub fn declared_tools(messages: &[Message]) -> Vec<ToolDeclaration> {
    let mut tools: Vec<ToolDeclaration> = Vec::new();
    for message in messages {
        let Message::System { tools_added, .. } = message else {
            continue;
        };
        for tool in tools_added {
            if let Some(existing) = tools.iter_mut().find(|existing| existing.name == tool.name) {
                *existing = tool.clone();
            } else {
                tools.push(tool.clone());
            }
        }
    }
    tools
}

/// Replay every system message into one leading [`Message::System`] holding the
/// current prompt and tools. Content parts join with `"\n\n"`, sections patch
/// by name in declaration order (`text: None` removes), and tools come from
/// [`current_tools`]. Returns `None` when no system message exists and no tools
/// were declared.
#[must_use]
pub fn current_system(messages: &[Message]) -> Option<Message> {
    let mut content_parts: Vec<String> = Vec::new();
    let mut sections: Vec<PromptSection> = Vec::new();
    let mut first_id: Option<OinoId> = None;
    for message in messages {
        let Message::System {
            id,
            content,
            sections: message_sections,
            ..
        } = message
        else {
            continue;
        };
        first_id.get_or_insert(*id);
        if !content.is_empty() {
            content_parts.push(content.clone());
        }
        for section in message_sections {
            if section.text.is_none() {
                sections.retain(|existing| existing.name != section.name);
            } else if let Some(existing) = sections
                .iter_mut()
                .find(|existing| existing.name == section.name)
            {
                existing.text = section.text.clone();
            } else {
                sections.push(section.clone());
            }
        }
    }
    let tools = current_tools(messages);
    if first_id.is_none() && tools.is_empty() {
        return None;
    }
    Some(Message::System {
        id: first_id.unwrap_or_else(OinoId::nil),
        content: content_parts.join("\n\n"),
        sections,
        tools_added: tools,
        tools_removed: Vec::new(),
    })
}

/// Render a [`Message::System`] as prompt text: `content`, then each `Some`
/// section text in order, joined with `"\n\n"` and skipping empties.
#[must_use]
pub fn render_system_prompt(message: &Message) -> String {
    let Message::System {
        content, sections, ..
    } = message
    else {
        return String::new();
    };
    let mut parts: Vec<&str> = Vec::new();
    if !content.is_empty() {
        parts.push(content.as_str());
    }
    for section in sections {
        if let Some(text) = section.text.as_deref() {
            if !text.is_empty() {
                parts.push(text);
            }
        }
    }
    parts.join("\n\n")
}

/// Rebuild the transcript for APIs without mid-conversation system messages:
/// the replayed system message leads and every later system message is dropped.
#[must_use]
pub fn collapse(transcript: &Transcript) -> Transcript {
    let head = current_system(transcript.messages());
    let mut messages: Vec<Message> = Vec::new();
    if let Some(head) = head {
        messages.push(head);
    }
    messages.extend(
        transcript
            .messages()
            .iter()
            .filter(|message| !matches!(message, Message::System { .. }))
            .cloned(),
    );
    Transcript(messages)
}

/// Keep later system messages in place when the model accepts them; otherwise
/// collapse them.
#[must_use]
pub fn resolve(transcript: Transcript, supports_mid_convo_system: bool) -> Transcript {
    if supports_mid_convo_system {
        transcript
    } else {
        collapse(&transcript)
    }
}

/// Canonical JSON for a tool declaration: object keys sorted recursively so
/// reordering map keys never counts as a change.
#[must_use]
pub fn tool_declaration_canonical(declaration: &ToolDeclaration) -> String {
    fn sorted(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), sorted(value)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(sorted).collect())
            }
            other => other.clone(),
        }
    }
    let canonical = serde_json::json!({
        "name": declaration.name,
        "description": declaration.description,
        "input_schema": sorted(&declaration.input_schema),
    });
    serde_json::to_string(&canonical).unwrap_or_else(|_| canonical.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OinoId;

    fn tool(name: &str) -> ToolDeclaration {
        ToolDeclaration {
            name: name.into(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn system(content: &str, added: Vec<ToolDeclaration>, removed: Vec<&str>) -> Message {
        Message::System {
            id: OinoId::new_v4(),
            content: content.into(),
            sections: Vec::new(),
            tools_added: added,
            tools_removed: removed.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn normalize_adds_leading_system_only_when_prompt_or_tools() {
        let messages = vec![Message::user_text("hi")];
        let empty = normalize(None, Vec::new(), messages.clone());
        assert_eq!(empty.messages(), messages.as_slice());

        let with_prompt = normalize(Some("be kind".into()), Vec::new(), messages.clone());
        assert!(matches!(
            with_prompt.messages().first(),
            Some(Message::System { content, .. }) if content == "be kind"
        ));
        assert_eq!(with_prompt.messages().len(), 2);

        let with_tools = normalize(None, vec![tool("read")], messages.clone());
        assert!(matches!(
            with_tools.messages().first(),
            Some(Message::System { tools_added, .. }) if tools_added.len() == 1
        ));
    }

    #[test]
    fn current_tools_applies_removes_then_adds_per_message() {
        let messages = vec![
            system("", vec![tool("a"), tool("b")], vec![]),
            system("", vec![], vec!["a"]),
            system("", vec![tool("c")], vec![]),
        ];
        let names: Vec<String> = current_tools(&messages)
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(names, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn current_tools_readd_after_removal_moves_to_end() {
        let mut readded = tool("a");
        readded.description = "re-added".into();
        let messages = vec![
            system("", vec![tool("a"), tool("b")], vec![]),
            system("", vec![], vec!["a"]),
            system("", vec![readded], vec![]),
        ];
        let tools = current_tools(&messages);
        let names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names, vec!["b".to_string(), "a".to_string()]);
        assert_eq!(tools[1].description, "re-added");
    }

    #[test]
    fn current_tools_replace_in_place_keeps_first_position() {
        let mut replaced = tool("a");
        replaced.description = "v2".into();
        let messages = vec![
            system("", vec![tool("a"), tool("b")], vec![]),
            system("", vec![replaced], vec![]),
        ];
        let tools = current_tools(&messages);
        assert_eq!(tools[0].name, "a");
        assert_eq!(tools[0].description, "v2");
        assert_eq!(tools[1].name, "b");
    }

    #[test]
    fn current_system_joins_content_and_patches_sections() {
        let first = Message::System {
            id: OinoId::new_v4(),
            content: "base".into(),
            sections: vec![
                PromptSection {
                    name: "project/AGENT.md".into(),
                    text: Some("rules".into()),
                },
                PromptSection {
                    name: "oino/cwd".into(),
                    text: Some("/repo".into()),
                },
            ],
            tools_added: vec![tool("read")],
            tools_removed: Vec::new(),
        };
        let delta = Message::System {
            id: OinoId::new_v4(),
            content: "extra".into(),
            sections: vec![
                PromptSection {
                    name: "project/AGENT.md".into(),
                    text: Some("rules v2".into()),
                },
                PromptSection {
                    name: "oino/cwd".into(),
                    text: None,
                },
            ],
            tools_added: vec![tool("write")],
            tools_removed: Vec::new(),
        };
        let replayed = match current_system(&[first, delta]) {
            Some(message) => message,
            None => panic!("expected replayed system message"),
        };
        let Message::System {
            content,
            sections,
            tools_added,
            tools_removed,
            ..
        } = replayed
        else {
            panic!("expected system message");
        };
        assert_eq!(content, "base\n\nextra");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "project/AGENT.md");
        assert_eq!(sections[0].text.as_deref(), Some("rules v2"));
        let names: Vec<String> = tools_added.iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names, vec!["read".to_string(), "write".to_string()]);
        assert!(tools_removed.is_empty());
    }

    #[test]
    fn render_system_prompt_skips_empty_parts() {
        let message = Message::System {
            id: OinoId::new_v4(),
            content: "base".into(),
            sections: vec![
                PromptSection {
                    name: "a".into(),
                    text: Some("section a".into()),
                },
                PromptSection {
                    name: "b".into(),
                    text: Some(String::new()),
                },
            ],
            tools_added: Vec::new(),
            tools_removed: Vec::new(),
        };
        assert_eq!(render_system_prompt(&message), "base\n\nsection a");
        assert_eq!(render_system_prompt(&Message::user_text("x")), "");
    }

    #[test]
    fn empty_transcript_has_no_system() {
        assert!(current_system(&[]).is_none());
        let empty = normalize(None, Vec::new(), Vec::new());
        assert!(empty.messages().is_empty());
        assert!(collapse(&empty).messages().is_empty());
    }

    #[test]
    fn collapse_replays_leading_system_and_drops_later_ones() {
        let messages = vec![
            system("base", vec![tool("a")], vec![]),
            Message::user_text("hi"),
            system("extra", vec![tool("b")], vec!["a"]),
            Message::assistant_text("ok", crate::StopReason::EndTurn),
        ];
        let collapsed = collapse(&Transcript::new(messages));
        assert_eq!(collapsed.messages().len(), 3);
        let Message::System {
            content,
            tools_added,
            ..
        } = &collapsed.messages()[0]
        else {
            panic!("expected leading replayed system");
        };
        assert_eq!(content, "base\n\nextra");
        assert_eq!(tools_added.len(), 1);
        assert_eq!(tools_added[0].name, "b");
        assert!(matches!(collapsed.messages()[1], Message::User { .. }));
        assert!(matches!(collapsed.messages()[2], Message::Assistant { .. }));
    }

    #[test]
    fn resolve_collapses_only_without_mid_convo_support() {
        let transcript = normalize(
            Some("base".into()),
            vec![tool("a")],
            vec![Message::user_text("hi"), system("extra", vec![], vec![])],
        );
        let kept = resolve(transcript.clone(), true);
        assert_eq!(kept.messages().len(), 3);
        let collapsed = resolve(transcript, false);
        assert_eq!(collapsed.messages().len(), 2);
    }

    #[test]
    fn tool_declaration_canonical_ignores_key_order() {
        let mut a = tool("x");
        a.input_schema = serde_json::json!({"type": "object", "properties": {"b": 1, "a": 2}});
        let mut b = tool("x");
        b.input_schema = serde_json::json!({"properties": {"a": 2, "b": 1}, "type": "object"});
        assert_eq!(
            tool_declaration_canonical(&a),
            tool_declaration_canonical(&b)
        );
    }
}
