# Sessions and history

Sessions are local conversation histories. Use them when you want to pause work, return to a previous thread, or continue a saved chat from a shell command.

Adjacent guides: [TUI shell](auth-openrouter-tui.md), [resources](resources.md), and [current limitations](../README.md#current-limitations).

## Start or continue in the TUI

| Task | Command or control |
|---|---|
| Start a new conversation | `/new` after the current session has messages. Blank sessions stay in place. |
| Browse saved sessions | `/sessions` |
| Search the browser | `/` in the sessions browser, then type to fuzzy-search title, id, preview, or project path. |
| Continue a session | Select a row and press Enter. |
| Refresh the browser | `r` |
| Close | Esc. When search is active, Esc clears search first. |
| Name the session | `/title <name>` |

The sessions browser lists saved sessions with at least one message. The current session is marked with a dot, rows are newest-first, and each row shows the title plus the latest useful preview. Opening `/sessions` alone does not write a blank startup session to disk.

You cannot start or switch sessions while a prompt is running. Stop or let the response finish first.

## Continue from the shell

Use the session id shown in `/sessions` or the file name under `~/.oino/sessions`:

```bash
oino --session <uuid>
oino --session <uuid> "continue this thread with a short plan"
oino --session <uuid> /title "Release checklist"
```

Start Oino from the project you intend to work in. Session rows show the project path recorded when the session was created, but `/sessions` is history navigation, not a general project switcher.

## What is saved

Oino stores session history locally under `~/.oino/sessions/<uuid>.jsonl`.

Saved:

- user, assistant, and tool-result messages
- the latest session title
- model and thinking-level changes made inside the session
- compaction or branch-summary entries

Not saved as part of the session history:

- unsent composer text
- send-panel queue and draft items
- open overlays, search text, and scroll position
- API keys, model-list cache, and user settings; those use separate files under `~/.oino`

Session files are an implementation format, not an import/export contract. Prefer `/sessions`, `/new`, `/title`, and `--session` over hand-editing JSONL files. Deleting, importing, and migrating sessions are still follow-up workflows.

## Contributor notes

- [`oino-session`](../crates/oino-session) owns the append-only session tree and JSONL load/save format.
- [`oino-harness`](../crates/oino-harness) rebuilds model context from the active branch and records model, thinking, title, message, and extension entries.
- [`oino-tui`](../crates/oino-tui) owns `/new`, `/sessions`, `/title`, search state, and browser controls.
- [`oino-app`](../crates/oino-app) wires the repository path, lazy blank-session behavior, non-interactive `--session`, and TUI session switching.
