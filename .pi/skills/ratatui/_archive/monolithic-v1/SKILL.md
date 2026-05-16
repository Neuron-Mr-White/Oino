---
name: ratatui
description: Craft high-quality Rust Ratatui terminal UIs and UX. Use when designing, implementing, reviewing, or polishing TUI apps, widgets, terminal event loops, layouts, keybindings, tables, modals, streaming views, themes, snapshots, or terminal compatibility.
---

# Ratatui Craft Skill

Use this skill whenever the task involves a Rust terminal UI built with `ratatui`, `crossterm`, `tui-input`, or adjacent TUI crates. The goal is to produce TUIs with the clarity of `bandwhich`, the rich inspector feel of `binsider`, and the robust interactive-agent shell patterns seen in DeepSeek-TUI / Codex-style apps.

## Non-negotiable mental model

Build the TUI as a deterministic state machine plus pure-ish renderers:

1. **State owns facts**: selected tab, focused pane, scroll offsets, input value, async loading state, pending actions, theme, terminal quirks.
2. **Events are normalized**: convert `crossterm` key/mouse/resize/tick/async messages into app-level `Event` or `Command` enums before mutating state.
3. **Commands mutate state**: `State::run_command(command, tx)` is where navigation, filters, tab changes, detail toggles, and async requests happen.
4. **Render reads state**: `render(state, frame)` computes layout from `frame.area()` and emits widgets. Do not perform blocking work from render.
5. **Effects are explicit**: modals/widgets return `Action`/`ViewEvent` rather than directly changing unrelated subsystems.

A good Ratatui app should be easy to test without a terminal: feed commands into state, render into a `Buffer`, and assert state or snapshots.

## Project setup checklist

When adding or revising a Ratatui UI:

- [ ] Put terminal setup/teardown in a `Tui` or `TerminalGuard` type.
- [ ] Hide cursor except when text input is focused; restore cursor on exit/panic.
- [ ] Enable raw mode + alternate screen; opt into mouse/bracketed paste only if used.
- [ ] Normalize events into app-specific `Event` and `Command` enums.
- [ ] Keep render functions side-effect-light; calculate layout every frame from current rect.
- [ ] Track focus explicitly (`FocusedPane`, `Mode`, `ViewStack`, `input_mode`, etc.).
- [ ] Add dynamic key hints/footer that reflects current mode and focus.
- [ ] Use width-aware truncation (`unicode-width`) for tables/paths.
- [ ] Add scrollbars or counts for long lists/tables.
- [ ] Coalesce redraws and cap frame rate for streaming/high-frequency updates.
- [ ] Add tests for key routing, state transitions, and at least core render snapshots.


## Pattern selection checklist

When a user asks for a Ratatui UI, pick the closest recipe before writing code:

- **Chat/agent shell**: transcript + dynamic composer + async backend + paste handling → use chat bubble/composer/event-broker patterns.
- **Inspector/explorer**: list/table + details + tabs + search → use focused panes, stateful lists, scrollbars, command footer.
- **Live monitor/dashboard**: continuous data + charts/logs + commands → use snapshot/FrameData, rerender flags, status-set modal routing, configurable keymaps.
- **Database/spreadsheet/grid**: dense two-axis data → use custom/offscreen table, x/y offsets, frozen/selected row-cell modes, width caps.
- **Document/markdown/schema viewer**: structured text + images/links/sections → parse off-thread, version events, section-height model, status input queue.
- **API/request builder**: panes that contribute to a request → page/pane architecture, footer command line, independent request-builder panes.
- **Game/board/fixed grid**: spatial state → fixed cell constraints, centered board, cell view objects, overlay final state.
- **Picker/palette**: short-lived selection → TEA update loop, impossible-state enum, preview hidden at small sizes, centered popup.

For every pattern, still enforce: terminal guard, explicit focus/mode, normalized commands, responsive fallback for tiny terminals, width-aware text, and render tests.

## File/module architecture that scales

Prefer this shape for serious apps:

```text
src/tui/
  mod.rs              # Tui<TerminalBackend>, init, draw, reset, exit
  event.rs            # Event enum + event handler / async broker
  command.rs          # Command enum + From<KeyEvent>/From<MouseEvent>
  state.rs            # State/App model + run_command reducers
  ui.rs               # top-level layout + render(state, frame)
  theme.rs            # palette, status colors, low-color fallbacks
  widgets/            # self-contained renderable widgets
  views/              # modal/view stack, ViewAction, ViewEvent
```

For small apps, `event.rs`, `state.rs`, and `ui.rs` may be enough. For agent shells or database/browser-style apps, add `views/` and `widgets/` early to avoid one giant `ui.rs`.

## Terminal lifecycle pattern

Use a guard so panics and early returns do not leave the user terminal broken.

```rust
pub struct Tui<B: Backend> {
    terminal: Terminal<B>,
    pub events: EventHandler,
}

impl<B: Backend> Tui<B> {
    pub fn init(&mut self) -> anyhow::Result<()> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
        )?;
        self.terminal.hide_cursor()?;
        self.terminal.clear()?;
        Ok(())
    }

    pub fn draw(&mut self, state: &mut State) -> anyhow::Result<()> {
        self.terminal.draw(|f| render(state, f))?;
        Ok(())
    }

    pub fn exit(&mut self) -> anyhow::Result<()> {
        crossterm::terminal::disable_raw_mode()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        )?;
        self.terminal.show_cursor()?;
        self.events.stop();
        Ok(())
    }
}
```

Robust apps also install a panic hook that resets the terminal before printing panic diagnostics. Interactive-agent shells may need bracketed paste, focus events, keyboard enhancement flags for Shift+Enter/Esc disambiguation, and terminal-quirk recovery after child processes.

## Event loop patterns

### Threaded tick/event channel

Good for sync apps (`binsider`-style): one thread polls terminal input with a timeout and sends `Tick` when no input arrives.

```rust
pub enum Event {
    Tick,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    DataLoaded(Result<Data>),
}
```

Rules:
- Use `event::poll(timeout)`; emit `Tick` at a stable interval.
- Include async/work results in the same event stream (`DataLoaded`, `TraceResult`, `Restart`).
- Add `key_input_disabled` or pause mode if you temporarily leave alt-screen for subprocesses.

### Redraw coalescing for streaming apps

Good for chat/agent shells:

- Mutations set `state.needs_redraw = true`.
- The loop draws only when dirty.
- A frame limiter caps draws (e.g. 120 FPS normal, 30 FPS low-motion).
- Multiple model/tool chunks between polls collapse into one frame.

This prevents streaming text from redrawing hundreds of times per second.

### Resize discipline

On resize:
- Drain/coalesce rapid resize events if possible.
- Clear or force a full repaint when layout shape changes.
- Recompute layout from the new `frame.area()`; never cache rects across frames except as hints.

## Command routing and focus

Do not scatter key handling through render code. Convert keys to commands first.

```rust
impl From<KeyEvent> for Command {
    fn from(key: KeyEvent) -> Self {
        match key.code {
            KeyCode::Right | KeyCode::Char('l') => Command::Next(FocusTarget::Pane, 1),
            KeyCode::Left  | KeyCode::Char('h') => Command::Previous(FocusTarget::Pane, 1),
            KeyCode::Down  | KeyCode::Char('j') => Command::Next(FocusTarget::List, 1),
            KeyCode::Up    | KeyCode::Char('k') => Command::Previous(FocusTarget::List, 1),
            KeyCode::PageDown | KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => Command::PageDown,
            KeyCode::PageUp   | KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Command::PageUp,
            KeyCode::Tab => Command::Next(FocusTarget::Tab, 1),
            KeyCode::BackTab => Command::Previous(FocusTarget::Tab, 1),
            KeyCode::Char('/') => Command::EnterSearch,
            KeyCode::Esc | KeyCode::Char('q') => Command::ExitOrClose,
            _ => Command::Nothing,
        }
    }
}
```

Focus rules:
- If a modal/view stack is open, it receives keys first and returns `ViewAction`.
- If text input is active, route printable keys/backspace/enter/esc to `tui_input::Input` before global shortcuts.
- If an embedded component has its own key handler (hex viewer, editor), delegate while it is focused.
- Offer both arrows and vim keys for navigation in data-heavy TUIs.
- Mouse wheel should map to the same scroll commands as keyboard navigation.

## Layout recipes

### Standard app shell

Use fixed one-line header/footer, a flexible body, and optional dynamic-height areas.

```rust
let chunks = Layout::vertical([
    Constraint::Length(1),          // header/status
    Constraint::Min(1),             // main body
    Constraint::Length(preview_h),   // 0 when empty
    Constraint::Length(input_h),     // composer/search/input
    Constraint::Length(1),          // footer/help
]).split(frame.area());
```

Use `saturating_sub` when computing dynamic heights. Keep a minimum body height so the composer/search box cannot starve the main content.

### Progressive multi-pane layout

From `bandwhich`: preserve utility on small terminals by showing fewer panes and cycling them with Tab.

- 1 child: full body.
- 2 children: horizontal split on wide terminals; vertical split on tall/narrow terminals; if both width and height are too small, show only one child.
- 3 children: two top panes + one bottom pane on large terminals; otherwise drop/cycle lower-priority panes.

This is better than squeezing three unreadable panels into a tiny terminal.

### Detail inspector layout

From `binsider`: combine tabs + focused blocks + scrollable tables.

- Top header: bordered title centered + selected tab strip + right-aligned path/breadcrumb.
- Body: bordered panels. Focused panel gets accent/yellow border; inactive panels get muted gray.
- Bottom: contextual key-hint strip.
- Tables: show selected row count in `title_bottom` (`12/498`) and search input in another bottom title.
- Long paragraphs get a `Scrollbar` with `ScrollbarState::new(max).position(scroll)`.

### Sidebar and optional panes

Only show sidebars/file trees when width crosses a minimum breakpoint. Otherwise hide them behind a command/modal. Reserve a fixed or percentage width, then render main content in the remaining area.

## Tables and dense data

High-quality terminal tables adapt columns to width:

1. Define width cutoffs and desired column widths.
2. Select which columns are visible for each layout.
3. Proportionally shrink/expand columns to available width.
4. Use width-aware truncation; for paths/connections, truncate the middle so both ends remain visible.

```rust
fn truncate_middle(s: &str, max: u16) -> String {
    use unicode_width::UnicodeWidthStr;
    if max < 6 { return s.chars().take(max as usize).collect(); }
    if s.width() as u16 <= max { return s.to_string(); }
    let ellipsis = "..";
    let suffix_len = (max as usize - ellipsis.len()) / 2;
    let prefix_len = max as usize - ellipsis.len() - suffix_len;
    let prefix: String = s.chars().take(prefix_len).collect();
    let suffix: String = s.chars().rev().take(suffix_len).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{prefix}{ellipsis}{suffix}")
}
```

For selected rows, prefer a clear but not garish highlight (`fg(Color::Green)` or accent + bold). Add scrollbars when the selected index can leave the visible page.

## Text input, search, and command palettes

Use `tui-input` or an equivalent input model. Keep `input_mode` or `Mode::Search` explicit.

Good behavior:
- `/` enters search, `Enter` confirms, `Esc` exits and clears if appropriate.
- Backspace on an empty inactive search can resume search with the backspace event or close input mode.
- Search changes should reset relevant scroll offsets to the top.
- Render search/query in a bottom title or footer so it does not steal main content height.
- For command palettes/file pickers/model pickers, implement them as modal views that emit typed events (`Selected`, `Deleted`, `Applied`) instead of directly mutating global state.

## Modal/view stack pattern

For rich TUIs, define a modal trait:

```rust
pub enum ViewAction { None, Close, Emit(ViewEvent), EmitAndClose(ViewEvent) }

pub trait ModalView {
    fn kind(&self) -> ModalKind;
    fn handle_key(&mut self, key: KeyEvent) -> ViewAction;
    fn handle_mouse(&mut self, mouse: MouseEvent) -> ViewAction { ViewAction::None }
    fn render(&mut self, area: Rect, buf: &mut Buffer);
}
```

The app owns `view_stack`. Top-level routing:
1. If view stack non-empty, send input to top view.
2. Host handles emitted `ViewEvent` (copy, approve, select model, update config, etc.).
3. View closes or stays open.
4. Main UI renders first; modal renders last with `Clear`/popup area.

This keeps modals reusable and testable.

## Visual design rules for terminal UX

- **Use one accent color** for active/focused/selected elements.
- **Use muted gray** for borders, separators, inactive metadata, and brackets.
- **Use semantic colors sparingly**: green success/traffic-down, yellow warning/rate-up/search hits, red errors/destructive.
- **Keep borders purposeful**: border panels in inspector apps; avoid boxing every tiny label in chat shells.
- **Prefer one-line status surfaces**: header for identity/context, footer for keys/status.
- **Make current focus obvious** via border color/title style, not just cursor position.
- **Show empty/loading states**: logo/splash, `No traffic`, `Loading strings…`, `Trace not run: press r`, etc.
- **Respect low-motion**: disable spinners/animations or cap to 30 FPS.
- **Use width-aware text APIs** (`unicode-width`, `Line::width`) before deciding to hide/clip hints.

## Header/footer/key-hint patterns

Header should answer: where am I, what mode/model/file/tab, is it active/loading?

Footer should answer: what can I press right now?

Dynamic footer examples:
- Normal list: `[j/k→move] [Tab→next tab] [/→search] [Enter→details] [q→quit]`
- Search mode: `[Enter→apply] [Esc→cancel] [type→filter]`
- Paused network monitor: `Paused - press Space to resume, q to quit`

Hide or simplify hints if their rendered width would collide with input text. Never let hints obscure active user input.

## Async work and temporary terminal release

For subprocesses, external editors, tracers, or shell jobs:

- Pause input handling.
- Leave alternate screen and disable raw mode if the child needs the real terminal.
- Run the child/subprocess.
- Restore raw mode, alternate screen, mouse/paste/focus capabilities.
- Force a full repaint/clear after returning.
- Communicate result through the event channel (`TraceResult`, `EditorClosed`, `ShellJobComplete`).


## Complex text composers and bottom panes

For agent shells, treat the composer as its own state machine, not as a raw text box:

- Separate body text, attachments, remote image rows, placeholder elements, popup state, history search, and paste-burst detection.
- Route keys in layers: active popup first (slash/file/mention/skill picker), then composer mode, then global shortcuts.
- Normalize paste (`\r` → `\n`) before inserting.
- If the terminal lacks reliable bracketed paste, detect rapid key bursts and flush them as paste instead of triggering shortcut side effects for pasted `?`, `/`, etc.
- Preserve kill/yank buffers when submitting or running local composer actions so users can recover draft fragments.
- History recall should rehydrate rich local entries (attachments/elements) and plain persistent entries differently.
- Footer state is part of the composer UX: while typing, hide noisy shortcuts; when idle, show mode/queue/context hints.

A robust bottom pane can host multiple child views (`approval`, `user input`, `list selection`, `feedback`, `help`) by defining a `BottomPaneView` trait with `handle_key_event`, `is_complete`, `completion`, optional paste handling, optional request consumption, and `next_frame_delay` for timed redraws.

## Async frame requester / event broker recipe

For async apps, avoid direct `terminal.draw()` calls from background tasks. Instead:

1. Give widgets/tasks a cloneable `FrameRequester`.
2. `schedule_frame()` sends a desired draw instant to a scheduler task.
3. Scheduler coalesces requests, clamps to a target interval, and broadcasts `Draw`.
4. Main event stream merges `Draw`, `Resize`, `Key`, and `Paste`.

This allows animations, streaming responses, paste-burst flushes, and external async events to request redraws without racing the renderer.

When launching an external editor/subprocess, drop or pause the underlying crossterm event stream, not merely stop polling it. Crossterm can otherwise keep reading stdin in a background thread and steal input or terminal query responses from the child process.

## Board/grid interaction recipe

For chess/board/game UIs:

- Represent interaction separately from rendering: cursor coordinate, selected square, legal-target list, last move, mouse-used flag, board flipped flag, promotion cursor, hidden-cursor/read-only flag.
- Compute square size from available area each frame: `cell_w = area.width / 8`, `cell_h = area.height / 8`, then center the board with leftover border padding.
- Store `top_x/top_y/cell_w/cell_h` during render so mouse clicks can map back to board coordinates.
- Render state layers in order: base checkerboard, danger/check state, selected square, last move, legal moves, cursor, then piece glyph centered inside the cell.
- Support keyboard and mouse parity. When keyboard follows a mouse interaction, reconcile cursor state to the mouse-selected square or reset it to a safe default.
- Use ticks for low-frequency UI animation such as selection blink; when animation is active, prefer a non-blocking event loop plus ~16 ms sleep to avoid input lag.
- For custom skins, route all colors through an adapter that can degrade unsupported RGB colors.

## Spreadsheet/data-grid recipe

For CSV/spreadsheet viewers, a hand-rendered `StatefulWidget` often beats `ratatui::Table`:

- Compute column widths from visible header + cell content, then cap any one column to a fraction of frame width (e.g. 30%) so a huge cell cannot hide everything else.
- If clipping leaves unused space, redistribute it among clipped columns greedily so the right border remains visible.
- Track row heights when wrapping is enabled; stop expensive wrapping calculations once accumulated height fills the frame.
- Keep row numbers in a fixed left gutter and render custom separators/intersections directly into the buffer.
- Support horizontal scrolling via `cols_offset`, plus optional frozen columns with a double vertical separator.
- Expose selection modes (`row`, `column`, `cell`), marked rows, and selected/found styles as independent overlays.
- Put status/search/filter/goto input in a bottom status line; set the terminal cursor from buffered input state.
- For find/filter, show progressive status like `[Find "foo": 2/17+]` where `+` means search is still running/streaming.

## Input buffering and modal command lines

Some TUIs need Vim-like prefix buffers rather than a permanent input box:

- Digits can start `GotoLine` mode (`125g` or `125 Enter`).
- `/` starts find, `&` starts row filter, `*` starts column filter, `f` can start freeze-column numeric input, `-` can start option mode.
- Up/Down should recall per-mode input history. Sharing history between find/filter can be useful.
- `Esc` resets the buffer and any temporary filter preview.
- Normalize Shift modifiers across platforms before key matching; terminals disagree on whether symbols like `>` carry `SHIFT`.


## Background scan/tree-browser recipe

For disk analyzers, package browsers, file explorers, and other tree UIs:

- Keep traversal/scanning as a background producer and UI input as another producer; use `select!`/channels to process whichever event arrives first.
- Integrate partial scan events into a persistent tree model, then recompute sorted visible entries from `view_root`, sort mode, filters, and mark state.
- Preserve the user's selection across refresh/rescan by matching the previously selected name/path first, then falling back to previous index, then first entry.
- Represent navigation as `view_root`, `selected`, and optional alternate navigation for filtered/search result roots.
- Support dual quit semantics for destructive/important apps: first `q/Esc` sets `pending_exit` and shows a high-contrast footer; second confirms.
- Mark panes should be explicit modes with their own focus, list state, and confirmation copy. Render marked items with distinct color independent from selection.
- Header/footer can reflect operational state: total bytes, entries scanned, scan rate, sort mode, current path, and transient warning.
- Inline help in panel corners should be drawn only if `title_width + help_width <= area.width`.

## Multi-panel productivity app recipe

For RSS/mail/issue-tracker style apps:

- Use a central `Message` bus with `Event` and `Command` variants. Every component implements a `process_command(&Message)`/receiver method.
- The app loop can route each message to input mapping, batch processor, focused panels, command input, confirmations, and help popups in a stable order.
- Keep panel focus as app state (`FeedSelection`, `ArticleSelection`, `ContentSelection`, `DistractionFree`). Use focus to change layout ratios.
- Store last rendered panel `Rect`s for hit-testing. Mouse clicks first change focus, then send semantic events such as `MouseArticleClick(row_offset)`.
- Render focused panels last so their borders overwrite neighboring inactive borders.
- Support user-resizable splits by detecting mouse down on separator, tracking drag state, clamping to minimum panel heights, and redrawing only when dimensions change.
- A bottom status bar can combine tooltip text, offline/online indicator, and spinner; in distraction-free mode, hide it except for warnings/errors.
- Configurable key sequences should accumulate a `KeySequence`, show prefix-match help while waiting, execute on direct match/submit/timeout, and show a warning tooltip for unknown sequences.

## Fuzzy picker with preview recipe

For command/file/task pickers:

- Model impossible states explicitly: `SelectCommand`, `ExecuteCommand`, `ShouldQuit`; inside selection state, track current pane, search textarea, command list state, history list state, optional argument popup, notifications.
- Use TEA-like flow: `handle_key_input -> Message`, `update(model, message)`, `ui(frame, model)`.
- Search text changes reset list selection to the first filtered result; if no results, selection is `None` and Enter does nothing.
- Split UI into search/notification row, command/history panes, optional preview pane, footer hints. Hide the preview below a height threshold instead of cramping everything.
- Distinguish focus with both color and border type (e.g. thick green for active pane, plain dark gray for inactive). Disable active-pane styling behind a modal popup.
- Preview the selected command near its source line by centering the target line in the preview area when possible; prefix line numbers for orientation.
- Syntax highlighting in previews should be bounded and defensive: replace tabs for stable layout, cache/extract themes once per version, and skip known pathological inputs.
- Additional-arguments popups should render with `Clear` over a centered rect, route all keys except Esc/Enter into a separate textarea, and append arguments at execution time.
- After leaving alt-screen to run a selected command, print/show the command before executing so the shell transcript explains what happened.


## Git client / large component app recipe

For mature, multi-surface TUIs like Git clients:

- Split every pane/popup into a `Component` trait with `event`, `commands`, `draw`, `is_visible`, and optional focus methods.
- Send input to visible/focused components first with an `event_pump`; if consumed, skip global shortcuts.
- Gather contextual command-bar items through a `command_pump`; let visible modal components return `Blocking` so commands underneath do not leak into the quickbar.
- Keep an internal `Queue<InternalEvent>` for component-to-app requests: open popup, confirm action, switch tab, update data, show info/error, launch editor, fuzzy-find changed, etc.
- Main loop should select across input events, async Git/work events, app events, file-watch/tick notifications, and spinner ticks. Spinner ticks should draw only the spinner cell/area rather than redrawing the whole app.
- Support pausing input polling before external editor launch. When input thread reports `Paused`, run the editor, request full redraw, then resume polling; when it reports `Polling`, re-hide cursor.
- Top-level layout can be `top tabs/status`, active tab body, dynamic command bar. Fullscreen popups suppress body rendering but still draw top/command surfaces as appropriate.
- Command bars should wrap by Unicode width, compute their own height, and provide a compact `more [.]` affordance when expanded content exceeds one row.
- For scrollable rich text, build a `StatefulParagraph` that tracks `scroll.x`, `scroll.y`, wrapped line count, and visible height; use custom line composers for word-wrap vs horizontal truncation.

## Palette and color-system recipe

For color pickers and theme tooling:

- Represent colors as typed constants (e.g. `HexColor`) and implement `From<HexColor> for ratatui::style::Color` behind a feature flag.
- For color swatches, compute a uniform grid from `frame.area()`: `sq_width = width / columns`, `sq_height = available_height / rows`, then center the full grid with leftover margins.
- Choose foreground by luminance/tone rather than one static value; light variants need black text, dark variants need white text.
- Use the footer/input area to explain interaction, echo the current typed code, and show transient clipboard/result messages.
- Treat tiny utilities as still needing terminal hygiene: raw mode, alt-screen, draw loop, Esc exit, Backspace edit, and restore terminal on errors.

## Markdown/document reader recipe

For markdown/document viewers with images and links:

- Keep parsing, image loading, resizing, and header rendering off the UI thread in a worker. UI sends `Parse(document_id, width, text, cache)` and receives section-level events.
- Version async parse events with a `DocumentId`/reload id. Ignore stale events from older parses after reload/resize.
- Represent the document as sections with explicit heights: text lines, image placeholders, loaded images, header placeholders, rendered headers. Rendering then becomes a simple y-offset pass from `-scroll`.
- On resize, reparse using the new inner width and skip immediate render until worker events arrive; width changes affect markdown wrapping and image sizing.
- Cache image protocols across reparses and send cached image events before `ParseDone`; leave placeholders for uncached images and update sections as images arrive.
- Track extra metadata per line (`Link`, `SearchMatch`) so the renderer can draw overlays on top of already-rendered text.
- Link selection should highlight all wrapped line segments for the same URL and place the cursor at the selected segment. For hidden URLs, show selected URL in the status line.
- Search mode should update highlights incrementally as the user types, use a distinct style for the active match, and let Enter jump to first/next match.
- Support Vim-like counts (`10j`, `3n`), half/full-page scroll, `g/G`, search `/`, link open `Enter`, and `z/t/b` cursor positioning commands with a small status-line input queue.
- Reserve the last line for status/input; do not render document content into it.


## Terminal game / fixed-grid recipe

For puzzle, board, and game-like TUIs where spatial clarity matters more than data density:

- Precompute fixed `Constraint::Length(cell_w/cell_h)` vectors from game options, then center the whole board inside the available body each frame. Use symmetric spacer chunks and `saturating_sub`/checked math for tiny terminals.
- Keep the game state tiny and explicit: `active_row`, `active_col`, flags/reveals/selection, `lost/won`. Navigation methods should clamp/saturate, never wrap accidentally unless wrap is a designed rule.
- Model each rendered cell as a lightweight view object over app state (`Cell { app, row, col }`) with methods for `is_active`, `is_exposed`, `is_flagged`, `text_style`, and `block`. This concentrates cell styling and avoids giant nested `match` blocks in the draw loop.
- Render grid layers predictably: outer title border, centered info/help areas, board block, row/column cell layouts, then win/loss overlay. Use `Clear` before a centered overlay so the banner remains readable over the grid.
- Use glyphs only where they add instant recognition (`💣`, `⛳`) and ensure the cell text is horizontally/vertically padded to the configured cell size.
- Align help text around a delimiter (`movement: hjkl / arrows`) and center it to board width, so the help remains visually tied to the grid rather than the full terminal.
- Accept both arrows and vim movement. After terminal draw, block for the next input event unless animation/timers matter; simple games can avoid busy redraws.

## Chat bubble / conversational agent recipe

For LLM/chat shells, conversational layout quality comes from message geometry, caching, and input gating:

- Split the screen vertically into scrollable transcript and dynamic-height composer. The composer height should be `textarea.lines().len() + chrome`, capped so the transcript always keeps at least one row.
- Before rendering, check minimum usable width from author labels, border glyphs, and padding. If too narrow, render a plain fallback message instead of panicking or wrapping every bubble into garbage.
- Render messages as semantic bubbles: user messages right-aligned, assistant/system messages left-aligned, errors in red/alert styling, code-block numbers appended to fences for later actions.
- Compute bubble max width from terminal width minus outer padding percentage and border/padding budget. Clamp each bubble to the available width, but keep author labels visible even for short messages.
- Convert messages to `Vec<Line>` once and cache by message index, text length, code-block count, and line width. Clear the cache on width change; for streaming, recalculate only the last/incomplete message.
- Preserve syntax highlighting inside code blocks by feeding highlighters complete newline-terminated lines, then wrapping spans while retaining style.
- Track scroll as its own service with `ScrollbarState`; when a new response streams and the user was already at bottom, auto-follow. If the user scrolled up, do not yank them back down.
- Gate input while waiting for the backend: ignore normal typing/submission, let Ctrl-C abort the backend, and show a loading composer. Use double Ctrl-C or `/quit` as an intentional exit confirmation.
- Enable bracketed paste; normalize pasted CRLF to `\n`; route paste to the textarea yank/paste path instead of injecting key-by-key shortcuts.

## Operations dashboard / live resource monitor recipe

For Docker/Kubernetes/network/system dashboards with live data, charts, commands, logs, and mouse support:

- Separate shared data (`AppData`) from UI interaction state (`GuiState`). Build a per-frame immutable-ish `FrameData` snapshot by locking once, then pass `&FrameData` to draw functions to reduce mutex churn and keep render code deterministic.
- Use an atomic `Rerender` flag with `update_draw`, `swap_draw`, and optional `set_clear`. Redraw when data/input requests it or when a minimum refresh interval elapses; do not draw every poll loop.
- Store active `Status` values in a set (`Help`, `Filter`, `SearchLogs`, `Inspect`, `DeleteConfirm`, `Exec`, `Error`). Route keys by modal priority: error/help/filter/search/delete/inspect before normal dashboard shortcuts.
- Represent focus as `SelectablePanel` and skip hidden panels when cycling. If logs are hidden or there are no containers, panel navigation should jump to the next visible target.
- Let every panel draw function register its `Rect` into a region map (`Panel`, `Header`, `DeleteButton`, `HelpPanel`). Mouse clicks then become semantic actions: focus panel, sort header, click confirm/cancel, open/close help. Clear these maps on resize.
- Main layout should degrade by data/state: optional one-line filter/search bar, containers/logs stack, commands sidebar only when items exist, lower charts/ports only when data exists. Use `Constraint::Min` for primary content and `Max` for measured side panels (e.g. ports width from longest port strings).
- Allow user-controlled panel geometry: log height percentage, show/hide logs, horizontal log scroll, inspect x/y offsets, scroll-many modifier for coarse movement.
- For actions that leave the TUI (exec shell/editor), reset terminal, run the child, then reinitialize terminal, clear, remove `Exec` status, and force redraw.
- Loading animation should be reference-counted by operation id: starting a job inserts its UUID and advances spinner; stopping removes it and aborts the animation task only when no jobs remain.
- Make colors and keymaps config-driven. Generate typed color structs per UI surface (headers, borders, charts, logs, popups) and typed keymap fields, then render help from the active configuration rather than hardcoded strings.
- Test dashboards with `TestBackend` snapshots plus cell-level assertions for border colors, selected styles, spinner frames, empty/loading/error states, and custom color overrides.


## API explorer / request builder recipe

For OpenAPI/Postman-like TUIs that browse endpoints, edit request inputs, send calls, and inspect responses:

- Model the app as pages plus panes. A home page can own API/tag/address/request/response panes; a call page can own address/parameter/body/response panes. Each `Pane` exposes `height_constraint`, `handle_events`, `update`, and `draw`.
- Keep navigation state outside panes: active page, pane focus index, optional fullscreen pane index, popup pane, command footer, and a history map of temporarily closed call pages keyed by operation id.
- Use event responses with propagation control: child pane returns `Continue(action)` to emit an action but let others handle the event, or `Stop(action)` to consume it. Route popup first, active page second, footer/command line last, then global keymap.
- Use a global `InputMode::{Normal, Insert, Command}`. Normal mode navigates panes/items; Insert mode delegates all keys to the active editor; Command mode routes to a footer command input.
- The footer should be both contextual status line and command line: `FocusFooter(cmd, seed)` enters command mode, prefixes the prompt (`/` or `:`), seeds optional text, keeps command history, shows `[N]/[I]/[C]`, and sets terminal cursor position manually.
- Pane focus should change border type as well as color (e.g. thick + green for focused, plain for inactive). Fullscreen toggles are cheap when every pane draws independently.
- Treat request construction as composition: each request pane implements `RequestBuilder` methods like `path(url)` and `request(builder)`, then the page folds panes to produce a `reqwest::Request`. This makes path/query/header/body/auth editors independent and testable.
- Parameter editors work well as tabbed tables by location (`Path`, `Query`, `Header`, `Cookie`), with required markers, colored tabs, inline one-cell editing, and add/remove commands from the footer.
- Response viewers should have explicit modes: normal formatted body, search-result list, and transformed output (`jq`). Cache syntax-highlighted lines by formatted body text; add line numbers before rendering; show response status/version/size and mode hints in `title_bottom`.
- For schema viewers, build a pure render model from the OpenAPI schema: resolve `$ref`s with recursion guards, represent `allOf`/`oneOf`/`anyOf` as markers/variant blocks, support annotated and raw YAML modes, and track per-variant selection by stable node id.

## Database SQL workbench recipe

For SQL/database TUIs, combine an editor, schema/menu browser, results grid, history/favorites, confirmations, and async query state:

- Split surfaces into components (`Menu`, `Editor`, `Data`, `History`, `Favorites`) with `register_action_handler`, `register_config_handler`, `init(area)`, `handle_events`, `update`, and `draw`. Broadcast actions to all components unless a focused popup consumes them.
- Focus should be a first-class enum (`Menu`, `Editor`, `Data`, `History`, `Favorites`, `PopUp`) with last-focused tab/component memory. Tabs can restore the last editor/history/favorites surface while side panels keep independent focus.
- Use async DB polling as part of the main loop: poll `get_query_results`, convert `Finished/Pending/ConfirmTx/NoTask` into component state or popup state, then process terminal events/actions.
- Query execution should add history before dispatch, parse execution type, optionally show confirmation/bypass/transaction popups, then set data state to loading. Abort sets a distinct cancelled state instead of silently returning to blank.
- The editor can use `tui-textarea` plus a small Vim state machine. Configure cursor style by mode, line-number style by focus, keyword search highlighting from SQL keywords, paste insertion, and alt/ctrl-enter to execute.
- Build a custom horizontally-scrollable table by rendering a normal `Table` into an offscreen `Buffer` with requested width, then copying the visible slice into the real buffer. This permits horizontal scrolling, current-column detection, cell selection, and both vertical/horizontal scrollbars.
- Results should have explicit display states: blank, loading, no results, rows affected, statement completed, explain text, table results, error, cancelled. One-cell result sets can render as a paragraph instead of a table.
- Selection modes (`None`, `Row`, `Cell`, `Copied`) make copy UX clear. The title can show row count, selected row, selected cell preview, or `copied!`; `Enter`/`Backspace` transition between selection modes.
- For large text/EXPLAIN output, keep independent x/y offsets and clamp them against measured content width/height each draw.
- Config should parse key sequences (`<ctrl-x><g>` style) and styles from strings, then display active key hints based on current focus and query-running state.

## Network telemetry / traceroute visualizer recipe

For network monitors that continuously update hops, charts, flows, maps, and settings:

- Snapshot live core state once per frame (`snapshot_trace_data`) before drawing. If the UI is frozen, keep rendering the frozen snapshot while the backend continues collecting.
- Use a synchronous loop when the refresh rate is low and predictable: snapshot/clamp/update counts, draw, then `event::poll(refresh_rate)` for one key. Filter `KeyEventKind::Press`.
- Offer deliberate exit behavior: normal quit leaves alt-screen, while preserve-screen quit moves cursor below current output and appends a line so the final TUI remains visible in the terminal transcript.
- Top-level layout should be mode-dependent: header always, optional target tabs or flow chart, body (`table`/`chart`/`map`/`splash`/`error`), footer charts, one-line info/config bar.
- Body should be a single switch: error screen first, splash until first data, then chart/map/table according to toggles. Users should never see half-composed panels while data is unavailable.
- Make table columns user-configurable: each column has type, shown/hidden status, fixed/variable width, a short config character, and display name. Settings dialog can toggle visibility and move columns up/down.
- Variable-width columns should receive leftover width after fixed columns. Use `unicode-width` for display names and labels; hide/trim data through configuration (`max_addrs`, privacy TTL, address mode).
- Tables can change row height for selected/detail rows. Selected style should distinguish current-round data from stale/inactive data.
- Charts should share selection context: selected hop gets accent color, all others muted. Add zoom factor to the axis label so users understand sample compression.
- Geo maps use `Canvas`: draw world map, layer pins, accuracy radius, selected-hop rectangle, then overlay a small info panel with `Clear`. Respect privacy by hiding locations for configured TTL ranges.
- Help/settings dialogs should be centered, `Clear`ed overlays with their own tabs/table/info sections. Settings should display current runtime config, key bindings, theme, and visible column set rather than static docs.
- The info bar should separate immutable run config (protocol, privilege, locale) from togglable runtime state (ASN, details, address mode, privacy, max hosts) with filled/empty markers.


## Streaming event-log / Kafka browser recipe

For Kafka/event-stream/log browsers where records arrive continuously and users inspect selected events:

- Use a bounded ring/circular buffer for visible records. Pick a platform-safe capacity, track `read`, `matched`, and `buffer_size`, and expose these metrics in the table title or right corner.
- Decouple ingestion from rendering with channels: network consumer task sends raw messages, parser/search task converts to domain records and match stats, UI component drains a bounded number of channel messages per draw/tick (e.g. max 500) to avoid frame starvation.
- Restarting a search/topic selection should cancel the previous worker with a `CancellationToken`, reset record buffers, emit `NewConsumer/Consuming/StopConsuming`, and show a notification like `Searching` or `Waiting for new events`.
- Keep follow mode explicit. When follow is enabled, selecting the newest record on arrival is okay; any manual navigation disables follow and refreshes shortcuts so the footer says `Follow` vs `Unfollow`.
- For very large streams, render only rows near the current viewport/selection and use empty rows for far-off records if using `Table`; otherwise hand-render visible rows. Sorting should skip work when no new records arrived and use parallel sort only on the bounded buffer.
- Model views as a stack (`TopicsAndRecords`, `Records`, `RecordDetails`, `Schemas`, `Help`) with `focus_history` and per-view `focus_order`. `Esc` should pop a view; at root it should show a notification explaining the real quit shortcut rather than exiting accidentally.
- Contextual shortcuts belong in a footer fed by the focused component. Components return `shortcuts()`, root augments them with global shortcuts and view-stack-dependent items like `Esc Close`.
- Search inputs should validate asynchronously with debounce/cancellation, preserve query history, offer autocomplete from history, and highlight the unparsed/remaining input segment on parse errors.
- Topic pickers should combine filtering and multi-selection: render `[x] topic`, show selected count in title, keep filter input inline only when non-empty/focused, and set cursor manually in that input.
- Record details should precompute highlighted `Line`s when the record changes, not on every frame. Include metadata first, then syntax-highlighted key/value/body; use a scroll state and render scrollbars for long payloads.
- Use schema/detail side views as separate stack entries requested by actions (`RequestSchemasOf`, `Schemas`) so expensive schema-registry calls happen asynchronously and results hydrate the view later.
- Notifications should be typed (`Info/Warn/Error`) and generated for copy/export/open/consumer errors, not just logged.


## Performance and correctness pitfalls

Avoid:
- Rendering on every network/model chunk without a frame cap.
- Blocking filesystem/network calls inside render.
- Assuming byte length equals terminal width.
- Caching layout rects across resizes.
- Using `rect.height - 2` without checking underflow; prefer `saturating_sub`.
- Handling `KeyEventKind::Release` as a second press; filter for `KeyEventKind::Press` when needed.
- Forgetting to restore terminal modes on panic.
- Showing all panes/columns at tiny sizes; drop, collapse, or cycle instead.

## Testing playbook

- Reducer tests: feed commands, assert state (`tab`, `selected`, `scroll`, `input_mode`).
- Keymap tests: `KeyEvent -> Command` for arrows, vim keys, Ctrl chords, Shift+Tab.
- Buffer/snapshot tests: render into `ratatui::backend::TestBackend` or `Buffer` and assert stable UI.
- Resize tests: render common sizes (80x24, 120x30, very narrow) and assert no panic/readable fallbacks.
- Terminal lifecycle tests: fake backend for draw/end; event iterator for deterministic quit/resize/pause flows.

## Pattern index from studied references

- **DeepSeek-TUI / Codex-like agent shell**: robust terminal guard, keyboard enhancement/paste/focus support, view stack with typed `ViewEvent`, streaming redraw coalescing, frame limiter, dynamic composer height, pending input preview, toast stack, sidebar at width breakpoints.
- **bandwhich**: simple monitor loop with separate display and terminal-event threads, fixed header/footer, progressive pane dropping/cycling, adaptive table columns, middle truncation by Unicode width, raw text output mode for non-TUI use.
- **binsider**: `Tui` lifecycle wrapper, threaded event handler with tick + async result events, `Command` enum from key/mouse, tabbed inspector UI, focused block borders, scrollbars, bottom key hints, `tui-input`, splash/logo, and integration with an embedded hex viewer.
- **chess-tui**: page/popup routed handlers, filtering `KeyEventKind::Press`, board-coordinate state separated from rendering, cell size from area/8 with centered leftover padding, mouse-to-board mapping, render layers for board/selection/last move/legal moves/cursor/pieces, skin/display-mode switching, animation fast-loop with nonblocking event polling.
- **openai/codex**: async `FrameRequester` actor that coalesces scheduled redraws, event broker that can drop/recreate crossterm `EventStream`, inline viewport management, pending history lines above the active viewport, bottom-pane views with completion semantics, rich composer state machine, paste normalization/burst detection, exhaustive snapshot tests for footer/composer/list views.
- **csvlens**: `Control` enum from input handler, platform-normalized Shift, transient command buffers for find/filter/goto/options, per-mode history, hand-rendered CSV grid as `StatefulWidget`, row-number gutter, custom border intersections, capped/redistributed column widths, wrapping row heights, frozen columns, find/filter status line, selection/mark overlays, and many `TestBackend` interaction tests.
- **dua-cli**: background traversal integrated with terminal input via channel select, persistent tree/navigation model, selection preservation across rescan, focusable main/help/mark/glob panes, pending-exit footer, inline corner help that only appears when space allows, custom list wrapper that preserves offset, direct buffer drawing for width-aware text.
- **eilmeldung**: central async message bus, components as message receivers, configurable multi-key sequences with prefix help and timeout spinner, batch command processor gated by async operations, focus-dependent panel ratios, stored panel rects for mouse hit-testing, drag-resizable splits, status bar with tooltip/offline/spinner, render focused panel last.
- **fzf-make**: TEA-like `Message` update loop, impossible-state enum, fuzzy filtered command list plus history pane, responsive preview hidden below height threshold, centered source-line preview with line numbers and syntax highlighting safeguards, thick/colored active borders, `tui-textarea` search and argument popup, clipboard notification, post-TUI command execution transcript.
- **gitui**: component trait architecture, event/command pumps with modal command blocking, internal event queue, crossbeam select over input/async/tick/watch/spinner, input polling pause/resume for external editor, redraw-required flag, dynamic Unicode-width command bar with `more`, fullscreen popups, stateful paragraph with custom reflow/truncation.
- **material crate**: typed color constants convertible to Ratatui `Color`, centered swatch grid, contrast-aware foreground, tiny terminal app loop for typed two-character color lookup and clipboard feedback.
- **mdfried/mdfrier/ratskin**: worker-driven markdown parsing/images, section-height document model, stale parse event filtering by document id, image protocol cache across reparses, resize-triggered reparse, line extras for link/search overlays, Vim-like movement counts/search/link navigation, status-line input queue, markdown theme trait for Ratatui spans.
- **minesweep-rs**: fixed-size centered grid from precomputed row/column constraints, cell view objects encapsulating style/content, aligned board-local help, gauge/info widgets above the grid, arrows+vim input, simple thread-based input/tick channel, Ctrl-C atomic shutdown, and `Clear`ed win/loss overlay.
- **oatmeal**: chat transcript/composer split, minimum-width fallback, bubble line cache invalidated on width/streaming changes, author-aligned bordered bubbles, syntax-highlighted code spans with codeblock numbering, scroll service with scrollbar and bottom-follow semantics, bracketed paste normalization, backend-wait input gating, Ctrl-C abort/double-exit.
- **oxker**: live dashboard with `AppData`/`GuiState` split and `FrameData` snapshots, atomic rerender/clear flags, status-set modal routing, focusable visible panels, rect region maps for mouse hit-testing, optional filter/search bars/logs/charts/ports, configurable colors/keymaps/help, reference-counted spinner animation, external exec terminal release, and extensive TestBackend snapshot/cell assertions.
- **openapi-tui**: page/pane architecture with propagation-controlling event responses, global input modes, footer-as-command-line with history and cursor positioning, fullscreen panes, request built by folding independent request panes, tabbed parameter editor, response viewer modes for formatted/search/jq output, syntax-highlight cache with line numbers, and schema viewer render blocks for refs/composition variants.
- **rainfrog**: SQL workbench with focus enum and last-focused tab/component memory, component trait plus action bus, async DB polling feeding data states/popups, Vim `tui-textarea` SQL editor, dynamic focus hints, custom offscreen-buffer horizontal table scroller, explicit result states, row/cell/copied selection modes, export/yank confirmation popups, and config-parsed key sequences/styles.
- **trippy**: traceroute visualizer with per-frame trace snapshots and freeze mode, preserve-screen exit option, mode-dependent header/tabs-or-flows/body/footer/info-bar layout, body switch among splash/error/table/chart/map, user-configurable columns with fixed/variable widths and settings dialog, selected-hop-aware charts, canvas GeoIP map overlays, privacy-aware labels, centered help/settings overlays, and runtime config info bar.
- **yozefu**: Kafka event-stream browser with stderr-backed TUI, panic restore hook, async render/tick/input loop, component registry with view stack and focus history, root-managed contextual shortcuts, bounded circular record buffer with read/matched metrics, channel-drained records component, cancellation-token consumer/search workers, follow/unfollow mode, async debounced search validation and autocomplete, topic multi-select/filter UI, record detail precomputed highlighted lines, schema detail views, and typed notifications for copy/export/open/errors.

Update this section as more reference apps are studied.
