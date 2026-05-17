#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpEntry {
    Heading(&'static str),
    Item(&'static str, &'static str),
    Text(&'static str),
    Blank,
}

pub(crate) const HELP_ENTRIES: &[HelpEntry] = &[
    HelpEntry::Text("Type /help any time to reopen this guide. Esc or q closes it."),
    HelpEntry::Blank,
    HelpEntry::Heading("Composer"),
    HelpEntry::Item(
        "Enter",
        "send a prompt; while the assistant is streaming, send steering text",
    ),
    HelpEntry::Item("Ctrl-J / Alt-Enter / Shift-Enter", "insert a newline"),
    HelpEntry::Item(
        "/",
        "open fuzzy command suggestions at the start of the input",
    ),
    HelpEntry::Item(
        "@",
        "fuzzy search project file paths; Tab inserts the highlighted path",
    ),
    HelpEntry::Item(
        "/P:<query> / /S:<query>",
        "scope slash suggestions to prompt templates or skills",
    ),
    HelpEntry::Item(
        "Paste",
        "large or multiline pastes collapse visually but still submit in full",
    ),
    HelpEntry::Item("Ctrl-O e", "expand a collapsed pasted block at the cursor"),
    HelpEntry::Blank,
    HelpEntry::Heading("Commands"),
    HelpEntry::Item("/help", "open this help overlay"),
    HelpEntry::Item("/new", "start a fresh session after this one has messages"),
    HelpEntry::Item(
        "/sessions",
        "browse saved sessions; press Enter to continue one",
    ),
    HelpEntry::Item("/settings", "open settings pages"),
    HelpEntry::Item(
        "/prompts",
        "browse prompt templates from <project>/.oino/prompts/",
    ),
    HelpEntry::Item(
        "/skills",
        "browse skills from ~/.oino/skills/ and <project>/.oino/skills/",
    ),
    HelpEntry::Item("/reload", "reload SYSTEM.md, AGENT.md, prompts, and skills"),
    HelpEntry::Item("/skill:<name>", "load and run a selected skill"),
    HelpEntry::Item(
        "/model <provider:model>",
        "change model directly, or /model to open model selection",
    ),
    HelpEntry::Item(
        "/thinking <level>",
        "set reasoning level: off, minimal, low, medium, high, xhigh",
    ),
    HelpEntry::Blank,
    HelpEntry::Heading("Transcript"),
    HelpEntry::Item("PgUp / PgDn", "scroll by page"),
    HelpEntry::Item("Alt-Up / Alt-Down", "scroll by line"),
    HelpEntry::Item("Ctrl-Home / Ctrl-End", "jump to top or bottom"),
    HelpEntry::Item(
        "Ctrl-O t",
        "focus transcript; then j/k, Home/End, g/G navigate and Esc returns",
    ),
    HelpEntry::Item(
        "Ctrl-click links/images",
        "open visible URL or image placeholders when the terminal supports it",
    ),
    HelpEntry::Blank,
    HelpEntry::Heading("Streaming, queue, and drafts"),
    HelpEntry::Item(
        "Enter while streaming",
        "steer the current response with the current input",
    ),
    HelpEntry::Item(
        "Ctrl-O s",
        "open the send panel for steering history, queue, and drafts",
    ),
    HelpEntry::Item("Send panel q", "queue current input for the next turn"),
    HelpEntry::Item("Send panel d", "move current input to Draft"),
    HelpEntry::Item(
        "Send panel x",
        "delete selected queued/draft item after confirmation",
    ),
    HelpEntry::Blank,
    HelpEntry::Heading("Overlays and exit"),
    HelpEntry::Item(
        "Esc",
        "close the top overlay, clear search, or stop a running response; it does not quit",
    ),
    HelpEntry::Item("Ctrl-C twice", "quit Oino"),
];

#[must_use]
pub(crate) fn help_entry_match_text(entry: &HelpEntry) -> String {
    match entry {
        HelpEntry::Heading(text) | HelpEntry::Text(text) => (*text).to_string(),
        HelpEntry::Item(key, description) => format!("{key} {description}"),
        HelpEntry::Blank => String::new(),
    }
}
