//! Static command registry, package order, labels and tones (unipi `src/constants.ts`,
//! `core/src/package-colors.ts`) plus pi's built-in slash commands.

/// Packages by display priority (top-to-bottom in autocomplete).
pub const PACKAGE_ORDER: &[&str] = &[
    "workflow",
    "long-horizon",
    "memory",
    "btw",
    "mcp",
    "utility",
    "skill-registry",
    "ask-user",
    "info",
    "web-api",
    "compact",
    "notify",
    "kanboard",
    "footer",
    "updater",
    "input-shortcuts",
    "image",
    "subagents",
    "background-tasks",
    "fusion",
    "cron",
];

/// Command name → package (unipi `COMMAND_REGISTRY` without the `unipi:` prefix; `/cron` is
/// the Oino-native scheduler (`unipi.cron`, unipi-port §3.24); `/settings` and `/changelog`
/// merged into the core built-ins; the Fusion model picker is `/fusion`).
pub const COMMAND_REGISTRY: &[(&str, &str)] = &[
    ("plan", "workflow"),
    ("permission", "workflow"),
    ("goal", "long-horizon"),
    ("ralph", "long-horizon"),
    ("swarm", "long-horizon"),
    ("graph", "long-horizon"),
    ("regular", "long-horizon"),
    ("visualize-progress", "long-horizon"),
    ("memory", "memory"),
    ("memory-process", "memory"),
    ("memory-consolidate", "memory"),
    ("memory-search", "memory"),
    ("global-memory-search", "memory"),
    ("memory-forget", "memory"),
    ("global-memory-list", "memory"),
    ("btw", "btw"),
    ("subagents", "subagents"),
    ("agents", "subagents"),
    ("mcp-status", "mcp"),
    ("continue", "utility"),
    ("retry", "utility"),
    ("cleanup", "utility"),
    ("doctor", "utility"),
    ("dream", "utility"),
    ("answer", "utility"),
    ("summarize", "utility"),
    ("move", "utility"),
    ("skills", "skill-registry"),
    ("bg", "background-tasks"),
    ("bg-tasks", "background-tasks"),
    ("fusion", "fusion"),
    ("fusion-stats", "fusion"),
    ("info", "info"),
    ("hint", "info"),
    ("compact-vcc", "compact"),
    ("compact-by-llm", "compact"),
    ("session-recall", "compact"),
    ("compact-stats", "compact"),
    ("compact-doctor", "compact"),
    ("compact-help", "compact"),
    ("notify-event", "notify"),
    ("cron", "cron"),
    ("kanboard", "kanboard"),
    ("kanboard-add", "kanboard"),
    ("kanboard-do", "kanboard"),
    ("kanboard-autowork", "kanboard"),
    ("footer", "footer"),
    ("readme", "updater"),
];

/// Short descriptions used when the host's command list lacks one (unipi `COMMAND_DESCRIPTIONS`).
pub const COMMAND_DESCRIPTIONS: &[(&str, &str)] = &[
    (
        "cron",
        "Scheduled, unattended Oino runs — jobs, run history, pause/resume (see skill:cron)",
    ),
    (
        "plan",
        "Plan mode — investigate read-only, then approve a plan",
    ),
    (
        "permission",
        "Permission mode — ask · auto (jev-judged) · full",
    ),
    (
        "goal",
        "One objective until verifiably true · medium complexity · pareto cost/success",
    ),
    (
        "ralph",
        "Checklist grind over iterations · enumerable chores · low cost, solid success",
    ),
    (
        "swarm",
        "Parallel fan-out + synthesis · complex decomposable · higher cost, high coverage",
    ),
    (
        "graph",
        "Dependent multi-step work · later steps need earlier results · highest cost",
    ),
    (
        "regular",
        "Regular mode — stop the active owner and run prompts without long-horizon routing",
    ),
    (
        "visualize-progress",
        "Live view of the goal / ralph / swarm / graph run (Esc closes)",
    ),
    ("memory", "Memory palace — status, recall/write toggles"),
    (
        "memory-process",
        "Analyze text and store extracted memories",
    ),
    (
        "memory-consolidate",
        "Consolidate the current session into memory",
    ),
    ("memory-search", "Search project memories"),
    (
        "global-memory-search",
        "Search memories across all projects",
    ),
    ("memory-forget", "Delete a memory by title"),
    (
        "global-memory-list",
        "List all memories across all projects",
    ),
    ("btw", "Ask a side question in an inline panel (read-only)"),
    (
        "subagents",
        "Open the subagent panel (also ↓ from an empty input)",
    ),
    (
        "agents",
        "Manage subagent profiles — create, edit, copy, delete custom agents",
    ),
    ("mcp-status", "Show MCP server status"),
    ("continue", "Continue from where the agent stopped (/retry)"),
    ("retry", "Retry the last turn (alias of /continue)"),
    (
        "cleanup",
        "Remove stale Oino temp files and leftovers (preview first)",
    ),
    (
        "doctor",
        "Check Oino's runtime: config, model cache, Decision Model, skills",
    ),
    (
        "answer",
        "Answer the last reply — fixed input under the scrollable reply, per question, or web form",
    ),
    (
        "summarize",
        "Summarize your last reply again (answer first, then findings, open items, questions). Add text to ask about it",
    ),
    (
        "skills",
        "Manage skills — on/off per project, listed or not, the skill vault",
    ),
    (
        "move",
        "Re-link sessions, trust, worktrees, boards and memories after moving this project",
    ),
    (
        "kanboard",
        "Kanboard — capture tasks, run them, open the board",
    ),
    ("kanboard-add", "Kanboard — capture a task into Backlog"),
    (
        "kanboard-do",
        "Kanboard — let the agent work the board for a turn",
    ),
    (
        "kanboard-autowork",
        "Kanboard — start or stop the queue runner",
    ),
    (
        "info",
        "Dashboard — this session, context, usage, compaction savings, modules",
    ),
    ("hint", "Browse and search Unicrab hints, or cycle/reset"),
    (
        "compact-vcc",
        "Lossless compaction now — no model call (keep:N)",
    ),
    ("compact-by-llm", "Compact now with a model-written summary"),
    (
        "session-recall",
        "Search session history, including compacted-away messages",
    ),
    ("compact-stats", "Show this session's compaction savings"),
    ("compact-doctor", "Check compaction settings"),
    ("compact-help", "Show compactor command help"),
    (
        "notify-event",
        "Toggle a notify event without the TUI: <event> <on|off>",
    ),
    ("footer", "Toggle footer on or off"),
    ("readme", "Browse package README files"),
    ("bg", "Start a shell command as a tracked background task"),
    ("bg-tasks", "Open the background task manager UI"),
    ("dream", "Dream status, report and proposal approval"),
    (
        "fusion",
        "Pick a model or Fusion lead+sidekick pair (Devin-style picker)",
    ),
    (
        "fusion-stats",
        "Estimated Fusion savings (sidekick tokens priced at lead rates)",
    ),
];

/// Package tag labels (unipi `PACKAGE_LABELS`).
pub const PACKAGE_LABELS: &[(&str, &str)] = &[
    ("long-horizon", "long-horizon"),
    ("workflow", "workflow"),
    ("ralph", "long-horizon"),
    ("memory", "memory"),
    ("btw", "btw"),
    ("mcp", "mcp"),
    ("utility", "utility"),
    ("skill-registry", "skills"),
    ("ask-user", "ask-user"),
    ("info", "info"),
    ("web-api", "web-api"),
    ("compact", "compact"),
    ("notify", "notify"),
    ("cron", "cron"),
    ("kanboard", "kanboard"),
    ("footer", "footer"),
    ("updater", "updater"),
    ("input-shortcuts", "input-shortcuts"),
    ("image", "image"),
    ("subagents", "subagents"),
    ("background-tasks", "background-tasks"),
    ("fusion", "fusion"),
];

/// Package colour as an OUS tone (unipi `PACKAGE_COLORS`, ANSI → categorical palette: packages
/// sharing an ANSI colour in unipi share a tone here).
pub const PACKAGE_TONES: &[(&str, &str)] = &[
    ("autocomplete", "c12"),
    ("workflow", "c1"),
    ("long-horizon", "c2"),
    ("memory", "c3"),
    ("btw", "c4"),
    ("mcp", "c5"),
    ("utility", "c6"),
    ("ask-user", "c7"),
    ("info", "c8"),
    ("web-api", "c4"),
    ("compact", "default"),
    ("notify", "c9"),
    ("cron", "c10"),
    ("kanboard", "c10"),
    ("footer", "c11"),
    ("updater", "c3"),
    ("input-shortcuts", "c4"),
    ("image", "c8"),
    ("subagents", "c11"),
    ("background-tasks", "c1"),
    ("fusion", "c9"),
    ("watchdog", "muted"),
    ("skill-registry", "c10"),
];

/// pi 1.4.0 `BUILTIN_SLASH_COMMANDS` (`pi` → `Oino` in the quit text, D13): `(name, description,
/// argument hint)`. Frontends list these in their own slash group; the enchanted group re-ranks
/// them together with extension commands.
pub const BUILTIN_COMMANDS: &[(&str, &str, Option<&str>)] = &[
    ("settings", "Open settings menu", None),
    (
        "model",
        "Select model (opens selector UI)",
        Some("<provider/model>"),
    ),
    ("tree", "Navigate session tree (switch branches)", None),
    ("thinking", "Set thinking level", Some("<level>")),
    (
        "scoped-models",
        "Enable/disable models for Ctrl+P cycling",
        None,
    ),
    (
        "export",
        "Export session (HTML default, or specify path: .html/.jsonl)",
        None,
    ),
    (
        "import",
        "Import and resume a session from a JSONL file",
        None,
    ),
    (
        "share",
        "Share session via a Radius artifact or a GitHub gist",
        None,
    ),
    (
        "bug",
        "Report a bug to the Pi developers",
        Some("<description>"),
    ),
    ("copy", "Copy last agent message to clipboard", None),
    ("name", "Set session display name", None),
    ("session", "Show session info and stats", None),
    ("changelog", "Show changelog entries", None),
    ("hotkeys", "Show all keyboard shortcuts", None),
    (
        "fork",
        "Create a new fork from a previous user message",
        None,
    ),
    (
        "clone",
        "Duplicate the current session at the current position",
        None,
    ),
    (
        "trust",
        "Save project trust decision for future sessions",
        None,
    ),
    (
        "login",
        "Configure provider authentication",
        Some("<provider>"),
    ),
    ("logout", "Remove provider authentication", None),
    ("new", "Start a new session", None),
    ("compact", "Manually compact the session context", None),
    ("resume", "Resume a different session", None),
    (
        "reload",
        "Reload keybindings, extensions, skills, prompts, themes, and context files",
        None,
    ),
    ("quit", "Quit Oino", None),
];

/// Looks up `key` in a `(key, value)` table.
#[must_use]
pub fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Package of a registered unipi command.
#[must_use]
pub fn package_of(command: &str) -> Option<&'static str> {
    lookup(COMMAND_REGISTRY, command)
}

/// Display index of a package (unknown packages sort last, like JS `indexOf` = -1 … but stable).
#[must_use]
pub fn package_rank(package: &str) -> isize {
    PACKAGE_ORDER
        .iter()
        .position(|p| *p == package)
        .and_then(|i| isize::try_from(i).ok())
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_and_descriptions_cover_the_same_commands() {
        let mut registry: Vec<&str> = COMMAND_REGISTRY.iter().map(|(c, _)| *c).collect();
        let mut described: Vec<&str> = COMMAND_DESCRIPTIONS.iter().map(|(c, _)| *c).collect();
        registry.sort_unstable();
        described.sort_unstable();
        assert_eq!(registry, described);
        // Core built-ins always win: no ported command shadows one.
        for (name, _, _) in BUILTIN_COMMANDS {
            assert!(!registry.contains(name), "{name} collides with a built-in");
        }
    }

    #[test]
    fn every_registry_package_has_a_label_and_tone() {
        for (_, pkg) in COMMAND_REGISTRY {
            assert!(lookup(PACKAGE_LABELS, pkg).is_some(), "{pkg} label");
            assert!(lookup(PACKAGE_TONES, pkg).is_some(), "{pkg} tone");
        }
        assert_eq!(package_rank("workflow"), 0);
        assert_eq!(package_rank("nope"), -1);
    }
}
