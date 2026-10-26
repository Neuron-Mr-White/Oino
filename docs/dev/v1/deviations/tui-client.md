# Deviations: tui-client

Owner: `tui-client` (`crates/oino-tui`, `crates/oino-view`). Format per README §5: section,
change, reason, impact. pi 1.4.0 `dist/modes/interactive/*` is the reference where modes.md §7
is silent.

## Contract differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | architecture §2 (`oino-view` deps: `oino-protocol`) | `oino-view` and `oino-tui` depend on `oino-core` (types only: `events`, `ui`), `oino-ai`, `oino-session` and `oino-agent` for wire types. Neither calls any `oino-core` API. | `oino-protocol` does not exist yet; `oino-client` exposes `oino_core` types in its public API. | Switch to `oino-protocol` once it lands (mechanical import change). `cargo xtask layering` rule 5 still holds: no core internals are used. |
| 2 | modes §7.1 (`oino_tui::run(backend, TuiOptions)`) | Signature is `oino_tui::run(backend: Arc<dyn SessionBackend>, host: Arc<dyn TuiHost>, options: TuiOptions)`. | Session listing, settings writes, `keybindings.json`, `@` path completion, clipboard images, renderer calls, panel actions and shortcut invocation have no `SessionBackend` command yet (`_oino/list_sessions`, `_oino/get_settings`/`set_setting`, `_oino/get_keybindings`, `_oino/render`, `_oino/ui_action`, `_oino/invoke_shortcut`, `_oino/editor_event`). `TuiHost` carries them; `oino_tui::host::LocalHost` (feature `local-host`, default) implements them over the local agent dir. | Binary passes `LocalHost::new(cwd, agent_dir, sessions_root, trusted)`. `--remote` works for everything except those host features until the `_oino/*` commands exist; then a backend-driven host replaces `LocalHost`. |
| 3 | modes §7.2 (fullscreen) | The client always runs in the alternate screen with its own scrollback (pi's `tui.altScreen.*` behaviour); pi's inline (non-fullscreen) renderer is not ported. | ratatui owns a full frame; inline rendering with terminal scrollback needs a different renderer. | Transcript scrolling uses `tui.altScreen.*` bindings; terminal scrollback/search is not used. `previousPrompt`/`nextPrompt` scroll by half a page instead of jumping to prompt markers. |
| 4 | ui-protocol §6.1 `image`, pi inline images | Images (user attachments, tool results, OUS `image`) render as `[image: <mime>]` placeholders; no kitty/iTerm/sixel output. Capability `images: false` is declared in `_oino/hello`. | Graphics protocols need per-terminal detection and raw escape output outside ratatui's cell buffer. | Visual only; data is unaffected. Follow-up: kitty graphics via a post-draw escape pass. |
| 5 | ui-protocol §7.2 editor intents | `editor_event` intents (`changed`, `cleared`, `emptyArrowDown`, `submit`, `tokenBackspace`) are not sent. | No backend command (`_oino/editor_event`) yet. | Extensions relying on editor intents (input-shortcuts, command-enchantment) see nothing from the TUI. |
| 6 | ui-protocol §7.3 autocomplete | Completion is local: built-in commands + `get_commands`, and `@`/Tab path completion over one directory level. No `get_completions` merge, no `complete-command` argument completion. | `_oino/get_completions` is not implemented by the backend. | Extension argument completion and `complete-input` providers are not offered. |
| 7 | ui-protocol §6.6 settings hub | `/settings` edits pi core settings (auto-compact, auto-retry, steering/follow-up mode, default thinking level, hide thinking, theme, images, skill commands, double-escape action, tree filter, quiet startup, cache-miss notices) through RPC commands where they exist and `TuiHost::set_setting` (global scope). The extension settings hub (namespaces, scopes, field types) is not rendered. `openSettings{namespace}` opens core settings with a notice. | `_oino/get_settings_schema` and friends are not implemented by the backend. | unipi `/unipi:settings` cannot edit extension settings from the TUI yet. |
| 8 | modes §1.3 built-ins | `/login`, `/logout`, `/trust`, `/extensions` show an explanatory notice; `/reload` reloads keybindings, settings and the command list only; `/changelog` shows `TuiHost::changelog()` (none in `LocalHost`). `/share`, `/bug`, `/llama` are not offered (pi-only services). | No backend commands for OAuth login, trust decisions, extension management or runtime reload (core deviation: no reload host op). | Users edit `auth.json`/`trust.json` or use `oino install`/`list`. |
| 9 | pi `app.model.cycleBackward` | Backward cycling selects the previous entry of `get_available_models` (not the scoped `--models` list). | RPC has only forward `cycle_model`. | With scoped models, backward cycling walks all available models. |
| 10 | pi session selector rename | Rename (`ctrl+r`) works for the current session only (`set_session_name`). | Renaming another file needs a session-file API outside the backend. | Other sessions show "Only the current session can be renamed here". |
| 11 | pi tree navigator | Selecting a different entry always asks "No summary / Summarize / Summarize with custom prompt" before `_oino/navigate_tree`; pi asks only when branch summaries are enabled. Label timestamps show `HH:MM` from the ISO time. | `branchSummary` settings are not exposed to frontends. | One extra confirmation when summaries are disabled. |
| 12 | pi footer | The cache-hit-rate segment (`CH…%`) and subscription marker are omitted; usage comes from `get_session_stats` (refreshed on `message_end`/`agent_settled`). | Needs per-message usage scanning that the backend does not expose. | Footer shows ↑ ↓ R W $ and context %, model, thinking level, routed model. |
| 13 | pi clipboard | Copy uses OSC 52; paste of images/text shells out to `wl-paste`/`xclip`/`pngpaste`/`pbpaste` (in `LocalHost`). Dropped image paths are attached as file tokens (`@path` on submit). | No native clipboard dependency (keeps the crate pure Rust, no system libs). | Terminals without OSC 52 do not receive copies. |
| 14 | pi edit diff | Diffs colour `+`/`-`/context lines; pi's intra-line inverse word highlighting is omitted. | No word-diff dependency in the client. | Visual only. |

## Requests for other work

- **modes** (`crates/oino`): interactive mode is wired by tui-client in `crates/oino/src/modes/interactive.rs`
  (cross-owner edit, commit `[tui-client]`): `InProcessBackend` + `LocalHost`, `has_ui = true` for
  interactive, `Started` carries `sessions_root`/`project_trusted`, `@file` images go to
  `TuiOptions::initial_images`; pty test `crates/oino/tests/interactive.rs`. Still open for modes:
  `--remote` (needs a network `oino_rpc::Transport` and `oino serve`); the preflight still rejects it.
- **modes** (`oino-client` / RPC): implement `_oino/list_sessions`, `_oino/get_settings`/`_oino/set_setting`,
  `_oino/get_keybindings`, `_oino/get_completions`, `_oino/render`, `_oino/ui_action`,
  `_oino/invoke_shortcut`, `_oino/editor_event`, `_oino/get_settings_schema` and the extension
  settings commands, `_oino/extensions`, `_oino/login`/`_oino/logout` (modes §4). The TUI already
  produces the matching intents ([`TuiHost`] methods / `Effect::UiAction` / `Effect::InvokeShortcut`).
- **core-types-session**: `oino-protocol` with `events`/`ui`/`rpc` so frontends drop the `oino-core`
  dependency; a `Deserialize` impl for `SessionTreeNode` (the view parses `get_tree` JSON itself).
- **integration**: delete `legacy/crates/oino-tui` and `legacy/crates/oino-app` (superseded by
  `crates/oino-tui` + `crates/oino-view`; reused pieces: markdown renderer structure, syntect scope
  mapping, composer kill-ring/undo ideas); root `Cargo.toml` members `crates/oino-view`,
  `crates/oino-tui` and workspace dependency `insta` were appended by this item.
