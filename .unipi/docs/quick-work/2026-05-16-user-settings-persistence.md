---
title: "User Settings Persistence"
type: quick-work
date: 2026-05-16
---

# User Settings Persistence

## Task
Persist user settings so selected model and thinking level survive app restarts, preferably under `~/.oino`.

## Changes
- `crates/oino-app/src/user_settings.rs`: added JSON persistence for user settings at `~/.oino/settings.json`, currently storing `model` and `thinking_level`.
- `crates/oino-app/src/main.rs`: loads persisted settings at startup, applies persisted thinking level to the harness config, and saves settings after successful model/thinking changes from the TUI.
- `README.md` and `docs/auth-openrouter-tui.md`: documented the settings file and `OINO_MODEL` override behavior.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
`OINO_MODEL` remains a startup override when present, but settings selected from `/settings` are written to `~/.oino/settings.json` for future launches.
