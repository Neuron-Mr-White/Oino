# Round 5 scout — GPUI component sources for D4 ("scout real implementations")

Date: 2026-10-29 · Branch: `v1-alpha` · Read-only scout, nothing modified.

Method: inspected the crates **already in Oino's apps lockfile** (`apps/Cargo.lock`,
registry sources under `~/.cargo/registry/src/`), the `awesome-gpui` catalog
(`zed-industries/awesome-gpui` `projects.json`, 100+ entries), and the upstream
`longbridge/gpui-kit` repo (examples, releases, license). GitHub star/license/push
numbers fetched via the GitHub API on 2026-10-29.

## 0. Headline finding: Oino already vendors 95% of what it needs

`apps/Cargo.toml` pins:

| Crate | Pin | What it actually contains |
|---|---|---|
| `gpui` (= `gpui-pre`) | `=0.3.10` | Zed's GPUI, crates.io republish (Apache-2.0). Includes `spring.rs` (spring physics) and `elements/animation.rs` (easings, `SpringAnimationElement`). |
| `gpui-kit` | `=0.8.1`, features `component,assets,syntect` | Umbrella crate; its `component` feature pulls in **the full `gpui-component` 0.8.1** (60+ components) and `gpui-base` 0.8.1 (editor engine, LSP provider traits, motion system, tree, dock). Apache-2.0. |
| `gpui-mobile` (`gpui-pre-mobile`) | vendored `apps/vendor/gpui-mobile` (longbridge/gpui-mobile @ `c24a130`) | Mobile platform. Tri-licensed GPL/AGPL/Apache — Oino uses it under **Apache-2.0** (`apps/vendor/gpui-mobile/VENDOR.md`). |

The decisive fact: **`gpui-kit`'s `component` feature = `dep:gpui-component`**
(registry `gpui-kit-0.8.1/Cargo.toml` `[features]`). So the entire component library,
the LSP-aware code-editor engine, the tree, the dock, the settings framework and the
motion system are **already compiled into `oino-ui`** at the exact pinned GPUI version —
Oino just barely uses them (grep of `apps/oino-ui/src` + `apps/oino-desktop/src`: only
`input`, `text`, `highlighter`, `Sizable` are imported today). Every candidate below in
§2 costs zero version-compatibility risk: it is the same crates, same source tree,
already in `apps/Cargo.lock` (incl. `lsp-types 0.97.0`, lock line ~4662).

## 1. Version compatibility baseline

- Oino GPUI: `gpui-pre =0.3.10` (gpui-kit's `gpui-pre` alias). `gpui-component 0.8.1`
  declares `gpui = { version = "=0.3.10", package = "gpui-pre" }` — exact match, no drift.
- `gpui-kit` upstream renamed from `longbridge/gpui-component` → `longbridge/gpui-kit`;
  awesome-gpui still lists the old URL (redirects). 18,735★, pushed 2026-10-29,
  releases `v0.8.0` 2026-10-19, `v0.8.1` 2026-10-26. Very mature and fast-moving.
- License: repo ships `LICENSE-APACHE` (Apache-2.0); crates.io metadata: Apache-2.0.
  Charter-compatible.
- One gap: `gpui-kit` does **not** pass through `gpui-component`'s `tree-sitter` feature
  (only `syntect`, `speech`, …). To enable tree-sitter grammars, depend on
  `gpui-component = "=0.8.1"` directly (same crate, same version, already resolved).

## 2. Candidates per issue

### #24 — Code editor + simple LSP (diagnostics, hover, completion) + formatting + file tree

**Primary: the editor engine already at Oino's pin — `gpui-base 0.8.1`, surfaced via
`gpui-component`'s `input`/`highlighter`/`tree`/`dock`.**

`gpui-pre`/`gpui-base` are the Longbridge packaging of Zed's editor stack; the engine
lives in registry `gpui-base-0.8.1/src/input/`:

- `editor/` — `mod.rs` (`Editor`, `EditorState`, hover popovers, inline completion),
  `language.rs` (`LanguageProvider` trait), `language_config.rs` (`LanguageConfig` with
  `brackets`, `auto_closing_pairs`, `indentation_rules`), `indent.rs`, `auto_close.rs`,
  `search.rs` (in-editor search), `diagnostics.rs` (push-based `DiagnosticSet`,
  `DiagnosticEntry`, squiggles + diagnostic popover; `base/state.rs:1167-1172`
  `diagnostics()/diagnostics_mut()`), `display_map/` (folds, wraps).
- `editor/lsp/` — provider traits, all returning `Task<Result<…>>` so a real async LSP
  client plugs straight in: `mod.rs` (`Lsp` struct: `completion_provider`,
  `code_action_providers`, `hover_provider`, `definition_provider`,
  `document_color_provider`, `semantic_tokens_provider`, `show_document` hook;
  `apply_lsp_edits()`), `completions.rs` (`CompletionProvider` + label highlighters
  `highlight_prefix/contains/characters`), `hover.rs` (`HoverProvider`),
  `code_actions.rs`, `definitions.rs`, `document_colors.rs`, `semantic_tokens.rs`.
- `highlighter/` (in `gpui-component`) — `registry.rs` (`LanguageRegistry`, singleton),
  `syntect_backend.rs` (**already enabled** via gpui-kit's `syntect` feature),
  `languages.rs` (~60 language ids incl. rust, python, ts/js, go, bash, json, md),
  `diagnostic_styles.rs` (theme-aware error/warn/info/hint colors),
  `language_config.rs`. Optional `tree-sitter` feature adds real grammars (per-language
  features `tree-sitter-rust`, `tree-sitter-python`, … in gpui-component's Cargo.toml).

**Reference implementation to copy: `longbridge/gpui-kit` `examples/editor/src/main.rs`**
(1,208 lines, `cargo run -p example-editor`). It wires an `ExampleLspStore` implementing
`CompletionProvider` / `HoverProvider` / `CodeActionProvider` / `DefinitionProvider` /
`DocumentColorProvider` with `lsp_types` types, registers languages on
`LanguageRegistry::singleton()`, and applies text edits with
`state.apply_lsp_edits(&text_edits, window, cx)` (line 312) — which is also exactly the
**formatting** path: implement `textDocument/formatting` (or shell out to rustfmt /
prettier / gofmt → whole-document TextEdit) and feed it to `apply_lsp_edits`. There is
no built-in format command in the engine; that glue is Oino's to write (~small).

**Missing piece — the LSP transport.** The engine has no JSON-RPC client; the example
uses fixtures. Options (all GPUI-agnostic, feed the provider traits):
- Hand-rolled stdio JSON-RPC over Oino's existing tokio runtime in `oino-core`/`oino-ui`
  (~300–500 LOC, matches "simple LSP", no new deps beyond `lsp-types` — **already at
  0.97.0 in `apps/Cargo.lock`**, the version gpui-base requires, `features=["proposed"]`).
- `async-lsp` 0.2.4 (1.9M dl), `tower-lsp-server` 0.24.0-rc (3.0M dl),
  `lsp-server` 0.10 (rust-analyzer's, sync/threaded, 17.7M dl). Any works; hand-rolled
  keeps the dependency surface small and the Charter's "beautiful Rust" bar.

**File tree:** `gpui-base-0.8.1/src/tree.rs` — `TreeItem`/`TreeState` + `tree()` element
(re-exported through `gpui-component::tree`), with `context_menu` hook and scroll bounce.
App supplies lazy directory loading. For a Zed-like workbench shell, `dock/` exists in
both gpui-base (engine: `gpui_base::dock::Panel`) and gpui-component (`Dock`, `TabPanel`,
`Panel` trait; `examples/dock/`, `examples/js_dock/` upstream) — panels are draggable
between docks with drag ghosts, i.e. a working drag-drop reference for the kanban too.
Oino can adopt just `tree` + its own shell if the full dock is too much.

**Zed itself (D4):** `zed-industries/zed` (92,349★) — the *app* crates (`editor`,
`project_panel`, `lsp`) are **GPL-3.0-or-later** (repo license "Other"/NOASSERTION =
GPL-3.0+ with Apache exceptions for framework crates like `gpui`). Do **not** copy code
out of them into MIT/Apache Oino; use as UX reference only. The Longbridge engine above
*is* the Apache-2.0 extraction of that stack.

**Verdict:** use the in-pin engine + port `examples/editor` glue. No new upstream repo.

### #8 — Kanban drag-and-drop, animated reflow, collapsing columns

No mature GPUI kanban exists in awesome-gpui (checked all 100+ entries; the only
"KanbanBoard" is on **gpui-box** (`fran0220/gpui-box`, MIT, **12★**, pushed 2026-10-20 —
immature, unknown gpui pin; docs site `gpui-kit.origingame.dev` did not render via
curl). Not recommended as a dependency.

What to build from (all already available):
- **Drag primitives** in `gpui-pre-0.3.10/src/elements/div.rs`: `on_drag` (line 639 —
  takes a drag value `T` + ghost-view constructor), `on_drop` (592), `on_drag_move` +
  `DragMoveEvent` (67/360 — gives live cursor position for insert-line and column
  highlighting). Same primitives Zed's project panel/tabs use.
- **Motion for the reflow/collapse** (D2): `gpui-base-0.8.1/src/motion/` — `Transition`
  / `transition()` (`motion.rs:112,268`), `Easing` (`cubic_bezier`, `steps`,
  `linear_stops`, `motion/easing.rs:114-185`), `Keyframes`, `Presence` (enter/exit
  lifecycle, `presence.rs:28`), `MotionReveal` (clip reveal, `reveal.rs:7`),
  `Sequence`, `Stagger`, `Timing` (delay/iterations/direction). Plus
  `gpui-pre-0.3.10/src/spring.rs` (`SpringConfig`, `SpringState::step`/`step_ramp`,
  `SpringTarget`, `settle_time`) and `SpringAnimationElement`
  (`elements/animation.rs:169`). Column-collapse = animate lane width to a strip with
  `Easing::cubic_bezier(0.4, 0, 0.2, 1)`-style timing; card reflow = `Presence` +
  stagger.
- **Behaviour spec to port:** unipi board (`~/Projects/Personal/archived/unipi/crates/
  kanboard/web/src/Board.tsx`) — HTML5 DnD with midpoint insert-line test (lines
  179-182), edge auto-scroll while dragging (114-156), `is-dragging` board state, and
  **disallowed lanes collapse to a strip** (`styles.css:2786-2804` — exactly feedback
  #8's "non-related columns collapse while dragging"), `drop-ok`/`drop-bad` lane
  feedback. Oino's current `apps/oino-ui/src/views/board.rs` (912 lines) has no drag
  at all — this is a build, not a fix, but every primitive is in-pin.

### #1 — Modal pickers with search (+ Devin-style pinning)

- `gpui-component-0.8.1/src/command/` — command-palette modal: `Command` +
  `CommandState`, `searchable(bool)`, `filterable(bool)`, `on_query` (async-capable),
  `on_select`/`on_confirm`/`on_cancel`, groups/separators, `placeholder`, `header`/
  `footer`, `max_h`, scroll bounce. This is the unipi-style modal picker (#1) ready to
  mount.
- `src/searchable_list/` — `SearchableListDelegate` (sections, items, filtering;
  `delegate.rs:54`), `SearchableVec`/`SearchableGroup` (`vec.rs:76,184`) — reusable
  filtering data structures for any custom picker.
- `src/combobox.rs` — `ComboboxState<D: SearchableListDelegate>`, `searchable(true)`,
  single/multiple selection (line 287-293).
- Modal shell: `src/dialog/dialog.rs` (`Dialog` — "A modal to display content in a
  dialog box", trigger/content builders) with entrance/exit animation
  (`src/dialog/entrance.rs`); `src/sheet.rs` (`Sheet`) for the mobile bottom-sheet
  variant (Oino mobile already uses sheets).
- **Pinning** (learn from Devin): app-level — sort `CommandItem`s / delegate items by a
  pinned-models set in settings before render. No library support needed.

### #16 — Virtual lists (conversation, long file lists, tables)

- Core: `gpui-pre` `uniform_list` / `list` — Oino already uses `list()` for the
  conversation (`apps/oino-ui/src/components/conversation.rs:627`), so row
  virtualization exists; what's missing is chat-app ergonomics.
- `gpui-component-0.8.1/src/message_scroller.rs` — `MessageScrollerState`/`MessageScroller`:
  a chat-specific virtualized scroller over gpui `ListState` with **follow-tail**
  (`is_following_tail`, `FollowMode`), `splice`/`append`/`prepend`/`remeasure` for
  streaming updates, 400px overdraw, bottom-fade + jump-to-latest button with 200ms
  transitions (`JUMP_BUTTON_TRANSITION`, `BOTTOM_FADE_TRANSITION`). Direct answer to
  #16 and to #21 ("chat bounces repeatedly" — follow-tail + remeasure-on-splice is the
  fix pattern; Zed's agent panel does the same thing).
- `virtual_list.rs` — `VirtualList`, `VirtualListScrollHandle`, `v_virtual_list`,
  `h_virtual_list` (re-exported from `gpui_base`) for variable-height grids/2-D cases
  (board columns with many cards).

### D2 — Animations (ZCode-like motion)

Already inventoried under #8; summary: three layers, all in-pin —
1. `gpui-pre` `Animation` + easings (`ease_in_out`, `ease_out_quint`, `bounce`,
   `pulsating_between`; `elements/animation.rs:506-542`), `with_animation`, repeat/sync;
   spring physics (`spring.rs`).
2. `gpui-base` `motion` module — the ZCode/CSS-style system: `Easing` (cubic-bezier),
   `Keyframes`, `Transition`, `Presence` (mount/unmount), `MotionReveal`, `Sequence`,
   `Stagger`, `Timing` (delay/repeat/direction). This is the right vocabulary for a
   design-tokens motion spec (duration/easing pairs per interaction).
3. `gpui-component` ready-made motion: `shimmer.rs`, `skeleton.rs`, dialog
   `entrance.rs`, scroll bounce on `tree`/`command`, `message_scroller` transitions.

### #14 — Settings forms (ZCode layout)

- `gpui-component-0.8.1/src/setting/` — `Settings` (settings.rs:34) with resizable
  sidebar (`sidebar_width`, `sidebar_size_range`), pages (`SettingPage`, `page.rs`),
  groups (`group.rs`, `GroupBoxVariant`), items (`item.rs`), and typed fields
  (`fields/`: bool toggle, string, number, dropdown, raw element) — a two-pane
  ZCode-style settings window. Replaces Oino's hand-rolled
  `apps/oino-ui/src/views/settings_page.rs` (1,857 lines) incrementally; Global/Project
  tabs (#19) become two `SettingPage`s with the project named in the sidebar entry.
- `src/form/` — `Form` + `Field` (label/description/visible, horizontal/vertical
  layouts, label width, columns) for the kanboard modal (#9) and any config dialogs.
- Optional sugar: **`gpui-form`** (`stayhydated/gpui-form`, Apache-2.0, 26★, pushed
  2026-10-25) — derive macro generating gpui-component form fields from a struct.
  Small and new; evaluate only after the plain builder API feels verbose.

## 3. Other awesome-gpui libraries — considered and parked

| Repo | License | Stars / last push | Assessment for Oino |
|---|---|---|---|
| `wess/guise` | MIT | 140 / 2026-10-27 | Mantine-inspired set; active but a **second design system** on top of gpui-component's theme. Park. |
| `Augani/adabraka-ui` | MIT | 482 / 2026-02-19 | Decent size, but last push ~8 months ago; gpui pin unclear. Park. |
| `crabtalk/bezel` | MIT | 193 / 2026-10-28 | SwiftUI-lean, env-based styling; interesting architecture, immature. Watch. |
| `ELY-Open-Source/Ely-GPUI-Components` | MIT | 111 / 2026-10-29 | Active, light/dark themes, WASM gallery; overlaps gpui-component. Park. |
| `LukeTandjung/base-gpui` | MIT | 93 / 2026-09-29 | Headless primitives (Base UI port); could inform a11y/behavior of custom widgets. Park. |
| `pacifio/gpui-flow`, `tu6ge/ferrum-flow` | — | — | Node editors; not on the roadmap. |
| `zed-industries/create-gpui-app`, `joris-gallot/gpui-devtools` | — | — | Dev tooling; devtools may help GUI-driver debugging later. |
| `zed-industries/zed` app crates | GPL-3.0+ | 92,349 | **Reference only, no code copying** (license). |

GPUI-CE (community GPUI distribution) is irrelevant while the `gpui-pre` pin works.

## 4. Recommended integration map (no code changed by this scout)

| Issue | Take | From (already in `apps/Cargo.lock` unless noted) |
|---|---|---|
| #24 editor | `input::Editor`/`EditorState` + `highlighter` (syntect now; tree-sitter via direct `gpui-component =0.8.1` dep), `tree` file pane, hand-rolled stdio LSP client feeding the provider traits, `apply_lsp_edits` for format/code-actions | gpui-base 0.8.1 + port of `longbridge/gpui-kit examples/editor/src/main.rs` (fetch from upstream) |
| #24 layout | `tree` + optional `dock` (`examples/dock`) | gpui-base/gpui-component 0.8.1 |
| #8 kanban | `on_drag`/`on_drag_move`/`on_drop` + `Transition`/`Presence`/spring; port unipi `Board.tsx` collapse/insert-line model | gpui-pre 0.3.10 + gpui-base motion + unipi reference |
| #1 pickers | `command` palette or `combobox`/`searchable_list` inside `Dialog` (desktop) / `Sheet` (mobile); pinning = app sort | gpui-component 0.8.1 |
| #16/#21 lists | `message_scroller` for the conversation; `virtual_list` for card grids | gpui-component 0.8.1 |
| D2 motion | tokens over `gpui_base::motion` + gpui `Animation`/spring | gpui-base 0.8.1 |
| #14 settings | `setting/` pages+fields, `form/` for modals | gpui-component 0.8.1 |

## 5. Risks / open questions

- **Editor engine maturity at the edge**: the LSP trait surface (`0.97` `lsp-types`
  with `proposed` features) is new in the 0.8.x line; hover/completion UX quality needs
  a spike against rust-analyzer on `ti` before committing the whole files-page design.
- **tree-sitter grammars** add heavy compile time (one dep per language); syntect-only
  first is fine, and matches the already-enabled feature.
- **Dock** (`gpui_base::dock` + component wrapper) is powerful but opinionated; adopting
  it wholesale may fight Oino's existing shell. Tree-only is the safe first slice.
- **Zed GPL**: any "take the full component from Zed" (feedback #24 wording) means
  GPL-3.0-or-later contamination of `apps/` — the Apache-2.0 Longbridge engine is the
  clean path and is the same lineage.
- gpui-box (kanban) and gpui-form (derive macro) are too small/young to depend on now;
  revisit in a later round.
- Could not determine: exact runtime behaviour of `command` palette animation curves
  (needs a GUI spike); whether `gpui-kit` will add a `tree-sitter` passthrough feature
  upstream (if it does, drop the direct gpui-component dep).
