//! System-message replay (pi-ai `utils/transcript.js`, `utils/text.js`) and the
//! `diffSystemPromptSections` patch helper.
//!
//! A transcript carries the prompt and tool loadout in system messages: the first declares the
//! base state; later ones patch `sections` by name (`null` removes) and list
//! `toolsAdded`/`toolsRemoved`. Replaying them in order yields the current state.

use indexmap::IndexMap;

use crate::content::UserTextField;
use crate::message::{Message, Sections, SystemMessage, ToolDecl, ToolRef};
use crate::primitives::TimestampMs;
use crate::stream::Context;

/// Replayed prompt/tool state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemState {
    /// Non-empty contents joined with a blank line.
    pub content: String,
    /// Live sections in insertion order.
    pub sections: IndexMap<String, String>,
    /// Live tools in declaration order.
    pub tools: Vec<ToolDecl>,
    /// Timestamp of the first system message, if any.
    pub first_timestamp: Option<TimestampMs>,
}

impl SystemState {
    /// Folds system messages in order (pi `getCurrentSystemMessage` internals).
    pub fn fold<'a>(systems: impl IntoIterator<Item = &'a SystemMessage>) -> Self {
        let mut content = Vec::new();
        let mut sections = IndexMap::new();
        let mut tools: IndexMap<String, ToolDecl> = IndexMap::new();
        let mut first_timestamp = None;
        for message in systems {
            first_timestamp.get_or_insert(message.timestamp);
            let text = message.content.text("\n");
            if !text.is_empty() {
                content.push(text);
            }
            for (name, value) in message.sections.iter().flatten() {
                match value {
                    Some(text) => {
                        sections.insert(name.clone(), text.clone());
                    }
                    None => {
                        sections.shift_remove(name);
                    }
                }
            }
            for removed in message.tools_removed.iter().flatten() {
                tools.shift_remove(&removed.name);
            }
            for added in message.tools_added.iter().flatten() {
                tools.insert(added.name.clone(), added.clone());
            }
        }
        Self {
            content: content.join("\n\n"),
            sections,
            tools: tools.into_values().collect(),
            first_timestamp,
        }
    }

    /// The equivalent single system message, or `None` when no system message existed and no
    /// tool is live (pi returns `undefined`).
    #[must_use]
    pub fn into_message(self) -> Option<SystemMessage> {
        if self.first_timestamp.is_none() && self.tools.is_empty() {
            return None;
        }
        Some(SystemMessage {
            content: UserTextField::Text(self.content),
            sections: (!self.sections.is_empty()).then(|| {
                self.sections
                    .into_iter()
                    .map(|(name, text)| (name, Some(text)))
                    .collect()
            }),
            tools_added: (!self.tools.is_empty()).then_some(self.tools),
            tools_removed: None,
            timestamp: self.first_timestamp.unwrap_or(0),
        })
    }
}

fn systems(messages: &[Message]) -> impl Iterator<Item = &SystemMessage> {
    messages.iter().filter_map(|message| match message {
        Message::System(system) => Some(system),
        _ => None,
    })
}

/// Replayed state of all system messages in `messages`.
#[must_use]
pub fn current_system(messages: &[Message]) -> SystemState {
    SystemState::fold(systems(messages))
}

/// pi `getCurrentSystemMessage`.
#[must_use]
pub fn current_system_message(messages: &[Message]) -> Option<SystemMessage> {
    current_system(messages).into_message()
}

/// pi `getCurrentTools`.
#[must_use]
pub fn current_tools(messages: &[Message]) -> Vec<ToolDecl> {
    current_system(messages).tools
}

/// pi `getDeclaredTools`: every tool ever declared (latest definition per name).
#[must_use]
pub fn declared_tools(messages: &[Message]) -> Vec<ToolDecl> {
    let mut definitions: IndexMap<String, ToolDecl> = IndexMap::new();
    for added in systems(messages).flat_map(|m| m.tools_added.iter().flatten()) {
        definitions.insert(added.name.clone(), added.clone());
    }
    definitions.into_values().collect()
}

/// pi `getSystemMessageText`: content plus live section texts joined with blank lines.
#[must_use]
pub fn system_message_text(message: &SystemMessage) -> String {
    std::iter::once(message.content.text("\n"))
        .chain(
            message
                .sections
                .iter()
                .flatten()
                .filter_map(|(_, text)| text.clone()),
        )
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// pi `renderSystemMessageUpdate`: human-readable rendering of a system patch.
#[must_use]
pub fn render_system_message_update(message: &SystemMessage) -> String {
    let mut parts = Vec::new();
    let text = message.content.text("\n");
    if !text.is_empty() {
        parts.push(text);
    }
    for (name, value) in message.sections.iter().flatten() {
        parts.push(match value {
            None => format!("Removed system prompt section \"{name}\"."),
            Some(value) => format!("Updated system prompt section \"{name}\":\n\n{value}"),
        });
    }
    parts.join("\n\n")
}

/// pi `diffSystemPromptSections(previous, current)`: changed/added sections with their text,
/// then removed sections as `null`; `None` when nothing changed.
#[must_use]
pub fn diff_sections(
    previous: &IndexMap<String, String>,
    current: &IndexMap<String, String>,
) -> Option<Sections> {
    let mut patch = Sections::new();
    for (name, text) in current {
        if previous.get(name) != Some(text) {
            patch.insert(name.clone(), Some(text.clone()));
        }
    }
    for name in previous.keys() {
        if !current.contains_key(name) {
            patch.insert(name.clone(), None);
        }
    }
    (!patch.is_empty()).then_some(patch)
}

/// pi `createInitialSystemMessage`: a leading system message for a prompt and tools, or
/// `None` when both are empty.
#[must_use]
pub fn initial_system_message(system_prompt: &str, tools: &[ToolDecl]) -> Option<SystemMessage> {
    if system_prompt.is_empty() && tools.is_empty() {
        return None;
    }
    Some(SystemMessage {
        content: UserTextField::Text(system_prompt.to_owned()),
        sections: None,
        tools_added: (!tools.is_empty()).then(|| tools.to_vec()),
        tools_removed: None,
        timestamp: 0,
    })
}

/// pi `collapseSystemMessages`: one leading folded system message, other messages unchanged.
#[must_use]
pub fn collapse_system_messages(context: Context) -> Context {
    let head = current_system_message(&context.messages);
    let rest = context
        .messages
        .into_iter()
        .filter(|message| !matches!(message, Message::System(_)));
    Context {
        messages: head.map(Message::System).into_iter().chain(rest).collect(),
    }
}

/// How much mid-conversation system support a provider has (pi `resolveTranscriptTools`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolChangeSupport {
    /// Tools are only sent as request-level declarations.
    None,
    /// Tool changes can be expressed inline.
    Changes,
    /// Additions inline; removals/redefinitions force folding.
    Additive,
}

/// Result of [`resolve_transcript_tools`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedTools {
    /// Tools to declare at request level.
    pub request_tools: Vec<ToolDecl>,
    /// Number of leading system messages folded into `request_tools`.
    pub folded_system_messages: usize,
}

/// pi `resolveTranscriptTools`.
#[must_use]
pub fn resolve_transcript_tools(messages: &[Message], support: ToolChangeSupport) -> ResolvedTools {
    let system_messages: Vec<&SystemMessage> = systems(messages).collect();
    match support {
        ToolChangeSupport::None => ResolvedTools {
            request_tools: SystemState::fold(system_messages.iter().copied()).tools,
            folded_system_messages: system_messages.len(),
        },
        ToolChangeSupport::Changes => ResolvedTools::default(),
        ToolChangeSupport::Additive => {
            let leading = usize::from(matches!(messages.first(), Some(Message::System(_))));
            let folded =
                leading.max(last_non_additive_change(&system_messages).map_or(0, |i| i + 1));
            ResolvedTools {
                request_tools: SystemState::fold(system_messages[..folded].iter().copied()).tools,
                folded_system_messages: folded,
            }
        }
    }
}

fn last_non_additive_change(system_messages: &[&SystemMessage]) -> Option<usize> {
    let mut declared = std::collections::HashSet::new();
    let mut last = None;
    for (index, message) in system_messages.iter().enumerate() {
        if message
            .tools_removed
            .as_ref()
            .is_some_and(|removed| !removed.is_empty())
        {
            last = Some(index);
        }
        for tool in message.tools_added.iter().flatten() {
            if !declared.insert(tool.name.as_str()) {
                last = Some(index);
            }
        }
    }
    last
}

/// pi `hasNonAdditiveToolChanges`.
#[must_use]
pub fn has_non_additive_tool_changes(messages: &[Message]) -> bool {
    last_non_additive_change(&systems(messages).collect::<Vec<_>>()).is_some()
}

/// pi `getToolStateChanges(previous, current)`: tools to add (new or changed) and remove.
#[must_use]
pub fn tool_state_changes(
    previous: &[ToolDecl],
    current: &[ToolDecl],
) -> (Vec<ToolDecl>, Vec<ToolRef>) {
    let find = |tools: &[ToolDecl], name: &str| tools.iter().find(|t| t.name == name).cloned();
    let added = current
        .iter()
        .filter(|tool| find(previous, &tool.name).as_ref() != Some(*tool))
        .cloned()
        .collect();
    let removed = previous
        .iter()
        .filter(|tool| find(current, &tool.name).as_ref() != Some(*tool))
        .map(|tool| ToolRef {
            name: tool.name.clone(),
        })
        .collect();
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::UserMessage;
    use serde_json::json;

    fn system(raw: serde_json::Value) -> Message {
        serde_json::from_value(raw).unwrap_or(Message::User(UserMessage::default()))
    }

    #[test]
    fn replay_patches_sections_and_tools() {
        let messages = vec![
            system(
                json!({"role":"system","content":"base","sections":{"a":"A","b":"B"},
                "toolsAdded":[{"name":"read","description":"r","parameters":{}},{"name":"write","description":"w","parameters":{}}],"timestamp":5}),
            ),
            Message::User(UserMessage::default()),
            system(
                json!({"role":"system","content":"","sections":{"a":null,"c":"C","b":"B2"},"toolsRemoved":[{"name":"read"}],"timestamp":9}),
            ),
        ];
        let current = current_system_message(&messages);
        assert_eq!(
            current.and_then(|m| serde_json::to_value(Message::System(m)).ok()),
            Some(
                json!({"role":"system","content":"base","sections":{"b":"B2","c":"C"},
                "toolsAdded":[{"name":"write","description":"w","parameters":{}}],"timestamp":5})
            )
        );
        assert!(has_non_additive_tool_changes(&messages));
        let resolved = resolve_transcript_tools(&messages, ToolChangeSupport::Additive);
        assert_eq!(resolved.folded_system_messages, 2);
        assert_eq!(
            collapse_system_messages(Context { messages })
                .messages
                .len(),
            2
        );
    }

    #[test]
    fn no_system_messages_yields_none() {
        assert_eq!(
            current_system_message(&[Message::User(UserMessage::default())]),
            None
        );
    }

    #[test]
    fn diff_sections_matches_pi() {
        let previous: IndexMap<String, String> = [("a", "1"), ("b", "2")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into_iter()
            .collect();
        let current: IndexMap<String, String> = [("b", "3"), ("c", "4")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into_iter()
            .collect();
        let patch = diff_sections(&previous, &current);
        assert_eq!(
            serde_json::to_value(patch).ok(),
            Some(json!({"b":"3","c":"4","a":null}))
        );
        assert_eq!(diff_sections(&current, &current), None);
    }
}
