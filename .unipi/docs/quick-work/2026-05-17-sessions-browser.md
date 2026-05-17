---
title: "Sessions Browser"
type: quick-work
date: 2026-05-17
---

# Sessions Browser

## Request
Add `/sessions` so users can browse past local sessions and press Enter to continue one. Also ensure a blank run of `/sessions` does not create a new session file.

## Implementation
- Added `/sessions` to slash command suggestions and parsing.
- Added a Sessions overlay with Up/Down or j/k selection, Enter to continue, `r` to reload, and Esc to close.
- Added session list rows with session id, name, message count, preview text, cwd, and current-session marker.
- Opening a session loads its JSONL file, swaps the active harness session, restores agent messages/model/thinking from the loaded session context, updates the TUI transcript, and moves future saves to that session file.
- Changed default startup session creation to be lazy/in-memory. Without `--session`, Oino now allocates a session id/path but does not write `~/.oino/sessions/<uuid>.jsonl` until a prompt/settings save actually occurs.
- `SessionManager::save_jsonl` now creates parent directories, and repository listing returns an empty list when the session root does not exist.
- Non-interactive `oino /sessions` prints the saved session list and does not create a blank session file.

## Files Modified
- `crates/oino-tui/src/command.rs`
- `crates/oino-tui/src/action.rs`
- `crates/oino-tui/src/app.rs`
- `crates/oino-tui/src/render.rs`
- `crates/oino-tui/src/lib.rs`
- `crates/oino-agent/src/lib.rs`
- `crates/oino-harness/src/lib.rs`
- `crates/oino-session/src/lib.rs`
- `crates/oino-app/src/main.rs`
- `README.md`

## Verification
```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Also manually verified that a clean-home non-interactive `/sessions` run creates no files:

```bash
tmp_home=$(mktemp -d)
HOME="$tmp_home" cargo run -q -p oino-app -- /sessions
find "$tmp_home" -maxdepth 3 -type f -print
```

Output listed `No saved sessions` and no files.
