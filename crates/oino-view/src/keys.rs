//! Key ids and keybindings (pi `keybindings.md`, `KeybindingsManager`).
//!
//! A [`KeyId`] is pi's `modifier+key` syntax (`ctrl+shift+p`, `alt+enter`, `f5`, `ctrl+]`).
//! [`Keybindings`] holds every pi action id with its default keys, overridden per action by
//! the user's `keybindings.json` (a configured value replaces the default; an empty list
//! unbinds).

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;

/// Modifier set of a key press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four independent modifier flags mirror the terminal key report"
)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt / Option / Meta.
    pub alt: bool,
    /// Super / Command (Kitty keyboard protocol only).
    pub super_key: bool,
}

impl Modifiers {
    /// No modifier.
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
        super_key: false,
    };
}

/// The key part of a [`KeyId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character (letters are stored lowercase).
    Char(char),
    /// Escape.
    Escape,
    /// Enter / Return.
    Enter,
    /// Tab.
    Tab,
    /// Space bar.
    Space,
    /// Backspace.
    Backspace,
    /// Delete (forward).
    Delete,
    /// Insert.
    Insert,
    /// Clear.
    Clear,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Function key `f1`..`f12`.
    F(u8),
}

impl Key {
    fn parse(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        Some(match lower.as_str() {
            "escape" | "esc" => Self::Escape,
            "enter" | "return" => Self::Enter,
            "tab" => Self::Tab,
            "space" => Self::Space,
            "backspace" => Self::Backspace,
            "delete" => Self::Delete,
            "insert" => Self::Insert,
            "clear" => Self::Clear,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" => Self::PageUp,
            "pagedown" => Self::PageDown,
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => {
                if let Some(number) = lower.strip_prefix('f')
                    && let Ok(n) = number.parse::<u8>()
                    && (1..=12).contains(&n)
                {
                    return Some(Self::F(n));
                }
                let mut chars = name.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                if c.is_ascii_alphanumeric() || "`-=[]\\;',./!@#$%^&*()_+|~{}:<>?".contains(c) {
                    Self::Char(c.to_ascii_lowercase())
                } else {
                    return None;
                }
            }
        })
    }

    fn name(self) -> String {
        match self {
            Self::Char(c) => c.to_string(),
            Self::Escape => "escape".into(),
            Self::Enter => "enter".into(),
            Self::Tab => "tab".into(),
            Self::Space => "space".into(),
            Self::Backspace => "backspace".into(),
            Self::Delete => "delete".into(),
            Self::Insert => "insert".into(),
            Self::Clear => "clear".into(),
            Self::Home => "home".into(),
            Self::End => "end".into(),
            Self::PageUp => "pageUp".into(),
            Self::PageDown => "pageDown".into(),
            Self::Up => "up".into(),
            Self::Down => "down".into(),
            Self::Left => "left".into(),
            Self::Right => "right".into(),
            Self::F(n) => format!("f{n}"),
        }
    }
}

/// A key press in pi `KeyId` form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyId {
    /// Key.
    pub key: Key,
    /// Modifiers.
    pub mods: Modifiers,
}

impl KeyId {
    /// A key without modifiers.
    #[must_use]
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            mods: Modifiers::NONE,
        }
    }

    /// A key with modifiers.
    #[must_use]
    pub const fn new(key: Key, mods: Modifiers) -> Self {
        Self { key, mods }
    }

    /// A control chord (`ctrl+<c>`).
    #[must_use]
    pub const fn ctrl(c: char) -> Self {
        Self {
            key: Key::Char(c),
            mods: Modifiers {
                ctrl: true,
                shift: false,
                alt: false,
                super_key: false,
            },
        }
    }

    /// The printable character this press inserts, if any (no ctrl/alt/super).
    #[must_use]
    pub fn printable(&self) -> Option<char> {
        if self.mods.ctrl || self.mods.alt || self.mods.super_key {
            return None;
        }
        match self.key {
            Key::Char(c) if self.mods.shift && c.is_ascii_lowercase() => {
                Some(c.to_ascii_uppercase())
            }
            Key::Char(c) => Some(c),
            Key::Space => Some(' '),
            _ => None,
        }
    }
}

/// Invalid key id text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidKeyId(pub String);

impl fmt::Display for InvalidKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid key id \"{}\"", self.0)
    }
}

impl std::error::Error for InvalidKeyId {}

impl FromStr for KeyId {
    type Err = InvalidKeyId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidKeyId(text.to_owned());
        let (prefix, key_name) = if let Some(prefix) = text.strip_suffix("++") {
            (prefix, "+")
        } else if text == "+" {
            ("", "+")
        } else {
            match text.rsplit_once('+') {
                Some((prefix, key)) => (prefix, key),
                None => ("", text),
            }
        };
        let mut mods = Modifiers::NONE;
        for part in prefix.split('+').filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" => mods.alt = true,
                "super" => mods.super_key = true,
                _ => return Err(invalid()),
            }
        }
        let key = Key::parse(key_name).ok_or_else(invalid)?;
        Ok(Self { key, mods })
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("ctrl+")?;
        }
        if self.mods.shift {
            f.write_str("shift+")?;
        }
        if self.mods.alt {
            f.write_str("alt+")?;
        }
        if self.mods.super_key {
            f.write_str("super+")?;
        }
        f.write_str(&self.key.name())
    }
}

/// One pi action with its default keys and description.
#[derive(Clone, Copy, Debug)]
pub struct ActionSpec {
    /// Action id (`app.interrupt`).
    pub id: &'static str,
    /// Default key ids.
    pub defaults: &'static [&'static str],
    /// Description (pi `keybindings.md`).
    pub description: &'static str,
}

const fn spec(
    id: &'static str,
    defaults: &'static [&'static str],
    description: &'static str,
) -> ActionSpec {
    ActionSpec {
        id,
        defaults,
        description,
    }
}

#[cfg(windows)]
const UNDO: &[&str] = &["ctrl+z"];
#[cfg(not(windows))]
const UNDO: &[&str] = &["ctrl+-"];
#[cfg(windows)]
const SUSPEND: &[&str] = &[];
#[cfg(not(windows))]
const SUSPEND: &[&str] = &["ctrl+z"];
#[cfg(windows)]
const PASTE_IMAGE: &[&str] = &["alt+v"];
#[cfg(not(windows))]
const PASTE_IMAGE: &[&str] = &["ctrl+v"];
#[cfg(windows)]
const MODEL_BACK: &[&str] = &["alt+p"];
#[cfg(not(windows))]
const MODEL_BACK: &[&str] = &["shift+ctrl+p"];
#[cfg(windows)]
const FOLLOW_UP: &[&str] = &["ctrl+q"];
#[cfg(not(windows))]
const FOLLOW_UP: &[&str] = &["alt+enter"];
#[cfg(windows)]
const DEQUEUE: &[&str] = &["alt+q"];
#[cfg(not(windows))]
const DEQUEUE: &[&str] = &["alt+up"];
#[cfg(windows)]
const PREV_PROMPT: &[&str] = &["ctrl+shift+up", "ctrl+up"];
#[cfg(not(windows))]
const PREV_PROMPT: &[&str] = &["ctrl+shift+up"];
#[cfg(windows)]
const NEXT_PROMPT: &[&str] = &["ctrl+shift+down", "ctrl+down"];
#[cfg(not(windows))]
const NEXT_PROMPT: &[&str] = &["ctrl+shift+down"];

/// Every pi action (pi `keybindings.md` order).
pub const ACTIONS: &[ActionSpec] = &[
    spec(
        "tui.editor.cursorUp",
        &["up"],
        "Move cursor up, browsing older history at the top",
    ),
    spec(
        "tui.editor.cursorDown",
        &["down"],
        "Move cursor down, browsing newer history at the bottom",
    ),
    spec(
        "tui.editor.historyPrevious",
        &[],
        "Select the previous prompt history entry",
    ),
    spec(
        "tui.editor.historyNext",
        &[],
        "Select the next prompt history entry",
    ),
    spec(
        "tui.editor.cursorLeft",
        &["left", "ctrl+b"],
        "Move cursor left",
    ),
    spec(
        "tui.editor.cursorRight",
        &["right", "ctrl+f"],
        "Move cursor right",
    ),
    spec(
        "tui.editor.cursorWordLeft",
        &["alt+left", "ctrl+left", "alt+b"],
        "Move cursor word left",
    ),
    spec(
        "tui.editor.cursorWordRight",
        &["alt+right", "ctrl+right", "alt+f"],
        "Move cursor word right",
    ),
    spec(
        "tui.editor.cursorLineStart",
        &["home", "ctrl+a"],
        "Move to line start",
    ),
    spec(
        "tui.editor.cursorLineEnd",
        &["end", "ctrl+e"],
        "Move to line end",
    ),
    spec(
        "tui.editor.jumpForward",
        &["ctrl+]"],
        "Jump forward to character",
    ),
    spec(
        "tui.editor.jumpBackward",
        &["ctrl+alt+]"],
        "Jump backward to character",
    ),
    spec(
        "tui.editor.pageUp",
        &["pageUp", "ctrl+pageUp"],
        "Scroll up by page",
    ),
    spec(
        "tui.editor.pageDown",
        &["pageDown", "ctrl+pageDown"],
        "Scroll down by page",
    ),
    spec(
        "tui.editor.deleteCharBackward",
        &["backspace"],
        "Delete character backward",
    ),
    spec(
        "tui.editor.deleteCharForward",
        &["delete", "ctrl+d"],
        "Delete character forward",
    ),
    spec(
        "tui.editor.deleteWordBackward",
        &["ctrl+w", "alt+backspace"],
        "Delete word backward",
    ),
    spec(
        "tui.editor.deleteWordForward",
        &["alt+d", "alt+delete"],
        "Delete word forward",
    ),
    spec(
        "tui.editor.deleteToLineStart",
        &["ctrl+u"],
        "Delete to line start",
    ),
    spec(
        "tui.editor.deleteToLineEnd",
        &["ctrl+k"],
        "Delete to line end",
    ),
    spec(
        "tui.editor.yank",
        &["ctrl+y"],
        "Paste most recently deleted text",
    ),
    spec(
        "tui.editor.yankPop",
        &["alt+y"],
        "Cycle through deleted text after yank",
    ),
    spec("tui.editor.undo", UNDO, "Undo last edit"),
    spec(
        "tui.input.newLine",
        &["shift+enter", "ctrl+j"],
        "Insert new line",
    ),
    spec("tui.input.submit", &["enter"], "Submit input"),
    spec("tui.input.tab", &["tab"], "Tab or autocomplete"),
    spec("tui.input.copy", &["ctrl+c"], "Copy selection"),
    spec("tui.select.up", &["up"], "Move selection up"),
    spec("tui.select.down", &["down"], "Move selection down"),
    spec("tui.select.pageUp", &["pageUp"], "Page up in list"),
    spec("tui.select.pageDown", &["pageDown"], "Page down in list"),
    spec("tui.select.confirm", &["enter"], "Confirm selection"),
    spec(
        "tui.select.cancel",
        &["escape", "ctrl+c"],
        "Cancel selection",
    ),
    spec(
        "tui.altScreen.pageUp",
        &["pageUp"],
        "Scroll the transcript up by one page",
    ),
    spec(
        "tui.altScreen.pageDown",
        &["pageDown"],
        "Scroll the transcript down by one page",
    ),
    spec(
        "tui.altScreen.halfPageUp",
        &[],
        "Scroll the transcript up by half a page",
    ),
    spec(
        "tui.altScreen.halfPageDown",
        &[],
        "Scroll the transcript down by half a page",
    ),
    spec(
        "tui.altScreen.lineUp",
        &[],
        "Scroll the transcript up by one line",
    ),
    spec(
        "tui.altScreen.lineDown",
        &[],
        "Scroll the transcript down by one line",
    ),
    spec(
        "tui.altScreen.previousPrompt",
        PREV_PROMPT,
        "Jump to the previous marked message",
    ),
    spec(
        "tui.altScreen.nextPrompt",
        NEXT_PROMPT,
        "Jump to the next marked message",
    ),
    spec(
        "tui.altScreen.top",
        &["ctrl+home"],
        "Scroll to the beginning of the transcript",
    ),
    spec(
        "tui.altScreen.bottom",
        &["ctrl+end"],
        "Scroll to the transcript end and follow new output",
    ),
    spec("app.interrupt", &["escape"], "Cancel / abort"),
    spec(
        "app.clear",
        &["ctrl+c"],
        "Clear editor (first) / exit (second)",
    ),
    spec("app.exit", &["ctrl+d"], "Exit (when editor empty)"),
    spec("app.suspend", SUSPEND, "Suspend to background"),
    spec(
        "app.editor.external",
        &["ctrl+g"],
        "Open in external editor",
    ),
    spec(
        "app.clipboard.pasteImage",
        PASTE_IMAGE,
        "Paste files on macOS, images, or text from clipboard",
    ),
    spec("app.session.new", &[], "Start a new session (/new)"),
    spec(
        "app.session.tree",
        &[],
        "Open session tree navigator (/tree)",
    ),
    spec("app.session.fork", &[], "Fork current session (/fork)"),
    spec(
        "app.session.resume",
        &[],
        "Open session resume picker (/resume)",
    ),
    spec("app.session.togglePath", &["ctrl+p"], "Toggle path display"),
    spec("app.session.toggleSort", &["ctrl+s"], "Toggle sort mode"),
    spec(
        "app.session.toggleNamedFilter",
        &["ctrl+n"],
        "Toggle named-only filter",
    ),
    spec("app.session.rename", &["ctrl+r"], "Rename session"),
    spec("app.session.delete", &["ctrl+d"], "Delete session"),
    spec(
        "app.session.deleteNoninvasive",
        &["ctrl+backspace"],
        "Delete session when query is empty",
    ),
    spec("app.model.select", &["ctrl+l"], "Open model selector"),
    spec("app.model.cycleForward", &["ctrl+p"], "Cycle to next model"),
    spec(
        "app.model.cycleBackward",
        MODEL_BACK,
        "Cycle to previous model",
    ),
    spec(
        "app.models.save",
        &["ctrl+s"],
        "Save the selected default model or scoped model configuration to settings",
    ),
    spec("app.thinking.cycle", &["shift+tab"], "Cycle thinking level"),
    spec(
        "app.thinking.save",
        &["ctrl+s"],
        "Save current thinking level to settings",
    ),
    spec(
        "app.thinking.toggle",
        &["ctrl+t"],
        "Collapse or expand thinking blocks",
    ),
    spec(
        "app.tools.expand",
        &["ctrl+o"],
        "Collapse or expand tool output",
    ),
    spec(
        "app.message.copy",
        &["ctrl+x"],
        "Copy the selected message in /tree, otherwise the last assistant message",
    ),
    spec("app.message.followUp", FOLLOW_UP, "Queue follow-up message"),
    spec(
        "app.message.dequeue",
        DEQUEUE,
        "Restore queued messages to editor",
    ),
    spec(
        "app.tree.foldOrUp",
        &["ctrl+left", "alt+left"],
        "Fold current branch segment, or jump to the previous segment start",
    ),
    spec(
        "app.tree.unfoldOrDown",
        &["ctrl+right", "alt+right"],
        "Unfold current branch segment, or jump to the next segment start or branch end",
    ),
    spec(
        "app.tree.editLabel",
        &["shift+l"],
        "Edit the label on the selected tree node",
    ),
    spec(
        "app.tree.toggleLabelTimestamp",
        &["shift+t"],
        "Toggle label timestamps in the tree",
    ),
    spec(
        "app.tree.filter.default",
        &["ctrl+d"],
        "Set tree filter to default view",
    ),
    spec(
        "app.tree.filter.noTools",
        &["ctrl+t"],
        "Toggle tree filter that hides tool results",
    ),
    spec(
        "app.tree.filter.userOnly",
        &["ctrl+u"],
        "Toggle tree filter that shows only user messages",
    ),
    spec(
        "app.tree.filter.labeledOnly",
        &["ctrl+l"],
        "Toggle tree filter that shows only labeled entries",
    ),
    spec(
        "app.tree.filter.all",
        &["ctrl+a"],
        "Toggle tree filter that shows all entries",
    ),
    spec(
        "app.tree.filter.cycleForward",
        &["ctrl+o"],
        "Cycle tree filter forward",
    ),
    spec(
        "app.tree.filter.cycleBackward",
        &["shift+ctrl+o"],
        "Cycle tree filter backward",
    ),
    spec(
        "app.models.enableAll",
        &["ctrl+a"],
        "Enable all models (or all matching the current search)",
    ),
    spec(
        "app.models.clearAll",
        &["ctrl+x"],
        "Clear all models (or all matching the current search)",
    ),
    spec(
        "app.models.toggleProvider",
        &["ctrl+p"],
        "Toggle all models for the current provider",
    ),
    spec(
        "app.models.reorderUp",
        &["alt+up"],
        "Move the selected model up in the cycle order",
    ),
    spec(
        "app.models.reorderDown",
        &["alt+down"],
        "Move the selected model down in the cycle order",
    ),
];

/// Resolved bindings: action id → key ids (with the text the user wrote, for display).
#[derive(Clone, Debug)]
pub struct Keybindings {
    bindings: IndexMap<String, Vec<(KeyId, String)>>,
    /// Extension shortcut overrides (`extensionShortcuts` in `keybindings.json`).
    extension_overrides: IndexMap<String, Vec<String>>,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self::defaults()
    }
}

fn parse_keys(keys: &[impl AsRef<str>]) -> Vec<(KeyId, String)> {
    keys.iter()
        .filter_map(|k| {
            let text = k.as_ref();
            text.parse::<KeyId>().ok().map(|id| (id, text.to_owned()))
        })
        .collect()
}

impl Keybindings {
    /// pi defaults.
    #[must_use]
    pub fn defaults() -> Self {
        let bindings = ACTIONS
            .iter()
            .map(|a| (a.id.to_owned(), parse_keys(a.defaults)))
            .collect();
        Self {
            bindings,
            extension_overrides: IndexMap::new(),
        }
    }

    /// Defaults with user overrides applied; returns warnings for invalid key ids.
    #[must_use]
    pub fn with_overrides<I, K>(overrides: I) -> (Self, Vec<String>)
    where
        I: IntoIterator<Item = (String, Vec<K>)>,
        K: AsRef<str>,
    {
        let mut bindings = Self::defaults();
        let mut warnings = Vec::new();
        for (id, keys) in overrides {
            let mut parsed = Vec::new();
            for key in &keys {
                match key.as_ref().parse::<KeyId>() {
                    Ok(k) => parsed.push((k, key.as_ref().to_owned())),
                    Err(error) => warnings.push(format!("{id}: {error}")),
                }
            }
            bindings.bindings.insert(id, parsed);
        }
        (bindings, warnings)
    }

    /// Sets extension shortcut overrides (`extensionShortcuts`).
    pub fn set_extension_overrides(&mut self, overrides: IndexMap<String, Vec<String>>) {
        self.extension_overrides = overrides;
    }

    /// Effective keys of an extension shortcut: user override, else its declared keys.
    #[must_use]
    pub fn extension_keys(&self, shortcut_id: &str, declared: &[String]) -> Vec<KeyId> {
        self.extension_overrides
            .get(shortcut_id)
            .map_or(declared, Vec::as_slice)
            .iter()
            .filter_map(|k| k.parse().ok())
            .collect()
    }

    /// Whether `key` triggers `action`.
    #[must_use]
    pub fn matches(&self, action: &str, key: &KeyId) -> bool {
        self.bindings
            .get(action)
            .is_some_and(|keys| keys.iter().any(|(k, _)| k == key))
    }

    /// Keys bound to `action`.
    #[must_use]
    pub fn keys(&self, action: &str) -> Vec<KeyId> {
        self.bindings
            .get(action)
            .map(|keys| keys.iter().map(|(k, _)| *k).collect())
            .unwrap_or_default()
    }

    /// pi `keyText`: the configured key texts joined with `/` (empty when unbound).
    #[must_use]
    pub fn key_text(&self, action: &str) -> String {
        self.bindings
            .get(action)
            .map(|keys| {
                keys.iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default()
    }

    /// Whether `key` is bound to any `app.*`/`tui.*` action (frontend-reserved keys win over
    /// extension shortcuts, ui-protocol §8).
    #[must_use]
    pub fn is_reserved(&self, key: &KeyId) -> bool {
        self.bindings
            .iter()
            .filter(|(id, _)| {
                !id.starts_with("app.tree.")
                    && !id.starts_with("app.models.")
                    && !id.starts_with("app.session.toggle")
            })
            .any(|(_, keys)| keys.iter().any(|(k, _)| k == key))
    }

    /// Every action with its current key text and description, in pi order.
    #[must_use]
    pub fn rows(&self) -> Vec<(&'static str, String, &'static str)> {
        ACTIONS
            .iter()
            .map(|a| (a.id, self.key_text(a.id), a.description))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_pi_key_ids() {
        let k: KeyId = "ctrl+shift+p".parse().unwrap();
        assert!(k.mods.ctrl && k.mods.shift && !k.mods.alt);
        assert_eq!(k.key, Key::Char('p'));
        assert_eq!(k.to_string(), "ctrl+shift+p");
        assert_eq!("shift+ctrl+p".parse::<KeyId>().unwrap(), k);
        assert_eq!("pageUp".parse::<KeyId>().unwrap().key, Key::PageUp);
        assert_eq!("ctrl+]".parse::<KeyId>().unwrap().key, Key::Char(']'));
        assert_eq!("ctrl++".parse::<KeyId>().unwrap().key, Key::Char('+'));
        assert_eq!("f5".parse::<KeyId>().unwrap().key, Key::F(5));
        assert_eq!("esc".parse::<KeyId>().unwrap().key, Key::Escape);
        assert!("hyper+x".parse::<KeyId>().is_err());
        assert!("ctrl+foo".parse::<KeyId>().is_err());
    }

    #[test]
    fn overrides_replace_defaults_and_empty_unbinds() {
        let (kb, warnings) = Keybindings::with_overrides([
            ("app.session.new".to_owned(), vec!["ctrl+shift+n"]),
            ("app.tools.expand".to_owned(), vec![]),
            ("app.exit".to_owned(), vec!["bogus+x"]),
        ]);
        assert_eq!(warnings.len(), 1);
        assert!(kb.matches("app.session.new", &"ctrl+shift+n".parse().unwrap()));
        assert!(!kb.matches("app.tools.expand", &KeyId::ctrl('o')));
        assert!(kb.matches("app.interrupt", &KeyId::plain(Key::Escape)));
        assert_eq!(kb.key_text("app.tools.expand"), "");
        assert_eq!(kb.key_text("tui.input.newLine"), "shift+enter/ctrl+j");
    }

    #[test]
    fn printable_characters() {
        assert_eq!(KeyId::plain(Key::Char('a')).printable(), Some('a'));
        assert_eq!(
            KeyId::new(
                Key::Char('a'),
                Modifiers {
                    shift: true,
                    ..Modifiers::NONE
                }
            )
            .printable(),
            Some('A')
        );
        assert_eq!(KeyId::ctrl('a').printable(), None);
        assert_eq!(KeyId::plain(Key::Space).printable(), Some(' '));
    }
}
