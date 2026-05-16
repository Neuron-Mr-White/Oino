---
title: "TUI Model Title"
type: quick-work
date: 2026-05-16
---

# TUI Model Title

## Task
Replace assistant bubble title text with the model name.

## Changes
- `crates/oino-tui/src/message.rs`: added optional `MessageView::title` and project assistant provider metadata model name into it.
- `crates/oino-tui/src/app.rs`: streaming assistant messages use the current selected model as their title, and final messages keep that title if provider metadata does not include a model.
- `crates/oino-tui/src/render.rs`: bubble borders now display `MessageView::title` when present, falling back to role.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
The internal role remains `assistant` for styling and behavior; only the visible bubble title changes to the model name.
