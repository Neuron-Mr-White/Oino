---
title: "TUI Thinking Section"
type: quick-work
date: 2026-05-16
---

# TUI Thinking Section

## Task
Stop displaying thinking content as plain inline text and render it as a distinct thinking section in assistant messages.

## Changes
- `crates/oino-tui/src/message.rs`: projects `ContentBlock::Thinking` into separate `MessageView::thinking` fields instead of merging it into normal message content.
- `crates/oino-tui/src/render.rs`: renders thinking inside assistant bubbles as a muted `◌ thinking` section before the final answer, including redacted-thinking handling.
- `crates/oino-tui/src/app.rs`: streaming content projection no longer formats thinking as `<thinking:...>` if thinking blocks appear in updates.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
This is the first slice of richer message rendering. Thinking is visually separated but not yet collapsible/toggleable.
