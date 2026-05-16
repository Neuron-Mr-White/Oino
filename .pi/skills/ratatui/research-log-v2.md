# Ratatui skill research log v2

Second pass goal: rebuild the monolithic Ratatui skill into progressive-disclosure subskill files organized by component type.

## Iteration 1 scaffold
- Snapshotted previous monolithic skill into `_archive/monolithic-v1/`.
- Rewrote main `SKILL.md` as a concise router with an explicit subskill selection table.
- Created initial reference subskill files under `references/` for architecture, layouts, tables/grids, input/focus, streaming/async, inspectors/explorers, chat/agent, document viewers, games/boards, telemetry dashboards, theming/polish, and testing/evals.
- Next iterations will re-study all source libraries and replace scaffolding notes with concrete source-derived patterns.

## Iteration 2: agent shells, telemetry dashboards, inspectors/workbenches

### DeepSeek-TUI + OpenAI Codex TUI
Re-read source areas:
- DeepSeek: `crates/tui/src/tui/ui.rs`, `event_broker.rs`, `composer_ui.rs`, surrounding widgets/routes listed in imports (`footer_ui`, `tool_routing`, `pending_input_preview`, `paste_burst`).
- Codex: `tui.rs`, `tui/frame_requester.rs`, `app_event.rs`, `app.rs`, `bottom_pane/chat_composer/draft_state.rs`.

Patterns extracted into `architecture.md`, `chat-agent.md`, `input-focus.md`, `streaming-async.md`:
- Agent shells need a central terminal wrapper with bracketed paste, focus change events, mouse, keyboard enhancement detection, cursor/alt-screen lifecycle, and child-process terminal restoration.
- Codex `FrameRequester` is the reusable redraw pattern: cloneable request handles send deadlines to a scheduler actor, scheduler clamps to max FPS and coalesces many requests into a single broadcast draw notification.
- Event streams should be pausable through an `EventBroker` before external editors/subprocesses to avoid stdin conflicts; after child exits, restore terminal modes, flush pending input, re-enter alt-screen, resume events.
- Transcript should be source-backed; Codex has resize reflow logic so wrapped scrollback can be rebuilt from source on resize instead of reusing old wrapped rows.
- Composer state should be isolated: textarea + textarea state + mode flags + pending pastes + input-enabled placeholder + paste burst + mention bindings.
- Escape action is contextual: close slash menu, cancel request, discard queued draft, clear input, then noop.
- Paste handling needs CR→LF normalization, bracketed paste, and burst handling before feeding multiline text to composer.
- Streaming chunks/tools should update transcript state and request/coalesce frames, not directly draw.

### bandwhich + oxker + trippy
Re-read source areas:
- bandwhich: `display/components/layout.rs`.
- oxker: `ui/mod.rs`, `ui/gui_state.rs`.
- trippy: `frontend/render/app.rs`.

Patterns extracted into `telemetry-dashboards.md`, `layouts.md`, `streaming-async.md`:
- bandwhich progressive layout: reserve header/body/footer; for 2 or 3 tables, drop panes under width/height breakpoints and let user cycle which table appears in limited slots.
- oxker splits data (`AppData`) from GUI state (`GuiState`) and draws from a `FrameData` snapshot to reduce lock churn inside draw blocks.
- oxker redraw loop combines explicit atomic rerender requests with timed fallback; clear requests trigger terminal clear and redraw.
- oxker `GuiState` stores selected panel, status set, region maps for panels/headers/buttons, log height, inspect offsets, loading spinner set, and info text; region maps are cleared on resize.
- oxker layouts hide optional areas based on state/data: filters/search bar, logs, commands sidebar, charts, ports. Ports width is measured from longest port fields.
- trippy top-level layout changes by mode: header always; optional tabs or flows; body; footer charts; info bar. Body switches between splash/error/table/chart/map and help/settings overlays sit on top.

### binsider + openapi-tui + rainfrog + yozefu
Re-read source areas:
- binsider: `tui/state.rs`, `tui/command.rs`.
- openapi-tui: `pages/home.rs`, `panes/response_viewer.rs`.
- rainfrog: `components/scroll_table.rs`, `components/editor.rs`.
- yozefu: `component/root_component.rs`, `component/records_component.rs`.

Patterns extracted into `inspectors-explorers.md`, `tables-grids.md`, `input-focus.md`, `document-viewers.md`, `layouts.md`:
- binsider keeps command conversion explicit and state mutation centralized in `State::run_command`, including special command routing for embedded hexdump widget focus.
- openapi-tui home is a pane-owned page: APIs/tags/address/request/response panes, focused pane index, fullscreen toggle, footer command mode, and propagation-style key responses.
- openapi-tui response viewer has explicit modes (`Normal`, `Search`, `Jq`), content-type tabs, syntax-highlight cache keyed by body, dim line numbers, and jq/search error display as content.
- rainfrog `ScrollTable` stores requested width, column widths, column offsets, x/y offsets, max offsets, page height, and selection mode; it renders/copies a horizontal viewport and draws scrollbars only when needed.
- rainfrog editor uses `tui-textarea`, a Vim state machine, SQL keyword regex highlighting, focus-dependent border/cursor style, paste insertion, and Alt/Ctrl-Enter query execution.
- yozefu root uses component registry + view stack + focus history/focus order; root merges component shortcuts into footer and root `Esc` warns instead of quitting.
- yozefu records component drains up to 500 records per draw into a bounded buffer, tracks follow mode, read/matched metrics, timestamp format, column size, copy/export/open/detail actions, and disables follow on manual navigation.

Updated subskill/reference files this iteration:
- `architecture.md`
- `chat-agent.md`
- `input-focus.md`
- `streaming-async.md`
- `telemetry-dashboards.md`
- `layouts.md`
- `inspectors-explorers.md`
- `tables-grids.md`
- `document-viewers.md`

## Iteration 3: dense grids, productivity navigation, games/boards

### csvlens
Re-read source areas: `src/view.rs`, `src/ui.rs`, `src/app.rs`, `src/input.rs`.

Patterns extracted into `tables-grids.md` and `input-focus.md`:
- Dense grids should model row and column selection independently via `SelectionDimension { index, bound, last_selected }` and cycle selection type Row → Column → Cell while preserving last row/column.
- Frozen columns are a first-class offset object (`num_freeze`, `num_skip`) with helpers to decide if a filtered column should render, is visible, and what skip offset is needed to bring a found column into view.
- Column widths start from effective header names (including sort indicators), scan visible row content, respect user overrides by original column index, cap un-overridden columns to a frame-width fraction, then greedily redistribute unused width to clipped columns.
- Row-height calculation for wrapping exits early after available height is filled, because wrapping all rows is too expensive for large CSVs.
- Rendering is custom buffer work: row numbers, header top/bottom borders, row-number separator, bottom status separator, freeze separator, match highlighting, marked-row style, selected row/column/cell style.
- Input model uses buffering modes for find, filter, column filter, option, freeze columns, and goto line. It normalizes Shift modifiers cross-platform before matching.

### dua-cli, eilmeldung, fzf-make, gitui
Re-read source areas:
- dua-cli: `interactive/app/eventloop.rs`, `interactive/app/state.rs`, `interactive/widgets/main.rs`.
- eilmeldung: `ui/mod.rs`.
- fzf-make: `usecase/tui/app.rs`, `usecase/tui/ui.rs`.
- gitui: `app.rs`, `components/mod.rs`.

Patterns extracted into `layouts.md`, `streaming-async.md`, `inspectors-explorers.md`, `architecture.md`:
- dua-cli selects across terminal events and `BackgroundTraversal` events, integrates traversal updates, recomputes sizes when finished, restores previous selection by name/index, and redraws while scanning.
- dua-cli layout has header/content/footer, optional right-side help/mark panes, optional bottom glob pane with cursor, and focus-specific border style. It keeps normal and glob navigation separate.
- eilmeldung uses central `Message` bus and broadcasts every command/event/batch to components. Batch startup commands wait while async operations run and while the receiver has pending messages.
- eilmeldung stores panel areas for mouse; clicks focus panels and select feed/article rows; scrolls route to panel; drag on horizontal border updates article/content split height with minimum row clamps.
- fzf-make models impossible states with `AppState::{SelectCommand, ExecuteCommand, ShouldQuit}` and TEA flow (`handle_key_input -> Message -> update -> ui`). It hides preview below height threshold, centers preview around selected command source line, skips syntax-highlighting danger cases, supports additional-args popup, prints command transcript after TUI shutdown, then executes outside alt-screen.
- gitui uses explicit component composition: `event_pump` forwards events until one component consumes; `command_pump` collects command bar entries until a visible component blocks; popups are components with visibility; async git notifications update all tabs/popups and queue command refreshes; external editor pauses input polling and resumes with redraw.

### chess-tui and minesweep-rs
Re-read source areas: chess `ui/game_ui.rs`, `event.rs`, `app/input.rs`; minesweep-rs `ui.rs`.

Patterns extracted into `games-boards.md`:
- Board games need game logic independent from UI; UI state owns cursor, selected square/cell, promotion cursor, overlays, board flip, and visual settings.
- chess-tui layout uses ratio splits for board, rank labels, file labels, clocks, captured material, move history, and overlays. Active clock is highlighted only under text width.
- chess input handles promotion before normal move processing, gates online/multiplayer by turn/color, validates puzzle moves, flips board only in appropriate modes, and opens game-end overlays immediately when state changes.
- minesweep-rs computes exact grid width/height from cell dimensions, centers it, precomputes fixed row/column constraints, uses `Cell` view objects over `App`, aligns help text to board width, and clears/render win/loss overlay over board.

Updated subskill/reference files this iteration:
- `tables-grids.md`
- `input-focus.md`
- `layouts.md`
- `streaming-async.md`
- `inspectors-explorers.md`
- `architecture.md`
- `games-boards.md`

## Iteration 4: theming, markdown documents, chat bubbles

### material crate
Re-read source areas: `material-0.1.1/src/color_data.rs`, `src/ui.rs`, `src/app.rs`, README Ratatui integration.

Patterns extracted into `theming-polish.md`:
- Palette UIs can be modeled as color families × variants with stable short codes (`A0` etc.).
- Center swatch grid by deriving `sq_width` and `sq_height` from frame size and number of colors/variants; compute left/top offsets to center the full grid.
- Contrast-aware foreground can be chosen by shade index: dark text for light variants and white text for middle/dark variants.
- Palette picker footer includes instruction, current two-character input, and copy feedback; once two chars are entered, reset input and copy matching color to clipboard.
- Library colors should convert into `ratatui::style::Color` so apps can use semantic material constants directly in `Style`.

### mdfried / mdfrier / ratskin
Re-read source areas: `mdfried/src/model.rs`, `src/view.rs`, `src/worker.rs`, `mdfrier/src/ratatui.rs`.

Patterns extracted into `document-viewers.md`, `theming-polish.md`, and `streaming-async.md`:
- Full markdown reader uses a source-backed document section model (`Lines`, `Image`, `ImagePlaceholder`, `Header`, `HeaderPlaceholder`) with section heights and line extras for links/search matches.
- Model tracks `document_id` and ignores stale parse/image events when resize/reload starts a new parse.
- Parse/reparse sends command with document id, inner width, text, and optional image cache; existing terminal image protocols are preserved across reparses to avoid flicker.
- Worker thread owns parser and async runtime, emits `NewDocument`, incremental `Parsed(section)`, `ParseDone(last_section_id)`, then async image/header hydration events.
- Render skips sections above viewport, stops before status line, overlays selected links/search matches over already-rendered lines, and uses status-line input for links/search/movement counts/cursor commands.
- mdfrier’s Ratatui theme trait separates semantic markdown surfaces (blockquote depth colors, link fg/bg/wrappers, code fg/bg, emphasis/strong/strike, table borders/header, horizontal rule) from symbol mapping.

### oatmeal
Re-read source areas: `src/domain/services/bubble.rs`, `bubble_list.rs`, `code_blocks.rs`, `src/application/ui.rs`.

Patterns extracted into `chat-agent.md`, `architecture.md`, and `theming-polish.md`:
- Chat bubble geometry computes max content width from message text and author name, clamps by terminal width minus border/padding, and aligns user messages right while assistant/system messages align left.
- Bubble top border embeds author name; bubble border color changes by author/error state.
- Bubble list caches rendered lines by message index; width changes clear cache, and the last message re-renders when text length changes so streaming output updates while stable past messages stay cached.
- Code blocks are detected from fenced blocks, syntax highlighted with syntect, numbered at fence start, and exposed through a service that can return latest/specific/range/list blocks for slash commands.
- UI checks minimum viable line width before rendering rich bubbles; if too small, it renders a simple “make me bigger” message.
- App shell uses message list + scrollbar + growing textarea; backend-wait state replaces composer with loading widget and gates input; Ctrl-C aborts backend or requires double-press to quit; pasted text normalizes CR to LF.

Updated subskill/reference files this iteration:
- `theming-polish.md`
- `document-viewers.md`
- `chat-agent.md`
- `streaming-async.md`
- `architecture.md`

## Finalization: cross-links and eval assets

- Added `## Related references` sections to all 12 reference subskill files so Claude can load adjacent concerns selectively without bloating the main router.
- Replaced `references/testing-evals.md` scaffold with concrete testing guidance: state/key tests, `TestBackend` render tests, snapshot/cell assertions, source-derived test targets, Pi-runtime review plan, and grading rubric.
- Created `evals/evals.json` with 7 practical eval prompts and objective assertions:
  - live Kafka/log browser
  - SQL/data-grid horizontal scroll
  - streaming agent chat
  - network telemetry dashboard
  - markdown/schema viewer
  - fixed-grid game
  - Ratatui audit
- Created `evals/manual-review.md` as a human review artifact because Skill Creator `eval-viewer/generate_review.py` tooling was not found in this Pi runtime.
- Validated eval JSON with `python3 -m json.tool` and scanned reference files to remove placeholder notes.

## Location correction

- Moved the completed Ratatui skill from Claude-style project path `.pi/skills/ratatui/` to Pi's project skill folder `.pi/skills/ratatui/`.
- Updated `.unipi/ralph/ratatui-component-subskills.md` and `evals/manual-review.md` references to the new Pi skill path.
- Per Pi docs, `.pi/skills/` is a project-level skill location and directories containing `SKILL.md` are discovered recursively.
