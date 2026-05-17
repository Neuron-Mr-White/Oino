---
title: "Command Paths, CLI Settings, and Session Continuation"
type: quick-work
date: 2026-05-16
---

# Command Paths, CLI Settings, and Session Continuation

Implemented a unified command-path direction for settings and non-interactive runs.

## Decisions

- Canonical model identifier format is `provider:model-id`, e.g. `openrouter:xai/glm-5.1`.
- Slash commands and CLI settings use the same command semantics.
- Settings commands are hierarchical:
  - `/settings`
  - `/settings model <provider:model-id>`
  - `/settings thinking <off|minimal|low|medium|high|xhigh>`
  - `/settings collapse <thinking|tool> <full|truncate|collapse>`

## Changes

- Added `Model::identifier()` and `Model::from_identifier()` to `oino-types`.
- Refactored TUI command parsing from single-command exact matching into parsed command enums.
- Added context-aware slash suggestions:
  - `/settings <caret>` suggests `model`, `thinking`, `collapse`
  - `/settings model <caret>` suggests cached model ids
  - `/settings thinking <caret>` suggests thinking levels
  - `/settings collapse <caret>` suggests targets
  - `/settings collapse thinking <caret>` suggests collapse modes
- Added composer range replacement so completions can replace only the active token.
- OpenRouter model catalog options now expose canonical ids such as `openrouter:openai/gpt-4o-mini`.
- Added CLI parsing for:
  - `oino --settings --model openrouter:xai/glm-5.1`
  - `oino --session <uuid> <message-or-command>`
- TUI and non-interactive runs now create/load JSONL sessions under `~/.oino/sessions` and save after prompts/settings changes.
- Harness now seeds agent state from loaded session context, so `--session <uuid>` continues prior messages/model/thinking.

## Validation

- `cargo fmt --all`
- `cargo test -p oino-tui -p oino-app`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`
