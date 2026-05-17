---
title: "Help Overlay and No Footer Help"
type: quick-work
date: 2026-05-17
---

# Help Overlay and No Footer Help

## Task
Stop showing persistent help messages under the input box. Add `/help` with a well-designed help UX that explains commands and shortcuts.

## Changes
- `crates/oino-tui/src/help.rs`: added structured help content for composer usage, commands, transcript navigation, streaming/queue/drafts, overlays, and exit behavior.
- `crates/oino-tui/src/command.rs`: added `/help` command parsing and suggestions.
- `crates/oino-tui/src/app.rs`: added Help overlay state, `/help` handling, scrolling, and close behavior.
- `crates/oino-tui/src/render.rs`: removed the persistent footer under the composer, added a scrollable Help overlay, and moved transient non-help statuses into the transcript area.
- `crates/oino-tui/src/composer.rs`: changed placeholder to mention `/help` and `@ file paths`.
- `crates/oino-app/src/main.rs`: handled non-interactive `/help` with a short textual response.
- `README.md`: documented `/help` and the removal of persistent help text under input.

## Verification
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All passed.

## Notes
- Overlay-local controls still appear inside overlays, not under the composer.
- A pre-existing `.gitignore` worktree change was left untouched.
