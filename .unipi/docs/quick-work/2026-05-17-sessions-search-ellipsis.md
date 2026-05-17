---
title: "Sessions Search and Ellipsis"
type: quick-work
date: 2026-05-17
---

# Sessions Search and Ellipsis

## Task
Fix the saved sessions browser so long rows are truncated with an ellipsis instead of being clipped by the panel edge, and add `/` fuzzy search similar to model selection. Use a strong Rust fuzzy search library if needed.

## Changes
- `Cargo.toml`, `Cargo.lock`, `crates/oino-tui/Cargo.toml`: added `nucleo-matcher` for high-quality fuzzy matching. This is the Helix/Nucleo matcher and is faster/better ranked than simple substring matching.
- `crates/oino-tui/src/app.rs`: added sessions search state, Nucleo-backed fuzzy filtering over session name/id/preview/cwd, `/` to enter search, typed query handling, Up/Down over filtered results, Esc to clear search, and Enter to continue the highlighted session.
- `crates/oino-tui/src/render.rs`: added sessions search line and footer controls, filtered counts in the title, and ellipsis truncation for long session rows.
- `crates/oino-tui/src/text.rs`: added `truncate_with_ellipsis` helper.
- `README.md`: documented `/sessions` search.

## Verification
```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Targeted tests added/updated:
- `sessions_command_opens_browser_and_enter_selects_session`
- `render_sessions_overlay_ellipsizes_long_rows`

## Notes
Inside `/sessions`, press `/` to search. Search uses fuzzy matching across session name, UUID, preview text, and cwd. Enter still continues the selected filtered session.
