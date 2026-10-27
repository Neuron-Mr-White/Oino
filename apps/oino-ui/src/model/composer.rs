//! Composer intents (gpui-app §5.3, pi TUI semantics): `Enter` sends (or steers while a run is
//! active), `Alt/Ctrl+Enter` queues a follow-up, `!cmd` runs a shell command (`!!` keeps it
//! out of the context). Also prompt history, per-session drafts and slash-command matching.

use std::collections::HashMap;

use oino_ai::ImageContent;
use oino_client::{PiCommand, RpcCommand};
use oino_core::session::StreamingBehavior;
use oino_view::commands::{Submitted, classify};

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
    /// A pi built-in (`/new`, `/compact`, `/model`…) handled by the frontend (GA-3).
    Builtin(Builtin, Option<String>),
}

pub use oino_view::commands::{BUILTINS, Builtin};

/// Resolves the intent of submitting `text`.
#[must_use]
pub fn intent(text: &str, busy: bool, key: SubmitKey) -> Intent {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Intent::Nothing;
    }
    if let Submitted::Builtin(builtin, arg) = classify(trimmed) {
        return Intent::Builtin(builtin, arg);
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
            Self::Nothing | Self::Builtin(..) => return None,
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

/// Workflow permission mode shown by the composer mode chip (design.md §4.11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeChip {
    /// `ask`: ask before changes.
    Ask,
    /// Plan mode.
    Plan,
    /// `auto`: jev-judged.
    Auto,
    /// `full`: full access.
    Full,
}

impl ModeChip {
    /// Menu order.
    pub const ALL: [Self; 4] = [Self::Ask, Self::Plan, Self::Auto, Self::Full];

    /// Label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ask => "Ask before changes",
            Self::Plan => "Plan",
            Self::Auto => "Auto",
            Self::Full => "Full access",
        }
    }

    /// Lucide icon stem.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Ask => "hand",
            Self::Plan => "list-checks",
            Self::Auto => "sparkles",
            Self::Full => "shield-off",
        }
    }

    /// The workflow commands selecting this mode from `current`.
    #[must_use]
    pub fn commands(self, current: Option<Self>) -> Vec<String> {
        let mode = match self {
            Self::Plan => return vec!["/plan on".to_owned()],
            Self::Ask => "ask",
            Self::Full => "full",
            Self::Auto => "auto",
        };
        let mut out = Vec::new();
        if current == Some(Self::Plan) {
            out.push("/plan off".to_owned());
        }
        out.push(format!("/permission {mode}"));
        out
    }

    /// The mode from the sticky `unipi:workflow:status` topic (`{planMode, permissionMode}`,
    /// GA-25: never from footer segments).
    #[must_use]
    pub fn from_topic(data: &oino_ai::JsonValue) -> Option<Self> {
        if data.get("planMode").and_then(oino_ai::JsonValue::as_bool) == Some(true) {
            return Some(Self::Plan);
        }
        let mode = data.get("permissionMode")?.as_str()?;
        Some(match mode.trim().to_lowercase().as_str() {
            "ask" => Self::Ask,
            "full" => Self::Full,
            _ => Self::Auto,
        })
    }

    /// The mode from the footer frame's `plan` / `permission` segments.
    #[must_use]
    pub fn from_segments<'a>(
        segments: impl IntoIterator<Item = &'a crate::model::ui_state::Segment>,
    ) -> Option<Self> {
        let mut permission = None;
        for seg in segments {
            match seg.id.as_deref() {
                Some("plan") => return Some(Self::Plan),
                Some("permission") => {
                    permission = Some(match seg.text.trim().to_lowercase().as_str() {
                        "ask" => Self::Ask,
                        "full" => Self::Full,
                        _ => Self::Auto,
                    });
                }
                _ => {}
            }
        }
        permission
    }
}

/// Width below which the composer toolbar compresses (design.md §4.11).
pub const COMPRESS_BELOW: f32 = 520.0;

/// The completion trigger under the cursor: `(prefix char, char offset of the token start)`.
#[must_use]
pub fn trigger(text: &str, cursor: usize) -> Option<(char, usize)> {
    let before: Vec<char> = text.chars().take(cursor).collect();
    let start = before
        .iter()
        .rposition(|c| c.is_whitespace())
        .map_or(0, |i| i + 1);
    let first = *before.get(start)?;
    match first {
        '@' | '$' | '#' => Some((first, start)),
        '/' if start == 0 => Some(('/', 0)),
        _ => None,
    }
}

/// One merged completion row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRow {
    /// Section title (`Files`, `Commands`, …).
    pub section: String,
    /// Replacement text.
    pub value: String,
    /// Label.
    pub label: String,
    /// Description.
    pub description: Option<String>,
    /// Source tag (`skill`, `dir`, extension id).
    pub tag: Option<String>,
    /// Tone of the tag (`c1`…).
    pub tone: Option<String>,
    /// Char offset where the value replaces up to the cursor.
    pub from: usize,
}

fn section_of(source: &str) -> String {
    match source {
        "commands" => "Commands".to_owned(),
        "files" => "Files".to_owned(),
        "skills" => "Skills".to_owned(),
        "prompts" | "templates" => "Prompts".to_owned(),
        "sessions" => "Sessions".to_owned(),
        s if s.starts_with("command:") => format!("/{}", &s["command:".len()..]),
        other => other.to_owned(),
    }
}

/// Merges `_oino/get_completions` groups into rows (sorted by tier within a group, at most
/// `limit`). `default_from` is the token start for groups without `from`.
///
/// One group per source (A2.11, GA-4): an extension group that lists slash commands (the
/// command enchantment) replaces the base `commands` group. Its rows come first under
/// "Commands" and keep their tones; base rows it already lists are dropped.
#[must_use]
pub fn merge_completions(
    groups: &[oino_core::session::CompletionGroup],
    default_from: usize,
    limit: usize,
) -> Vec<CompletionRow> {
    use oino_core::session::CompletionGroup;
    let base: std::collections::HashSet<&str> = groups
        .iter()
        .filter(|g| g.source == "commands")
        .flat_map(|g| g.items.iter().map(|i| i.value.trim_end()))
        .collect();
    // The enchantment sends bare names (`compact`) for the `/name` token and its own
    // `replaces: "commands"` marker is not carried by core: recognise it by its items naming
    // the base commands.
    let slashed = |g: &CompletionGroup, v: &str| -> String {
        if v.starts_with('/') || g.source == "commands" {
            v.to_owned()
        } else {
            format!("/{v}")
        }
    };
    let enchanted = |g: &CompletionGroup| {
        if g.source == "commands" || g.source.starts_with("command:") || g.items.is_empty() {
            return false;
        }
        if g.items.iter().all(|i| i.value.starts_with('/')) {
            return true;
        }
        let known = g
            .items
            .iter()
            .filter(|i| base.contains(format!("/{}", i.value.trim_end()).as_str()))
            .count();
        !base.is_empty() && known * 2 >= g.items.len()
    };
    let mut ordered: Vec<&CompletionGroup> = groups.iter().filter(|g| enchanted(g)).collect();
    ordered.extend(groups.iter().filter(|g| !enchanted(g)));
    let replaced: std::collections::HashSet<String> = ordered
        .iter()
        .filter(|g| enchanted(g))
        .flat_map(|g| g.items.iter().map(|i| slashed(g, i.value.trim_end())))
        .collect();
    let mut out: Vec<CompletionRow> = Vec::new();
    for group in ordered {
        let is_enchanted = enchanted(group);
        let section = if is_enchanted {
            "Commands".to_owned()
        } else {
            section_of(&group.source)
        };
        let mut items: Vec<&oino_core::session::CompletionEntry> = group.items.iter().collect();
        items.sort_by_key(|i| i.sort_tier.unwrap_or(0));
        for item in items {
            let value = if is_enchanted {
                slashed(group, &item.value)
            } else {
                item.value.clone()
            };
            if group.source == "commands" && replaced.contains(value.trim_end()) {
                continue;
            }
            if out.iter().any(|r| r.value.trim_end() == value.trim_end()) {
                continue;
            }
            if out.len() >= limit {
                return out;
            }
            let label = item.label.clone().unwrap_or_else(|| value.clone());
            out.push(CompletionRow {
                section: section.clone(),
                label: if is_enchanted && !label.starts_with('/') {
                    format!("/{label}")
                } else {
                    label
                },
                value,
                description: item.description.clone(),
                tag: item.tag.clone(),
                tone: item.tone.clone(),
                from: if is_enchanted {
                    0
                } else {
                    group.from.unwrap_or(default_from)
                },
            });
        }
    }
    out
}

/// Built-in rows (pi interactive commands, GA-3) matching a composer `/partial`.
#[must_use]
pub fn builtin_completions(text: &str) -> Vec<CompletionRow> {
    let Some(partial) = text.strip_prefix('/') else {
        return Vec::new();
    };
    if partial.contains(char::is_whitespace) {
        return Vec::new();
    }
    let needle = partial.to_lowercase();
    BUILTINS
        .iter()
        .filter(|b| b.name.starts_with(&needle))
        .map(|b| CompletionRow {
            section: "Built-in".to_owned(),
            value: format!("/{}", b.name),
            label: if b.args.is_empty() {
                format!("/{}", b.name)
            } else {
                format!("/{} {}", b.name, b.args)
            },
            description: Some(b.description.to_owned()),
            tag: None,
            tone: None,
            from: 0,
        })
        .collect()
}

/// Applies a completion: replaces chars `from..cursor` of `text` with `value` (+ a space when
/// the value does not end a path segment). Returns the new text and cursor.
#[must_use]
pub fn apply_completion(text: &str, cursor: usize, from: usize, value: &str) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let from = from.min(chars.len());
    let cursor = cursor.clamp(from, chars.len());
    let mut out: String = chars[..from].iter().collect();
    out.push_str(value);
    let keep_open = value.ends_with('/');
    if !keep_open {
        out.push(' ');
    }
    let new_cursor = out.chars().count();
    out.extend(chars[cursor..].iter());
    (out, new_cursor)
}

/// Standard base64 (RFC 4648, padded) for pasted image attachments.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(ABC[(n >> 18) as usize & 63]));
        out.push(char::from(ABC[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ABC[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ABC[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// Greeting by local hour (`ZCode` `chat-empty-time-greeting.md`).
#[must_use]
pub const fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=8 => "Morning, ready when you are",
        9..=11 => "Morning, how can I help?",
        12..=13 => "Noon break?",
        14..=17 => "Good afternoon! Leave the rest to me.",
        18..=22 => "Evening, nice work today",
        _ => "It's late — remember to rest.",
    }
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
    fn mode_chip_and_commands() {
        use crate::model::ous::Tone;
        use crate::model::ui_state::{Align, Segment};
        let seg = |id: &str, text: &str| Segment {
            id: Some(id.into()),
            text: text.into(),
            spans: None,
            command: None,
            tone: Tone::Default,
            icon: None,
            priority: 0,
            align: Align::Right,
        };
        assert_eq!(
            ModeChip::from_segments(&[seg("permission", "ask")]),
            Some(ModeChip::Ask)
        );
        assert_eq!(
            ModeChip::from_segments(&[seg("permission", "full"), seg("plan", "PLAN")]),
            Some(ModeChip::Plan)
        );
        assert_eq!(ModeChip::from_segments(&[seg("branch", "main")]), None);
        assert_eq!(
            ModeChip::Full.commands(Some(ModeChip::Ask)),
            vec!["/permission full"]
        );
        assert_eq!(ModeChip::Plan.commands(None), vec!["/plan on"]);
        assert_eq!(ModeChip::Ask.commands(Some(ModeChip::Plan)).len(), 2);
    }

    #[test]
    fn triggers_and_completions() {
        assert_eq!(trigger("/comp", 5), Some(('/', 0)));
        assert_eq!(trigger("fix @src/pa", 11), Some(('@', 4)));
        assert_eq!(trigger("a $tdd", 6), Some(('$', 2)));
        assert_eq!(trigger("a /x", 4), None, "slash only at the start");
        assert_eq!(trigger("plain", 5), None);
        let groups: Vec<oino_core::session::CompletionGroup> = serde_json::from_value(serde_json::json!([
            {"source":"files","from":4,"items":[{"value":"@src/","tag":"dir","sortTier":1},{"value":"@src/parser.rs","sortTier":0}]},
            {"source":"unipi.command-enchantment","items":[{"value":"#s1","label":"Fix parser","tone":"c3"}]}
        ])).unwrap();
        let rows = merge_completions(&groups, 0, 10);
        assert_eq!(rows[0].value, "@src/parser.rs");
        assert_eq!(rows[1].tag.as_deref(), Some("dir"));
        assert_eq!(rows[2].section, "unipi.command-enchantment");
        assert_eq!(rows[2].from, 0);
        assert_eq!(merge_completions(&groups, 0, 1).len(), 1);
        assert_eq!(
            apply_completion("fix @src/pa now", 11, 4, "@src/parser.rs"),
            ("fix @src/parser.rs  now".to_owned(), 19)
        );
        assert_eq!(apply_completion("@s", 2, 0, "@src/").0, "@src/");
        assert_eq!(greeting(13), "Noon break?");
        assert_eq!(greeting(2), "It's late — remember to rest.");
    }

    #[test]
    fn builtins_are_local_and_enchanted_groups_replace_base() {
        assert_eq!(
            intent("/new", false, SubmitKey::Enter),
            Intent::Builtin(Builtin::New, None)
        );
        assert_eq!(
            intent("/compact keep tests", true, SubmitKey::Enter),
            Intent::Builtin(Builtin::Compact, Some("keep tests".into()))
        );
        assert_eq!(
            intent("/plan on", false, SubmitKey::Enter),
            Intent::Prompt("/plan on".into())
        );
        assert!(
            Intent::Builtin(Builtin::New, None)
                .command(Vec::new())
                .is_none()
        );
        assert!(builtin_completions("/ne").iter().any(|r| r.value == "/new"));
        assert!(builtin_completions("/new x").is_empty());
        let groups: Vec<oino_core::session::CompletionGroup> = serde_json::from_value(serde_json::json!([
            {"source":"commands","from":0,"items":[{"value":"/compact"},{"value":"/plan"},{"value":"/other"}]},
            {"source":"unipi.command-enchantment","items":[{"value":"plan","tag":"workflow","tone":"c3"},{"value":"compact","tag":"compactor","tone":"c5"}]}
        ])).unwrap();
        let rows = merge_completions(&groups, 0, 10);
        let values: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(values, vec!["/plan", "/compact", "/other"]);
        assert!(rows.iter().all(|r| r.section == "Commands"));
        assert_eq!(rows[0].tone.as_deref(), Some("c3"));
        assert_eq!(
            ModeChip::from_topic(&serde_json::json!({"planMode": false, "permissionMode": "ask"})),
            Some(ModeChip::Ask)
        );
        assert_eq!(
            ModeChip::from_topic(&serde_json::json!({"planMode": true, "permissionMode": "full"})),
            Some(ModeChip::Plan)
        );
        assert_eq!(
            ModeChip::from_topic(&serde_json::json!({"planMode": false})),
            None
        );
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn slash_matching() {
        let commands = vec![
            ("compact".to_owned(), Some("Compact".to_owned())),
            ("model".to_owned(), None),
            ("compact-now".to_owned(), None),
        ];
        let labels: Vec<String> = slash_completions(&commands, "/comp", 10)
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["/compact", "/compact-now"]);
        assert!(slash_completions(&commands, "/compact now", 10).is_empty());
        assert!(slash_completions(&commands, "compact", 10).is_empty());
        assert_eq!(slash_completions(&commands, "/", 2).len(), 2);
    }
}
