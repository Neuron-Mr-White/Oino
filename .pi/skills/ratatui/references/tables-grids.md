# Tables and grids subskill

Use for tables, CSVs, DB result grids, Kafka/log records, file lists, process/network tables, and horizontal scrolling.

## Source inspirations
bandwhich, binsider, csvlens, rainfrog `ScrollTable`, yozefu records, gitui, dua-cli.

## Related references

- `input-focus.md`
- `inspectors-explorers.md`
- `streaming-async.md`
- `testing-evals.md`

## Basic table rules

- Use `TableState`/`ListState` for vertical selection.
- Track selected row by stable ID when data refreshes; index alone is fragile.
- Use `unicode-width` for display width; byte/string length is wrong for CJK/emoji/box drawing.
- Prefer one-line rows in dense tables. Wrapping every visible cell is expensive and hard to scan.
- Show counts: selected index/total, filtered total, read/matched, or sort column.

## Width allocation

- Fixed columns: timestamps, ports, offsets, status icons.
- Variable columns: names, paths, values, hostnames.
- After fixed widths, divide leftover among variable columns; cap columns with maximum useful width.
- For paths/topics, use middle truncation so both prefix and basename/suffix remain visible.
- Let users resize important columns with Left/Right or config.

## Horizontal scrolling / offscreen table

For DB/spreadsheet results, normal `Table` is not enough. Use rainfrog’s `ScrollTable` pattern:

1. Store `column_widths`, cumulative `column_offsets`, `x_offset`, `y_offset`, max offsets, page height, and selection mode.
2. Render table into an offscreen `Buffer` at requested full width.
3. Copy a viewport slice starting at `x_offset` into the real area.
4. Draw horizontal and vertical scrollbars only when max offsets are nonzero.
5. Add column-jump helpers: next/prev/first/last column based on offset boundaries.

## Selection modes

Use explicit modes for copy UX:

```rust
enum SelectionMode { Row, Cell, Copied }
```

- Row mode highlights whole row and copies row.
- Cell mode highlights active cell and copies cell.
- Copied mode temporarily changes title/style to acknowledge copy.
- Ask confirmation before copying/exporting all rows.

## Live record/log tables

- Use ring buffers with platform-safe capacity for unbounded streams.
- Drain a bounded number of records per frame.
- Manual navigation disables follow mode; follow mode selects newest row on arrival.
- Sort only when new records arrived and only over bounded buffer.
- Render only rows near viewport/selection if table size is large.

## Testing checks

- Wide Unicode truncation.
- Horizontal scroll viewport and scrollbars.
- Selection survives data refresh/overflow.
- Empty/loading/error table states.
- Copy mode transitions and clipboard/export actions.


## CSV/dataframe viewer pattern (csvlens)

Use this when building CSV/Parquet/log viewers with tens of thousands of rows or many columns.

### View state

Model row and column selection independently:

```rust
enum SelectionType { Row, Column, Cell, None }
struct Selection { row: SelectionDimension, column: SelectionDimension }
struct SelectionDimension { index: Option<u64>, bound: u64, last_selected: Option<u64> }
```

Keep `last_selected` so toggling Row → Column → Cell restores the previous row/column instead of jumping to zero.

### Frozen columns and horizontal offsets

Use a column offset object:

```rust
struct ColumnsOffset { num_freeze: u64, num_skip: u64 }
```

Rules:
- columns `< num_freeze` are always visible;
- non-frozen columns start at `num_freeze + num_skip`;
- when jumping to a found column, compute `num_skip` needed to make it visible;
- draw a distinct freeze separator so users understand why left columns do not scroll.

### Column widths

- Start widths from header/sort indicator text.
- Scan visible rows, splitting multiline cells, unless a user override exists for that origin column.
- Cap un-overridden columns to a fraction of frame width (csvlens uses ~30%).
- Redistribute unused space back to clipped columns from narrowest to widest so wide terminals are used without one column consuming everything.
- Keep width overrides keyed by original column index, not filtered/visible index.

### Row heights and wrapping

- Default dense mode: each row height = 1.
- If wrapping is enabled, compute row heights only until visible area is filled; after that, use cheap placeholder height. This avoids pathological wrapping cost on huge CSVs.
- Support `WrapMode::{Disabled, Chars, Words}` and show transient messages when toggled.

### Search/filter/sort/mark UX

- Use input-buffer modes for find/filter/filter-columns/goto-line/options/freeze-columns.
- Highlight regex matches in cells and use a different background for the active match.
- `FindLikeCell` and `FilterLikeCell` should seed query from the selected cell.
- Mark rows separately from selection; marked style should not override selected style.
- Sort indicator belongs in the header (`name [▴]`, `name [▾N]` for natural sort).

### Borders and status

- Hand-render row numbers, header separators, bottom status separator, column end separator, and freeze separator when using a custom buffer renderer.
- Keep status footer separated from the table; include mode, file, row/column, filter/find state, and transient messages.
