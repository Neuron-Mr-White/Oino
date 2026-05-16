---
name: ratatui
description: Build, redesign, review, or polish Rust Ratatui terminal UIs. Use this skill whenever the user mentions ratatui, crossterm, tui-rs, terminal dashboards, terminal tables, TUIs, CLI apps with interactive screens, keybindings, panes, modals, streaming logs, chat terminals, inspectors, data grids, terminal games, terminal charts/maps, or snapshot-tested terminal widgets. This skill routes to focused Ratatui component subskills; prefer it even when the user asks for “just a small TUI” because terminal UX fails easily without layout, focus, event-loop, and teardown discipline.
---

# Ratatui Craft Skill

Use this skill for any Rust terminal interface built with `ratatui`, `crossterm`, `tui-input`, `tui-textarea`, or adjacent crates.

This skill uses progressive disclosure: read this router first, then load only the component subskill files needed for the user’s task.

## Core working rule

Design the app as a deterministic state machine plus side-effect-light renderers:

```text
terminal/input/async source -> Event -> Command/Action -> State update -> render(State, Frame)
```

Keep terminal lifecycle, event normalization, state mutation, rendering, async effects, and tests in separate modules. If the project already has a TUI architecture, adapt to it rather than replacing it wholesale.

## Start every Ratatui task with this checklist

1. Identify the UI type from the prompt and read the matching reference files below.
2. Find existing project TUI modules (`src/tui`, `ui.rs`, `app.rs`, `state.rs`, `component`, `widgets`).
3. Preserve terminal safety: raw mode, alternate screen, cursor restore, panic/error cleanup.
4. Define explicit focus/mode/view-stack state before writing widgets.
5. Make layouts responsive from `Frame::area()` every frame; include tiny-terminal fallbacks.
6. Render text width-aware (`unicode-width`) and test at narrow widths.
7. Add dynamic key hints/status/footer tied to focus and mode.
8. Avoid blocking work in render; use events/channels for async data.
9. Add `TestBackend`/snapshot tests for core rendering and state-transition tests for keys.

## Which subskill files to read

Read the minimum set that matches the task:

| User task / UI shape | Read these files |
| --- | --- |
| New app architecture, broken event loop, teardown, async broker, snapshot tests | `references/architecture.md`, `references/testing-evals.md` |
| Panes, app shell, modals, popups, splitters, tiny terminal behavior | `references/layouts.md`, `references/input-focus.md` |
| Tables, CSV/grid, DB results, Kafka/log records, horizontal scroll | `references/tables-grids.md`, plus `references/inspectors-explorers.md` for workbenches |
| Keybindings, focus bugs, search bars, command footer, autocomplete, paste | `references/input-focus.md` |
| Streaming output, model/tool chunks, live logs, background scans, redraw jank | `references/streaming-async.md` |
| API explorer, DB workbench, Git client, binary inspector, Kafka browser, file tree | `references/inspectors-explorers.md` |
| ChatGPT/agent terminal, transcript, composer, bubbles, markdown/code in chat | `references/chat-agent.md`, `references/document-viewers.md`, `references/streaming-async.md` |
| Markdown/schema/JSON/body viewer, syntax highlighting, links, images, search | `references/document-viewers.md` |
| Chess/minesweeper/board/fixed-grid terminal game | `references/games-boards.md` |
| Bandwidth/docker/traceroute/system telemetry dashboards, charts/maps | `references/telemetry-dashboards.md`, `references/streaming-async.md` |
| Colors, themes, selected/focused states, palette systems, visual polish | `references/theming-polish.md` |
| Evaluating whether this skill works | `references/testing-evals.md` |

## Source inspiration map

- **Agent/chat shells**: DeepSeek-TUI, OpenAI Codex TUI, oatmeal.
- **Telemetry/live dashboards**: bandwhich, oxker, trippy.
- **Inspectors/workbenches**: binsider, openapi-tui, rainfrog, gitui, yozefu, dua-cli.
- **Dense data grids**: csvlens, rainfrog, yozefu.
- **Pickers/productivity**: fzf-make, eilmeldung, gitui.
- **Games/boards**: chess-tui, minesweep-rs.
- **Documents/schemas/palettes**: mdfried/mdfrier/ratskin, openapi-tui schema viewer, material crate.

## Implementation output format

When implementing or reviewing a Ratatui UI, report:

1. **Reference files used** — list the subskills you loaded.
2. **Architecture choice** — event loop, state model, component boundaries.
3. **UX behaviors** — focus, navigation, resizing, empty/loading/error states, key hints.
4. **Files changed** — paths and purpose.
5. **Validation** — commands run, tests added, snapshots/manual checks.

## Common failure modes to prevent

- Terminal left in raw mode after panic or error.
- Render functions starting network/file/blocking work.
- Focus inferred from widget order instead of explicit state.
- Tables using byte length instead of displayed width.
- Static key hints that lie after mode/focus changes.
- Cached `Rect`s reused after resize without invalidation.
- Streaming views redrawing on every chunk without coalescing.
- Modals that let background panes also consume the same key.
- Good-looking wide layout with no narrow-terminal fallback.
