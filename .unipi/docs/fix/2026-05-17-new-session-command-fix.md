---
title: "/new Session Command — Quick Fix"
type: quick-fix
date: 2026-05-17
---

# /new Session Command — Quick Fix

## Bug
Typing `/new` in the TUI did not open a new session. The slash-command parser only recognized settings/model/thinking commands, so `/new` was treated as an unknown command.

## Root Cause
There was no session command variant in the TUI command parser/action flow, and the app runtime had no path to replace the active harness session file while the TUI stayed open.

## Fix
Added `/new` as a slash command that creates a fresh local JSONL session, replaces the active harness session, resets the agent transcript/queues, clears TUI transcript/send-panel state, and updates the active `session_path` for future saves.

### Files Modified
- `crates/oino-tui/src/command.rs` — added `/new` command spec and parser support.
- `crates/oino-tui/src/action.rs` — added `TuiAction::NewSession`.
- `crates/oino-tui/src/app.rs` — mapped `/new` to the new action and added `reset_for_new_session`.
- `crates/oino-harness/src/lib.rs` — added `replace_session` to reset the agent and swap session manager.
- `crates/oino-app/src/main.rs` — creates the new session file under `~/.oino/sessions`, updates active path, and handles `/new` in the TUI.
- `README.md` — documented `/new` and the local session JSONL location/format.

## Verification

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Both passed.

## Notes
Sessions are local JSONL files at `~/.oino/sessions/<uuid>.jsonl`. The first JSONL line is the session header (`session_id`, `name`, `cwd`, version, current leaf), and following lines are append-only session entries such as messages, model changes, thinking-level changes, compactions, labels, and leaf moves.
