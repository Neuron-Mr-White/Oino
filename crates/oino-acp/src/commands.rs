//! Slash commands: the adapter's built-ins (pi frontend commands that call the session API,
//! modes §1.3) plus the session's extension commands, prompt templates and skills
//! (`available_commands_update`).

use oino_core::session::AgentSession;
use serde_json::{Value as JsonValue, json};

/// A built-in command the adapter runs itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuiltinCommand {
    /// `/compact [instructions]`.
    Compact(Option<String>),
    /// `/name <name>`.
    Name(String),
    /// `/export [path]` (HTML).
    Export(Option<String>),
    /// `/session`: session file, id and stats.
    Session,
}

/// `(name, description, input hint)` of the built-ins.
const BUILTINS: [(&str, &str, Option<&str>); 4] = [
    (
        "compact",
        "Compact the conversation (optional custom instructions)",
        Some("instructions"),
    ),
    ("name", "Set the session name", Some("name")),
    ("export", "Export the session as HTML", Some("output path")),
    ("session", "Show session info and stats", None),
];

fn rest(text: &str) -> Option<String> {
    let rest = text.trim();
    (!rest.is_empty()).then(|| rest.to_owned())
}

impl BuiltinCommand {
    /// Parses `/compact …`, `/name …`, `/export …`, `/session`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().strip_prefix('/')?;
        let (name, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        match name {
            "compact" => Some(Self::Compact(rest(args))),
            "name" => rest(args).map(Self::Name),
            "export" => Some(Self::Export(rest(args))),
            "session" => Some(Self::Session),
            _ => None,
        }
    }
}

/// The `available_commands_update` payload of `session`.
#[must_use]
pub fn available_commands(session: &AgentSession) -> JsonValue {
    let mut commands: Vec<JsonValue> = BUILTINS
        .iter()
        .map(|(name, description, hint)| {
            let mut command = json!({ "name": name, "description": description });
            if let Some(hint) = hint {
                command["input"] = json!({ "hint": hint });
            }
            command
        })
        .collect();
    for info in session.commands() {
        if BUILTINS.iter().any(|(name, _, _)| *name == info.name) {
            continue;
        }
        let description = info
            .description
            .unwrap_or_else(|| match info.source.as_str() {
                "skill" => "Skill".to_owned(),
                "prompt" => "Prompt template".to_owned(),
                _ => "Extension command".to_owned(),
            });
        commands.push(json!({
            "name": info.name,
            "description": description,
            "input": { "hint": "arguments" },
        }));
    }
    json!({ "sessionUpdate": "available_commands_update", "availableCommands": commands })
}

/// Text reported by `/session`.
#[must_use]
pub fn session_report(session: &AgentSession) -> String {
    let stats = session.session_stats();
    let mut lines = vec![format!("Session: {}", stats.session_id)];
    if let Some(file) = &stats.session_file {
        lines.push(format!("File: {file}"));
    }
    if let Some(name) = session.session_name() {
        lines.push(format!("Name: {name}"));
    }
    if let Some(model) = session.model() {
        lines.push(format!("Model: {}/{}", model.provider, model.id));
    }
    lines.push(format!(
        "Messages: {} user, {} assistant, {} tool calls",
        stats.user_messages, stats.assistant_messages, stats.tool_calls
    ));
    lines.push(format!(
        "Tokens: {} in, {} out, {} cache read, {} cache write",
        stats.tokens.input, stats.tokens.output, stats.tokens.cache_read, stats.tokens.cache_write
    ));
    lines.push(format!("Cost: ${:.4}", stats.cost));
    if let Some(usage) = stats.context_usage
        && let Some(tokens) = usage.tokens
    {
        lines.push(format!("Context: {tokens} / {}", usage.context_window));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_builtins() {
        assert_eq!(
            BuiltinCommand::parse("/compact keep the API"),
            Some(BuiltinCommand::Compact(Some("keep the API".into())))
        );
        assert_eq!(
            BuiltinCommand::parse("/compact"),
            Some(BuiltinCommand::Compact(None))
        );
        assert_eq!(
            BuiltinCommand::parse("/name  my work "),
            Some(BuiltinCommand::Name("my work".into()))
        );
        assert_eq!(BuiltinCommand::parse("/name"), None);
        assert_eq!(
            BuiltinCommand::parse("/session"),
            Some(BuiltinCommand::Session)
        );
        assert_eq!(BuiltinCommand::parse("/skill:x do"), None);
        assert_eq!(BuiltinCommand::parse("hello"), None);
    }
}
