//! `<agent>/keybindings.json` (pi `keybindings.ts`, `key-id-schema.ts`): load, migrate legacy
//! names, validate key ids. Semantics (defaults, conflicts, dispatch) belong to frontends.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use oino_settings::JsonObject;
use oino_settings::paths::KEYBINDINGS_FILE;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diagnostics::ResourceDiagnostic;

/// A binding value: one key id or a list (pi `KeyId | KeyId[]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyBinding {
    /// Single key id, e.g. `ctrl+p`.
    One(String),
    /// Several key ids (an empty list unbinds).
    Many(Vec<String>),
}

impl KeyBinding {
    /// The key ids.
    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        match self {
            Self::One(key) => vec![key.as_str()],
            Self::Many(keys) => keys.iter().map(String::as_str).collect(),
        }
    }

    /// pi `KeybindingValueSchema` check.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::String(key) if is_valid_key_id(key) => Some(Self::One(key.clone())),
            Value::Array(items) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|key| is_valid_key_id(key))
                        .map(str::to_owned)
                })
                .collect::<Option<Vec<_>>>()
                .map(Self::Many),
            _ => None,
        }
    }
}

/// The loaded keybindings file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeybindingsFile {
    /// File path.
    pub path: PathBuf,
    /// Valid user bindings, migrated and in pi order (known ids first, extras sorted).
    pub bindings: IndexMap<String, KeyBinding>,
    /// Whether legacy names were migrated (the file is not rewritten).
    pub migrated: bool,
    /// Unreadable file or dropped invalid entries (pi drops them silently).
    pub diagnostics: Vec<ResourceDiagnostic>,
}

impl KeybindingsFile {
    /// Load `<agent_dir>/keybindings.json` (pi `KeybindingsManager.loadFromFile`); a missing or
    /// unreadable file yields no bindings.
    #[must_use]
    pub fn load(agent_dir: &Path) -> Self {
        let path = agent_dir.join(KEYBINDINGS_FILE);
        let mut file = Self {
            path: path.clone(),
            ..Self::default()
        };
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return file,
            Err(error) => {
                file.diagnostics
                    .push(ResourceDiagnostic::warning(error.to_string(), &path));
                return file;
            }
        };
        let raw = match serde_json::from_str(oino_settings::json::strip_bom(&content)) {
            Ok(Value::Object(raw)) => raw,
            Ok(_) => {
                file.diagnostics.push(ResourceDiagnostic::warning(
                    "keybindings must be a JSON object",
                    &path,
                ));
                return file;
            }
            Err(error) => {
                file.diagnostics
                    .push(ResourceDiagnostic::warning(error.to_string(), &path));
                return file;
            }
        };
        let (config, migrated) = migrate_keybindings_config(&raw);
        file.migrated = migrated;
        for (id, value) in config {
            if id == "$schema" {
                continue;
            }
            match KeyBinding::from_value(&value) {
                Some(binding) => {
                    file.bindings.insert(id, binding);
                }
                None => file.diagnostics.push(ResourceDiagnostic::warning(
                    format!("invalid key binding for \"{id}\""),
                    &path,
                )),
            }
        }
        file
    }
}

/// pi `migrateKeybindingsConfig`: rename legacy names (an existing new name wins), then order
/// known ids first and extras sorted.
#[must_use]
pub fn migrate_keybindings_config(raw: &JsonObject) -> (JsonObject, bool) {
    let mut config = JsonObject::new();
    let mut migrated = false;
    for (key, value) in raw {
        let next = LEGACY_KEYBINDING_NAMES
            .iter()
            .find(|(legacy, _)| legacy == key)
            .map_or(key.as_str(), |(_, id)| id);
        if next != key {
            migrated = true;
            if raw.contains_key(next) {
                continue;
            }
        }
        config.insert(next.to_owned(), value.clone());
    }
    (order_keybindings_config(config), migrated)
}

fn order_keybindings_config(mut config: JsonObject) -> JsonObject {
    let mut ordered = JsonObject::new();
    for id in KEYBINDING_IDS {
        if let Some(value) = config.shift_remove(id) {
            ordered.insert(id.to_owned(), value);
        }
    }
    let mut extras: Vec<(String, Value)> = config.into_iter().collect();
    extras.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    ordered.extend(extras);
    ordered
}

/// pi `KeyIdSchema`: up to four distinct `ctrl+`/`shift+`/`alt+`/`super+` modifiers, then one
/// base key (`[a-z0-9]` or a named key). Lowercase only.
#[must_use]
pub fn is_valid_key_id(key: &str) -> bool {
    const MODIFIERS: [&str; 4] = ["ctrl+", "shift+", "alt+", "super+"];
    if MODIFIERS.iter().any(|modifier| {
        key.find(modifier)
            .is_some_and(|i| key[i + modifier.len()..].contains(modifier))
    }) {
        return false;
    }
    let mut rest = key;
    for _ in 0..4 {
        match MODIFIERS
            .iter()
            .find_map(|modifier| rest.strip_prefix(modifier))
        {
            Some(stripped) => rest = stripped,
            None => break,
        }
    }
    let single = rest.len() == 1
        && rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    single || NAMED_KEYS.contains(&rest)
}

/// Known keybinding ids in pi order (`KEYBINDINGS`: pi-tui then app), used to order files.
pub const KEYBINDING_IDS: [&str; 90] = [
    "tui.editor.cursorUp",
    "tui.editor.cursorDown",
    "tui.editor.historyPrevious",
    "tui.editor.historyNext",
    "tui.editor.cursorLeft",
    "tui.editor.cursorRight",
    "tui.editor.cursorWordLeft",
    "tui.editor.cursorWordRight",
    "tui.editor.cursorLineStart",
    "tui.editor.cursorLineEnd",
    "tui.editor.jumpForward",
    "tui.editor.jumpBackward",
    "tui.editor.pageUp",
    "tui.editor.pageDown",
    "tui.editor.deleteCharBackward",
    "tui.editor.deleteCharForward",
    "tui.editor.deleteWordBackward",
    "tui.editor.deleteWordForward",
    "tui.editor.deleteToLineStart",
    "tui.editor.deleteToLineEnd",
    "tui.editor.yank",
    "tui.editor.yankPop",
    "tui.editor.undo",
    "tui.input.newLine",
    "tui.input.submit",
    "tui.input.tab",
    "tui.input.copy",
    "tui.select.up",
    "tui.select.down",
    "tui.select.pageUp",
    "tui.select.pageDown",
    "tui.select.confirm",
    "tui.select.cancel",
    "tui.altScreen.pageUp",
    "tui.altScreen.pageDown",
    "tui.altScreen.halfPageUp",
    "tui.altScreen.halfPageDown",
    "tui.altScreen.lineUp",
    "tui.altScreen.lineDown",
    "tui.altScreen.previousPrompt",
    "tui.altScreen.nextPrompt",
    "tui.altScreen.search",
    "tui.altScreen.searchNext",
    "tui.altScreen.searchPrevious",
    "tui.altScreen.searchClose",
    "tui.altScreen.top",
    "tui.altScreen.bottom",
    "app.interrupt",
    "app.clear",
    "app.exit",
    "app.suspend",
    "app.thinking.cycle",
    "app.thinking.save",
    "app.model.cycleForward",
    "app.model.cycleBackward",
    "app.model.select",
    "app.tools.expand",
    "app.thinking.toggle",
    "app.session.toggleNamedFilter",
    "app.editor.external",
    "app.message.copy",
    "app.message.followUp",
    "app.message.dequeue",
    "app.clipboard.pasteImage",
    "app.session.new",
    "app.session.tree",
    "app.session.fork",
    "app.session.resume",
    "app.tree.foldOrUp",
    "app.tree.unfoldOrDown",
    "app.tree.editLabel",
    "app.tree.toggleLabelTimestamp",
    "app.session.togglePath",
    "app.session.toggleSort",
    "app.session.rename",
    "app.session.delete",
    "app.session.deleteNoninvasive",
    "app.models.save",
    "app.models.enableAll",
    "app.models.clearAll",
    "app.models.toggleProvider",
    "app.models.reorderUp",
    "app.models.reorderDown",
    "app.tree.filter.default",
    "app.tree.filter.noTools",
    "app.tree.filter.userOnly",
    "app.tree.filter.labeledOnly",
    "app.tree.filter.all",
    "app.tree.filter.cycleForward",
    "app.tree.filter.cycleBackward",
];

/// Named base keys accepted after modifiers (pi-tui `Key` values; plus single `[a-z0-9]`).
const NAMED_KEYS: [&str; 61] = [
    "escape",
    "esc",
    "enter",
    "return",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "clear",
    "home",
    "end",
    "pageUp",
    "pageDown",
    "up",
    "down",
    "left",
    "right",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "`",
    "-",
    "=",
    "[",
    "]",
    "\\",
    ";",
    "'",
    ",",
    ".",
    "/",
    "!",
    "@",
    "#",
    "$",
    "%",
    "^",
    "&",
    "*",
    "(",
    ")",
    "_",
    "+",
    "|",
    "~",
    "{",
    "}",
    ":",
    "<",
    ">",
    "?",
];

/// Legacy name → id (pi `KEYBINDING_NAME_MIGRATIONS`).
pub const LEGACY_KEYBINDING_NAMES: [(&str, &str); 59] = [
    ("cursorUp", "tui.editor.cursorUp"),
    ("cursorDown", "tui.editor.cursorDown"),
    ("cursorLeft", "tui.editor.cursorLeft"),
    ("cursorRight", "tui.editor.cursorRight"),
    ("cursorWordLeft", "tui.editor.cursorWordLeft"),
    ("cursorWordRight", "tui.editor.cursorWordRight"),
    ("cursorLineStart", "tui.editor.cursorLineStart"),
    ("cursorLineEnd", "tui.editor.cursorLineEnd"),
    ("jumpForward", "tui.editor.jumpForward"),
    ("jumpBackward", "tui.editor.jumpBackward"),
    ("pageUp", "tui.editor.pageUp"),
    ("pageDown", "tui.editor.pageDown"),
    ("deleteCharBackward", "tui.editor.deleteCharBackward"),
    ("deleteCharForward", "tui.editor.deleteCharForward"),
    ("deleteWordBackward", "tui.editor.deleteWordBackward"),
    ("deleteWordForward", "tui.editor.deleteWordForward"),
    ("deleteToLineStart", "tui.editor.deleteToLineStart"),
    ("deleteToLineEnd", "tui.editor.deleteToLineEnd"),
    ("yank", "tui.editor.yank"),
    ("yankPop", "tui.editor.yankPop"),
    ("undo", "tui.editor.undo"),
    ("newLine", "tui.input.newLine"),
    ("submit", "tui.input.submit"),
    ("tab", "tui.input.tab"),
    ("copy", "tui.input.copy"),
    ("selectUp", "tui.select.up"),
    ("selectDown", "tui.select.down"),
    ("selectPageUp", "tui.select.pageUp"),
    ("selectPageDown", "tui.select.pageDown"),
    ("selectConfirm", "tui.select.confirm"),
    ("selectCancel", "tui.select.cancel"),
    ("interrupt", "app.interrupt"),
    ("clear", "app.clear"),
    ("exit", "app.exit"),
    ("suspend", "app.suspend"),
    ("cycleThinkingLevel", "app.thinking.cycle"),
    ("cycleModelForward", "app.model.cycleForward"),
    ("cycleModelBackward", "app.model.cycleBackward"),
    ("selectModel", "app.model.select"),
    ("expandTools", "app.tools.expand"),
    ("toggleThinking", "app.thinking.toggle"),
    ("toggleSessionNamedFilter", "app.session.toggleNamedFilter"),
    ("externalEditor", "app.editor.external"),
    ("followUp", "app.message.followUp"),
    ("dequeue", "app.message.dequeue"),
    ("pasteImage", "app.clipboard.pasteImage"),
    ("newSession", "app.session.new"),
    ("tree", "app.session.tree"),
    ("fork", "app.session.fork"),
    ("resume", "app.session.resume"),
    ("treeFoldOrUp", "app.tree.foldOrUp"),
    ("treeUnfoldOrDown", "app.tree.unfoldOrDown"),
    ("treeEditLabel", "app.tree.editLabel"),
    ("treeToggleLabelTimestamp", "app.tree.toggleLabelTimestamp"),
    ("toggleSessionPath", "app.session.togglePath"),
    ("toggleSessionSort", "app.session.toggleSort"),
    ("renameSession", "app.session.rename"),
    ("deleteSession", "app.session.delete"),
    ("deleteSessionNoninvasive", "app.session.deleteNoninvasive"),
];

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn key_ids_validate_like_pi() {
        for ok in [
            "a",
            "ctrl+p",
            "shift+ctrl+o",
            "ctrl+shift+alt+super+x",
            "alt++",
            "ctrl+-",
            "pageUp",
            "f12",
        ] {
            assert!(is_valid_key_id(ok), "{ok}");
        }
        for bad in [
            "",
            "A",
            "ctrl+ctrl+a",
            "ctrl+",
            "ab",
            "ctrl+shift+alt+super+ctrl+a",
            "Ctrl+a",
            "pageup",
        ] {
            assert!(!is_valid_key_id(bad), "{bad}");
        }
    }

    #[test]
    fn migration_renames_orders_and_prefers_new_names() {
        let raw = json!({"zzz.custom": "a", "cursorUp": "up", "app.interrupt": "escape", "interrupt": "x", "aaa": "b"});
        let (config, migrated) = migrate_keybindings_config(raw.as_object().unwrap());
        assert!(migrated);
        assert_eq!(
            config.keys().map(String::as_str).collect::<Vec<_>>(),
            ["tui.editor.cursorUp", "app.interrupt", "aaa", "zzz.custom"]
        );
        assert_eq!(config["app.interrupt"], json!("escape"));
    }
}
