# Ratatui architecture subskill

Use for app structure, terminal lifecycle, event loops, reducers, component boundaries, async draw scheduling, child-process handoff, and testing seams.

## Source inspirations
DeepSeek-TUI (`crates/tui/src/tui/ui.rs`, `event_broker.rs`, `frame_rate_limiter.rs`), Codex TUI (`tui.rs`, `tui/frame_requester.rs`, `app_event.rs`, `app.rs`), binsider (`state.rs`, `command.rs`), openapi-tui, rainfrog, gitui, yozefu.

## Related references

- `input-focus.md`
- `streaming-async.md`
- `layouts.md`
- `testing-evals.md`

## State/event/render model

Design the app so every visible change flows through a small number of paths:

```text
crossterm/EventStream + async workers + ticks
  -> TuiEvent/Event
  -> AppEvent/Action/Command
  -> state/component update
  -> draw request
  -> render(state snapshot, frame)
```

Use this file layout unless the project already has an equivalent:

```text
src/tui/
  mod.rs or tui.rs       # Terminal wrapper, init/exit, event stream
  event.rs               # Event enum and event source
  action.rs/command.rs   # normalized user/app actions
  app.rs/state.rs        # reducers and app state
  component/ or widgets/ # components with update/draw/shortcuts
  ui.rs/render.rs        # top-level layout only
```

## Terminal lifecycle

- Put raw mode, alternate screen, mouse, bracketed paste, focus-change reporting, keyboard-enhancement flags, cursor hide/show, and terminal title in one guard type.
- Restore terminal in `Drop` and in a panic hook. DeepSeek explicitly resets origin/scroll region and batches terminal writes with DEC synchronized update to reduce flicker; Codex restores terminal before external editors and re-enters after.
- If the TUI should coexist with stdout output, use `CrosstermBackend<std::io::Stderr>` like yozefu; otherwise stdout is fine.
- Provide `pause_events()` / `resume_events()` or drop/recreate the crossterm `EventStream` before launching subprocesses, external editors, or shell execs. Codex uses an `EventBroker`; oxker releases/reinitializes around docker exec.

## Event loops

### Sync poll loop
Use for simple inspectors/games:
- `event::poll(timeout)` + `event::read()`.
- Emit `Tick` if no event arrives.
- Filter key events to `KeyEventKind::Press` for Windows compatibility.

### Async stream loop
Use for API/DB/Kafka/agent tools:
- `crossterm::event::EventStream` plus `tokio::select!` over input, tick interval, render interval, worker channels, and cancellation token.
- Convert raw crossterm events to app events immediately (`Key`, `Mouse`, `Resize`, `Paste`, `FocusGained`, `FocusLost`, `Tick`, `Render`).
- In app loop, drain pending actions after each terminal event (`while let Ok(action) = rx.try_recv()`).

### Coalesced frame requester
Use for streaming text/animations:
- Give widgets/background tasks a cloneable `FrameRequester`.
- Requests send deadlines to a scheduler actor.
- Scheduler clamps to a max FPS and coalesces many requests before broadcasting one `Draw` event. Codex clamps draw notifications to ~120 FPS.
- Use `schedule_frame_in(duration)` for animations/spinners instead of sleeping in render.

## Component trait shape

For large apps, use component boundaries like yozefu/gitui/openapi-tui:

```rust
trait Component {
    fn id(&self) -> ComponentName;
    fn register_action_handler(&mut self, tx: Sender<Action>) {}
    fn handle_events(&mut self, event: Option<Event>) -> Result<Option<Action>>;
    fn update(&mut self, action: Action) -> Result<Option<Action>>;
    fn draw(&mut self, frame: &mut Frame, area: Rect, state: &State) -> Result<()>;
    fn shortcuts(&self) -> Vec<Shortcut> { vec![] }
}
```

Rules:
- Components emit actions; root/app owns cross-component consequences.
- Components can be in a registry (`HashMap<ComponentName, Arc<Mutex<dyn Component>>>`) when view stacks need dynamic dispatch.
- Root broadcasts actions to all components only after handling app-level effects.

## Reducers and commands

Binsider shows a clean command reducer: `Command::from(KeyEvent)` maps keys/mouse to semantic commands; `State::run_command(command, sender)` handles mutation and async event requests. Copy that for small/medium apps. For larger apps use `Action` enums but keep the same principle: key handling produces intent; state update applies intent.

## Resize discipline

- Treat resize as a first-class event, not as a side effect.
- Clear region maps and cached image/preview placement on resize.
- Recompute all layout from `frame.area()` every draw.
- If transcript/document wrapping depends on width, mark reflow required and rebuild from source text rather than from already-wrapped lines (Codex resize reflow pattern).

## Testing checks

- Unit-test command conversion and reducers.
- Use `TestBackend` to snapshot `draw` at wide, narrow, and tiny sizes.
- Test terminal guards via abstraction where possible: cleanup should run on normal exit and error path.
- For frame requester, test that multiple immediate requests produce one draw and delayed requests respect time.


## Large component app pump (gitui)

When a TUI grows beyond a few panes, use a pump instead of ad-hoc routing:

- `Component::event(&Event) -> EventState` returns consumed/not-consumed.
- `event_pump(event, components)` iterates children in priority order and stops at first consumed event.
- `Component::commands(out, force_all) -> CommandBlocking` lets command bar/help collection stop at the active visible component unless `force_all` is requested.
- Parents explicitly enumerate child components (gitui uses accessor macros). This is repetitive, but it prevents hidden magic and makes ownership/lifetimes clearer.
- Queue internal events/actions after input handling; process queue with flags like `NeedsUpdate::COMMANDS` so command bar updates are batched.


## Tiny-width guard for rich widgets

Rich UIs with borders/bubbles/code blocks should check minimum viable size before rendering. Oatmeal computes whether author name + bubble padding + border elements fit in the current width; if not, it renders a simple message. Apply the same guard to dashboards, grids, and board games: a graceful tiny-state is better than broken box drawing or panics from subtraction underflow.
