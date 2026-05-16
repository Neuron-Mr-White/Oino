---
title: "TUI Chord Mode Hint"
type: quick-work
date: 2026-05-16
---

# TUI Chord Mode Hint

## Task
Make chord mode visibly obvious and ensure `Esc` exits chord mode.

## Changes
- `crates/oino-tui/src/app.rs`: added explicit test coverage that `Esc` exits `Ctrl-O` chord mode without opening an overlay.
- `crates/oino-tui/src/render.rs`: added a full-screen red chord-mode border with a title hint, currently `Ctrl-O chord: s settings • Esc cancel`.

## Verification
- `cargo fmt`
- `cargo test -p oino-tui`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
The footer status hint remains, and the full red border makes chord mode hard to miss without replacing the current UI.
