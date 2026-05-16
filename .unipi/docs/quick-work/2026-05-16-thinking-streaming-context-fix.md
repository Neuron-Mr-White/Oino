---
title: "Thinking Streaming and Context Fix"
type: quick-work
date: 2026-05-16
---

# Thinking Streaming and Context Fix

## Task
Fix thinking blocks not streaming and prevent second-message stream errors after an assistant response includes thinking content.

## Changes
- `crates/oino-agent-loop/src/lib.rs`: thinking deltas now emit `MessageUpdate` events and are included in streamed current content.
- `crates/oino-tui/src/app.rs`: streamed thinking blocks are projected into the TUI thinking section instead of being discarded.
- `crates/oino-tui/src/message.rs`: exposed content-block projection so streaming and final messages use the same thinking/content split.
- `crates/oino-provider-openrouter/src/lib.rs`: assistant thinking blocks are ignored when rebuilding OpenRouter chat history, so private reasoning is not replayed and no longer causes unsupported-content errors on follow-up prompts.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
Root cause: thinking deltas were accumulated silently until finalization, and finalized thinking blocks were then present in transcript history. The OpenRouter adapter rejected `ContentBlock::Thinking` when serializing prior assistant messages for the next request.
