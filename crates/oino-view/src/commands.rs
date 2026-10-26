//! Built-in interactive slash commands (pi `slash-commands.md`, modes.md §1.3) and the
//! command menu model merged with `get_commands` results.

use crate::fuzzy;

/// A built-in command handled by the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    /// `/settings`.
    Settings,
    /// `/model [provider/model]`.
    Model,
    /// `/thinking [level]`.
    Thinking,
    /// `/scoped-models`.
    ScopedModels,
    /// `/login [provider]`.
    Login,
    /// `/logout`.
    Logout,
    /// `/new`.
    New,
    /// `/resume`.
    Resume,
    /// `/name [name]`.
    Name,
    /// `/session`.
    Session,
    /// `/tree`.
    Tree,
    /// `/fork`.
    Fork,
    /// `/clone`.
    Clone,
    /// `/compact [instructions]`.
    Compact,
    /// `/import <path>`.
    Import,
    /// `/copy`.
    Copy,
    /// `/export [path]`.
    Export,
    /// `/trust`.
    Trust,
    /// `/reload`.
    Reload,
    /// `/hotkeys`.
    Hotkeys,
    /// `/changelog`.
    Changelog,
    /// `/extensions` (Oino).
    Extensions,
    /// `/quit`.
    Quit,
}

/// Name, argument hint and description of a built-in.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinSpec {
    /// Command.
    pub builtin: Builtin,
    /// Name without `/`.
    pub name: &'static str,
    /// Argument hint.
    pub args: &'static str,
    /// Description (pi).
    pub description: &'static str,
}

const fn b(
    builtin: Builtin,
    name: &'static str,
    args: &'static str,
    description: &'static str,
) -> BuiltinSpec {
    BuiltinSpec {
        builtin,
        name,
        args,
        description,
    }
}

/// Built-ins in pi menu order.
pub const BUILTINS: &[BuiltinSpec] = &[
    b(Builtin::Settings, "settings", "", "Open settings"),
    b(
        Builtin::Model,
        "model",
        "[provider/model]",
        "Select a model",
    ),
    b(
        Builtin::Thinking,
        "thinking",
        "[level]",
        "Set the thinking level",
    ),
    b(
        Builtin::ScopedModels,
        "scoped-models",
        "",
        "Configure the models used by interactive cycling",
    ),
    b(
        Builtin::Login,
        "login",
        "[provider]",
        "Add provider authentication",
    ),
    b(
        Builtin::Logout,
        "logout",
        "[provider]",
        "Remove provider authentication",
    ),
    b(Builtin::New, "new", "", "Start a new session"),
    b(
        Builtin::Resume,
        "resume",
        "",
        "Switch to another saved session",
    ),
    b(
        Builtin::Name,
        "name",
        "[name]",
        "Set the session display name, or show the current name when omitted",
    ),
    b(
        Builtin::Session,
        "session",
        "",
        "Show current session information and statistics",
    ),
    b(Builtin::Tree, "tree", "", "Navigate the session tree"),
    b(
        Builtin::Fork,
        "fork",
        "",
        "Create a new session from an earlier user message",
    ),
    b(
        Builtin::Clone,
        "clone",
        "",
        "Duplicate the current session at its current position",
    ),
    b(
        Builtin::Compact,
        "compact",
        "[instructions]",
        "Compact the current context, optionally with custom instructions",
    ),
    b(
        Builtin::Import,
        "import",
        "<path>",
        "Import and resume a JSONL session",
    ),
    b(Builtin::Copy, "copy", "", "Copy the last assistant message"),
    b(
        Builtin::Export,
        "export",
        "[path]",
        "Export the session as HTML or JSONL",
    ),
    b(
        Builtin::Trust,
        "trust",
        "",
        "Save a project trust decision for future Oino processes",
    ),
    b(
        Builtin::Reload,
        "reload",
        "",
        "Reload keybindings, extensions, skills, templates, themes, and context files",
    ),
    b(
        Builtin::Hotkeys,
        "hotkeys",
        "",
        "Show active keyboard shortcuts",
    ),
    b(
        Builtin::Changelog,
        "changelog",
        "",
        "Show changelog entries",
    ),
    b(
        Builtin::Extensions,
        "extensions",
        "",
        "List, enable or disable extensions",
    ),
    b(Builtin::Quit, "quit", "", "Quit Oino"),
];

/// What a submitted line means (pi `setupEditorSubmitHandler` order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submitted {
    /// A built-in with its argument (trimmed, `None` when absent).
    Builtin(Builtin, Option<String>),
    /// `!cmd` / `!!cmd`.
    Bash {
        /// Command line.
        command: String,
        /// `!!`.
        exclude_from_context: bool,
    },
    /// Anything else (prompt, extension command, skill, template).
    Prompt(String),
}

/// Classifies a trimmed submission. Bare `!`/`!!` are prompts (pi only runs non-empty
/// commands).
#[must_use]
pub fn classify(text: &str) -> Submitted {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('/') {
        let (name, arg) = match rest.split_once(char::is_whitespace) {
            Some((name, arg)) => (name, Some(arg.trim())),
            None => (rest, None),
        };
        if let Some(spec) = BUILTINS.iter().find(|s| s.name == name) {
            let takes_args = !spec.args.is_empty();
            if takes_args || arg.is_none_or(str::is_empty) {
                let arg = arg.filter(|a| !a.is_empty()).map(str::to_owned);
                return Submitted::Builtin(spec.builtin, arg);
            }
        }
    }
    if let Some(rest) = text.strip_prefix('!') {
        let (exclude, command) = match rest.strip_prefix('!') {
            Some(command) => (true, command.trim()),
            None => (false, rest.trim()),
        };
        if !command.is_empty() {
            return Submitted::Bash {
                command: command.to_owned(),
                exclude_from_context: exclude,
            };
        }
    }
    Submitted::Prompt(text.to_owned())
}

/// Source of a menu item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandSource {
    /// Frontend built-in.
    Builtin,
    /// Extension command.
    Extension,
    /// Prompt template.
    Prompt,
    /// Skill.
    Skill,
}

/// One entry of the `/` menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandItem {
    /// Name without `/`.
    pub name: String,
    /// Description.
    pub description: String,
    /// Source.
    pub source: CommandSource,
}

/// Built-ins plus the session's commands (`get_commands`), deduplicated by name (built-ins
/// win, pi).
#[must_use]
pub fn menu_items(session_commands: &[(String, Option<String>, String)]) -> Vec<CommandItem> {
    let mut items: Vec<CommandItem> = BUILTINS
        .iter()
        .map(|s| CommandItem {
            name: s.name.to_owned(),
            description: s.description.to_owned(),
            source: CommandSource::Builtin,
        })
        .collect();
    for (name, description, source) in session_commands {
        if items.iter().any(|i| &i.name == name) {
            continue;
        }
        items.push(CommandItem {
            name: name.clone(),
            description: description.clone().unwrap_or_default(),
            source: match source.as_str() {
                "prompt" => CommandSource::Prompt,
                "skill" => CommandSource::Skill,
                _ => CommandSource::Extension,
            },
        });
    }
    items
}

/// Menu entries matching `query` (the text after `/`), best first.
#[must_use]
pub fn filter_menu<'a>(items: &'a [CommandItem], query: &str) -> Vec<&'a CommandItem> {
    if query.is_empty() {
        return items.iter().collect();
    }
    let prefix: Vec<&CommandItem> = items.iter().filter(|i| i.name.starts_with(query)).collect();
    let mut rest: Vec<&CommandItem> = fuzzy::filter(items, query, |i| i.name.clone())
        .into_iter()
        .map(|i| &items[i])
        .filter(|i| !i.name.starts_with(query))
        .collect();
    let mut out = prefix;
    out.append(&mut rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_like_pi() {
        assert_eq!(classify("/model"), Submitted::Builtin(Builtin::Model, None));
        assert_eq!(
            classify("/model  gpt "),
            Submitted::Builtin(Builtin::Model, Some("gpt".into()))
        );
        assert_eq!(classify("/tree x"), Submitted::Prompt("/tree x".into()));
        assert_eq!(
            classify("!! ls -la"),
            Submitted::Bash {
                command: "ls -la".into(),
                exclude_from_context: true
            }
        );
        assert_eq!(classify("!"), Submitted::Prompt("!".into()));
        assert_eq!(
            classify("/skill:git do it"),
            Submitted::Prompt("/skill:git do it".into())
        );
    }

    #[test]
    fn menu_merges_and_filters() {
        let items = menu_items(&[
            ("review".into(), Some("Review code".into()), "prompt".into()),
            ("model".into(), None, "extension".into()),
        ]);
        assert_eq!(items.iter().filter(|i| i.name == "model").count(), 1);
        let found = filter_menu(&items, "re");
        assert_eq!(found[0].name, "resume");
        assert!(found.iter().any(|i| i.name == "review"));
    }
}
