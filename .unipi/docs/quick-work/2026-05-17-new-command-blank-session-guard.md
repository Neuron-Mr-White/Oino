---
title: "New Command Blank Session Guard"
type: quick-work
date: 2026-05-17
---

# New Command Blank Session Guard

## Task
Ensure `/new` does not start/create another new session when the current session has no user input/content.

## Changes
- `crates/oino-tui/src/app.rs`: `/new` is now a no-op in a blank session and reports `Already in a blank session`; once a session has content, `/new` still clears transient state and requests a new session.
- `crates/oino-app/src/main.rs`: new TUI sessions are created lazily in memory instead of immediately writing an empty JSONL header file; the file appears only when future content/settings are saved.
- `crates/oino-app/Cargo.toml`: added `tempfile` as a dev dependency for the lazy-session test.
- `crates/oino-tui/src/help.rs`, `README.md`, `AGENT.md`: updated `/new` wording/convention.

## Verification
```bash
cargo fmt --all
cargo test -p oino-tui new_command_is_noop_in_blank_session -- --nocapture
cargo test -p oino-app new_tui_session_is_lazy_until_saved -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All passed.

## Notes
- Pre-existing `.gitignore` worktree changes were left untouched.
