---
title: "TUI Settings Overlay"
type: quick-work
date: 2026-05-16
---

# TUI Settings Overlay

## Task
Build a reusable TUI overlay and add `/settings` for model and thinking-level changes. Load OpenRouter models from a fast cache and refresh the full model list in the background.

## Changes
- `crates/oino-tui/src/settings.rs`: added reusable settings state, nested settings pages, model options, thinking-level availability, and keyboard handling.
- `crates/oino-tui/src/render.rs`: added centered overlay rendering with a settings menu plus dedicated Model Selection and Thinking Level child pages with visible arrow markers. Model Selection now uses a cursor-centered visible window so long model lists scroll and keep the active `›` pointer visible, plus an inline `/` search box that filters models.
- `crates/oino-tui/src/command.rs`: added slash-command registry and suggestion filtering for `/` composer input.
- `crates/oino-tui/src/app.rs` / `action.rs` / `lib.rs`: added `/settings`, command-suggestion lifecycle, overlay lifecycle, and setting-change actions.
- `crates/oino-app/src/model_catalog.rs`: added non-blocking OpenRouter model cache loading/refreshing at `~/.oino/openrouter-models.json`.
- `crates/oino-app/src/main.rs`: wired settings actions into `Harness::set_model` / `set_thinking_level`, persists selected settings, loads persisted settings at startup, and spawned the model catalog refresh task.
- `crates/oino-app/src/user_settings.rs`: added `~/.oino/settings.json` persistence for selected model and thinking level.
- `crates/oino-provider-openrouter/src/lib.rs`: added OpenRouter model listing and reasoning-effort serialization for non-off thinking levels.
- `README.md` and `docs/auth-openrouter-tui.md`: documented `/settings`, model cache, and thinking-level behavior.

## Verification
- `cargo fmt`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes
Model refresh is interval-based and non-blocking. The UI uses cached models immediately when available and keeps existing cached entries visible if a refresh fails. Slash-command suggestions appear only for the first composer token and can be dismissed with Esc before Esc quits the app.
