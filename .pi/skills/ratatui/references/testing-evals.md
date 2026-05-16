# Testing and evals subskill

Use for Ratatui render tests, snapshot tests, key-routing tests, terminal lifecycle checks, and evaluating whether this skill improves Claude Code output.

## Related references

- `architecture.md`
- `layouts.md`
- `tables-grids.md`
- `input-focus.md`
- `streaming-async.md`
- `theming-polish.md`

## Implementation test checklist

For every non-trivial Ratatui feature, add tests at two levels:

1. **State/key tests**: feed `KeyEvent`, `MouseEvent`, `Action`, or `Command` into reducers/components and assert state changes.
2. **Render tests**: render with `ratatui::backend::TestBackend` at multiple sizes and assert snapshots or specific cells.

Minimum useful cases:
- wide, medium, narrow, very short, and tiny-terminal sizes;
- empty/loading/error/success states;
- focused and unfocused selected rows;
- modal open with background content blocked;
- text input focused vs not focused;
- resize after content exists;
- terminal cleanup path if abstracted.

## Snapshot and cell assertion pattern

Use snapshots for layout regressions, then add cell-level assertions for fragile UX details:

```rust
let backend = TestBackend::new(80, 24);
let mut terminal = Terminal::new(backend)?;
terminal.draw(|f| render(&mut app, f))?;
let buffer = terminal.backend().buffer();
assert_eq!(buffer.get(0, 0).symbol(), "╭");
```

Assert these details explicitly when they matter:
- focused border color/type;
- selected row foreground/background;
- footer shortcut text;
- cursor position in input mode;
- scrollbar presence;
- tiny fallback message;
- modal overlay `Clear` area.

## Source-derived test targets

- **Architecture**: terminal guard restores raw mode/alt screen; event loop maps crossterm events to app events; frame requester coalesces repeated draw requests.
- **Input/focus**: modal consumes keys before background; focus cycling skips hidden panes; command footer returns structured result; paste normalizes CR/LF.
- **Tables/grids**: Unicode width truncation; frozen columns remain visible; horizontal scroll offsets map to correct columns; selection survives refresh; row/column/cell copy modes.
- **Streaming**: stale worker result ignored by id; cancellation stops old task; ring buffer selection remains valid; follow mode disables on manual navigation.
- **Inspectors**: selected resource detail updates once; async detail/schema result hydrates current view only; copy/export/open show notifications.
- **Chat**: streaming last message re-renders but cached old bubbles stay stable; Escape priority; code block numbering and extraction; tiny-width guard.
- **Documents**: section skipping at scroll offset; stale parse/image events ignored; link/search overlays position correctly; status-line input queue.
- **Dashboards**: breakpoints hide/cycle panes correctly; region maps clear on resize; freeze/pause snapshot holds while navigation works.
- **Games**: cursor clamps, mouse-to-cell works with board flip, overlay blocks board input, end-state overlay appears.
- **Theming**: selected-focused vs selected-unfocused contrast; low-color labels; palette foreground readable.

## Skill eval prompts

The practical eval set lives in `../evals/evals.json`. Use it to test whether agents reading this skill produce better plans/code than baseline.

Expected eval coverage:
1. Live Kafka/log browser with search/follow/detail views.
2. SQL/CSV data grid with frozen columns and horizontal scrolling.
3. Streaming agent chat with composer, bubbles, paste, code blocks, tool events.
4. Traceroute/system telemetry dashboard with charts, freeze, settings/help.
5. Markdown/schema viewer with async parsing, images/links/search.
6. Terminal game board with fixed grid, mouse mapping, overlays.
7. Review/audit task finding focus, resize, and terminal lifecycle bugs.

## Pi-runtime review plan

The Skill Creator `eval-viewer/generate_review.py` tooling was not found in this Pi environment. Until Claude Code skill-eval tooling is available, use the manual artifact at `../evals/manual-review.md`:

1. For each eval prompt, run an agent with this skill loaded.
2. Save the output/patch under `ratatui-eval-workspace/iteration-N/<eval-id>/with_skill/`.
3. Optionally run a baseline without reading `.pi/skills/ratatui/` and save `without_skill/`.
4. Grade against the assertions in `evals.json`.
5. Ask the human reviewer to inspect `manual-review.md` and record notes.

## Grading rubric

A response passes only if it is concrete enough to implement:
- names the reference files/subskills it used;
- proposes explicit state, event/action, layout, focus, async, and test structure;
- handles narrow terminals and terminal cleanup;
- includes source-derived UX details relevant to the prompt;
- avoids blocking work in render;
- adds or describes meaningful tests.
