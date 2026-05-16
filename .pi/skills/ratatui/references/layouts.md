# Ratatui layout subskill

Use for shells, panes, popups, view stacks, splitters, full-screen panes, tiny terminal fallbacks, and responsive behavior.

## Source inspirations
bandwhich progressive split, openapi-tui five-pane home, rainfrog left/right workbench, oxker dashboard, trippy mode-dependent shell, yozefu view stack, minesweep fixed board.

## Related references

- `architecture.md`
- `input-focus.md`
- `telemetry-dashboards.md`
- `inspectors-explorers.md`
- `games-boards.md`

## App shell

Start with a shell and then specialize:

```text
header/status       0-4 lines
body                flexible
input/search        0-3+ lines
footer/help         1-2 lines
```

Compute from `Frame::area()` every frame. Do not cache layout rects except for mouse hit-testing, and clear those maps on resize.

## Pane allocation recipes

- **List + detail**: left fixed/min list, right flexible detail. Hide detail first when narrow.
- **Workbench**: left menu 20-30%, right editor/data stack; expand footer hints to two lines only above wide threshold.
- **API explorer**: left API/tag panes, right address/request/response panes; support focused-pane full-screen toggle.
- **Telemetry**: header + optional tabs/flows + body + charts + info bar; body switches by mode.
- **Tiny app**: if below minimum viable size, render one centered symbol/message and skip normal widgets.

## View stack pattern

Maintain:

```rust
views: Vec<ComponentName>,
focus_order: Vec<ComponentName>,
focus_history: Vec<ComponentName>,
```

- `Esc` pops top view; root `Esc` can show a quit hint instead of exiting.
- Opening a view pushes it after deduplicating existing copies.
- On close, restore last focused component if it exists in new focus order; otherwise use first focusable.
- Footer should know stack depth so it can show `Esc Close` only when relevant.

## Modal and popup overlays

- Render parent first, then `Clear` over centered popup rect.
- Popup gets first event priority and returns `Stop(action)` or `Continue(action)` if using propagation control.
- Give destructive confirmations explicit buttons/shortcuts and block background actions.

## Resizable/draggable splits

- Store split ratios/percentages in state, not rects.
- On mouse drag, map position to a clamped ratio and request redraw.
- Persist user-adjusted ratios if the app has config.

## Progressive degradation

When size shrinks:
1. Hide decorative/help text.
2. Hide previews/details/charts.
3. Collapse sidebars into full-screen cycling panes.
4. Reduce table columns.
5. Show tiny fallback.

## Testing checks

Snapshot at wide, medium, narrow, very short, and tiny sizes. Verify focus never points to a hidden pane and footer hints do not overflow.


## File/tree browser layout (dua-cli)

For disk/file/tree explorers:

- Use header/content/footer shell. Header shows context/path; footer shows total bytes, entries traversed, elapsed scan time, sort, message, pending-exit hint.
- Optional right panes (`Help`, `Mark`) split the content horizontally; if both are open, split the right side vertically.
- Optional bottom input pane (`Glob`) consumes three lines under the entries list and owns cursor placement.
- Border focus should be obvious: focused pane bold, unfocused panes dark gray.
- Background scans should keep the entries pane live while updating counts and preserving selection.
