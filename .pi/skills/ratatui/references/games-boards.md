# Games and fixed boards subskill

Use for chess, minesweeper, fixed-size grids, mouse-to-cell mapping, animations, board games, and terminal game overlays.

## Source inspirations
chess-tui (`ui/game_ui.rs`, `event.rs`, `app/input.rs`) and minesweep-rs (`ui.rs`).

## Related references

- `layouts.md`
- `input-focus.md`
- `theming-polish.md`
- `testing-evals.md`

## Architecture

- Keep rules/game state independent from UI state. Board legality, mines, clocks, puzzle validation, and win/loss should not depend on Ratatui.
- UI state owns cursor coordinates, selected square/cell, promotion cursor, overlays/popups, board flip, and visual settings.
- Use a tick-capable event handler for animations/clocks; otherwise render on input/state changes.
- A threaded poller with `Tick`, `Key`, `Mouse`, `Resize` is enough for local games. Include non-blocking `try_next()` if animations need to drain input without blocking.

## Fixed board layout

Two good approaches:

### Ratio board + sidebar (chess-tui)

- Split main area vertically for top padding, board region, bottom padding.
- Split board region horizontally into left padding, rank labels, board, right padding, sidebar.
- Board area can include clock row above and file labels below.
- Sidebar can stack captured material, move history, hints, or status.

### Centered exact-size grid (minesweep-rs)

- Compute exact `grid_width = cell_width * columns + padding*2` and `grid_height = cell_height * rows + padding*2`.
- Center it with horizontal and vertical padding constraints.
- Precompute row constraints and column constraints as `Length(cell_height)` / `Length(cell_width)`.
- Render help text and gauges aligned to the same grid width so the board feels anchored.

## Cell view objects

Create lightweight view objects over state:

```rust
struct Cell<'a> { app: &'a App, row: usize, column: usize }
```

Expose methods like `is_active`, `is_exposed`, `is_flagged`, `is_mine`, `block(lost)`, and `text_style()`. This keeps render loops declarative and avoids duplicating style logic.

## Rendering layers

1. Outer game shell/border/title.
2. Board container block.
3. Coordinates/labels/clocks/sidebar.
4. Cells/pieces/flags/mines.
5. Cursor/selection/legal moves/highlights.
6. Modal overlays: promotion, move input, end screen, help, error.

Use `Clear` before overlays so the board does not bleed through. End-state overlays should appear immediately when state changes; chess-tui opens end screen during render if checkmate/draw state is present.

## Input

- Provide arrows and `hjkl` for cursor movement.
- Space/Enter selects or exposes active cell.
- `f` flags mines or toggles game-specific marker.
- In chess-like games, `process_cell_click` should handle promotion state first, then turn ownership, legal move validation, puzzle validation, and game-end checks.
- Multiplayer/online modes must gate input by player color/turn, but promotion selection may need to stay interactive after move submission.

## Mouse mapping

- Store/render board rect.
- Convert mouse `(x,y)` into row/column only if inside board rect.
- Apply board flip before translating displayed coordinates to logical squares.
- If a piece/cell is already selected, a mouse click on a legal target should execute the move; otherwise it selects.

## Styling

- Active cell should alter both border and content style.
- Revealed/hidden/flagged/mine/lost states need distinct foreground/background.
- For chess clocks, highlight only the text-width area of the active player clock so the layout remains compact.
- Align help strings by delimiter (`movement:`, `flag:`) and center them to board width.

## Testing checks

- Cursor movement clamps to board boundaries.
- Mouse-to-cell mapping works with board flip and padding.
- Promotion and end overlays block normal board input.
- Fixed grid remains centered at odd/even terminal sizes.
- Win/loss exposes/flags board and displays overlay.
