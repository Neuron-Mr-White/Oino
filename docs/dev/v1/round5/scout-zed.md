# Round 5 scout — what Oino can take from Zed

Read-only study of `github.com/zed-industries/zed` for round5-feedback issues
#16/#21, #17, #1, #14, #24. Written 2026-10-29.

- Zed commit studied: `02d47bf64fcece0edc5b849d25e3a60a67d01c83` (2026-10-28),
  shallow clone at `/tmp/zed-src`.
- All `zed:` paths below are relative to that clone; `gpui-pre:` paths refer to the
  local cargo registry copy of Oino's pinned fork (`gpui-pre-0.3.10`).

## 0. Cross-cutting facts

### 0.1 gpui version comparison

| | Oino | Zed |
|---|---|---|
| crate | `gpui-pre =0.3.10` (longbridge fork, pinned exact in `apps/Cargo.toml:15`) | in-tree `crates/gpui`, version `0.2.2` (`crates/gpui/Cargo.toml:3`) |
| license | Apache-2.0 | Apache-2.0 (`crates/gpui/Cargo.toml` `license` field) |
| API parity vs Zed main | `src/elements/list.rs` is line-for-line the same file as Zed's: `FollowMode` (99), `set_follow_mode` (605), `splice` (490), `splice_focusable` (498), `remeasure_items` (411), `set_item_measurement_readiness` (650), `is_following_tail` (656) all present | same |

**Conclusion: the gpui-pre pin already contains every list/virtualization primitive Zed's
agent panel uses.** No gpui bump or fork port is needed for #16/#21. gpui-kit 0.8.1 /
gpui-component 0.8.1 (re-exported as `component`, `gpui-kit/src/lib.rs:170`) are also
Apache-2.0 and upstream-track Zed's UI ideas.

### 0.2 License compatibility (decides "port" vs "reimplement" everywhere)

Per-crate `license` fields in Zed's `crates/*/Cargo.toml`:

- **Apache-2.0 (directly takeable by MIT-licensed Oino):** the whole `gpui` family
  (`gpui`, `gpui_platform`, `gpui_macros`, `gpui_*` platform crates), plus `util`,
  `collections`, `sum_tree`, `path`, `http_client`, `refineable`, `watch`, `zlog`, …
- **GPL-3.0-or-later (NOT directly takeable):** `agent_ui`, `agent`, `acp_thread`,
  `markdown`, `editor`, `language`, `lsp`, `project`, `picker`, `settings_ui`,
  `language_models`, `theme`, `ui`, `workspace` — i.e. **every app-level crate listed
  in this report**.

Oino is `license = "MIT OR Apache-2.0"` (root `Cargo.toml` `[workspace.package]`),
`LICENSE` file = MIT. Copying any GPL-3.0 crate into Oino and distributing the desktop
binary would force the combined work to GPL-3.0. **D4's "take components directly"
is only license-clean for the gpui family; everything else must be reimplemented from
a written understanding of the mechanism, not copy-pasted.** (If the owner ever wants
verbatim GPL ports, that is an explicit relicense decision for the whole app — flagged,
not assumed.)

## 1. #16 + #21 — conversation virtualization & no-bounce scrolling

### Zed references

- `crates/gpui/src/elements/list.rs` — virtualized `list()` (24) + `ListState`:
  `new(count, ListAlignment, overdraw)` (314), `reset` (355), `remeasure` (399),
  `remeasure_items(range)` (411, anchor-preserving `ScrollAnchorMode::Absolute`),
  `splice` (490), `scroll_to_end` (592), `set_follow_mode(FollowMode::Tail)` (605),
  `pause_following_tail` (637), `set_item_measurement_readiness` (650),
  `is_following_tail` (656). `FollowMode::Tail` (99–105): "auto-scroll along with the
  tail, when scrolled to bottom"; user scroll-up internally calls `stop_following`
  (564) — pinning and unpinning are the list's job, not the app's.
- `crates/agent_ui/src/conversation_view.rs:1595-1597` — thread list setup:
  `ListState::new(0, ListAlignment::Top, px(2048.0))` (2048 px overdraw),
  `set_follow_mode(Tail)`, `set_item_measurement_readiness(...)` (an item reports
  itself not-ready while its backing data is still loading, so the list never measures
  a half-built row and never jumps because of it).
- Event → list-op mapping in the thread subscription:
  `NewEntry` → `view_state.sync_entry` + `list_state.splice_focusable(index..index, …)`
  (1987–2007); `EntryUpdated(ix)` (streaming token!) → `sync_entry(ix)` +
  `remeasure_items(ix..ix+1)` only (2009–2028); `EntriesRemoved(range)` →
  `splice(range, 0)` (2030–2036); turn `Stopped` → `if is_following_tail()
  { scroll_to_end() }` (2087–2097).
- `crates/agent_ui/src/conversation_view/thread_view.rs` — `list(list_state, …)`
  with `.with_sizing_behavior(ListSizingBehavior::Auto)` (7186–7202); the "generating"
  indicator is a *spliced extra list item*, not a fixed footer
  (`sync_generating_indicator`, 8408–8424); expanding a compaction block pauses tail
  following so reading isn't yanked to the bottom (5037–5056); scroll position is
  persisted per thread via `list_state.logical_scroll_top()` → `set_ui_scroll_position`
  (1328–1335); scroll-to-top/bottom affordances (7897–7915, 8119–8152). Horizontal
  code-block scroll keeps vertical wheel events for the outer scroller via
  `restrict_scroll_to_axis()` (498–514; gpui-pre has the same API,
  `gpui-pre:src/div.rs:1590`).

### Mechanism in short

One virtualized `list()` per thread view; every thread event is translated into the
minimal list mutation (`splice` for inserts/removals, `remeasure_items` for a changed
row, `reset` only on load). Tail-following is built into `ListState`: new content at
the end auto-scrolls only while the user is at the bottom, and any upward scroll
breaks the follow, so streaming can never fight the user. Row heights are only ever
remeasured for rows whose content changed, with the scroll anchor preserved — that is
the no-bounce guarantee.

### Oino status and recommendation

Oino already uses the same primitives: `apps/oino-ui/src/views/session_pane.rs:476`
(`set_follow_mode(FollowMode::Tail)`), `:903-909` (`splice`/`remeasure_items` per
`RowChange`), `:875`/`:1162` (targeted remeasures), and `components/keys.rs:116`,
`components/query_rail.rs:61` already `pause_following_tail()` on interactions.
Remaining bounce sources to audit, Zed-style:

1. Full-list `self.list.remeasure()` during live turns — `session_pane.rs:1202`
   (checkpoint load completion) and `:596` (settings change; that one is fine, the
   checkpoint one can land mid-stream). Replace with `remeasure_items` on the affected
   rows, or defer until the turn ends.
2. No `set_item_measurement_readiness` equivalent — rows that render async content
   (markdown, checkpoints) measure at a provisional height then grow. Either gate
   readiness like Zed (1597) or reserve stable row heights.
3. Unipi-style "generating" strip: make it a spliced item (Zed 8408–8424) so it never
   introduces a layout discontinuity at the viewport edge.

**Recommendation: reimplement (already 80 % done) — copy Zed's event→list-op
discipline, not code.** Effort: 1–2 days audit + fixes (#21), plus a `ti` GUI
scroll test pass.

## 2. #17 — markdown rendering colors

### Zed references

- `crates/markdown/src/markdown.rs` (9,488 lines, **GPL-3.0-or-later**) — one
  `Markdown` entity per message; agent_ui creates them per entry
  (`conversation_view.rs:980, 1775, 2509`; `thread_view.rs` `Markdown::new_with_options`
  with `MarkdownOptions { defer_parsing }`). Colors come from Zed's `SyntaxTheme`
  (`theme::SyntaxTheme` import, markdown.rs:78) applied as `TextStyleRefinement`s per
  token/block type (headings, quotes with `BlockQuoteKindColors`, links via
  `LinkStyleCallback`, markdown.rs:68-80), and code via `language` tree-sitter
  highlights. Deps pull `language`, `theme`, `ui`, `sum_tree`, `mermaid_render`,
  `html5ever` — i.e. most of Zed's stack.

### Mechanism in short

Parse (pulldown-cmark, deferred/async) → walk blocks → style every element from a
theme-color table keyed by markdown role, falling back to syntax themes for code.

### Oino recommendation

Do **not** port the crate (GPL + heavy deps, and Oino already renders markdown).
Oino uses gpui-kit `TextViewState::markdown` (`apps/oino-ui/src/views/session_pane.rs:1013`;
gpui-component `src/text/compat.rs:68`, Apache-2.0). The owner's ask is to **copy
unipi's markdown color tokens** (round5 #17: "copy them") into Oino's theme — land
them in `apps/oino-ui/src/design/theme.rs` as a markdown palette (heading/code/quote/
link/table colors) and map them onto TextView's styling surface (gpui-component
`TextViewPlugin`s / theme tokens in `gpui-component:src/text/style.rs`). Where the
unipi tokens live in `/home/oi/Projects/Personal/archived/unipi` was not located in
this scout (quick grep found no obvious markdown css/ts token file) — derive them from
the owner's screenshots or ask the owner for the file. **Copy-adapt colors only.**
Effort: 0.5–1 day for the palette; +1 day if code-block copy buttons / heading anchors
are wanted via a custom TextView plugin.

## 3. #1 — model picker (modal, search, pins, full provider/model names)

### Zed references

- `crates/picker/src/picker.rs:105-108` — `PickerStyle::Modal { resizable }`
  ("self-contained modal: draws its own elevated background and dismisses on
  click-out"), plus `.popover()` / `.embedded()` styles; fuzzy matching via
  `fuzzy_nucleo`; `persistence.rs` remembers geometry.
- `crates/agent_ui/src/language_model_selector.rs` (830 lines, GPL) — `Picker`
  delegate with entries grouped `Favorite` → `Recommended` → per-provider separators
  (`GroupedModels::entries`, 242–300); search candidates include the full
  `"provider/model"` string (378) so typing `anth/claude` matches; each row is a
  `ModelSelectorListItem` with provider icon, model name, cost info, selected state
  and a **favorite star toggle** (504–541); favorites are read from
  `AgentSettings.favorite_models` (63–95).
- `crates/agent_ui/src/favorite_models.rs:8-33` — pinning = write/remove the model
  selection in the settings file (`agent.add_favorite_model`), so pins persist.
- `crates/agent_ui/src/model_search.rs:8-31` — async two-tier fuzzy search (name
  matches ranked above metadata matches, metadata only *expands* results).

### Mechanism in short

A modal list picker whose delegate (a) orders entries favorites-first with section
separators, (b) fuzzy-searches over `name` + `provider/name` metadata off-thread, and
(c) persists pins as settings entries toggled from each row.

### Oino recommendation

**Reimplement in Oino on existing MIT-friendly parts** (Zed's picker/modal code is
GPL): reuse Oino's palette component (`apps/oino-ui/src/components/palette.rs`,
uniform_list) inside a real modal (gpui-component dialog/sheet), with: provider/model
full display strings from `oino-models`, favorites-first sections + per-provider
separators, prefix/fuzzy search (start with substring match; `fuzzy-matcher`/`nucleo`
if needed), a pin star per row persisted in `oino-settings` (new `favorite_models`
key), and "custom lead/sidekick" selection support (round5 #1). Current state to
replace: provider-submenu overlay in `session_pane.rs:2611-2618` +
`settings_page.rs:726-768`. **Reimplement.** Effort: 2–4 days.

## 4. #14 — settings UI layout (ZCode-style, roomy)

### Zed references

- `crates/settings_ui/src/settings_ui.rs` (7,799 lines, GPL): `SettingsWindow`
  (953) = title bar + left navbar (`navbar_entries`, `UniformListScrollHandle`, 962-966)
  + a searchable content column (`search_bar: Entity<Editor>`, 969; precomputed
  `filter_table` + `search_index` hide non-matching items, 975-982) + pages composed of
  focusable sections. Scope switching is file-based: `SettingsUiFile::{User, Project,
  Server}` (1725-1730) with `setting_type()` labels (1733-1741) and project names from
  `worktree_root_dirs: HashMap<WorktreeId, String>` (957, filled 2594-2640) — this is
  exactly round5 #19's "Project tab must label which project".
- Reusable controls live in `crates/settings_ui/src/components/` (dropdown,
  input_field, number_field, theme pickers, section_items).

### Mechanism in short

Declarative page schema → navbar + sections; a search editor filters a boolean
`filter_table` over all settings items; project settings are just a second settings
file whose navbar entry carries the worktree name.

### Oino recommendation

Do not port (GPL, deeply Zed-wired). Oino already ships the framework equivalent:
gpui-component's `setting` module (Apache-2.0) — `Settings` with resizable sidebar +
pages (`src/setting/settings.rs:34-108`), `SettingPage` (`page.rs:22`), `SettingGroup`
(`group.rs:18`), typed fields under `src/setting/fields/`. Migrate
`apps/oino-ui/src/views/settings_page.rs` (1,857 lines) onto it, add a search field
that filters groups (Zed's filter_table idea), and label the Project tab with the
active project name (Oino session/project state; fixes #19/#22 leftovers). **Reimplement
on gpui-component `setting`.** Effort: 3–5 days including every setting re-verified
working (round5 #22).

## 5. #24 — built-in files view / editor with LSP + formatting

### Zed references and weight

- Sizes (this clone): `crates/editor` **190,699 LOC / 72 workspace deps**;
  `crates/project` **85,885 LOC / 75 deps**; `crates/language` 28,403 LOC / 50 deps;
  `crates/lsp` 2,944 LOC / 17 deps. `editor` + `project` pull `client`, `rpc` (collab
  protocol), `dap`, `node_runtime` (prettier), `remote`, `terminal`, `worktree`,
  `entity_db`, `git`, `telemetry` — the whole Zed runtime. All GPL-3.0-or-later.
- Coupling: the editor is built on Zed's `project`/`worktree`/`buffer` entity graph
  and Zed's gpui `workspace` patterns; there is no standalone "editor widget" seam.
  Zed itself extracts none of these as a library (only gpui + utils are published).

### What it would take

- **Take Zed's editor: not viable** — GPL (forces Oino to GPL), ~300k LOC transitive,
  months of decoupling from `project`/`workspace`. Reject.
- **Lighter path (recommended):** Oino already depends on gpui-component 0.8.1
  (Apache-2.0), whose `input` module is a real editing surface: multi-line
  `textarea.rs`, `editor.rs`, syntax highlighting via `highlighter/`
  (`syntect_backend.rs`, tree-sitter feature, `syntax_context.rs`), in-editor search
  (`input/search.rs`), completion/hover popovers (`input/popovers/completion_menu.rs`,
  `hover_popover.rs`). Add: (1) a thin LSP client over JSON-RPC/stdio using
  `lsp-types` (MIT/Apache) — didOpen/change, publishDiagnostics → gutter/underline
  decoration, hover, format-on-save; (2) external formatters (`rustfmt`, `prettier -w
  --stdin`) reusing Oino's existing subprocess harness; (3) a files tree view backed by
  Oino's fs APIs. The awesome-gpui ecosystem has no materially better maintained
  editor widget than gpui-component's input today (gpui_editor etc. are smaller/less
  maintained).
- **Reimplement.** Effort: read-only files view + editor + diagnostics + format:
  ~1–2 weeks; hover/completions and editing polish: +1–2 weeks. Phase it: files view +
  syntax-highlighted read-only first (2–3 days), then editing + LSP.

## 6. Summary table

| Issue | Mechanism source (Zed) | Recommendation | Effort |
|---|---|---|---|
| #16/#21 | gpui `ListState` follow-tail + splice/remeasure discipline | reimplement on gpui-pre (already mostly present); audit full `remeasure()` and readiness gating | 1–2 d |
| #17 | `crates/markdown` theme-color table | copy-adapt unipi color tokens into Oino theme + TextView plugin | 0.5–1 d |
| #1 | `picker` modal + `language_model_selector` (favorites/pinned, provider/model search) | reimplement on Oino palette + modal; pins in oino-settings | 2–4 d |
| #14 | `settings_ui` navbar+search+scope labels | reimplement on gpui-component `setting` framework | 3–5 d |
| #24 | `editor`/`project`/`language`/`lsp` | do NOT embed (GPL + weight); gpui-component input + own thin LSP client + external formatters | 1–2 wk (phase 1: 2–3 d) |
| gpui | `crates/gpui` 0.2.2 vs `gpui-pre` 0.3.10 | no action: API parity verified for the relevant subsystems | — |

## 7. Open questions for the owner

1. License: confirm "learn-and-reimplement only" for GPL crates (everything except
   gpui/gpui-kit/gpui-component), or explicitly relicense Oino to GPL-3.0 to allow
   verbatim ports (affects #24 most).
2. #17: which file defines unipi's markdown colors (screenshots unavailable to
   agents) — or approve deriving the palette from ZCode/unipi visuals.
