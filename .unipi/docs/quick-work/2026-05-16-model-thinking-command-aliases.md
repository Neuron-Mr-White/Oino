---
title: "Model and Thinking Command Aliases"
type: quick-work
date: 2026-05-16
---

# Model and Thinking Command Aliases

## Task

Add common-setting aliases so users can use `/model` and `/thinking` directly instead of always typing `/settings model` or `/settings thinking`.

## Changes

- `crates/oino-tui/src/command.rs`
  - Added root commands `/model` and `/thinking`.
  - `/model <provider:model-id>` aliases `/settings model <provider:model-id>`.
  - `/thinking <level>` aliases `/settings thinking <level>`.
  - `/model <caret>` suggests model ids.
  - `/thinking <caret>` suggests thinking levels.
- `crates/oino-tui/src/app.rs`
  - Bare `/model` opens the settings overlay directly on Model Selection.
  - Bare `/thinking` opens the settings overlay directly on Thinking Level.
- `crates/oino-tui/src/settings.rs`
  - Added helpers to open model/thinking child pages directly.
- `crates/oino-app/src/main.rs`
  - Updated CLI help and non-interactive error guidance for aliases.
- `README.md`, `docs/auth-openrouter-tui.md`
  - Documented the aliases.

## Verification

- `cargo fmt --all`
- `cargo test -p oino-tui -p oino-app`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes

The original `/settings model ...` and `/settings thinking ...` commands still work unchanged.
