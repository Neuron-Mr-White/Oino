# Oino Agent Conventions

From today onwards, keep all conventions we know so far in this file so future agents can discover and follow them.

## Project workflow

- Before making decisions, search project memory for relevant Oino context.
- Prefer small, focused changes; avoid unrelated refactors.
- For code changes, run:
  - `cargo fmt --all`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
- Record completed implementation notes under `.unipi/docs/quick-work/` or `.unipi/docs/fix/` as appropriate, then commit with a descriptive message.

## Search, fuzzy matching, and pickers

- All interactive fuzzy/search/autocomplete features in Oino should use the high-level `nucleo` crate.
- Do **not** build new fuzzy matchers by hand and do **not** use low-level `nucleo-matcher` directly for fzf-like UI features.
- Do not rescore large candidate sets inside Ratatui render paths. Cache filtered indices/views in state and refresh them only when the query or candidate list changes.
- Current fuzzy/search users include `/sessions`, model selection search, slash-command suggestions, and `@` file suggestions.
- For future search features, first look for and reuse `crates/oino-tui/src/fuzzy.rs`.

## Ratatui / TUI work

- For any Ratatui, crossterm, terminal UI, chat transcript, table/grid, modal, autocomplete, search bar, markdown viewer, or streaming UI task, first read `.pi/skills/ratatui/SKILL.md` and the referenced subskill files that match the task.
- Keep the TUI as a deterministic state machine with side-effect-light renderers:
  `terminal/input/async source -> Event -> Command/Action -> State update -> render(State, Frame)`.
- Avoid blocking work in render; use state, cached views, channels, or actions for expensive work.
- Keep focus/mode explicit. Key hints should reflect the active focus/mode.
- Render text width-aware and preserve tiny-terminal fallbacks.

## Oino resource layer direction

- Oino should load Oino-owned paths by default rather than silently coping with external conventions such as `~/.agents`, `.agents/`, `.pi/`, `AGENTS.md`, or `CLAUDE.md`.
- Future migration/import commands can copy external conventions into Oino paths, but default loading should stay deterministic and explicit.
- Global resources should live under `~/.oino/`:
  - `SYSTEM.md` — global system prompt, created/populated on launch.
  - `settings.json` — existing global user settings.
  - `skills/<name>/SKILL.md` — global skills.
- Project resources should live under `<project>/.oino/`:
  - `AGENT.md` — project instructions, created/populated on launch.
  - `prompts/*.md` — prompt templates.
  - `skills/<name>/SKILL.md` — project skills.
- Resource/config files and directories should be visible and populated even in blank/default state; do not add new hidden files or folders without documenting them first.
- Prompt templates are one-shot reusable user prompts; skills are on-demand capability folders with `SKILL.md` plus optional scripts/references/assets.
- Resource command UI should label categories explicitly: `[SYS]` for system commands, `[PROMPT]` for prompt templates, and `[SKILL]` for skills.
- Prompt and skill inclusion is explicit: users include resources with `/prompt:<name>` and `/skill:<name>` tokens, and may repeat tokens to combine multiple resources. Do not reintroduce implicit prompt commands such as `/review` or automatic model skill loading by default.
- Bare `/` command suggestions should only trigger at the start of the composer and should list system commands; resource suggestions trigger anywhere only for `/prompt:`, `/skill:`, `/P:`, or `/S:` tokens.
- `/prompts` and `/skills` should open searchable browsers; `/P:<query>` and `/S:<query>` should scope slash suggestions to prompts or skills and complete to `/prompt:<name>` or `/skill:<name>`.

## Oino session behavior

- Local sessions live under `~/.oino/sessions/<uuid>.jsonl`.
- Session files are append-only JSONL: header first, then message/model/thinking/compaction/etc. entries.
- `/new` must be a no-op in a blank/no-input session; after the current session has content, it switches to a fresh lazy session, clears transient TUI state, and should not persist an empty session file until real content is saved.
- `/sessions` must let users browse saved sessions and press Enter to continue one.
- Running `/sessions` from a blank startup state must not create a new empty session file.

## TUI interaction expectations

- Composer remains editable while the model streams.
- Enter during streaming sends steering input.
- `Ctrl-O s` opens settings; `Ctrl-O q` opens the send panel.
- Esc should dismiss transient UI or stop a running response; it should not quit the app.
- Quit requires two Ctrl-C presses.
- Long rows in pickers/browsers should truncate with ellipsis instead of clipping.
