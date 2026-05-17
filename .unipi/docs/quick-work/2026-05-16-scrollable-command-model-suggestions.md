---
title: "Scrollable Command Model Suggestions"
type: quick-work
date: 2026-05-16
---

# Scrollable Command Model Suggestions

## Task

Make `/settings model <model>` command suggestions feel scrollable instead of only showing four visible model rows.

## Changes

- `crates/oino-tui/src/render.rs`
  - Model command suggestions now use a scrollable popup window of up to 5 visible rows.
  - Suggestions render a centered visible window around the selected item as arrows move.
  - Scroll position is visible in the title, e.g. `Models 40/60`.
- `crates/oino-tui/src/command.rs`
  - Removed the 50-model suggestion cap so the full filtered model list can be navigated.
  - Added regression coverage that model suggestions include all matches.

## Verification

- `cargo fmt --all`
- `cargo test -p oino-tui`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test`

## Notes

The generic command popup still stays compact at four rows. Model suggestions intentionally display five rows while remaining scrollable.
