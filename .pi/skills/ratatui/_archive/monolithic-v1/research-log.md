# Ratatui Skill Research Log

This log records patterns extracted from reference projects. Keep `SKILL.md` standalone and copy any reusable pattern there.

## Iteration 1 — scaffold + first 3 projects

### DeepSeek-TUI (`Hmbown/DeepSeek-TUI`)

Files inspected:
- `crates/tui/src/tui/ui.rs`
- `crates/tui/src/tui/frame_rate_limiter.rs`
- `crates/tui/src/tui/views/mod.rs`
- `crates/tui/src/tui/pager.rs` via search results

Patterns:
- Agent/chat shells need robust terminal setup: recover terminal modes, alternate screen, mouse/paste/focus/keyboard enhancement flags, color-depth detection, terminal-quirk handling, and cleanup guard.
- Render function clears full background first, then switches among onboarding vs main shell.
- Main layout: one-line header, flexible chat body, dynamic pending-input preview, dynamic composer, one-line footer.
- Composer height is computed by a widget (`desired_height(width)`) and capped so chat keeps minimum space.
- Pending queued/steered input gets visible preview above composer rather than disappearing while a turn runs.
- Body can add optional file tree (~25%) and sidebar only when width is sufficient.
- View stack renders after main UI; live transcript overlay refreshes dynamic data before render.
- Modal system uses `ModalKind`, `ViewEvent`, `ViewAction`, and `ModalView` trait; views emit typed events for host handling.
- Streaming redraws are coalesced and capped by `FrameRateLimiter` (120 FPS normal, 30 FPS low motion).
- Pager supports familiar navigation (`j/k`, arrows, PgUp/PgDn, Ctrl-d/u/f/b, space/shift-space, `g/G`, `/`, `n/N`, `c/y`).

### bandwhich (`imsnif/bandwhich`)

Files inspected:
- `src/display/ui.rs`
- `src/display/components/layout.rs`
- `src/display/components/table.rs`
- `src/main.rs`

Patterns:
- Network monitor uses separate display and terminal-event threads; display thread redraws at a fixed delta and event thread handles quit/pause/tab/resize.
- Fixed shell: header length 1, body `height - 2`, footer length 1.
- Progressive layout avoids unreadable panels: with two/three child tables, choose horizontal/vertical splits based on width/height breakpoints, or show fewer tables and let Tab cycle.
- Adaptive tables choose a `DisplayLayout` by width cutoffs, select visible columns per layout, and proportionally shrink/expand widths.
- Column spacing maxes at 2 cells.
- Text uses middle truncation with `unicode-width` so endpoints remain visible (connections, hosts, paths).
- TUI has alternate raw text output mode for non-interactive usage.

### binsider (`orhun/binsider`)

Files inspected:
- `src/tui/mod.rs`
- `src/tui/event.rs`
- `src/tui/command.rs`
- `src/tui/state.rs`
- `src/tui/ui.rs`

Patterns:
- Clean module split: `state`, `event`, `ui`, `widgets`, `command`.
- `Tui<B>` owns `Terminal<B>` and `EventHandler`; `init`, `draw`, `reset`, `exit` handle terminal lifecycle and panic reset.
- Event handler thread polls crossterm, emits `Tick`, `Key`, `Mouse`, `Resize`, and domain async events (`FileStrings`, `Trace`, `TraceResult`, `Restart`).
- Command routing maps keys/mouse into `Command` enum; state executes commands. Supports vim + arrow keys, Ctrl-d/u paging, Tab/BackTab, `/` or Ctrl-f for search, mouse wheel for list scroll.
- State tracks selected tab, info tab, block index, multiple scroll indices, `tui_input::Input`, input mode, loading flags, accent color, and logo splash.
- UI uses top bordered title with package name/version, tab strip, right-aligned file breadcrumb, bottom contextual key-hints, and focused-block border styles.
- Dense inspector tab uses multiple scrollable paragraphs/tables with `Scrollbar`, `TableState`, selected counts in bottom titles, search input in bottom title, and highlight of search results.
- Key hints are hidden if they would collide with active input.
- Embeds a hex viewer and delegates input when its search/jump windows are focused.

## Iteration 2 — chess-tui, openai/codex, csvlens

### chess-tui (`thomas-mauran/chess-tui`)

Files inspected:
- `src/ui/tui.rs`
- `src/event.rs`
- `src/main.rs`
- `src/handlers/handler.rs`
- `src/ui/game_ui.rs`
- `src/game_logic/ui.rs`

Patterns:
- Main loop draws every iteration; when animations are active it uses `try_next()` plus ~16 ms sleep, otherwise blocks on `events.next()`.
- Event handler emits Tick/Key/Mouse/Resize from a thread; `try_next` enables animation fast path.
- Key handling filters `KeyEventKind::Press` to avoid release/repeat duplicates.
- Router structure: if popup active, popup consumes keys; otherwise current page handler receives keys; chess-specific bindings shared across game modes; fallback handles global quit/skin cycle.
- Mouse events are only active on relevant game pages; popups and ended games block board clicks.
- Board UI state tracks cursor coordinate, selected square, promotion cursor, old cursor, `top_x/top_y/cell width/height`, mouse-used, blink visibility, and display mode/skin.
- Board rendering computes cell size from `area.width / 8` and `area.height / 8`, centers the board using leftover border padding, stores render geometry for mouse mapping, then renders layers: base squares, check/selected/last-move/legal-move highlights, cursor, centered piece glyph.
- Game side panel uses material boxes + move history; clocks are one-line text with active background only behind the text width.
- Good lesson: game UIs need keyboard/mouse reconciliation — after mouse selection, the next keyboard event moves the logical cursor to selected square or resets safely.

### openai/codex (`openai/codex`, `codex-rs/tui`)

Files inspected:
- `codex-rs/tui/src/tui.rs`
- `codex-rs/tui/src/tui/frame_rate_limiter.rs`
- `codex-rs/tui/src/tui/frame_requester.rs`
- `codex-rs/tui/src/tui/event_stream.rs`
- `codex-rs/tui/src/app.rs`
- `codex-rs/tui/src/bottom_pane/bottom_pane_view.rs`
- `codex-rs/tui/src/bottom_pane/chat_composer.rs`

Patterns:
- TUI init checks stdin/stdout are terminals, sets modes, flushes input buffer, installs panic hook, probes cursor position with timeout, and detects keyboard enhancement support before creating EventStream.
- `TuiEvent` normalizes `Key`, `Paste`, `Resize`, and scheduled `Draw`.
- `FrameRequester` is cloneable; background tasks/widgets call `schedule_frame`/`schedule_frame_in`; `FrameScheduler` coalesces requests and clamps to 120 FPS before broadcasting Draw.
- `EventBroker` owns the shared crossterm EventStream and can pause by dropping it, then recreate on resume. This prevents stdin stealing when external editors/subprocesses need terminal input.
- Draw path handles inline viewport areas, pending history lines above the active viewport, synchronized output, resize reflow, and cursor placement from widget state.
- App draw path calls `chat_widget.desired_height(width)`, renders into a custom frame buffer, applies cursor style/position, then optionally draws external overlays (pet images).
- Bottom-pane views implement completion semantics, child accept/cancel behavior, paste handling, request consumption, terminal-title action-required flag, and next-frame delay.
- Chat composer is a deep input state machine: slash/file/skill mention popups, attachments, remote image rows, history recall/search, paste placeholder elements, Windows/non-bracketed paste-burst detection, input-disabled mode, Enter vs Tab submit/queue, and footer hint state.
- Huge snapshot corpus for composer/footer/list/approval views demonstrates expected UX at narrow/wide sizes.

### csvlens (`YS-L/csvlens`)

Files inspected:
- `src/input.rs`
- `src/util/events.rs`
- `src/ui.rs`
- `src/app.rs`

Patterns:
- `InputHandler` converts polled crossterm/file-watch/tick events to a `Control` enum; app `step(control)` owns mutation.
- Shift modifiers are normalized cross-platform before key matching; terminals differ for uppercase letters vs symbols.
- Default mode supports rich Vim-like commands: `j/k/h/l`, `g/G`, `n/N`, `Ctrl-f/b/d/u`, page left/right, `?` help, `#/@` find/filter like selected cell, `m/M` mark/reset marks, `>/<` width, `f` freeze columns.
- Transient input buffer modes: digits start goto-line; `/` find; `&` filter rows; `*` filter columns; `-` options; `f` freeze-column number. Up/Down recall per-mode history; find and filter share history.
- Hand-rendered CSV table implements `StatefulWidget` over `Buffer`, not stock Table. It manually draws cells, line-number gutter, border intersections, status separator, and frozen-column double separator.
- Column widths come from header/cell content, cap at 30% frame width, then redistribute leftover among clipped columns while leaving space for right border.
- Wrapping row heights are computed only until available height is filled for performance.
- Status line doubles as command prompt and metadata display: filename/stdin, row/col, find/filter status, sort status, echo/ignore-case/reload/debug.
- App keeps `CsvTableState.cursor_xy`; after rendering, terminal cursor is positioned inside the status input.
- Tests use `TestBackend`, repeated `step_and_draw`, and assertions for horizontal scrolling, wrapping, filtering, marking, sorting, freezing, and copy selection.

## Iteration 3 — dua-cli, eilmeldung, fzf-make

### dua-cli (`Byron/dua-cli`)

Files inspected:
- `src/interactive/app/terminal.rs`
- `src/interactive/app/eventloop.rs`
- `src/interactive/widgets/main.rs`
- `src/interactive/widgets/entries.rs`
- `src/interactive/widgets/tui_ext.rs`
- `src/interactive/widgets/footer.rs`

Patterns:
- Terminal app owns config, background traversal, display options, `AppState`, and `MainWindow` widgets; initialization hides cursor, clears terminal, sets initial `view_root` and selected entry.
- During active filesystem traversal, event loop uses `crossbeam::select!` to process either terminal events or traversal events; traversal events update stats/tree, recompute sizes, update visible entries, and redraw.
- Selection is preserved while scan updates by remembering previous selected name + index; it matches by name first and index fallback.
- Input handling maps resize to a synthetic refresh key and ignores release events; Ctrl+C exits immediately, Tab cycles focus, `/` toggles glob pane, `?` help, q/Esc uses pending-exit confirmation.
- Focused panes: Main, Help, Mark, Glob. If search/filter is active, navigation uses alternate `glob_navigation` instead of main navigation.
- Main window layout: 1-line header, content, 1-line footer. Optional help/mark panes split right side; optional glob pane consumes a 3-line strip below entries.
- Focused border is bold; inactive borders dark gray. Header background changes if entries are marked. Footer reversed style shows scan stats and pending-exit warning.
- Entries pane uses custom list wrapper preserving scroll offset, computes metadata columns before name, uses Unicode width/segmentation to shorten names, renders scrollbar, and draws inline corner help only when title+help fits.
- `FunctionWidget` allows rendering a closure as a widget for tests/custom draw paths.

### eilmeldung (`christo-auer/eilmeldung`)

Files inspected:
- `src/main.rs`
- `src/input/mod.rs`
- `src/ui/mod.rs`
- `src/ui/view.rs`
- `src/ui/mouse.rs`

Patterns:
- App uses a central unbounded `Message` channel. A blocking input reader sends key/resize/mouse events; connectivity monitor and async operations also send messages.
- Main async loop combines render ticks, batch command processing, and message reception via `tokio::select!`. Tick frequency is configurable (`refresh_fps`).
- Message processing fans out in stable order to input-command generator, batch processor, app, feed list, article list, article content, command input, confirmations, and help popup.
- Batch processor is gated by async operations so scripted startup commands do not race long-running sync/import/logout operations.
- `InputCommandGenerator` implements configurable multi-key sequences: accumulates keys, shows prefix-match help popup while waiting, executes direct match when unique/timeout/submit, aborts or warns on unknown sequence, and uses a throbber as timeout countdown.
- Root `App` implements `Widget`, rendering status bar, optional command line, panels, and popup last.
- Layout adapts by focus: feed/article/content focused states alter panel width/height constraints; distraction-free content mode bypasses other panels.
- Focused panel is rendered last so its border overwrites shared borders.
- Panel areas are saved for mouse hit testing; clicks focus panels and send semantic click events, scroll sends semantic scroll events, and dragging the article/content border adjusts an override height clamped to minimums.
- Status bar combines tooltip, offline icon or async-operation throbber, and powerline-style separators; in distraction-free mode it hides unless tooltip is warning/error.

### fzf-make (`kyu08/fzf-make`)

Files inspected:
- `src/usecase/tui/app.rs`
- `src/usecase/tui/ui.rs`

Patterns:
- App state follows “make impossible states impossible”: `SelectCommand`, `ExecuteCommand`, `ShouldQuit`, with the large selection state boxed.
- Flow is TEA-like: `handle_key_input` maps current state/pane/popup to `Message`; `update` mutates model; `ui` renders. Tests assert message/update behavior heavily.
- Terminal lifecycle uses raw mode + alt screen + mouse capture; async body is wrapped in `catch_unwind`; shutdown restores terminal before printing/running selected command.
- Selection state tracks current pane, runners, search textarea, command list `ListState`, history `ListState`, optional additional-arguments popup, latest-version notification, and clipboard-copy result.
- Search textbox uses `tui-textarea`; typed chars/backspace reset command selection to first filtered result. History pane has separate cyclic navigation.
- Layout: bottom one-line hints; search/notification/version row; command/history panes; preview pane hidden if height < 20.
- Preview pane opens selected command source file, centers selected line in preview window, replaces tabs, prefixes line numbers, uses `syntect`/`syntect_tui`, and skips known pathological highlighting cases.
- Active pane uses thick green border and inactive uses plain dark gray; active styling is suppressed while argument popup is open.
- Popup uses centered rect + `Clear`, routes Esc/Enter specially, and routes other keys to dedicated arguments textarea.
- After selection, TUI exits and prints/shows the command before executing so the terminal transcript remains understandable.

## Iteration 4 — gitui, material, mdfried

### gitui (`gitui-org/gitui`)

Files inspected:
- `src/main.rs`
- `src/gitui.rs`
- `src/input.rs`
- `src/app.rs`
- `src/components/mod.rs`
- `src/cmdbar.rs`
- `src/ui/stateful_paragraph.rs`
- `src/ui/scrolllist.rs`

Patterns:
- Main loop selects across input, async git notifications, async app notifications, ticker/watch notifications, and spinner ticks via `crossbeam_channel::Select`.
- Spinner ticks are special-cased: update spinner and draw it without running the full app update/draw cycle.
- Input polling is a dedicated thread with desired/current polling state. It can be paused for external editor handoff, emits `InputState::Paused/Polling`, and filters non-press key events.
- App receives input events; visible components get first chance through `event_pump`. If consumed, command bar update is flagged; if not, global tab/options/cmdbar shortcuts run.
- Component architecture: `Component` trait exposes `commands`, `event`, `focused`, `focus`, `is_visible`, `hide`, `show`, `toggle_visible`; `DrawableComponent` handles draw.
- `command_pump` gathers contextual command bar entries until a component returns blocking. This keeps modal/popup commands from leaking commands underneath.
- App has an internal `Queue<InternalEvent>` so components request app-level actions: confirmations, popups, tab switches, async update flags, external editor launch, fuzzy finder updates, etc.
- Draw layout: 2-line top bar, active tab body, dynamic-height command bar. Fullscreen popups suppress tab body; popups draw over body but above command-bar area.
- Command bar computes wrapping by Unicode width, sorts commands by order, tracks expandable/multiline state, dynamically reports height, and displays `more [.]` / `less [.]` affordance.
- `StatefulParagraph` is a custom paragraph with horizontal/vertical scroll, word-wrap vs truncation composers, line count tracking, and alignment.
- Tests use `TestBackend` and synthetic input events to exercise startup/update/draw flows.

### material crate (`crates.io/crates/material` v0.1.1)

Files inspected:
- `src/lib.rs`
- `src/app.rs`
- `src/ui.rs`
- `src/main.rs`

Patterns:
- Library provides typed `HexColor` constants for Material colors and feature-gated `From<HexColor> for ratatui::style::Color` conversion.
- Example app computes uniform swatch grid from terminal size: square height from available rows / variants, square width from width / color groups, then centers grid with leftover margin.
- Each color variant block uses background color and tone-dependent black/white foreground for contrast; block title shows the short color code centered.
- Footer/input panel explains usage, echoes current two-character color code, and shows clipboard feedback.
- Minimal app still follows raw mode + alt screen + draw loop + Esc quit + Backspace edit + terminal restore.
- Useful skill takeaway: color systems should be typed/reusable, and color preview UIs should validate contrast and center their grid instead of hardcoding coordinates.

### mdfried / mdfrier / ratskin (`benjajaja/mdfried`)

Files inspected:
- `src/main.rs`
- `src/model.rs`
- `src/keybindings.rs`
- `src/view.rs`
- `src/worker.rs`
- `mdfrier/src/ratatui.rs`
- `ratskin/src/lib.rs`

Patterns:
- Markdown parsing, image loading/resizing/encoding, and header image rendering happen in a worker thread so the UI remains responsive.
- UI sends `Cmd::Parse(DocumentId, width, text, image_cache)` and receives `Event::NewDocument`, `Parsed`, `ParseDone`, `ImageLoaded`, `ImageFailed`, `HeaderLoaded`, `FileChanged`.
- `DocumentId`/reload id guards against stale events; model ignores events from previous parses after reload/resize.
- Document model is section-based with explicit heights: line sections, image placeholders, loaded images, header placeholders, rendered headers. Render scans sections from `y = -scroll` and skips invisible content.
- Resize triggers reparse with new inner width and returns `SkipRender` until worker events arrive because markdown wrapping/image sizing changes with width.
- Image protocols are cached across reparse and cached image events are sent before parse done; uncached images are processed after text sections.
- `LineExtra` metadata tracks links and search matches; renderer draws overlays on top of rendered text for selected links/search matches.
- Link selection highlights all wrapped segments that share the same URL and positions cursor on actual selected segment; hidden-URL mode shows selected URL in status line.
- Keybindings support Vim-like movement counts, `j/k/d/u/f/b/g/G`, link/search next/previous, `/` search input, Enter to open selected link, Esc to clear cursor/search/count modes, and `z/t/b` cursor positioning commands.
- Last screen line is reserved for status/input queue; document content never renders into it.
- `mdfrier::ratatui::Theme` separates markdown semantics from styling: blockquote depth colors, link styles, code style, table borders/headers, list marker prefix style, strikethrough, hide URLs.
- `ratskin` shows another markdown approach: adapt termimad output into wrapped Ratatui `Line`/`Span`s with a skin/theme layer.

## Iteration 5: minesweep-rs, oatmeal, oxker

### cpcloud/minesweep-rs
Files studied: `src/ui.rs`, `src/events.rs`.

Patterns extracted:
- Small fixed-grid game app: `App` owns `Board`, `active_row`, `active_column`; movement uses checked/saturating bounds rather than panicky index math.
- `Cell<'app>` is a view object over state with `is_active/is_exposed/is_flagged/is_mine`, `block(lost)`, `text_style()`, and `Display` impl. This keeps per-cell UX local.
- Board sizing is option-driven: `grid_width = cell_width * columns + 2 * padding`, `grid_height = cell_height * rows + 2 * padding`; row/column constraints are precomputed `Length(cell_h/cell_w)` vectors.
- Board is centered with spacer chunks: horizontal and vertical padding are computed from current terminal rect; help/info areas are aligned to board width, not full terminal width.
- Help text aligns labels around `:` before centering, making simple instructions visually tidy.
- Game overlays draw last: on win/loss, flags are shown, a centered popup rect is computed, `Clear` erases the board under it, and a thick bordered banner reports the result.
- Simple threaded event helper merges termion key input and a periodic tick into one channel; UI loop draws then waits for the next input event. Ctrl-C uses `Arc<AtomicBool>` to end the loop.

Reusable additions to skill: fixed-grid/game recipe, cell view object pattern, board-local help alignment, `Clear`ed final overlay.

### dustinblackman/oatmeal
Files studied: `src/application/ui.rs`, `src/domain/services/events.rs`, `src/domain/services/bubble.rs`, `src/domain/services/bubble_list.rs`, `src/domain/models/loading.rs`, `src/domain/services/app_state.rs`.

Patterns extracted:
- Chat shell with transcript/composer vertical split: transcript `Min(1)`, textarea `Max(textarea.lines().len() + 3)`.
- Minimum-width guard computes author-label and bubble chrome requirements; if insufficient, renders a simple fallback instead of a broken chat layout.
- `EventsService` merges backend events, crossterm `EventStream`, mouse scroll, paste, and a 500ms `UITick`; key events are normalized to app events before state mutation.
- Bracketed paste is enabled; paste text normalizes `\r` to `\n` and is inserted through textarea yank/paste APIs.
- While waiting for backend, normal typing/enter/paste is ignored, Ctrl-C aborts backend, and the composer becomes a loading box.
- Exit uses intentional friction: first Ctrl-C posts an assistant warning message; second Ctrl-C exits. Slash commands can also quit or trigger copy/codeblock/editor actions.
- Bubbles are semantic rendered lines: user right aligned, other authors left aligned, error/Oatmeal-specific styling, author name embedded into top border, outer padding percentage.
- Code fences switch syntect highlighter; code blocks are numbered inline at opening fence for later slash-command actions.
- `BubbleList` caches line renderings by message index, text length, codeblock count, and line width; clears cache on width change and recomputes the last streaming message as text grows.
- App state tracks scroll/scrollbar; when streaming and the user was at bottom, auto-follow remains at bottom, otherwise scroll position is preserved.

Reusable additions to skill: chat bubble/conversational agent recipe, width fallback, bubble cache invalidation, backend-wait input gating, bottom-follow scroll semantics.

### mrjackwills/oxker
Files studied: `src/ui/mod.rs`, `src/ui/gui_state.rs`, `src/ui/redraw.rs`, `src/input_handler/mod.rs`, `src/ui/draw_blocks/*`, `src/config/color_parser.rs`, `src/main.rs`.

Patterns extracted:
- Live operations dashboard with shared `AppData`, separate `GuiState`, and a per-frame `FrameData` snapshot to reduce lock reads in draw functions.
- Atomic `Rerender` controls redraws (`update_draw`, `swap_draw`) plus a separate clear flag. Main loop redraws only on requested draw or periodic docker interval.
- Status is a `HashSet<Status>` (`Error`, `Help`, `Filter`, `SearchLogs`, `Inspect`, `DeleteConfirm`, `Exec`, etc.) so modes can overlap; input routing checks status priority before normal shortcuts.
- Focus is `SelectablePanel::{Containers, Commands, Logs}` with next/prev methods that skip unavailable panels (e.g. commands hidden when no containers, logs hidden at zero height).
- Rendered panel/header/button/help rectangles are stored in `GuiState` maps via `update_region_map`; mouse clicks become semantic interactions: panel focus, header sort, confirm/cancel, help toggle. Maps clear on resize.
- UI layout degrades based on state/data: optional 1-line filter/search bar, containers/logs stack, commands sidebar only when containers exist, charts/ports only when data exists, ports width measured from longest port fields.
- User can adjust log panel height, toggle logs, horizontally scroll logs, scroll inspect x/y offsets, and use a scroll-many modifier.
- External exec releases terminal (`reset_terminal`), runs child command, reinitializes terminal, clears, removes status, and redraws.
- Loading spinner is reference-counted by UUIDs and animated in a Tokio task; task aborts only when all loading UUIDs are removed.
- Colors/keymaps are fully config-driven with generated typed structs per surface. Help screen is built from active keymap, not static text.
- Strong UI tests use `TestBackend`, snapshots, and cell-level assertions for border colors, selected styles, custom color overrides, and spinner frames.

Reusable additions to skill: operations dashboard/live monitor recipe, `FrameData` snapshot pattern, status-set modal routing, region-map hit-testing, config-driven color/keymap/help, reference-counted spinner, TestBackend assertion style.

## Iteration 6: openapi-tui, rainfrog, trippy

### zaghaghi/openapi-tui
Files studied: `src/tui.rs`, `src/app.rs`, `src/state.rs`, `src/action.rs`, `src/pages/home.rs`, `src/pages/phone.rs`, `src/panes/mod.rs`, `src/panes/footer.rs`, `src/panes/response_viewer.rs`, `src/components/schema_viewer.rs`, `src/panes/parameter_editor.rs`, `src/panes/apis.rs`.

Patterns extracted:
- TUI wrapper is an async event producer with `Tick` and `Render` intervals, crossterm `EventStream`, cancellation token, optional mouse/paste, suspend/resume support, and raw-mode/alt-screen cleanup in `Drop`.
- App is page-oriented: home page plus transient phone/request pages, page history keyed by operation id, header/footer always outside page, popup as a boxed pane overlay.
- Event routing uses propagation control. Popup handles first, active page second, footer third, then global keymap. `EventResponse::Continue(action)` emits without consuming; `Stop(action)` consumes.
- Global `InputMode::{Normal, Insert, Command}` controls routing. Normal keys produce actions, Insert delegates to the focused editor pane, Command delegates to footer input.
- Footer acts as command line and status line: `FocusFooter(cmd, args)` enters command mode, seeds optional text, tracks command history, renders prompt + input with visual scroll, sets cursor manually, and shows `[N]/[I]/[C]` right aligned.
- Home page has five panes (APIs, tags, address, request, response), focused pane index, fullscreen pane toggle, focused border style, and contextual status lines.
- Phone/request page builds `reqwest::Request` by folding panes implementing `RequestBuilder::path` and `RequestBuilder::request`; this decouples address/parameters/body/response accept headers/auth.
- Parameter editor groups params by location tabs, colors tabs by location, shows required marker, supports inline editing in a selected table cell, add/remove query/header commands, and applies path/query/header values to request builder.
- API list colors methods, filters by active tag and path substring, and uses `title_bottom` to show current index and filtered total.
- Response viewer has modes: normal formatted body, search results list with current match, jq transformed output with error styling. It caches highlighted lines by formatted body, adds line numbers, shows content-type variants as tabs, headers in a right pane, spinner for pending request, and status/version/size in title bottom.
- Schema viewer builds a pure `RenderBlock` model for annotated/YAML modes, resolves `$ref` with recursion guards, merges `allOf`, represents `oneOf/anyOf` as variant blocks with stable node ids and per-node selected variant.

Reusable additions to skill: API explorer/request-builder recipe, propagation-control event responses, footer command-line pattern, request-builder pane folding, response viewer modes, schema render-block model.

### achristmascarl/rainfrog
Files studied: `src/tui.rs`, `src/app.rs`, `src/components/mod.rs`, `src/ui.rs`, `src/components/editor.rs`, `src/components/scroll_table.rs`, `src/components/data.rs`, `src/config.rs`.

Patterns extracted:
- TUI wrapper is similar to openapi-tui but defaults to mouse+paste, sets terminal title, frame rate 15fps, and debounces mouse by storing last mouse event and flushing it on render tick.
- App state uses `Focus::{Menu, Editor, Data, History, Favorites, PopUp}`, with `last_focused_tab` and `last_focused_component` to restore focus after popups or tab clicks.
- Components implement a common trait for action/config registration, area init, event handling, update, and draw. Main loop broadcasts unconsumed events/actions to all component implementations in enum order.
- Async database state is polled each loop (`Finished`, `ConfirmTx`, `Pending`, `NoTask`) and converted into data state or confirmation popups.
- Query execution pipeline: add query to history, optionally bypass parser, parse execution type, show confirm/bypass/transaction popup when needed, set data loading, start async query/transaction. Abort sets a distinct cancelled state.
- Layout: left menu 25%, right panel 75%; right top tabs/editor/history/favorites and bottom data; hints footer height expands to two lines under width 160. Mouse up in tab/header zones changes focus or cycles tabs.
- Dynamic hints depend on focus and query-running state; query running prepends abort shortcut and suppresses normal execution hints.
- Editor uses `tui-textarea` plus a Vim state machine, SQL keyword regex highlighting, cursor style per mode, line number style by focus, paste insertion, alt/ctrl-enter execute, and Ctrl-F/Alt-F favorite popup.
- `ScrollTable` renders a normal `Table` to an offscreen `Buffer` sized to requested width, then copies a horizontally offset viewport into the real buffer. It tracks column offsets, x/y offsets, page height, max offsets, and row/cell selection mode; draws horizontal/vertical scrollbars only when needed.
- Data component has explicit states: blank, loading, no results, rows affected, statement completed, explain text, table results, error, cancelled. One-cell result sets render as a paragraph. Explain output has independent x/y scroll and scrollbars.
- Selection modes (`Row`, `Cell`, `Copied`) drive title text, copy behavior, and table highlight style. `Enter` and `Backspace` transition between modes; `y/Y` copy cell/row/all with confirmation for all rows.
- Config parses focus-scoped key sequences (`<ctrl-x><g>` style) and style strings into `KeyEvent` vectors and Ratatui `Style`s.

Reusable additions to skill: database SQL workbench recipe, offscreen-buffer horizontal table scroller, explicit result states, focus/tab restoration, Vim textarea editor, dynamic hints, config key/style parsing.

### fujiapple852/trippy
Files studied: `crates/trippy-tui/src/frontend.rs`, `frontend/tui_app.rs`, `frontend/columns.rs`, `frontend/render/app.rs`, `body.rs`, `table.rs`, `chart.rs`, `world.rs`, `help.rs`, `settings.rs`, `footer.rs`, `bar.rs`.

Patterns extracted:
- Terminal lifecycle installs a panic hook that disables raw mode and leaves alt-screen; exit can either leave alt-screen normally or preserve the drawn screen by moving cursor below the TUI and appending a line.
- Main loop snapshots trace data before draw unless frozen, clamps selected hop, updates ordered flow counts, draws, then polls a key for `refresh_rate`. This simple sync loop works because updates are periodic and low frequency.
- Input routing prioritizes help/settings modal modes, then normal mode. Help/settings have dedicated key subsets; pressing settings keys from help jumps directly to the relevant settings tab.
- `TuiApp` stores selected trace/hop/address/flow, ordered flow counts, show flags for help/settings/details/flows/chart/map, frozen start, and zoom factor.
- Top-level layout changes based on whether there are target tabs or flow view: header always, optional tabs/flows, body, footer charts, info bar. Body switches to error screen, splash until first data, chart/map/table based on toggles.
- Hop table columns are typed, user-configurable, hideable, reorderable, and serialize to short characters. Fixed-width columns keep known widths; variable columns divide leftover width after fixed columns. Settings dialog can toggle and move columns.
- Table rows derive height from selected-hop details; active/inactive round data have different colors; selected inactive rows use different selected style. Host rendering respects privacy TTL, max addresses, DNS/GeoIP/address mode settings.
- Chart view renders all hops as datasets, highlights selected hop, mutes others, uses Braille markers, and displays zoom factor in x-axis label.
- GeoIP map uses Ratatui `Canvas`: draws world map, pins, accuracy radius circles, selected-hop rectangle, and a `Clear`ed info panel. Privacy settings hide map locations for configured TTL ranges.
- Help and settings are centered `Clear` overlays with double borders. Settings is tabbed and renders runtime TUI/trace/DNS/GeoIP/bindings/theme/columns values plus info text.
- Footer combines history and frequency charts; one-line info bar splits immutable run config (protocol/privilege/locale) and togglable runtime state (ASN, details, address mode, privacy, max hosts) using filled/empty markers.

Reusable additions to skill: network telemetry/traceroute visualizer recipe, preserve-screen exit, frozen snapshots, configurable columns/settings dialog, selected-context charts, canvas map overlay, runtime info bar.

## Iteration 7: yozefu + synthesis

### MAIF/yozefu
Files studied: `crates/tui/src/tui.rs`, `component/mod.rs`, `component/root_component.rs`, `component/state.rs`, `component/ui.rs`, `action.rs`, `component/search_component.rs`, `component/footer_component.rs`, `component/records_component.rs`, `component/topics_component.rs`, `component/record_details_component.rs`, `component/vertical_scrollable_block.rs`, `records_buffer.rs`.

Patterns extracted:
- TUI backend writes to stderr (`CrosstermBackend<std::io::Stderr>`) so stdout can remain available for CLI/data output. Terminal lifecycle includes panic hook restore, raw mode, alternate screen, cursor hide/show, async `EventStream`, tick interval, render interval, and cancellation token.
- Component trait provides `id`, `handle_events`, `update`, `draw`, `shortcuts`, focus styling helpers, and action-handler registration. Components are stored in a `HashMap<ComponentName, Arc<Mutex<dyn Component>>>`.
- Root component owns a view stack (`views`), focus history, and per-view focus order. `Tab`/`BackTab` cycle through focus order; `Esc` pops a view; at root it shows “Press [CTRL + C] to exit” notification instead of quitting abruptly.
- Root augments focused-component shortcuts with global shortcuts and sends `Action::Shortcuts`/`Action::ViewStack` so footer is always contextual and view-stack aware.
- Layout has a tiny-terminal easter/fallback, then clear + header + main view + search + footer. `Ctrl+O` toggles between topics+records split and records-only view while preserving focus validity.
- UI run loop loads topics asynchronously, starts TUI, routes terminal events to root, processes actions, handles resize/render/search/topic selection/export/open/schema actions, and exits after worker cancellation.
- Kafka consumption is split into tasks: Kafka consumer batches raw `OwnedMessage`s, search/parser task converts messages to `KafkaRecord`, applies search query, tracks read/matched counts, sends `RecordsAndStats` to UI channel, respects limit, and cancels via `CancellationToken`.
- `RecordsComponent` drains up to 500 channel messages per draw, extends a bounded circular buffer, updates selection/follow behavior, sorts only when new records were read, and displays matched/read metrics plus “Live” throbber.
- Record navigation includes follow/unfollow, first/last via `[`/`]` or `gg/GG`-like buffered key events, timestamp format toggle, adjustable column sizing, copy/export/open, and detail view opening. Manual navigation disables follow.
- `RecordsBuffer` uses platform-specific capacity (500 normally, 120 on Windows), read/matched stats, and parallel sorting over the bounded buffer by timestamp/key/value/partition/offset/size/topic.
- `SearchComponent` has query history, history autocomplete, debounced async parse validation with cancellation, parse-error remaining-input highlight, paste/enter hack that converts accidental newline paste to space, and emits `Search`/`NewSearchPrompt`/notifications.
- `TopicsComponent` combines topic filtering and multi-select. It shows selected count in title, `[x]` rows for selected topics, inline search input only when filter is active, loading text while topics load, and Ctrl+U to clear selected topics.
- `RecordDetailsComponent` precomputes a vector of highlighted `Line`s when record changes, includes metadata/header/schema/key/value sections, supports scroll/top/bottom/open/schema/copy/export, and requests schemas asynchronously before opening schema view.
- `VerticalScrollableBlock` is a wrapper component for any `WithHeight` child that adds j/k/up/down/[ /] scrolling and a vertical scrollbar without changing child implementation.

Reusable additions to skill: streaming event-log/Kafka browser recipe, stderr-backed TUI note, component registry + view stack + focus history, contextual footer shortcuts, bounded channel-drained record buffer, cancelable ingestion/search workers, follow mode, debounced query validation/autocomplete, multi-select topic picker, precomputed record detail lines.

### Cross-project synthesis updates
- Added a concise Pattern selection checklist near the top of `.pi/skills/ratatui/SKILL.md` so agents can pick the right recipe quickly before implementation.
- Added a Streaming event-log / Kafka browser recipe distilled from yozefu and cross-referenced related live-dashboard patterns from oxker/trippy.
- The skill now covers the major studied categories: agent/chat shells, inspectors, monitors, dashboards, fixed grids/games, document/schema viewers, API request builders, SQL workbenches, network visualizers, event-stream browsers, fuzzy pickers, palettes, and large component apps.
