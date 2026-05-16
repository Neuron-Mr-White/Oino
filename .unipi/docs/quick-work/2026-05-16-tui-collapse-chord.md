---
title: "TUI Collapse Mode and Ctrl-O Chord"
type: quick-work
date: 2026-05-16
---

# TUI Collapse Mode and Ctrl-O Chord

## Task
Add settings for thinking/tool collapse modes and make `Ctrl-O s` open settings.

## Changes
- `crates/oino-tui/src/settings.rs`: added `CollapseMode` (`Full`, `Truncate`, `Collapse`), collapse targets, a Collapse Mode settings page, and cycling behavior.
- `crates/oino-tui/src/app.rs`: added a `Ctrl-O` chord state; `Ctrl-O s` opens settings. Added TUI actions for collapse-mode changes.
- `crates/oino-tui/src/render.rs`: thinking and tool messages now respect collapse modes. Thinking/tool content can render full, truncated, or collapsed.
- `crates/oino-app/src/user_settings.rs` and `crates/oino-app/src/main.rs`: persisted thinking/tool collapse modes in `~/.oino/settings.json`.
- `README.md` and `docs/auth-openrouter-tui.md`: documented `Ctrl-O s` and collapse modes.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
`Ctrl-O` is now the Oino chord leader. The first implemented chord is `Ctrl-O s` for settings.
