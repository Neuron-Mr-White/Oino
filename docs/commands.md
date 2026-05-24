# Commands, suggestions, and non-interactive use

Commands are the fastest way to navigate Oino without leaving the composer. For adjacent guides, see [TUI shell](auth-openrouter-tui.md), [transcript rendering](transcript-rendering.md), [sessions](sessions.md), [resources](resources.md), [themes](theme-system/README.md), and [extensions](extension-kernel/user-guide.md).

## Finding commands in the TUI

- Type `/` at the start of the composer to open command suggestions.
- Use the default Up/Down controls to choose, Tab to insert or complete the highlighted item, Enter to run a complete command, and Esc to dismiss suggestions. If you changed keymaps, `/help` shows the active labels.
- Type `/help` when you forget a shortcut. The help overlay always shows your active keymap labels, and `/` inside Help searches the help text.
- Use `@` anywhere in the composer to search project file paths. Dropped terminal file paths become `@relative/path` mentions when the terminal reports them.
- Use `/prompt:<name>` and `/skill:<name>` to include resources in the next message. `/P:<query>` and `/S:<query>` are composer search shortcuts that expand to the long resource tokens.

## Common commands

| Command | Use it for |
|---|---|
| `/help` | Open searchable help for commands and shortcuts. |
| `/new` | Start a fresh session after the current session has messages. See [sessions](sessions.md). |
| `/sessions` | Browse saved sessions. |
| `/title <name>` | Rename the current session. |
| `/settings` | Open the settings hub. |
| `/model` or `/model <provider:model-id>` | Open model selection or set the model directly. |
| `/thinking` or `/thinking <level>` | Open thinking-level selection or set `off`, `minimal`, `low`, `medium`, `high`, or `xhigh`. |
| `/settings collapse <thinking|tool> <full|truncate|collapse>` | Change transcript detail level. See [transcript rendering](transcript-rendering.md). |
| `/settings chat-style <chat|agentic|minimal>` | Switch transcript style. |
| `/settings tools` | Show enabled agent tools by scope. |
| `/settings keymaps` | Review or change shortcuts. |
| `/settings theme` or `/theme` | Open theme selection. See [themes](theme-system/README.md). |
| `/settings extensions` or `/extensions` | Manage installed extensions and contribution toggles. |
| `/login claude` or `/login chatgpt` | Run the official Claude Code or ChatGPT/Codex OAuth login flow. |
| `/prompts` | Browse prompt templates. See [resources](resources.md). |
| `/skills` | Browse skills. |
| `/reload` | Rescan `SYSTEM.md`, `AGENT.md`, prompts, skills, and extension resources after edits. |
| `/inspect` | Inspect the full prompt and export chat HTML from the inspect overlay. See [transcript rendering](transcript-rendering.md). |

## Non-interactive use

You can run a command or prompt from the shell:

```bash
oino "/sessions"
oino --session <uuid> "/title Release notes"
oino --session <uuid> "/model openrouter:xai/glm-5.1"
oino "/login claude"
oino "/login chatgpt"
oino --session <uuid> "Use /prompt:review to check this plan"
```

Rules to remember:

- A shell input that starts with a slash and has no resource tokens runs as a local command instead of calling the provider.
- `/sessions`, `/prompts`, `/skills`, `/inspect`, `/reload`, `/title <name>`, `/login claude`, `/login chatgpt`, `/model <provider:model-id>`, `/thinking <level>`, `/settings model ...`, `/settings thinking ...`, `/settings collapse ...`, and `/settings chat-style ...` work from the shell.
- Overlay-only commands such as `/settings`, `/model` with no value, `/thinking` with no value, `/settings tools`, `/settings keymaps`, `/settings theme`, `/theme`, `/settings extensions`, `/extensions`, and `/new` need the TUI.
- Long resource tokens `/prompt:<name>` and `/skill:<name>` expand in shell prompts. Short `/P:` and `/S:` are only composer search shortcuts.
- Non-interactive prompts print the assistant's final text and write the session id to stderr.

## Contributor notes

- [`oino-tui`](../crates/oino-tui) owns slash-command parsing, suggestions, help text, keymap-aware labels, resource-token suggestions, and file-path suggestions.
- [`oino-app`](../crates/oino-app) owns shell argument parsing, non-interactive command execution, resource expansion for CLI prompts, and persistence after command changes.
- When adding a command, update parser tests, help text, command suggestions, this guide, and any focused guide that owns the feature area.
