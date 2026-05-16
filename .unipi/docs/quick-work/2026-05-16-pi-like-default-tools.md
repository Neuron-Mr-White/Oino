---
title: "Pi-like Default Tools"
type: quick-work
date: 2026-05-16
---

# Pi-like Default Tools

## Research
Pi-coding-agent's README and system prompt show the default model-visible tool set as:

- `read`
- `bash`
- `edit`
- `write`

Pi's built-in tool implementations live under `dist/core/tools/` in the installed package. Important behaviors copied for Oino's first slice:

- `read` supports `path`, `offset`, `limit` and truncation/continuation hints.
- `bash` runs in the current working directory, accepts an optional timeout in seconds, returns stdout/stderr, and truncates large output.
- `edit` performs exact text replacements and requires unique non-overlapping matches.
- `write` creates parent directories and overwrites content.

## Changes
- Added `crates/oino-tools` with default local tools implemented on `oino_env::ExecutionEnv`.
- Wired `oino_tools::default_tools(...)` into `oino-app` harness startup.
- Added an Oino default system prompt listing the built-in tools and concise usage guidelines.
- Updated README and compatibility docs.

## Notes
- `write` and `edit` return sequential execution mode to avoid concurrent file mutations.
- Image reads currently return a text note, not a `ContentBlock::Image`, because the first OpenRouter adapter still rejects image content during message serialization.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`
