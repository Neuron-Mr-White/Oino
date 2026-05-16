# Inspectors and explorers subskill

Use for API explorers, DB workbenches, Git clients, binary inspectors, Kafka browsers, disk/tree browsers, and multi-pane inspection tools.

## Source inspirations
binsider, openapi-tui, rainfrog, gitui, yozefu, dua-cli, eilmeldung.

## Related references

- `layouts.md`
- `tables-grids.md`
- `input-focus.md`
- `document-viewers.md`
- `streaming-async.md`

## Common architecture

- Primary navigation pane: list/tree/table of resources.
- Detail/preview pane: rendered from selected item.
- Search/filter/command input.
- Footer/status line with contextual shortcuts.
- Optional stack views: details, schema, help, settings, history, confirmation.

Components emit actions; app/root handles async effects and cross-component navigation.

## Binary inspector (binsider)

- Use semantic `Command` enum for tabs, lists, blocks, hexdump, and input mode.
- Store per-tab scroll offsets and selected block/list indices.
- Delegate embedded hexdump widget keys while it is focused; map app keys to hexdump control events only where needed.
- Lazy-trigger expensive dynamic analysis/system call tracing via events (`Trace`, `Restart`) rather than render.

## API explorer/request builder (openapi-tui)

- Home page owns panes: APIs, tags, address, request, response.
- Focused pane index controls border style and which pane receives actions.
- Normal mode keys move pane focus, rows, tabs, variants; `/` and `:` focus footer command input.
- Full-screen focused pane is a first-class toggle.
- Request page builds a request by folding independent panes implementing request-builder traits (path/query/header/body/auth/accept headers).
- Response viewer has modes: normal formatted body, search results, jq transformed output. Cache syntax-highlighted lines by body string.
- Schema viewer should build pure render-blocks for `$ref`, `allOf`, `oneOf`, `anyOf` variants with stable node IDs.

## Database workbench (rainfrog)

- Focus enum includes menu, editor, data, history, favorites, popup; keep last focused tab/component for restoration.
- Editor uses `tui-textarea` plus Vim mode state, SQL keyword highlighting, focus-dependent border/cursor style, and Alt/Ctrl-Enter execution.
- Query execution is explicit: parse/bypass, confirm transaction/destructive statements, set loading, poll async DB result, then hydrate data or popup state.
- Data results have explicit states: blank, loading, no rows, rows affected, statement completed, explain text, table, error, cancelled.

## Kafka/event browser (yozefu)

- Use component registry + view stack (`TopicsAndRecords`, `Records`, `RecordDetails`, `Schemas`, `Help`).
- Topic picker combines filter input, multi-select `[x]` rows, selected count in title, and async topic loading.
- Records view drains a channel into a bounded ring buffer, tracks read/matched metrics, supports follow/unfollow, copy/export/open, timestamp format toggle, and detail view.
- Record detail precomputes highlighted `Line`s when selected record changes; schema views load asynchronously.

## File/tree and Git/productivity explorers

- Preserve selected item by stable ID/path across refresh.
- Show loading/empty/error states in each pane.
- For destructive actions, use confirmations with clear copy and blocked background shortcuts.
- For external editor/subprocess actions, release/reinitialize terminal.

## Testing checks

- Selection persists across refresh/reload.
- Detail pane updates exactly when selection changes.
- Footer shortcuts match focused pane.
- Async stale results do not hydrate a newer request/query.
- Copy/export/open actions emit notifications/errors.


## Navigation/productivity apps

### Disk tree browser (dua-cli)

- Keep separate `Navigation` objects for normal navigation and temporary glob navigation; route `navigation()` to glob when active.
- Preserve selection during background scan by path/name plus old index.
- `pending_exit` in footer is useful when quit/delete needs confirmation.
- Optional panes (`Help`, `Mark`, `Glob`) should be focusable and bordered, not hidden state inside the main list.

### RSS/productivity reader (eilmeldung)

- Use a central async message bus: every component receives `Message::Command/Event/Batch` and decides whether to react.
- Batch processing should wait while async operations are running and while the receiver still has messages, so scripted startup commands do not race live sync.
- Store panel areas every draw; mouse clicks focus panels, clicks on list rows select rows, scroll routes to panel-specific scroll events.
- Drag-resizable splits should store an explicit height override and clamp to minimum rows on both sides.
- Distraction-free/zen modes should be normal app states, not ad-hoc hidden booleans.

### Fuzzy command picker (fzf-make)

- Model states as an enum to make impossible states impossible: `SelectCommand`, `ExecuteCommand`, `ShouldQuit`.
- Use TEA shape: `handle_key_input -> Message`, then `update(model, message)`, then `ui(frame, model)`.
- Keep preview hidden under a height threshold; below that, give all space to command/history lists.
- Preview should center selected source line and guard syntax highlighter pathologies (e.g. skip problematic Makefile constructs that cause catastrophic regex backtracking).
- After TUI exits, print/show the selected command transcript and execute it outside the alternate screen.
- Additional argument entry is a popup state; while open, it captures keys and main pane ignores Tab/selection.

### Git client / large component app (gitui)

- Use explicit component composition: parents forward `draw`, `event`, and `commands` to known child components. This is verbose but testable.
- Use `event_pump` to pass events to components until one consumes it.
- Use `command_pump` to collect command bar entries until a visible component blocks command propagation.
- Keep popups as components with visibility; `any_popup_visible` prevents root quit, and fullscreen popups suppress normal tab drawing.
- Async git notifications update all relevant tabs/popups, then process a queue to refresh command hints.
- External editor flow should pause input polling, launch editor, show error popup on failure, then require redraw and resume polling.
