---
title: "Unified Nucleo Search Conventions"
type: quick-work
date: 2026-05-17
---

# Unified Nucleo Search Conventions

## Task
Use high-level `nucleo` for all Oino fuzzy/search UI components, including `@` file attach suggestions and future search features. Add persistent conventions to `AGENT.md`, including the rule to consult `.pi/skills/ratatui` for Ratatui/TUI work.

## Changes
- `crates/oino-tui/src/fuzzy.rs`: added a shared high-level `nucleo` fuzzy helper with text/path modes and tests.
- `crates/oino-tui/src/command.rs`: moved slash-command, settings value, model, and `@` file suggestions onto the shared Nucleo helper; removed the hand-written file fuzzy scorer.
- `crates/oino-tui/src/settings.rs`: moved Model Selection search to cached Nucleo-filtered indices.
- `crates/oino-tui/src/app.rs`: moved session search to the shared helper and cached command/file suggestion views so render does not rescore candidate lists.
- `crates/oino-tui/src/render.rs`: updated render tests to refresh the cached suggestion view when tests mutate composer text directly.
- `README.md`: documented Nucleo-backed command/file/model search.
- `AGENT.md`: added project conventions for memory/workflow, Nucleo search, Ratatui skill usage, sessions, and TUI interaction expectations.

## Verification

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All passed.

## Notes
- Future fuzzy/search features should reuse `crates/oino-tui/src/fuzzy.rs` and cache filtered views/indices outside render paths.
- Future Ratatui work should start by reading `.pi/skills/ratatui/SKILL.md` and only the relevant referenced subskills.
