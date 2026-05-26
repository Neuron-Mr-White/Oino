# TUI Development

The TUI lives in `crates/oino-tui` and is designed as a deterministic state machine.

## Flow

```text
terminal event -> TuiState update -> TuiAction -> app runtime -> render current state
```

Render functions should not perform slow work. Expensive filtering, indexing, IO, or network work should happen before render and be stored in state.

## Main areas

- `app.rs` — state transitions and user actions.
- `render.rs` — Ratatui layout and drawing.
- `command.rs` — slash parsing and suggestions.
- `settings.rs` — settings pages and model/theme selection state.
- `keymap.rs` — actions, presets, labels, and shortcuts.
- `theme.rs` — theme documents and resolved styles.
- `fuzzy.rs` — shared fuzzy search helpers.

## Rules of thumb

- Keep focus and modal state explicit.
- Make tiny terminals degrade gracefully.
- Cache filtered lists; do not rescore large lists in render.
- Use width-aware truncation for rows.
- Add tests for state transitions and command parsing.
