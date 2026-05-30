#![forbid(unsafe_code)]

use crate::keymap::{KeyAction, KeymapConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelpEntry {
    Heading(String),
    Item(String, String),
    Text(String),
    Blank,
}

impl HelpEntry {
    fn heading(text: impl Into<String>) -> Self {
        Self::Heading(text.into())
    }

    fn item(key: impl Into<String>, description: impl Into<String>) -> Self {
        Self::Item(key.into(), description.into())
    }

    fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }
}

#[must_use]
pub(crate) fn help_entries(keymap: &KeymapConfig) -> Vec<HelpEntry> {
    vec![
        HelpEntry::text(format!(
            "Type /help any time to reopen this guide. {} or q closes it.",
            keymap.primary_label(KeyAction::HelpClose)
        )),
        HelpEntry::Blank,
        HelpEntry::heading("Composer"),
        HelpEntry::item(
            keymap.label_for(KeyAction::ComposerSubmit),
            "send a prompt; while the assistant is streaming, send steering text",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::ComposerNewline),
            "insert a newline",
        ),
        HelpEntry::item(
            "/",
            "open fuzzy command suggestions at the start of the input",
        ),
        HelpEntry::item(
            "@",
            "fuzzy search project file paths; Tab inserts the highlighted path",
        ),
        HelpEntry::item(
            "/prompt:<name> / /skill:<name>",
            "include prompt templates or skills in the submitted message",
        ),
        HelpEntry::item(
            "/P:<query> / /S:<query>",
            "search prompt templates or skills anywhere in the composer",
        ),
        HelpEntry::item(
            "Paste",
            "large or multiline pastes collapse visually but still submit in full",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::ComposerExpandReference),
            "expand a collapsed pasted block at the cursor or prompt template references",
        ),
        HelpEntry::Blank,
        HelpEntry::heading("Commands"),
        HelpEntry::item("/help", "open this help overlay"),
        HelpEntry::item("/new", "start a fresh session after this one has messages"),
        HelpEntry::item(
            "/sessions",
            "browse saved sessions; press Enter to continue one",
        ),
        HelpEntry::item("/settings", "open settings pages"),
        HelpEntry::item("/theme", "open theme selection"),
        HelpEntry::item(
            "/router setup / /router status / /router models",
            "extension command from enabled builtin:router; provider auth/router setup",
        ),
        HelpEntry::item(
            "/auth / /account / /auth quickstart",
            "extension auth/runtime readiness plus OmniRoute-first setup guidance",
        ),
        HelpEntry::item(
            "/usage",
            "open the floating usage panel with session totals and provider readiness",
        ),
        HelpEntry::item(
            "/extensions",
            "install optional built-ins; manage extensions and contribution toggles",
        ),
        HelpEntry::item(
            "/prompts",
            "browse prompt templates from <project>/.oino/prompts/",
        ),
        HelpEntry::item(
            "/skills",
            "browse skills from ~/.oino/skills/ and <project>/.oino/skills/",
        ),
        HelpEntry::item("/reload", "reload SYSTEM.md, AGENT.md, prompts, and skills"),
        HelpEntry::item(
            "/inspect",
            "inspect full prompt; press e there to export chat HTML",
        ),
        HelpEntry::item(
            "/ralph start|continue|once|steer",
            "run optional Ralph loops with auto-continuation, steering, and promise tags",
        ),
        HelpEntry::item(
            "/compact [vcc|llm]",
            "compact session; override method; configure threshold, auto, model, prompt",
        ),
        HelpEntry::item(
            "/mode plan / /mode work / /mode <profile>",
            "switch optional sandbox profiles; project .oino overrides global ~/.oino files",
        ),
        HelpEntry::item(
            "/skill:<name>",
            "include a skill explicitly; repeat tokens to combine resources",
        ),
        HelpEntry::item(
            "/model <provider:model>",
            "change main chat model directly, or /model to open model selection",
        ),
        HelpEntry::item(
            "/model btw|notify-summary <model>",
            "configure model-backed features using the shared searchable model catalog",
        ),
        HelpEntry::item(
            "/btw / /btw new",
            "open a fresh side plan chat panel; type /new inside BTW to reset it",
        ),
        HelpEntry::item(
            "/thinking <level>",
            "set reasoning level: off, minimal, low, medium, high, xhigh",
        ),
        HelpEntry::item(
            "/title <text>",
            "set the title shown in the transcript and sessions list",
        ),
        HelpEntry::item(
            "/settings tools",
            "show registered agent tools by global/project scope",
        ),
        HelpEntry::item(
            "/settings auth",
            "open provider auth/account status inside settings",
        ),
        HelpEntry::item(
            "/settings extensions",
            "open the extension manager from settings",
        ),
        HelpEntry::item(
            "/settings notify",
            "configure builtin:notify ntfy server, topic, token, priority, tags, and events",
        ),
        HelpEntry::Blank,
        HelpEntry::heading("Optional built-ins"),
        HelpEntry::item(
            "builtin:footer-status",
            "install from /extensions to show composer-adjacent model/context/cwd lines",
        ),
        HelpEntry::item(
            "builtin:ralph-loop",
            "adds /ralph controller, task files, promise parsing, and loop docs",
        ),
        HelpEntry::item(
            "builtin:mode-sandbox",
            "adds /mode <profile>, tool allow-lists, prompts, and /skill:mode-sandbox guidance",
        ),
        HelpEntry::item(
            "builtin:notify",
            "adds ntfy notifications configured through /settings notify",
        ),
        HelpEntry::item(
            "builtin:craft-skill",
            "adds /skill:craft-skill for authoring Oino skills",
        ),
        HelpEntry::item(
            "builtin:vcc",
            "adds /compact, /recall, and model-visible vcc_recall history search",
        ),
        HelpEntry::item(
            "builtin:ask-user",
            "adds model-visible ask_user questions through an Oino TUI modal",
        ),
        HelpEntry::Blank,
        HelpEntry::heading("Transcript"),
        HelpEntry::item(
            keymap.label_for(KeyAction::TranscriptPageUp),
            "scroll by page up",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::TranscriptPageDown),
            "scroll by page down",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::TranscriptLineUp),
            "scroll by line up",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::TranscriptLineDown),
            "scroll by line down",
        ),
        HelpEntry::item(keymap.label_for(KeyAction::TranscriptTop), "jump to top"),
        HelpEntry::item(
            keymap.label_for(KeyAction::TranscriptBottom),
            "jump to bottom",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::TranscriptFocus),
            "focus transcript; navigation shortcuts then target transcript first",
        ),
        HelpEntry::item(
            "Ctrl-click links/images",
            "open visible URL or image placeholders when the terminal supports it",
        ),
        HelpEntry::Blank,
        HelpEntry::heading("Streaming, queue, and drafts"),
        HelpEntry::item(
            keymap.label_for(KeyAction::ComposerSubmit),
            "while streaming, steer the current response with the current input",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::ComposerQueuePrompt),
            "queue current input for the next turn without opening the send panel",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::ComposerDraftPrompt),
            "move current input to Draft without opening the send panel",
        ),
        HelpEntry::item(keymap.label_for(KeyAction::SettingsOpen), "open settings"),
        HelpEntry::item(
            keymap.label_for(KeyAction::SendPanelOpen),
            "open the send panel for steering history, queue, and drafts",
        ),
        HelpEntry::item(
            format!("Send panel {}", keymap.label_for(KeyAction::SendPanelQueue)),
            "queue current input for the next turn",
        ),
        HelpEntry::item(
            format!("Send panel {}", keymap.label_for(KeyAction::SendPanelDraft)),
            "move current input to Draft",
        ),
        HelpEntry::item(
            format!(
                "Send panel {}",
                keymap.label_for(KeyAction::SendPanelDelete)
            ),
            "delete selected queued/draft item after confirmation",
        ),
        HelpEntry::Blank,
        HelpEntry::heading("Overlays and exit"),
        HelpEntry::item(
            keymap.label_for(KeyAction::HelpClose),
            "close the top overlay, clear search, or stop a running response; it does not quit",
        ),
        HelpEntry::item(
            keymap.label_for(KeyAction::AppQuit),
            "press twice to quit Oino; Ctrl-C twice remains a hard safety fallback",
        ),
    ]
}

#[must_use]
pub(crate) fn help_entry_match_text(entry: &HelpEntry) -> String {
    match entry {
        HelpEntry::Heading(text) | HelpEntry::Text(text) => text.clone(),
        HelpEntry::Item(key, description) => format!("{key} {description}"),
        HelpEntry::Blank => String::new(),
    }
}
