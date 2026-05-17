---
title: "Searchable Help Overlay"
type: quick-work
date: 2026-05-17
---

# Searchable Help Overlay

## Task
Make `/help` documentation searchable by pressing `/` inside the Help overlay.

## Changes
- `crates/oino-tui/src/help.rs`: added searchable text extraction for help entries.
- `crates/oino-tui/src/app.rs`: added Help search state, cached filtered indices, `/` search activation, typing/backspace, Enter to keep results, Esc to clear search, and scroll handling over filtered results.
- `crates/oino-tui/src/render.rs`: added Help search line, match-count title, empty-search state, and search-mode controls.
- `README.md`: documented `/` inside Help for fuzzy-searching docs.

## Verification
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All passed.

## Notes
- Search uses the shared high-level Nucleo helper and cached filtered indices; render only reads cached state.
- Pre-existing `.gitignore` worktree changes were left untouched.
