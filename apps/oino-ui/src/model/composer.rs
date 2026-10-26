//! Composer intents (gpui-app §5.3, pi TUI semantics): `Enter` sends (or steers while a run is
//! active), `Alt/Ctrl+Enter` queues a follow-up, `!cmd` runs a shell command (`!!` keeps it
//! out of the context). Also prompt history, per-session drafts and slash-command matching.

use std::collections::HashMap;

use oino_ai::ImageContent;
use oino_client::{PiCommand, RpcCommand};
use oino_core::session::StreamingBehavior;

/// Which submit chord was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitKey {
    /// `Enter`.
    Enter,
    /// `Alt+Enter` / `Ctrl+Enter`: follow-up.
    FollowUp,
}

/// What a submit does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Nothing to send.
    Nothing,
    /// New prompt (slash commands, skills and templates are expanded by core).
    Prompt(String),
    /// Steer the running agent.
    Steer(String),
    /// Queue a follow-up.
    FollowUp(String),
    /// `!command` / `!!command`.
    Bash {
        /// Command line.
        command: String,
        /// `!!`: exclude from context.
        exclude: bool,
    },
}

/// Resolves the intent of submitting `text`.
#[must_use]
pub fn intent(text: &str, busy: bool, key: SubmitKey) -> Intent {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Intent::Nothing;
    }
    if let Some(rest) = trimmed.strip_prefix('!') {
        let (exclude, command) = match rest.strip_prefix('!') {
            Some(command) => (true, command),
            None => (false, rest),
        };
        let command = command.trim();
        if !command.is_empty() {
            return Intent::Bash {
                command: command.to_owned(),
                exclude,
            };
        }
    }
    let text = trimmed.to_owned();
    match (busy, key) {
        (false, SubmitKey::Enter) => Intent::Prompt(text),
        (true, SubmitKey::Enter) => Intent::Steer(text),
        (_, SubmitKey::FollowUp) => {
            if busy {
                Intent::FollowUp(text)
            } else {
                Intent::Prompt(text)
            }
        }
    }
}

impl Intent {
    /// The backend command for this intent.
    #[must_use]
    pub fn command(self, images: Vec<ImageContent>) -> Option<RpcCommand> {
        let command = match self {
            Self::Nothing => return None,
            Self::Prompt(message) => PiCommand::Prompt {
                message,
                images,
                streaming_behavior: None,
            },
            Self::Steer(message) => PiCommand::Prompt {
                message,
                images,
                streaming_behavior: Some(StreamingBehavior::Steer),
            },
            Self::FollowUp(message) => PiCommand::Prompt {
                message,
                images,
                streaming_behavior: Some(StreamingBehavior::FollowUp),
            },
            Self::Bash { command, exclude } => PiCommand::Bash {
                command,
                exclude_from_context: exclude.then_some(true),
            },
        };
        Some(command.into())
    }
}

/// Prompt history of one session (↑/↓ in an empty or recalled composer).
#[derive(Clone, Debug, Default)]
pub struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
}

impl History {
    /// Records a sent prompt (consecutive duplicates collapse) and resets navigation.
    pub fn push(&mut self, text: &str) {
        let text = text.trim();
        if !text.is_empty() && self.entries.last().map(String::as_str) != Some(text) {
            self.entries.push(text.to_owned());
        }
        self.cursor = None;
    }

    /// Older entry.
    pub fn older(&mut self) -> Option<&str> {
        let next = match self.cursor {
            None => self.entries.len().checked_sub(1)?,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.cursor = Some(next);
        self.entries.get(next).map(String::as_str)
    }

    /// Newer entry; `Some("")` past the newest.
    pub fn newer(&mut self) -> Option<&str> {
        let i = self.cursor?;
        if i + 1 >= self.entries.len() {
            self.cursor = None;
            return Some("");
        }
        self.cursor = Some(i + 1);
        self.entries.get(i + 1).map(String::as_str)
    }

    /// Whether the user is browsing history.
    #[must_use]
    pub fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }
}

/// Drafts and histories per session id.
#[derive(Clone, Debug, Default)]
pub struct Drafts {
    drafts: HashMap<String, String>,
    histories: HashMap<String, History>,
}

impl Drafts {
    /// Saves the draft of `session`.
    pub fn save(&mut self, session: &str, text: &str) {
        if text.is_empty() {
            self.drafts.remove(session);
        } else {
            self.drafts.insert(session.to_owned(), text.to_owned());
        }
    }

    /// Draft of `session`.
    #[must_use]
    pub fn draft(&self, session: &str) -> &str {
        self.drafts.get(session).map_or("", String::as_str)
    }

    /// History of `session`.
    pub fn history(&mut self, session: &str) -> &mut History {
        self.histories.entry(session.to_owned()).or_default()
    }
}

/// A slash-command candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Inserted text (`/name `).
    pub value: String,
    /// Label (`/name`).
    pub label: String,
    /// Description.
    pub description: Option<String>,
}

/// Slash-command completions for a composer whose whole text is `/partial` (no space yet):
/// prefix matches first, then substring matches, each alphabetically; at most `limit`.
#[must_use]
pub fn slash_completions(
    commands: &[(String, Option<String>)],
    text: &str,
    limit: usize,
) -> Vec<Completion> {
    let Some(partial) = text.strip_prefix('/') else {
        return Vec::new();
    };
    if partial.contains(char::is_whitespace) {
        return Vec::new();
    }
    let needle = partial.to_lowercase();
    let mut prefix = Vec::new();
    let mut infix = Vec::new();
    for (name, description) in commands {
        let lower = name.to_lowercase();
        let bucket = if lower.starts_with(&needle) {
            &mut prefix
        } else if lower.contains(&needle) {
            &mut infix
        } else {
            continue;
        };
        bucket.push(Completion {
            value: format!("/{name} "),
            label: format!("/{name}"),
            description: description.clone(),
        });
    }
    prefix.sort_by(|a, b| a.label.cmp(&b.label));
    infix.sort_by(|a, b| a.label.cmp(&b.label));
    prefix.into_iter().chain(infix).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intents_follow_pi_semantics() {
        assert_eq!(intent("  ", false, SubmitKey::Enter), Intent::Nothing);
        assert_eq!(
            intent("hi", false, SubmitKey::Enter),
            Intent::Prompt("hi".into())
        );
        assert_eq!(
            intent("hi", true, SubmitKey::Enter),
            Intent::Steer("hi".into())
        );
        assert_eq!(
            intent("hi", true, SubmitKey::FollowUp),
            Intent::FollowUp("hi".into())
        );
        assert_eq!(
            intent("hi", false, SubmitKey::FollowUp),
            Intent::Prompt("hi".into())
        );
        assert_eq!(
            intent("!!ls -la", false, SubmitKey::Enter),
            Intent::Bash {
                command: "ls -la".into(),
                exclude: true
            }
        );
        assert_eq!(
            intent("!", false, SubmitKey::Enter),
            Intent::Prompt("!".into())
        );
    }

    #[test]
    fn intents_map_to_commands() {
        let steer = Intent::Steer("x".into()).command(Vec::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&steer).unwrap(),
            serde_json::json!({"type":"prompt","message":"x","streamingBehavior":"steer"})
        );
        let bash = Intent::Bash {
            command: "ls".into(),
            exclude: false,
        }
        .command(Vec::new())
        .unwrap();
        assert_eq!(
            serde_json::to_value(&bash).unwrap(),
            serde_json::json!({"type":"bash","command":"ls"})
        );
        assert!(Intent::Nothing.command(Vec::new()).is_none());
    }

    #[test]
    fn history_navigation() {
        let mut h = History::default();
        assert_eq!(h.older(), None);
        h.push("one");
        h.push("two");
        h.push("two");
        assert_eq!(h.older(), Some("two"));
        assert_eq!(h.older(), Some("one"));
        assert_eq!(h.older(), Some("one"));
        assert_eq!(h.newer(), Some("two"));
        assert_eq!(h.newer(), Some(""));
        assert!(!h.is_browsing());
    }

    #[test]
    fn drafts_per_session() {
        let mut d = Drafts::default();
        d.save("a", "draft");
        assert_eq!(d.draft("a"), "draft");
        assert_eq!(d.draft("b"), "");
        d.save("a", "");
        assert_eq!(d.draft("a"), "");
        d.history("a").push("x");
        assert_eq!(d.history("a").older(), Some("x"));
    }

    #[test]
    fn slash_matching() {
        let commands = vec![
            ("compact".to_owned(), Some("Compact".to_owned())),
            ("model".to_owned(), None),
            ("unipi:compact-now".to_owned(), None),
        ];
        let labels: Vec<String> = slash_completions(&commands, "/comp", 10)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["/compact", "/unipi:compact-now"]);
        assert!(slash_completions(&commands, "/compact now", 10).is_empty());
        assert!(slash_completions(&commands, "compact", 10).is_empty());
        assert_eq!(slash_completions(&commands, "/", 2).len(), 2);
    }
}
