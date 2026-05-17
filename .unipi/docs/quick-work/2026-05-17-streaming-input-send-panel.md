---
title: "Streaming Input Send Panel"
type: quick-work
date: 2026-05-17
---

# Streaming Input Send Panel

## Task
Allow users to keep typing while the LLM is streaming, support steering with Enter, and add a chord-opened panel for queued follow-ups and drafts.

## Changes
- `crates/oino-tui/src/app.rs`: kept the composer enabled during streaming; Enter while working records/sends a steer action; added Send Panel state with Steer, Queue, and Draft sections; implemented `q` queue, `d` draft, Enter load, and `x` delete-confirm (`y`/`d` delete, `n` cancel).
- `crates/oino-tui/src/render.rs`: rendered streaming status as the newest transcript line; changed `Ctrl-O s` chord hint to the Send Panel; added the Send Panel overlay UI.
- `crates/oino-tui/src/action.rs`: added `SteerPrompt` and `QueuePrompt` actions.
- `crates/oino-harness/src/lib.rs`: exposed `Harness::steer` for TUI steering.
- `crates/oino-app/src/main.rs`: wired steer actions to the agent, kept queued prompts in TUI state, and automatically starts queued prompts when idle/after the current prompt finishes.
- `README.md`: documented live streaming input, steering, Send Panel controls, and runtime status line behavior.

## Verification
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Notes
`Ctrl-O s` now opens the Send Panel. Settings remain available via `/settings` and settings command paths.
