---
title: "Hide Tool-Call-Only Assistant Bubbles — Quick Fix"
type: quick-fix
date: 2026-05-16
---

# Hide Tool-Call-Only Assistant Bubbles — Quick Fix

## Bug

Assistant messages that only contained a tool call rendered as visible text like `<tool-call:write>`, even though the actual tool execution already appears as a separate `tool:write` bubble.

## Root Cause

`crates/oino-tui/src/message.rs` projected `ContentBlock::ToolCall` into display text. A tool-call-only assistant message therefore produced a normal assistant bubble. The TUI then also rendered the separate tool result bubble, causing duplicate visual tool information.

## Fix

Tool calls are no longer projected as assistant-visible text. Assistant messages with no visible text and no thinking are skipped by bubble rendering, so tool-call-only assistant messages stay in the transcript state/session context but do not create empty visible bubbles.

### Files Modified

- `crates/oino-tui/src/message.rs` — ignore `ContentBlock::ToolCall` in visible content projection and add a regression test.
- `crates/oino-tui/src/render.rs` — skip empty assistant bubbles and add a render regression test.

## Verification

- `cargo fmt --all`
- `cargo test -p oino-tui`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes

Tool result bubbles such as `tool:write` still render normally and remain controlled by the existing tool collapse mode.
