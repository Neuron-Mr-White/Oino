# Spec — Extension UI protocol v2 (ratatui vocabulary) + slots

Status: **implementation-ready spec** (v2 graph, work id `spec-ui-protocol`). Input contract:
[DIRECTION.md](../DIRECTION.md) §5 (owner-approved), §4.2 (UI rules), §4.3 (screens).
Supersedes [v1 ui-protocol.md](../../v1/ui-protocol.md) §5 (panels), §6 (OUS v1), §6.6
(settings schema) and §10 (capabilities). v1 §2–§4 (message set, routing, first-answer-wins,
dialog methods), §7 (editor model) and §8 (shortcuts) **stay in force** unless a section
below changes them.

Sibling specs (referenced, not redefined):

| Spec | What this spec takes from it |
|---|---|
| [host.md](host.md) | App ↔ host transport: frames, session streams (§7.3), event stream / resume / coalescing (§7.4), perf rules (§7.5), settings ops (§9.9). This spec only defines the **payloads** that ride `SessionSnapshot.ui` and `Control::Events`. |
| [ext-abi.md](ext-abi.md) | WIT `oino:extension@2.0.0` cutover (WP-X6), `host-service` (WP-X1), kanboard CLI (WP-X3), cron job files (WP-X4), browser service (WP-X5), `--exclude-extension` (WP-X2). The WIT deltas in §9.1 here ride the same 2.0.0 cut. |
| [core.md](core.md) | Extension crash → disabled for the session (§2.3), instance lifetime, package split (§2.4). Disposal rules in §6.6 key off those events. |
| [browser.md](browser.md) | Browser side-pane UX (§9). The `oino.browser/browser` reserved id (§8) opens it. |
| [../zcode-audit.md](../zcode-audit.md) | Z-5 (right side pane = tabs container), Z-95 (status panel / capsule), D-1 (turn file summary), D-17 (slot table), G-1/G-4 (gaps this spec closes). Phone mapping follows the Z-5 rec ("side-pane tabs → full-screen layers/sheets"). |

Sources studied: `crates/oino-core/src/ui.rs` (UiBridge, retention keys), `crates/oino-view/src/
{ous.rs,ui_state.rs,settings_hub.rs}`, `crates/oino-tui/src/render/{ous.rs,overlays.rs,mod.rs}`,
`wit/oino-extension/{ui.wit,guest.wit,registry.wit}`, `extensions/sdk/{oino-extension/src/ui.rs,
unipi-kit/src/ous.rs,unipi-kit/src/settings}`, every `extensions/**` OUS emitter (§12.3),
pi `docs/{extensions,tui,rpc-extension-ui,themes}.md` (pi 1.4.0), ratatui 0.29 (`layout/
{layout,flex}.rs` solver, widgets), koishijs/webui `packages/client/client/{data.ts,
components/slot.ts,plugins/{router,setting,loader}.ts}` (slot / DataService / settings model),
`apps/unipi/apps/mobile/src` (DialogCard, ControlCentre, StatusBar, shell, settings),
`apps/unipi/DESIGN.md` §1 (tokens), `style.css` `@theme`.

## Contents

1. [Decisions](#1-decisions)
2. [Architecture and layering](#2-architecture-and-layering)
3. [pi `ctx.ui` methods (unchanged) and their rendering](#3-pi-ctxui-methods-unchanged-and-their-rendering)
4. [Widget tree (ratatui vocabulary)](#4-widget-tree-ratatui-vocabulary)
5. [Theme tokens](#5-theme-tokens)
6. [Placements, slots and the entry registry](#6-placements-slots-and-the-entry-registry)
7. [Settings schema dialect (OSS-2)](#7-settings-schema-dialect-oss-2)
8. [Reserved panel ids (first-party components)](#8-reserved-panel-ids-first-party-components)
9. [Wire messages](#9-wire-messages)
10. [Capabilities and degradation](#10-capabilities-and-degradation)
11. [Guest builder crate `oino-ui`](#11-guest-builder-crate-oino-ui)
12. [Migration from OUS v1 and TUI renderer plan](#12-migration-from-ous-v1-and-tui-renderer-plan)
13. [SolidJS renderer in the app](#13-solidjs-renderer-in-the-app)
14. [Work packages](#14-work-packages)
15. [Risks](#15-risks)
16. [Owner questions](#16-owner-questions)
- [Appendix A — JSON Schema (normative)](#appendix-a--json-schema-normative)
- [Appendix B — layout vectors](#appendix-b--layout-vectors)

---

## 1. Decisions

| # | Decision | Why |
|---|---|---|
| D-U1 | pi `ctx.ui` records (`select`, `confirm`, `input`, `editor`, `notify`, `setStatus`, `setWidget`, `setTitle`, `set_editor_text`) stay **byte-for-byte** as pi 1.4.0 `rpc-extension-ui.md`. Oino dialogs (`multiSelect`, `form`, `approval`, `custom`, `clipboardReadImage`, `editExternally`) stay as v1 §4.3. | pi parity; DIRECTION §5 bullet 1. |
| D-U2 | Node trees use **ratatui names** with a PascalCase discriminator `t` (`"Paragraph"`, `"Layout"`, …). OUS v1 lowercase kinds (`text`, `stack`, `card`, …) are **dropped**; core rejects them with a migration diagnostic (§4.8). No runtime v1→v2 translator. | DIRECTION §5 ("old OUS v1 node names are dropped"); PascalCase makes a stray v1 node detectable. |
| D-U3 | Text = ratatui `Text`/`Line`/`Span` + `Style{fg,bg,underlineColor,addModifier,subModifier}`. Colours are **theme tokens only** (§5) — no hex, no RGB, no ANSI indices. | Same tree must theme correctly in TUI themes and the unipi palette. |
| D-U4 | Layout = `Layout{direction, constraints, children, margin, spacing, flex}` with `Constraint::{Length, Percentage, Ratio, Min, Max, Fill}` in ratatui's serde form (`{"Length":3}`). **Semantics are ratatui 0.29's solver** (`Flex::Start` default). TUI uses ratatui `Layout` directly; the app runs a port of the same constraint formulation (§4.3) checked against vectors generated from ratatui (Appendix B). | One meaning for both renderers; "ratatui vocabulary" taken literally. |
| D-U5 | Two sizing modes: **definite** (the slot gives a fixed main-axis size → solver) and **auto** (inline slots grow with content → constraints resolve to intrinsic sizes, §4.4). | Inline slots (above composer, turn footer) have no fixed height. |
| D-U6 | Extensions send **whole trees** (React-style) or explicit **patch ops**; core diffs whole trees into keyed patches (§4.7). Frontends only ever receive snapshot + patch. | Simple guest code, cheap wire over iroh. |
| D-U7 | **Entry registry** (cordis/Koishi slot model): every placement is an entry `{id, extensionId, slot, order, seq, view, …}` in a per-session (and a per-host) registry, pushed as `UiSnapshot` + `UiPatch` with a monotonic `rev`. Ordering: `order` desc, then `seq` asc. Entries are owned by an extension instance and **disposed with it**. | DIRECTION §5 "Slots (cordis-style …)"; webui `slot.ts` / `data.ts` patterns. |
| D-U8 | New slot `turnFooter` is **pull-based**: an extension registers a turn-footer *renderer*; frontends request content per **visible** turn (`_oino/render{target:"turnFooter"}`), cached by `(turnEntryId, rev)`. | Virtualized transcript: no unbounded per-turn entries. |
| D-U9 | First-party rich screens are **reserved ids** whose entry `view` is `{kind:"reserved"}`; the app renders its native component, gated on the owning extension being enabled (host-owned screens: on the host service being advertised). Third-party extensions get node trees in any slot, never reserved ids. | DIRECTION §5 last bullet. |
| D-U10 | Settings: one dialect (**OSS-2**, §7) — v1 §6.6 sections/fields kept, plus Schemastery semantics (`resolve` fills defaults and never throws, `simplify` stores only overrides, scope layering) and `list`/`dict`/`json`/`visibleIf`. Rendered as one Settings section per namespace in the single Settings screen. | DIRECTION §4.3 Settings; v1 data keeps working. |
| D-U11 | The shared wire types, builder, diff, validation and plain-text fallback live in **one serde-only crate `oino-ui`** (`crates/oino-ui`), used by core, TUI, `oino-view` and guests (`wasm32-wasip2`). Its API mirrors ratatui (`Block::bordered().title(…)`, `"x".bold()`, `Layout::vertical([…])`). | DIRECTION §5 "guest-side Rust builder crate mirrors ratatui's API". |
| D-U12 | App renderer: **one Solid component per node kind**, a keyed Solid store per entry, TanStack Virtual for `List`/`Table`/`Log`/`Tree` above 100 rows, Markdown via the existing `renderChatMarkdown` (DOMPurify). No extension JS, no iframes, no `innerHTML` elsewhere. | DIRECTION §5; §4.2 perf rules. |
| D-U13 | Frontends own local UI state (selection, scroll, expansion, typed text) like ratatui `StatefulWidget`s; the tree may carry `state` that **wins when it changes**. Interaction goes back as `oino_ui_action{panelId, actionId, value?, values?, path?}` (v1 event name kept). | Responsive UI without round-trips; v1 guest handlers keep their event. |
| D-U14 | Unknown props are ignored; unknown `t` renders `fallback` (a node) or nothing. Validation is **core's** job (one validator of record); the app validates only the registry envelope with zod. | Forward compatibility; zod on 10k-node trees is too slow on phones. |

---

## 2. Architecture and layering

```
 extension (wasm guest)                     oino-core (per session)                  frontends
 ─────────────────────                      ───────────────────────                  ─────────
 ui.dialog(json) ───────────────► UiBridge: dialogs, first answer wins ──► extension_ui_request (pi shape)
 ui.fire(json)   ─┐                                                     ◄── extension_ui_response
 ui.panel-open ───┤  oino-ui::validate ──► UiRegistry (session scope) ──► UiSnapshot / UiPatch (rev)
 ui.panel-update ─┤   (limits, v1 gate)     entries, order/seq, diff        │
 ui.panel-close  ─┘                          dispose on instance drop       ├─► TUI (in-process, oino-view mirror)
 guest.render ◄──── _oino/render ◄──────── (turnFooter / transcript)       ├─► oino --mode rpc (_oino/* events)
                                                                           └─► unipi-host ──► app (session stream,
 manifest.ui (reserved, settings) ──► host catalog ──► UiRegistry (host scope) ──► hub event stream   host.md §7.3/7.4)
 oino_ui_action ◄──────────────────────────────────────────────────────────── _oino/ui_action
```

Layers:

1. **L0 — pi dialogs and one-shot effects** (§3). Wire records unchanged. Dialogs are request/
   response with first-answer-wins (v1 §3). One-shot effects (`notify`, `set_editor_text`,
   `openUrl`, `clipboardWrite`, `osNotify`, `insertEditorToken`, `openSettings`) are delivered
   once, never retained.
2. **L1 — retained state as registry entries** (§6). Every retained fire-and-forget method
   (`setStatus`, `setWidget`, `setHeader`, `setFooter`, `setFrame`, `setWorkingMessage`,
   `setTitle`, `setShortcuts`, `openPanel`/`updatePanel`/`closePanel`) is a **producer** that
   mutates the `UiRegistry`. The v1 replay of raw retained requests (`retention_key` in
   `oino-core/src/ui.rs:415`) is replaced by the registry.
3. **L2 — node trees** (§4). Entry views of kind `tree` hold a validated `oino_ui::Node`.
   Core keeps the last tree per entry and emits keyed patches.
4. **L3 — slot hosts** in each frontend (§6.2 TUI column, §13 app): render the entries of a
   slot in registry order.

Scopes:

| Scope | Lives in | Contains | Delivered by |
|---|---|---|---|
| `session` | `AgentSession` (core) | entries created by the session's extension instances, open dialogs, title, working message, shortcuts | `SessionSnapshot.ui` + `Control::Events` (`oino_ui_patch` events) on the session stream (host.md §7.3/§7.4); `_oino/ui_snapshot` + `_oino/ui_patch` in `--mode rpc`; direct channel in the TUI |
| `host` | unipi-host extension catalog (or the `oino` process for the TUI) | reserved component entries, settings namespaces, extension enable state | hub event stream `events_open` (host.md §7.4) gains `HubEvent::Ui{snapshot|patch}`; in the TUI, built from the local catalog |

Producers never talk to frontends directly; frontends never mutate the registry except through
actions (§4.6) and dialog responses. Multiple attached frontends (TUI + app + phone) see the
same registry; local UI state (selection, scroll) is per frontend.

---

## 3. pi `ctx.ui` methods (unchanged) and their rendering

### 3.1 Wire and semantics (unchanged)

| pi method | Wire | Kind | Notes kept from v1 |
|---|---|---|---|
| `select(title, options, {timeout})` | `select{title, options, timeout?}` + Oino `optionDetails?` | dialog | answer `value` / `cancelled` |
| `confirm(title, message)` | `confirm{title, message, timeout?}` | dialog | `confirmed` / `cancelled` |
| `input(title, placeholder)` | `input{title, placeholder?, timeout?}` + Oino `prefill?`, `secret?` | dialog | |
| `editor(title, prefill)` | `editor{title, prefill?}` + Oino `language?`, `cursorLine?` | dialog | |
| `notify(msg, type)` | `notify{message, notifyType?}` | effect | |
| `setStatus(key, text)` | `setStatus{statusKey, statusText?}` + Oino `tone?`, `priority?`, `spans?: Line` | registry `status` | absent text clears |
| `setWidget(key, lines \| factory, {placement})` | `setWidget{widgetKey, widgetLines?, widgetPlacement?}` + Oino `widgetSpec?: Node` (v2 node), `order?` | registry `widget` | `widgetSpec` preferred by v2 frontends |
| `setTitle(title)` | `setTitle{title}` | registry meta | |
| `setEditorText` / `pasteToEditor` | `set_editor_text{text}` | effect | |
| `getEditorText` | WIT `editor-text` (last `editor_event` text, v1 §7.1) | — | |
| `setWorkingMessage/Visible` | `setWorkingMessage{message?, visible?}` | registry meta | |
| `setHeader(factory)` / `setFooter(factory)` | `setHeader{key, spec?: Node}` / `setFooter{key, spec?, segments?}` | registry `header` / `footer` | |
| `custom(factory)` | `custom{title?, spec: Node, size?, keys?, timeout?}` dialog, or `openPanel` | dialog / registry | answer `{actionId, value?, values?}` (§4.6) |
| `setTheme` / `getAllThemes` | `setTheme{name}`; `get_themes` | effect | app ignores (one palette) |
| `onTerminalInput`, `setEditorComponent` | not available (v1 §9) | — | use editor intents / `editorReplace` |

Oino dialogs (`multiSelect`, `form`, `approval`, `clipboardReadImage`, `editExternally`) keep
v1 §4.3 fields; `approval.detail` and `custom.spec` are now **v2 nodes**. Timeouts, dismissal
(`extension_ui_dismiss{reason}`), handler-deadline pausing and orphan withdrawal are v1 §3
verbatim.

### 3.2 Rendering

Dialogs in the app keep the unipi pattern: **they take over the composer slot**
(`components/chat/DialogCard.tsx`, `ChatView.tsx DialogStack`); several open dialogs queue in
arrival order with a "1 of N" pager. Phone and desktop share the card; desktop adds keys.

| Method | TUI (`oino-tui`) | App desktop | App phone |
|---|---|---|---|
| `select` | dialog block over composer, ↑↓/1–9, Enter, Esc (unchanged) | DialogCard: numbered option rows (`optionDetails` as second line), ↑↓/digits/Enter/Esc, countdown chip when `timeout` | same card; rows 56 px, tap |
| `confirm` | message + `[Yes] [No]` | DialogCard: markdown message, `.btn-primary` Yes + `.btn-secondary` No; Enter = Yes, Esc = No | same |
| `input` | single-line field (`secret` masked) | DialogCard input (44 px), `placeholder`, `prefill`; Enter submits | same; IME-safe (`lib/ime.ts`) |
| `editor` | multi-line editor block, Ctrl+Enter submits | DialogCard grows to a textarea (max 50 vh); `language` → read-write CodeMirror (existing `screens/Editor.tsx` setup, no LSP) | full-height sheet |
| `notify` | transient line in the footer (info/warn/error tone) | toast stack bottom-right (`info` 4 s, `warning` 6 s, `error` sticky until dismissed), through the app's single toast surface (placement ruled in zcode-audit §J) | toast top, under safe area |
| `setStatus` | footer status segments (priority order) | status bar slot (§6.2) | status strip under the chat header, overflow to popover |
| `setWidget` | lines/tree above (or below) the composer | above-composer slot (`belowEditor` renders **below the composer**, desktop only; phone folds it above) | above composer, collapsed to 1 row + "expand" when > 3 rows |
| `setTitle` | terminal title (OSC 0) | desktop window title while that chat is focused (`desktop_set_title`), never the in-page header | ignored |
| `set_editor_text` | composer text | composer text, undoable (`ComposerUndo`) | same |
| `setWorkingMessage` | working line text | the running-clock row text in the transcript | same |
| `setHeader` | header block above transcript | header slot (§6.2): band at the top of the transcript (scrolls with it) | same |
| `setFooter` / `setFrame` | footer rows / editor frame segments | rendered **inside the status bar** (no second footer surface, rule 4.2.2): `segments` → status segments; `spec` → one status segment rendering the node in auto mode, 1 row | status strip |
| `custom` | modal box (v1 behaviour, now buffer-rendered) | centred modal, `size` small/medium/large/full = 420/560/800 px/full window, first submitting action resolves | bottom sheet; `large`/`full` = full-height |
| `multiSelect` / `form` | v1 TUI forms | DialogCard ask-user pager (existing `AskUser`) | same |
| `approval` | v1 approval block | DialogCard approval: summary, `detail` node (auto mode, max 40 vh, scroll), option buttons by `kind` (allow → primary, reject → secondary, `*_always` → ghost), feedback field | same |
| `editExternally` | `$EDITOR` | in-app editor sheet (CodeMirror) | same |
| `clipboardReadImage` | OSC 52 / host | Tauri clipboard plugin | Tauri clipboard plugin |

`ctx.hasUI` is true when any v2-capable frontend is attached; with none, v1 §3 "no UI attached"
rules apply (dialogs resolve default, effects dropped, registry still maintained so a frontend
attaching later sees current state).

---

## 4. Widget tree (ratatui vocabulary)

### 4.1 Node shape

Every node is a JSON object `{"t": "<Kind>", "key"?: string, "style"?: Style, "block"?: BlockProps,
"fallback"?: Node, ...props}`.

- `t` — PascalCase kind (§4.5). Unknown kind ⇒ render `fallback`, else nothing.
- `key` — identity among siblings (1–128 chars, unique among the siblings of one collection).
  Missing ⇒ implicit positional key `#<index>`. Keys drive diffing (§4.7), action paths (§4.6)
  and the app's keyed reconciliation.
- `style` — ratatui `Widget::style()`: base style of the node's area (fg/bg/modifiers inherited
  by text inside it).
- `block` — ratatui `.block(Block)` on every leaf widget that ratatui allows it on (all except
  `Layout`, `Block`, `Spinner`, `KeyHints`, `Scrollbar`): the widget renders inside the block's
  inner area. Same props as the `Block` node minus `child`.

Props are camelCase forms of ratatui builder methods (`highlightStyle`, `columnSpacing`,
`barWidth`). Enum values use ratatui's Rust variant names (`"Vertical"`, `"Rounded"`,
`"Braille"`); modifiers use ratatui's constant names (`"BOLD"`). Slot / placement names keep
their v1/pi camelCase (`"sidePane"`).

### 4.2 Text model and style

```text
Span  = string | { content: string, style?: Style, link?: url }        // ratatui Span (+ link: GUI/OSC 8)
Line  = string | Span[] | { spans: Span[], style?: Style, alignment?: Alignment }
Text  = string | Line[] | { lines: Line[], style?: Style, alignment?: Alignment }   // string splits on "\n"
Style = { fg?: Color, bg?: Color, underlineColor?: Color,
          addModifier?: Modifier[], subModifier?: Modifier[] }
Modifier  = "BOLD" | "DIM" | "ITALIC" | "UNDERLINED" | "SLOW_BLINK" | "RAPID_BLINK"
          | "REVERSED" | "HIDDEN" | "CROSSED_OUT"
Alignment = "Left" | "Center" | "Right"
Color     = theme token (§5), e.g. "accent", "success", "muted", "c7", "surface2", "reset"
```

Which form a JSON array is (`Line[]` vs `Span[]`) is decided by the field's declared type.
Style composition follows ratatui `Style::patch`: child style patches parent; `subModifier`
removes inherited modifiers.

Rendering rules shared by both renderers:

| Feature | TUI | App |
|---|---|---|
| `fg`/`bg` token | theme colour (§5) | CSS variable (§5) |
| span with `bg` | cell background | **inline chip**: `padding: 0 .3em; border-radius: 4px` (badge idiom of ratatui UIs) |
| `BOLD` / `DIM` / `ITALIC` / `UNDERLINED` / `CROSSED_OUT` | SGR | `font-weight: 600` / `opacity: .6` / italic / underline / line-through |
| `REVERSED` | SGR 7 | swap fg/bg (fg default → `surface1`) |
| `HIDDEN` | SGR 8 | `visibility: hidden` |
| `SLOW_BLINK` / `RAPID_BLINK` | SGR 5/6 | ignored (motion rule: only the Unicrab loops) |
| `link` | OSC 8 hyperlink + underline | `<a>` opened via `openUrl` (never navigates the webview) |
| Wide chars / emoji | `unicode-width` | browser shaping |

### 4.3 Layout and constraints

```json
{"t":"Layout","direction":"Vertical","constraints":[{"Length":3},{"Fill":1},{"Length":1}],
 "spacing":0,"margin":{"horizontal":0,"vertical":0},"flex":"Start","collapse":60,
 "children":[ {…}, {…}, {…} ]}
```

| Prop | Type | Default | Meaning |
|---|---|---|---|
| `direction` | `"Vertical" \| "Horizontal"` | `"Vertical"` | main axis |
| `constraints` | `Constraint[]` | `Fill(1)` per child | one per child; fewer ⇒ missing ones are `Fill(1)`; more ⇒ extra ignored |
| `children` | `Node[]` | — | ≤ 64 per Layout |
| `spacing` | `u16` cells | 0 | gap between children (ratatui `Spacing::Space`; overlap not supported) |
| `margin` | `u16 \| {horizontal, vertical}` | 0 | inset of the whole area |
| `flex` | `"Legacy" \| "Start" \| "End" \| "Center" \| "SpaceBetween" \| "SpaceAround"` | `"Start"` | ratatui `Flex` (0.29 default is `Start`) |
| `collapse` | `u16` cells | 60 | horizontal only: when the available width < `collapse` columns, lay the children out **vertically in auto mode** (phone rule, §13.6). `0` disables |

`Constraint` (ratatui serde form; a bare integer `n` = `{"Length": n}`):

| Constraint | Definite mode (area of size `S` cells, from ratatui 0.29 `Layout::try_split`) | Auto mode (§4.4) |
|---|---|---|
| `{"Length": n}` | size = n (strength `LENGTH_SIZE_EQ`) | n |
| `{"Percentage": p}` | size = S·p/100 of the **whole** area (before spacing) | intrinsic |
| `{"Ratio": [a, b]}` | size = S·a/max(b,1) | intrinsic |
| `{"Min": n}` | ≥ n (strongest); with `Flex` ≠ `Legacy` it also grows like `Fill(1)` | max(n, intrinsic) |
| `{"Max": n}` | ≤ n (strongest), prefers exactly n | min(n, intrinsic) |
| `{"Fill": w}` | takes excess space proportional to `w` among `Fill`/`Min` siblings (`Fill(0)` ≈ 1e-6) | intrinsic |

Priority when space is short (ratatui strengths): `Min` ≥ `Max` > `Length` > `Percentage` >
`Ratio` > `Fill`; spacers of `Start` grow last. Results are rounded the ratatui way
(`changes_to_rects`: round to `FLOAT_PRECISION_MULTIPLIER`, then to integer cells).

**Units.** TUI: 1 unit = 1 terminal cell (column horizontally, row vertically). App: 1 column =
`--ui-col` = **0.5rem** (8 px at 16 px root), 1 row = `--ui-row` = **1.25rem** (20 px). The
app solves in fractional units of its container's pixel size (`px / unit`), so layouts are
smooth on any width and identical to the TUI at unit scale. Chrome rule: app `Block` chrome
(border + title bar + padding) never exceeds the cells the TUI spends on it (top 1 row, bottom
1 row, sides 1 column each), so cell-designed layouts never clip in the app.

**Solver in the app.** `src/ui/layout/solve.ts` ports `configure_*` of ratatui 0.29
(`layout/layout.rs`: area, variable ordering, flex spacers, constraint strengths, fill
proportions, `ALL_SEGMENT_GROW`) onto `@lume/kiwi` (MIT cassowary, ~13 KB min+gz). Results are
memoized by `(direction, constraints, flex, spacing, margin, size)`. Acceptance: every vector in
`crates/oino-ui/tests/layout_vectors.json` (generated from ratatui itself, Appendix B) matches
within ±1 cell at unit scale. An implementation that passes the vectors without kiwi is
acceptable.

### 4.4 Sizing modes and intrinsic size

A slot gives its root either a **definite** size on each axis or **auto** (grow with content
up to the slot max, then scroll). Width is always definite. Height:

| Definite height | Auto height |
|---|---|
| `overlay` / `custom` when the root requests it (below), `sidePane`, `bottomPane`, `fullscreen` | `widget`, `turnFooter`, `header`, `editorReplace`, `status`/`footer` (1 row), dialog `detail`, transcript render specs |

An `overlay`/`custom` root is definite (= the slot's max height) when its vertical root Layout
contains a `Fill`, `Percentage` or `Ratio`; otherwise it sizes to content. Same rule in both
renderers.

Intrinsic height `h(node, width)` (rows; app multiplies by `--ui-row` only for constraint
maths, real content uses natural CSS height):

| Node | Intrinsic height |
|---|---|
| `Paragraph` | wrapped line count at `width` (ratatui `Paragraph::line_count`), + block chrome |
| `List` / `Tree` | visible item rows (items × item height, expanded tree nodes) |
| `Table` | header + rows (row `height` + `bottomMargin`) + footer |
| `Tabs` | 1 + intrinsic of the selected pane (if `panes`) |
| `Gauge` | 1 (+ block chrome) |
| `LineGauge`, `KeyHints`, `Spinner`, `Button`, `Toggle`, `Select` | 1 |
| `Input` | 1, or `rows` when `multiline` (default 3) |
| `Sparkline` | 3 |
| `BarChart` | 8 |
| `Chart` | 12 |
| `Scrollbar` | 1 (horizontal) / parent height (vertical) |
| `Markdown` / `Code` / `Diff` / `Log` | rendered line count (Log: ≤ `maxLines`), capped by the slot |
| `Image` | `maxHeight` if set, else aspect-ratio height at width (TUI: cell aspect 1:2), ≤ 20 rows |
| `Empty` | 3 |
| `Block` | child + chrome (top/bottom border = 1 each, `padding`) |
| `Layout` vertical | Σ resolved child sizes (auto column of §4.3) + spacing + margin |
| `Layout` horizontal | max over children of their intrinsic height at their solved width |

Slot max heights (content beyond scrolls; TUI clips and shows a `…` row with the hidden count):

| Slot | TUI | App |
|---|---|---|
| `widget` (each) / all widgets | 10 rows / 40 % of screen | 240 px / 40 vh |
| `turnFooter` | 6 rows | 160 px (native file summary: own rules) |
| `header` | 12 rows | 30 vh |
| `editorReplace` | 50 % of screen | 50 vh |
| `status`, `footer` | 1 row | 1 row (28 px), overflow popover |
| `overlay` / `custom` | 90 % of screen | 80 vh (desktop), full sheet (phone) |
| dialog `detail` | 12 rows | 40 vh |

### 4.5 Widget catalog

Props marked † are GUI extras (ratatui has no equivalent; the TUI renders a documented
approximation). Every widget below also takes the common props of §4.1. Exact JSON Schema:
Appendix A. "Action" columns name the action props (§4.6).

#### 4.5.1 ratatui widgets

| `t` | Props | TUI rendering | App rendering | Actions → `value` |
|---|---|---|---|---|
| `Block` | `title?: Line \| [{content: Line, alignment?, position?: "Top"\|"Bottom"}]`, `titleAlignment?`, `borders?: "ALL"\|"NONE"\|("TOP"\|"RIGHT"\|"BOTTOM"\|"LEFT")[]` (default `"NONE"`; `Block::bordered()` = `"ALL"`), `borderType?: "Plain"\|"Rounded"\|"Double"\|"Thick"`, `borderStyle?`, `titleStyle?`, `padding?: u16 \| {left,right,top,bottom}`, `child?: Node`, †`collapsible?: bool`, †`collapsed?: bool`, †`rail?: Color` | ratatui `Block` + child in `inner(area)`; collapsible → `▸/▾` toggle in title, Enter toggles; `rail` → left border in that colour | `<section class="ui-block">`: `ALL` borders → 1 px `--color-line` border, radius 12 (`Rounded`) / 4 (`Plain`) / double = 3 px double; title row 20 px (`--ui-row`) 13/600; padding in units; collapsible → disclosure button; `rail` → 3 px left bar | `onToggle` → `bool` (collapsed) |
| `Paragraph` | `text: Text`, `wrap?: {trim: bool} \| false` (default `{trim:true}`), `scroll?: [y, x]`, `alignment?` | ratatui `Paragraph` | `<div class="ui-p">`, `white-space: pre-wrap` (or `pre` + horizontal scroll when `wrap:false`), lines as block spans; `scroll` sets initial scrollTop in rows | — |
| `List` | `items: ListItem[]`, `state?: {selected?: key, offset?: u32}`, `highlightStyle?`, `highlightSymbol?: string`, `repeatHighlightSymbol?`, `highlightSpacing?: "Always"\|"WhenSelected"\|"Never"`, `direction?: "TopToBottom"\|"BottomToTop"`, `scrollPadding?: u16`, †`multi?: bool`, †`scrollbar?: bool` | ratatui `List` + `ListState`; ↑↓ PgUp/PgDn Home/End; Enter activates; Space toggles (multi) | `role="listbox"` rows (32 px desktop / 44 px phone); `ListItem.detail` second line muted; `badge` right; selected row `surface-3` + 2 px shell left bar; virtualized > 100 items | `onSelect` → item key (multi: `key[]`); `onActivate` → item key |
| `ListItem` (inside `items`) | `Text` or `{key?, content: Text, style?, †detail?: Line, †badge?: Line, †icon?: Icon, †disabled?: bool}` | content lines; detail as 2nd dim line; badge right-aligned | as above | — |
| `Table` | `header?: Row`, `rows: Row[]`, `footer?: Row`, `widths: Constraint[]`, `columnSpacing?: u16` (1), `rowHighlightStyle?`, `highlightSymbol?`, `highlightSpacing?`, `state?: {selected?: key, offset?}`, †`sticky?: "FirstColumn"`, †`scrollbar?` | ratatui `Table` + `TableState`; widths solved horizontally like a Layout | `<table>`-free grid (`display:grid; grid-template-columns` from the horizontal solve); header sticky; row 32/44 px; virtualized > 100 rows; phone: horizontal scroll, first column sticky | `onSelect` / `onActivate` → row key |
| `Row` / `Cell` | Row `{key?, cells: Cell[], height?: u16 (1), bottomMargin?: u16, style?}`; Cell = `Text \| {content: Text, style?}` | ratatui `Row`/`Cell` | grid cells; cell `Line.alignment` → `text-align` | — |
| `Tabs` | `titles: [{key, title: Line, †badge?: Line}]`, `selected?: key`, `divider?: Span` (`"│"`), `padding?: [Line, Line]`, `highlightStyle?`, †`panes?: {[key]: Node}` | ratatui `Tabs` bar; with `panes`, the selected pane renders below (Layout `[Length(1), Fill(1)]`); ←→ / Tab cycles | segmented tab strip (`--chip-h` 38 px), scrollable on phone; selected pane below; panes switch locally (no round-trip) | `onSelect` → tab key |
| `Gauge` | `ratio: 0..1` (or `percent: 0..100`), `label?: Span`, `gaugeStyle?`, `useUnicode?: bool` | ratatui `Gauge` | 8 px rounded bar, fill `gaugeStyle.fg` (default `accent`), label centred above/inside | — |
| `LineGauge` | `ratio`, `label?: Line`, `filledStyle?`, `unfilledStyle?`, `lineSet?: "Normal"\|"Thick"\|"Double"` | ratatui `LineGauge` | label left + 4 px bar | — |
| `Sparkline` | `data: (u64\|null)[]` (≤ 1 000), `max?: u64`, `direction?: "LeftToRight"\|"RightToLeft"`, `barSet?: "Nine"\|"Three"`, `absentValueSymbol?` | ratatui `Sparkline` | inline SVG bars (`fg` colour), height = rows × `--ui-row` | — |
| `BarChart` | `data: [{label?: Line, bars: [{value: u64, label?: Line, textValue?: string, style?, valueStyle?}]}]`, `barWidth?` (1), `barGap?` (1), `groupGap?` (0), `direction?: "Vertical"\|"Horizontal"`, `max?`, `barStyle?`, `valueStyle?`, `labelStyle?` | ratatui `BarChart` (groups) | SVG; horizontal direction = labelled bar rows | `onSelect` → `"<groupIndex>:<barIndex>"` † |
| `Chart` | `datasets: [{name?: Line, data: [[f64, f64]], marker?: "Dot"\|"Braille"\|"Block"\|"Bar"\|"HalfBlock", graphType?: "Scatter"\|"Line"\|"Bar", style?}]` (Σ points ≤ 10 000), `xAxis`/`yAxis: {title?: Line, bounds: [f64, f64], labels?: Span[], labelsAlignment?, style?}`, `legendPosition?: "Top"\|"TopRight"\|"TopLeft"\|"Left"\|"Right"\|"Bottom"\|"BottomRight"\|"BottomLeft"\|null` | ratatui `Chart` | SVG polyline/scatter/bars, axes + labels, legend; points LTTB-downsampled to ≤ 2 × pixel width | — |
| `Scrollbar` | `orientation?: "VerticalRight"\|"VerticalLeft"\|"HorizontalBottom"\|"HorizontalTop"`, `state: {contentLength: u32, position: u32, viewportContentLength?: u32}`, `thumbStyle?`, `trackStyle?`, `beginSymbol?`, `endSymbol?` | ratatui `Scrollbar` + `ScrollbarState` | read-only position indicator (thin track + thumb, DESIGN scrollbar colours). Scrollable widgets get **native** scrollbars via their own `scrollbar?: bool` — standalone `Scrollbar` is for paged content | — |

#### 4.5.2 GUI extras (DIRECTION §5 list)

| `t` | Props | TUI rendering | App rendering | Actions → `value` |
|---|---|---|---|---|
| `Markdown` | `source: string` (≤ 256 KiB) | existing pulldown-cmark renderer (`render/markdown.rs`) | `renderChatMarkdown` (marked + DOMPurify, `.md` tokens); mermaid fences → `MermaidViewer` on tap | link click → `openUrl` (not an action) |
| `Code` | `source`, `language?`, `lineNumbers?: bool`, `startLine?: u32` (1), `highlightLines?: u32[]`, `wrap?: bool` (false) | syntect (`render/highlight.rs`) | the chat code-block renderer (sticky language + Copy header); highlighted lines `surface-3` | — |
| `Diff` | `patch?: string` (unified) **or** `old?: string, new?: string`; `path?`, `context?: u16` (3), `mode?: "Unified"\|"Split"` | coloured unified diff (`diffAdded/Removed/Context`) | the chat diff view (`--md-diff-*` tokens); `Split` on desktop ≥ 900 px, else unified | — |
| `Image` | `src: {data: base64, mimeType} \| {path} \| {blob: id}`, `alt: string`, `fit?: "Contain"\|"Cover"`, `maxWidth?: u16`, `maxHeight?: u16` | kitty / iTerm2 / sixel when the terminal supports it, else `[image: alt]` dim box | `<img>` (blob/path via host `blob_get`/`fs_read`, host.md §9.1–9.2), click → media viewer | — |
| `Log` | `lines: Line[]` (≤ `maxLines`), `maxLines?: u32` (2 000, ≤ 5 000, ring), `follow?: bool` (true), `wrap?: bool` (false), `lineNumbers?: bool` | last N rows, follow tail | monospace virtual list (20 px rows, `pre`), sticky-bottom follow; scrolling up pauses follow, "↓ live" pill resumes | — |
| `Tree` | `nodes: [{key, label: Line, icon?: Icon, badge?: Line, children?: TreeNode[], expanded?: bool, †lazy?: bool}]`, `state?: {selected?: key}` | indented rows `▸/▾`, ←→ collapse/expand | `role="tree"` rows, virtualized when > 100 visible | `onSelect`/`onActivate` → key; `onToggle` → `{key, expanded}` (lazy: extension replies with children via patch) |
| `KeyHints` | `hints: [{keys: string (pi KeyId), label: string, action?: actionId}]` | dim `keys label · …` line | desktop: `kbd` chips row; phone: hidden unless a hint has `action` (then a tappable chip) | hint with `action` → fires that action, `value` null |
| `Input` | `key` (required), `value?`, `placeholder?`, `label?: Line`, `multiline?: bool`, `rows?: u16`, `secret?: bool`, `maxLength?: u32`, `disabled?` | text field, cursor | `<input>`/`<textarea>` 44 px, `--radius-input` | `onSubmit` → string; `onChange` → string (debounced 150 ms) |
| `Toggle` | `key`, `label: Line`, `value: bool`, `disabled?` | `[x] label` | existing `Switch.tsx` | `onChange` → bool |
| `Select` | `key`, `options: [{value, label: Line, description?: string}]`, `value?`, `placeholder?`, `disabled?` | `‹ value ›` cycling field, Enter opens list | Kobalte select (sheet on phone) | `onChange` → value |
| `Button` | `label: Line`, `action: actionId` (required), `value?: json`, `variant?: "Primary"\|"Secondary"\|"Ghost"\|"Danger"` (`Secondary`), `icon?: Icon`, `disabled?`, `confirm?: string` | `[ Label ]`, focused = reversed; `confirm` → inline y/n | `.btn .btn-*` (44 px pill); `Danger` = `.btn-danger` with icon; `confirm` → small confirm popover | → `value` (or null) |
| `Spinner` | `label?: Line` | braille spinner on the shared ticker | CSS-animated spinner (no JS timer), `prefers-reduced-motion` → static dot | — |
| `Empty` | `title?: string`, `message?: string`, `icon?: Icon`, `action?: Button node` | dim centred text | `<EmptyState compact>` | via its Button |

`Icon` = one of `check, x, dot, warn, info, clock, gear, branch, bolt, bot, folder, file, search,
cloud, lock, board, terminal, globe, diff, list, chart, play, pause, stop, refresh, plus, trash`
(TUI: Unicode or Nerd Font per setting; app: Lucide).

### 4.6 Keys, local state, focus and actions

**Paths.** A node is addressed by the list of keys from the entry root, e.g.
`["main","tasks"]`. Child collections addressed by paths: `Layout.children`, `Block.child`
(key segment `"child"`), `Tabs.panes` (segment = tab key), `Empty.action` (`"action"`),
`fallback` (`"fallback"`). Items (`List.items`, `Table.rows`, `Tree.nodes`, `Log.lines`) are
not nodes; they are addressed by `(path, field, key)` in patches (§4.7).

**Local state** (per frontend, never sent unless an action fires): list/table/tree selection
and offset, scroll positions, tab selection when `panes` are local, tree expansion, collapsed
blocks, text typed into `Input`. A `state`/`value`/`selected`/`expanded`/`collapsed` prop
**wins when its value changes** in an update (compare with the previous tree), so an extension
can drive selection; otherwise the frontend's local state persists across updates. Local state
is keyed by `(entryId, path, item key)` and dropped with the entry.

**Focus.** Focusable nodes: `List`, `Table`, `Tree`, `Tabs`, `Input`, `Toggle`, `Select`,
`Button`, collapsible `Block`, `KeyHints` hints with `action`. Order = depth-first tree order.
TUI: Tab / Shift+Tab move focus within a focused entry; arrow keys act inside lists/tables/
tabs/trees; Enter activates; Esc closes when `closeOnEscape`; entry `keys` are bound while the
entry is focused. App: native focus order, the same entry `keys` bound while focus is inside the
entry, `:focus-visible` ring per DESIGN §1.6. `focus: true` on `openPanel` focuses the first
focusable node.

**Action props.** Action ids are strings `^[A-Za-z0-9_.:/-]{1,128}$` chosen by the extension.
Ids starting with `$` are system actions sent by frontends:

| System action | When | `value` |
|---|---|---|
| `$close` | user closed the entry (✕, Esc, tab close) | — |
| `$show` / `$hide` | the entry became visible / hidden in **any** / **no** attached frontend (core aggregates) | — |
| `$key` | an entry `keys` binding without its own `action` | the key id |

Extensions should pause expensive updates between `$hide` and `$show` (e.g. log tails).

**Delivery.** Frontend → core `_oino/ui_action{entryId, actionId, value?, values?, path?}`
(RPC command; same JSON over the session stream `SessionCmd`, host.md §7.3) → core emits the
`oino_ui_action` event to the owning extension instance (command-capable context, v1 U8):

```json
{"type":"oino_ui_action","panelId":"unipi.kanboard/board","actionId":"open-task",
 "value":"T-12","path":["main","tasks"],"values":{"filter":"open"},"source":"app"}
```

- `panelId` is the **entry id** (field name kept from v1 so existing handlers keep working).
- `values` = current values of every keyed `Input`/`Toggle`/`Select` in the entry, sent with
  `onSubmit`, `Button` and `custom` dialog submissions (form-style collection).
- `source` ∈ `tui | app | rpc`. Actions for unknown / disposed entries are dropped.
- `custom` dialogs: the first `Button` action or `onSubmit` resolves the dialog with
  `{actionId, value?, values}`; other actions (`onSelect`, `onChange`) are delivered as events
  without resolving.

### 4.7 Patches and diffing

Extensions may send a full tree (`openPanel{spec}`, `updatePanel{spec}`, `setWidget{widgetSpec}`)
or explicit ops (`updatePanel{patch: NodeOp[]}`). Core **always** emits ops to frontends.

```text
NodeOp =
  { op: "replace", path, node }                           // path [] = root
| { op: "set",     path, props: { <prop>: json | null } } // shallow merge; null unsets; not for collections
| { op: "splice",  path, field, index, remove: u32, insert: (Node | Item)[] }
| { op: "append",  path, field, items: Item[] }           // Log.lines, List.items, Table.rows; ring-trims to the limit
| { op: "item",    path, field, key, value: Item }        // replace one keyed item
field ∈ "children" | "items" | "rows" | "nodes" | "lines" | "titles" | "datasets" | "data" | "panes"
```

Diff algorithm (`oino_ui::patch::diff(old, new)`):

1. Different `t` or `key` at a position ⇒ `replace`.
2. Same kind: changed scalar/object props ⇒ one `set` with all changed props.
3. Collections: match by key. If the key sequence is unchanged, recurse/`item` per changed
   member. If it changed only by a tail append (Log/List/Table) ⇒ `append`. Otherwise one
   `splice` per maximal contiguous changed run (LIS keeps stable members in place).
4. If the encoded ops exceed 50 % of the encoded new node ⇒ a single `replace` instead.

Invariant (property-tested): `apply(old, diff(old, new)) == new`. Frontends apply ops in
order; any op that does not resolve (missing path/key) ⇒ the frontend requests a resync
(§9.3).

**Coalescing.** Core coalesces per entry: at most one emitted patch batch per 33 ms per entry
(ops composed; a `replace` absorbs earlier ops below its path); `setStatus` at most 4 Hz per
key. The host may coalesce further per stream (host.md §7.4) — ops are composable, so batching
never changes the result.

### 4.8 Limits and validation

| Limit | Value | On violation |
|---|---|---|
| Encoded tree per entry | 256 KiB | request rejected: `host-error` to the guest call + `extension_error` |
| Encoded patch per update | 128 KiB (send a full tree instead) | rejected |
| Depth | 32 | rejected |
| Nodes per tree | 5 000 (items count as nodes) | rejected |
| `List.items`, `Table.rows`, `Tree` nodes | 5 000 each | rejected |
| `Log.lines` | `maxLines` ≤ 5 000 (ring; older lines dropped silently) | trimmed |
| Span `content` / `Markdown.source` / `Code.source` / `Diff` | 64 KiB / 256 KiB (whole tree bound applies) | rejected |
| `Image` inline data | 1 MiB decoded (bigger ⇒ `{blob}` / `{path}`), decoded ≤ 4 MiB | rejected |
| `Chart` points | 10 000 total · `Sparkline` 1 000 | rejected |
| Children per `Layout` | 64 | rejected |
| Entries per extension per session | 64 (+ 1 `fullscreen`, 8 `sidePane`, 8 `bottomPane`) | `openPanel` fails with `host-error{code:"limit"}` |
| Key / action id length | 128 | rejected |
| Update rate | coalesced (§4.7), never rejected | — |

Validation (`oino_ui::validate`, run by core on every producer call, ≤ 1 ms for a 256 KiB tree):
parse tolerant (unknown props ignored, unknown `t` kept as `Unknown{fallback}`), then check
limits, duplicate sibling keys (⇒ rejected), action-id syntax, colour tokens (unknown token ⇒
dropped + diagnostic, not rejected). **v1 gate:** a lowercase `t` that is an OUS v1 kind
(`text`, `stack`, `split`, `card`, `section`, `kv`, `badge`, `meter`, `gauge`, `area`, …) ⇒
rejected with `"OUS v1 node '<t>' — see ui-protocol v2 §12.1"`. Diagnostics go to the session's
`extension_error` stream (shown in the TUI's extension log and the app's extension notices).

---

## 5. Theme tokens

Colours are **role tokens**. The TUI resolves them from the active pi theme
(`crates/oino-tui/src/theme.rs`, pi `themes.md` keys); the app resolves them to CSS variables
defined once in `src/ui/ui.css` on top of the `@theme` palette in `style.css` (DESIGN §1.1–1.2).
Any token may be used as `fg`, `bg` or `underlineColor`; `reset` = inherit / terminal default.
The table is normative; adding a token is a minor protocol change.

| Token | TUI (`Theme` field ← pi key) | App CSS variable (value) | Typical use |
|---|---|---|---|
| `reset` | `Color::Reset` | `inherit` | clear inherited colour |
| `text` | `text` | `--color-text` (#F5EFE6) | body text |
| `muted` | `muted` | `--color-text-muted` (#A39A90) | secondary text |
| `dim` | `dim` | `--color-text-dim` (#6E655D) | hints, timestamps |
| `accent` | `accent` | `--color-claw` (#F07818) | highlights, active marks |
| `primary` | `accent` | `--color-shell` (#F03018) | primary action, focus (shell red; never for errors) |
| `info` / `link` | `md_link` ← `mdLink` | `--color-wm-cyan` (#28BEF0) | info, links |
| `success` | `success` | `--color-wm-green` (#5AD25A) | done, ok |
| `warning` | `warning` | `--color-wm-yellow` (#FAC828) | warnings |
| `error` | `error` | `--color-wm-red` (#F03C32) | errors |
| `harness` | `border_accent` ← `borderAccent` | `--color-amber` (#F0A818) | harness / provenance rails |
| `thinking` | `thinking_text` | `--color-text-muted` | thinking text |
| `tool` | `tool_title` | `--color-text` | tool titles |
| `user` / `assistant` | `user_message_text` / `text` | `--color-text` | message-role tints |
| `code` | `md_code` ← `mdCode` | `--md-code-fg` (cream) | inline code |
| `border` | `border` | `--color-line` (#2E2825) | borders |
| `borderAccent` | `border_accent` | `--color-shell` | focused / accent borders |
| `borderMuted` | `border_muted` | `--ui-border-muted` (`rgb(46 40 37 / 60%)`) | quiet borders |
| `c1` … `c12` | `CATEGORY[0..12]` (theme.rs:185) | `--ui-c1` … `--ui-c12` = the same 12 hex values (#69ADD0, #68B78D, #CD9A22, #A798D7, #EA7F81, #5DB3BA, #DE8D5A, #9776E5, #DE54C1, #6185CC, #5EB286, #FE5462) | categorical (packages, series) |
| `surface1` / `surface2` / `surface3` | `custom_message_bg` / `user_message_bg` / `selected_bg` | `--color-surface-1/2/3` | panels, inputs, selection |
| `selectedBg` | `selected_bg` | `--color-surface-3` | selected rows |
| `successBg` / `errorBg` / `warningBg` | `tool_success_bg` / `tool_error_bg` / `tool_pending_bg` | `rgb(90 210 90 / 12%)` / `rgb(240 60 50 / 12%)` / `rgb(250 200 40 / 12%)` | tinted rows, badges |
| `infoBg` / `accentBg` | `custom_message_bg` / `selected_bg` | `rgb(40 190 240 / 12%)` / `rgb(240 120 24 / 14%)` | tinted rows |
| `diffAdded` / `diffRemoved` / `diffContext` | `diff_added` / `diff_removed` / `diff_context` | `--md-diff-add-fg` / `--md-diff-del-fg` / `--md-diff-hunk-fg` | diff-like text outside `Diff` |

Rules:

- **Contrast**: the app renders `fg` tokens on `bg` tokens as given; when an extension pairs a
  `*Bg` tint with the same-hue `fg`, contrast stays ≥ 4.5:1 on `--color-bg` (checked in
  WP-U9 golden captures for every token pair in the fixture).
- `primary` vs `accent`: DESIGN reserves shell red for primary actions and focus; `accent`
  therefore maps to `claw` in the app. In the TUI both are pi `accent`.
- Agent-state colours (DESIGN §1.3) are not tokens; extensions express state with
  `success`/`warning`/`error`/`muted` + an `Icon` + a word (colour never alone).
- Light themes: TUI resolves per theme; the app is dark-only (DESIGN §1.2) — a future light
  palette only redefines the `--ui-*` variables.

---

## 6. Placements, slots and the entry registry

### 6.1 Entry model

```text
UiEntry {
  id: string,                 // "<extensionId>/<name>"; reserved ids per §8
  extensionId: string,        // owner ("host" for host-owned reserved screens)
  scope: "session" | "host",
  slot: Slot,
  order: i32,                 // default 0; higher first
  seq: u64,                   // assigned by core at first registration; stable across updates
  title?: string, icon?: Icon, badge?: Line, tone?: Color,
  view: { kind: "tree",     root: Node }
      | { kind: "lines",    lines: string[] }                 // pi setWidget string arrays
      | { kind: "status",   text: Line }                      // setStatus / setFrame segment
      | { kind: "reserved", component: ReservedComponent, fallback?: Node, params?: json }
      | { kind: "renderer", key: string }                     // turnFooter contributors
      | { kind: "settings", namespace: SettingsNamespace },   // §7
  keys?: [{ key: KeyId, action?: actionId, label: string }],
  focus?: bool, closeOnEscape?: bool (true), size?: "small"|"medium"|"large"|"full",
  rev: u64                    // last change of this entry (cache key for frontends)
}
Slot = "overlay" | "sidePane" | "bottomPane" | "editorReplace" | "fullscreen"
     | "widget" | "widgetBelow" | "status" | "turnFooter" | "header" | "footer" | "settings"
```

### 6.2 Slots

| Slot (placement) | Producer | TUI | App desktop | App phone | Multiplicity | Height |
|---|---|---|---|---|---|---|
| `overlay` | `openPanel{placement:"overlay"}`, reserved `unipi.info-screen/dashboard` | centred modal box (`render/overlays.rs modal`) | **modal**: centred dialog (`--radius-float`), width by `size` (420/560/800/full), title bar + ✕ + `keys` as footer chips | bottom sheet (corvu), `large`/`full` = full height | stack; top = most recently opened; dialogs (§3) always above | definite if root requests (§4.4) |
| `sidePane` | `openPanel{placement:"sidePane"}`, reserved Files/Git/Browser/Terminal/Board | overlay (as v1) with a tab strip when > 1 sidePane entry | **right-pane tab** in the Z-5 tabs container (zcode-audit Z-5): tab = `icon` + `title` + `badge`; ✕ sends `$close` | full-screen layer (Z-5 phone rec), listed in the chat ⋯ menu group "Panes" | many; tab order §6.4 | definite |
| `bottomPane` | `openPanel{placement:"bottomPane"}`, reserved Terminal | `editorReplace` region (v1) | **bottom-panel tab** (terminal panel, host.md §9.3) | full-screen layer via the same "Panes" group | many | definite |
| `editorReplace` | `openPanel{placement:"editorReplace"}` (work tray, /btw) | replaces the composer block; Esc returns | **composer tray**: replaces the composer like DialogCard (dialogs win), Esc returns | same | stack; top visible | auto ≤ 50 vh |
| `fullscreen` | `openPanel{placement:"fullscreen"}`, reserved Board/Cron | takes over the transcript area until closed | **sidebar page**: nav item under group "Extensions" (reserved: own nav items), route `/x/<entryId>` | page from the Chat-tab menu "More" | ≤ 1 per extension; nav order §6.4 | definite |
| `widget` | `setWidget` (`aboveEditor`, default) | lines/tree above composer | stack **above the composer** (each card-less, 1 px top rule between) | same, > 3 rows collapsed with "expand" | many, ordered | auto |
| `widgetBelow` | `setWidget{widgetPlacement:"belowEditor"}` | below composer | below the composer (desktop); phone folds into `widget` | — | many | auto |
| `status` | `setStatus`, `setFrame` segments, `setFooter` | footer status line | **status bar** segments (chat footer row, zcode-audit G-4/Z-95 decide the row's final home) | status strip | many; `priority` → `order` | 1 row |
| `turnFooter` | renderer registration (§6.7), reserved file summary | lines after the turn's last row | **end of turn** inside the turn's virtual row | same | many per turn, ordered | auto ≤ 160 px |
| `header` | `setHeader` | header block above transcript | band at transcript top (scrolls away) | same | many, ordered | auto |
| `footer` | `setFooter{spec}` | footer rows | folded into the status bar (rule 4.2.2) | strip | many | 1 row |
| `settings` | manifest / `register-settings` namespaces (§7) | settings hub | **Settings section** per namespace in the one Settings screen (host.md §9.9) | same | one per namespace | — |

`header`, `widgetBelow` and `footer` are the pi placements not in DIRECTION's table; they map onto
existing surfaces so no redundant UI appears.

### 6.3 Producers → entries

| Producer call | Entry id | Slot | View |
|---|---|---|---|
| `setWidget{widgetKey:k}` | `<ext>/widget:<k>` | `widget` / `widgetBelow` | `tree` (`widgetSpec`) else `lines`; absent both ⇒ delete |
| `setStatus{statusKey:k}` | `<ext>/status:<k>` | `status` | `status` (`spans` or `statusText` + `tone`); `priority` → `order` |
| `setFrame{frameKey:k, top, bottom}` | `<ext>/frame:<k>:<i>` per segment | `status` | `status` (TUI keeps drawing the editor frame from the same entries) |
| `setHeader{key:k}` / `setFooter{key:k}` | `<ext>/header:<k>` / `<ext>/footer:<k>` | `header` / `footer` | `tree` |
| `openPanel{panelId?, placement}` | `panelId` or `<ext>/panel-<n>` (v1 U8) | placement | `tree`, or `reserved` for reserved ids (§8) |
| `updatePanel` / `closePanel` | same id | — | `meta` / tree ops / delete |
| `register-renderer{target: turn-footer, key}` | `<ext>/turnFooter:<key>` | `turnFooter` | `renderer` |
| manifest `ui.settings` / `register-settings` | `<ext>/settings:<namespace>` | `settings` | `settings` |
| manifest `ui.reserved` | the reserved id | its default slot | `reserved` |
| `setTitle`, `setWorkingMessage`, `setShortcuts` | registry **meta** (not entries) | — | `UiSnapshot.meta` |

An extension can only create ids under its own `<extensionId>/` prefix; foreign ids are rejected
(`host-error{code:"denied"}`), reserved ids only by their owner (§8).

### 6.4 Ordering and multiple contributors

- Multi-entry slots (`sidePane`, `bottomPane`, `fullscreen`, `widget*`, `status`, `turnFooter`,
  `header`, `footer`, `settings`) render in **`order` desc, then `seq` asc** (Koishi `insert()`
  semantics, stable). Updating an entry keeps its `seq`; closing and reopening assigns a new one.
- Stack slots (`overlay`, `editorReplace`) show the **most recently opened or focused** entry;
  the others wait underneath and reappear when it closes. A dialog (§3) always sits above both.
- Built-in order constants (reserved entries; extensions default to 0 so they come after):
  side pane Files 1000, Git 990, Browser 980, Terminal 970, Board 960; bottom panel Terminal
  1000; sidebar pages Board 1000, Cron 990; settings: core/pi namespaces 1000, app/host 900,
  extensions 0 (then title A→Z). Third-party `order` is clamped to `[-100, 100]`.
- Users may reorder side-pane / bottom-panel tabs; the app stores the order per workspace
  locally and applies it on top of registry order (registry order is the default only).
- Per-slot caps on the phone: `widget` shows ≤ 3 entries expanded (rest collapsed into
  "+N widgets"), `status` ≤ 4 segments (rest in the overflow popover).

### 6.5 Focus and visibility

- `focus: true` on `openPanel` activates the entry (selects its tab, opens its page or
  modal) **only if** the user is on that session; otherwise the entry gets an unread dot.
- Frontends report visibility changes with `_oino/ui_visible{entryId, visible}`; core
  aggregates over frontends and sends `$show` / `$hide` (§4.6).
- Re-opening an existing id replaces its view and keeps its tab position (`seq`).

### 6.6 Lifetime and disposal

Every session-scope entry is owned by `(sessionId, extensionId, instanceGeneration)`; host-scope
entries by `(extensionId)` in the host catalog. Core deletes entries (and withdraws the owner's
dialogs with `extension_ui_dismiss{reason:"cancelled"}`, v1 §3) when:

| Event | Removed |
|---|---|
| `closePanel` / clearing producer call (`setWidget` without content, …) | that entry |
| user `$close` on a panel | entry deleted after the event is delivered (the extension may reopen) |
| extension instance dropped: session closed/unloaded, `/reload`, extension **disabled** for the session, extension **trapped/crashed** (core.md §2.3) | all session entries of that instance |
| session hibernates (core.md §2.2) | nothing — the registry is kept as data; actions wake the session |
| extension disabled / uninstalled host-wide, safe mode (core.md §2.3) | its host-scope entries (reserved, settings) |
| `session_start` after resume | nothing to replay: extensions rebuild their UI from session `custom` entries + kv (DIRECTION §3.1.6) and re-emit; the registry starts empty for the new instance |

Frontends treat a deleted entry as gone: close its modal/tab/page (a focused page falls back to
Chat, like Koishi `Activity.dispose`'s redirect), drop its local state.

### 6.7 `turnFooter` (end of turn)

- Contributor: `register-renderer({target: turn-footer, key})` (WIT `renderer-target` gains
  `turn-footer`, §9.1). Core creates entry `<ext>/turnFooter:<key>`.
- A **turn** is identified by `turnEntryId` = the session entry id of the user message that
  started it. Frontends call `_oino/render{target:"turnFooter", key, turnEntryId, width,
  density}` for turns that are **visible** (virtualizer range ± 1 screen) and the running turn;
  core calls the guest `render` export (sync, 50 ms deadline, v1 §6.4) with
  `{target:"turnFooter", key, turnEntryId, turnIndex, running}` → `Node | null`.
- Cache: frontends cache by `(entryId, turnEntryId, footerRev)`. The extension invalidates with
  `fire{method:"invalidateTurnFooter", key, turnEntryIds?: string[]}` (absent = all) → core
  bumps the entry's `rev` and emits `{op:"invalidate", id, turns?}` (§9.2). The running turn is
  re-requested at most every 500 ms while it changes.
- App: the footer renders inside the turn's virtual row; a height change updates that turn's
  entry in the LRU row-height cache (DIRECTION §4.2.3) without re-measuring other turns.

---

## 7. Settings schema dialect (OSS-2)

One dialect describes core (pi `settings.json`), host/app and extension settings. It is v1 §6.6
(implemented in `oino-core/src/extensions/settings.rs`, `unipi-kit/src/settings/schema.rs`,
`oino-view/src/settings_hub.rs`) plus the additions marked **new**. Existing v1 namespaces stay
valid without edits.

```text
SettingsNamespace {
  namespace: string,              // unique per host; [a-z0-9][a-z0-9.-]*; collision ⇒ second registration rejected
  title: string, extensionId: string ("oino" for core, "host"/"app" for host.md §9.9 groups),
  icon?: Icon, order?: i32, description?: string,          // new: icon, order, description
  defaults?: object,              // values for keys without a field default
  sections: Section[]
}
Section { id, title, description?, advanced?: bool, fields: Field[] }
Field (common) {
  key: string (dot path into the namespace object), type, label, description?, hint?, badge?,
  default?: json, scope?: "both"|"global"|"project" (both), advanced?: bool,
  dynamic?: { service: string, op?: string },             // options via services/host-service call
  visibleIf?: Cond, disabledIf?: Cond,                    // new
  restart?: "session" | "host"                            // new: shows "applies after restart"
}
Field types:
  boolean
  enum        { options: [{value, label, description?}], allowCustom?: bool, display?: "select"|"segmented"|"radio" }
  multiselect { options, min?, max? }
  string      { multiline?, pattern?, placeholder?, minLength?, maxLength?, format?: "path"|"url"|"glob"|"keybinding" }
  number      { min?, max?, step?, unit?, integer?: bool }
  secret      { }                                          // stored via secrets, never echoed
  order       { items: string[] }
  model       { filter?: {input?: string[], reasoning?: bool} }
  action      { command: string, confirm?: string }       // runs command-invoke
  page        { sections: Section[] }
  list        { item: FieldWithoutKey, min?, max?, addLabel? }     // new (e.g. MCP servers)
  dict        { value: FieldWithoutKey, keyPattern?, keyLabel? }   // new (e.g. env maps)
  json        { schema?: JSONSchema }                               // new: escape hatch, code editor
Cond = { key: string, eq?: json, ne?: json, in?: json[], truthy?: bool }
```

Semantics (Schemastery model, implemented once in `oino_ui::settings` and used by core, TUI, and
— via a TS port — the app):

1. **resolve(stored, scope)** → effective value per key = field default ← `defaults` ← global
   stored ← project stored (project wins for `scope: both`). Invalid stored values (type,
   enum membership, pattern, range) resolve to the next layer down and produce a diagnostic
   `{namespace, key, message}`; resolve **never fails** (DESIGN §5.5 behaviour).
2. **simplify(effective)** → what is written: only keys that differ from the layer below
   (writing a default value **unsets** the key). Writes go through the existing commands
   `_oino/set_extension_setting` / `_oino/set_setting` / `host_settings_set` (host.md §9.9).
3. **Validation** happens in core (`check_value` extended for the new types); the app also
   validates for instant feedback with a zod schema generated at runtime from the field
   (`toZod(field)`), never trusting its own result over core's reply.
4. `visibleIf` / `disabledIf` are evaluated on effective values (same scope view).
5. Multiple contributors: each extension owns its namespaces; core contributes `oino.*`
   namespaces for pi settings (model, thinking, compaction, retry, shell, …) and the host
   contributes `host.*` / `app.*` (host.md §9.9) in the same dialect, so the app renders one
   form engine for all three.

**App rendering** (WP-U11): the Settings screen's section list (DESIGN §5.8 two columns) groups
`Agent` (oino), `Extensions` (one item per namespace, `icon` + `title`, order §6.4), `App & host`.
A section page renders fields as: boolean → `Switch.tsx` row; enum → select (`segmented` ≤ 4
options); multiselect → checkbox list; string → input/textarea; number → numeric input with unit
suffix + stepper; secret → masked input with "Replace…"; order → drag list (desktop) / up-down
buttons (phone); model → the model picker sheet; action → `.btn-secondary`; page → drill-in row;
list/dict → repeatable rows with add/remove; json → CodeMirror JSON. Every field row shows
label, description, "default" or "modified · Reset" and a scope badge; a scope toggle
(Global | Project) sits in the section header when any field is `both`/`project`. Search covers
labels, descriptions and keys of all namespaces. Disabling an extension hides its namespace but
keeps stored values. The per-extension enable toggle (DIRECTION §3.3.4) is the first row of each
extension's section (`_oino/set_extension_enabled`), so there is one place per extension.

**TUI rendering**: the existing settings hub (`oino-view/settings_hub.rs`) gains the new types
(`list`/`dict` as sub-pages, `json` via `editExternally`).

---

## 8. Reserved panel ids (first-party components)

Reserved entries render a **native app component** instead of a node tree. They exist in the
host-scope registry while their owner is enabled; extensions open/focus them with
`openPanel{panelId: <reserved id>, placement?, params?}` (empty `spec`) and receive `$close`
like any panel. They talk to the host through host services / the CLI (ext-abi.md WP-X1/X3/X4),
never through node trees.

| Reserved id | Component (app) | Owner — gate | Default slot (allowed) | TUI | Data path |
|---|---|---|---|---|---|
| `unipi.kanboard/board` | Board (`screens/Board.tsx`) | `unipi.kanboard` enabled **and** `unipi-kanboard` CLI present | `fullscreen` (`sidePane`) | owner's `fallback` tree (task list) | `unipi-kanboard` CLI via host (ext-abi.md §5.5) |
| `unipi.cron/jobs` | Cron (`screens/Cron.tsx`) | `unipi.cron` enabled and host service `cron` | `fullscreen` | `fallback` | `cron_*` ops / job files (ext-abi.md §6) |
| `host/files` | Files tree + editor (`screens/Files.tsx`, `Editor.tsx`) | host advertises `fs` | `sidePane` (`fullscreen`) | — (no entry) | `fs_*` (host.md §9.1) |
| `host/git` | Git pane (Z-96/Z-97) | host advertises `git` and session cwd is a repo | `sidePane` | — | `git` + `GitCmd::TurnFiles` (host.md §9.4) |
| `oino.browser/browser` | Browser pane (browser.md §9) | `oino.browser` enabled and host service `browser` | `sidePane` | — (tools report unavailable, browser.md §9.6) | `browser_*` + screencast |
| `host/terminal` | Terminal (xterm.js) | host advertises `pty` | `bottomPane` (`sidePane`) | — | `term_*` (host.md §9.3) |
| `unipi.info-screen/dashboard` | Info / Session modal: Context / Running / Info tabs (`ControlCentre.tsx` Session sheet) | `unipi.info-screen` enabled | `overlay` | owner's existing tree (v2 nodes) | session commands (`get_session_stats`, `_oino/agents`, …) |
| `unipi.long-horizon/visualize` | LH status section inside the Git pane + the collapsed-pane capsule (DIRECTION §4.3; `ProgressSheet.tsx` content) | `unipi.long-horizon` enabled | `sidePane` (opens/focuses Git, scrolls to the LH section) | owner's tree (overlay) | LH topics / `_oino/topics` |
| `<file-summary-ext>/turn-files` | Turn file summary "N files +X/−Y", expand, undo per file / set (zcode-audit D-1) | the file-snapshot extension enabled (id fixed by its implementation; proposed `unipi.file-summary`) | `turnFooter` | its turn-footer renderer output | `_oino/checkpoints` / `_oino/rewind` (`oino-core/src/session/checkpoints.rs`) |

Rules:

1. **Declaration.** The owner lists its reserved ids in its manifest
   (`"ui": {"reserved": ["unipi.kanboard/board"]}`); core accepts only ids from the table above
   whose owner prefix equals the manifest id. Host-owned ids (`host/*`) are created by the host
   from its advertised services (`host_info`), not by an extension.
2. **Gate.** Entry present ⇔ gate true. Disabling the owner (app toggle, `--exclude-extension`,
   safe mode, crash) deletes the entry; the app hides the nav item/tab and closes an open view
   (focus falls back to Chat). No "disabled" placeholders.
3. **Fallback.** Frontends declare which reserved components they implement (§10). For other
   frontends core delivers the entry as `view.kind:"tree"` with the owner's `fallback` (set via
   `updatePanel{panelId, fallback}`), or omits it when there is none.
4. **No duplicates** (DIRECTION §4.2.2): an extension must not open a tree panel that duplicates
   a reserved surface (review gate in WP-U7); e.g. kanboard's v1 in-chat board marker becomes a
   reserved open, the long-horizon `/visualize-progress` overlay becomes a reserved focus.
5. Third-party extensions cannot claim reserved ids or new ones; the table changes only by
   amending this spec.

---

## 9. Wire messages

### 9.1 Extension → core (WIT, rides the `oino:extension@2.0.0` cut of ext-abi.md WP-X6)

`ui.wit` keeps its function set; only the JSON accepted changes. Deltas:

| Item | Change |
|---|---|
| `ui.fire(json)` | methods of §3/§6.3 + **new** `invalidateTurnFooter{key, turnEntryIds?}`; `widgetSpec`/`spec` are v2 nodes |
| `ui.panel-open(json)` | `{panelId?, placement, title?, icon?, badge?, order?, spec?, keys?, focus?, closeOnEscape?, size?, params?}` → entry id. Reserved ids (§8) ignore `spec` |
| `ui.panel-update(id, json)` | `{spec?}` (whole tree, diffed) **or** `{patch: NodeOp[]}`, plus optional `title`, `icon`, `badge`, `order`, `keys`, `fallback` |
| `ui.panel-close(id)` | unchanged |
| `registry.renderer-target` | enum gains **`turn-footer`** |
| `registry` | **new** `register-settings: func(namespace: json)` (runtime namespaces; manifest `ui.settings` stays the static path) |
| `guest.render(request, ctx)` | request `target` may be `"turnFooter"` (§6.7); result = v2 `Node` JSON or none |
| manifest | **new** `"ui": {"reserved": [id…], "settings": [SettingsNamespace…]}` (`settings` replaces the v1 init-time `settings_namespace` registration; both accepted in 2.0) |
| event `oino_ui_action` | payload of §4.6 (`values`, `path`, `source` added) |

Errors: invalid trees/patches return `host-error{code:"invalid", message}` from the call **and**
emit `extension_error` (diagnostic text of §4.8).

### 9.2 Core → frontend

```text
UiSnapshot {                       // session scope: SessionSnapshot.ui; host scope: HubEvent::Ui
  scope: "session" | "host", sessionId?: string, rev: u64,
  entries: UiEntry[],              // registry order not implied; frontends sort (§6.4)
  dialogs: UiRequest[],            // open dialogs, pi request shape (first-answer-wins state)
  meta: { title?, workingMessage?, workingVisible?, shortcuts: Shortcut[] }
}
UiPatch { scope, sessionId?, rev: u64 /* = previous + 1 */, ops: RegistryOp[] }
RegistryOp =
  { op: "put",  entry: UiEntry }                       // add or replace whole entry
| { op: "del",  id }
| { op: "meta", id, set: { title?, icon?, badge?, order?, keys?, tone?, focus? } }
| { op: "tree", id, rev, ops: NodeOp[] }               // §4.7
| { op: "invalidate", id, turns?: string[] }           // turnFooter caches
| { op: "dialog", request: UiRequest }                 // dialog opened (also still sent as extension_ui_request for pi clients)
| { op: "dismiss", id, reason: "answered"|"timeout"|"cancelled" }
| { op: "sessionMeta", set: { title?, workingMessage?, workingVisible?, shortcuts? } }
| { op: "effect", request: UiRequest }                 // one-shot (notify, set_editor_text, openUrl, …); not in snapshots
```

- `rev` is per scope and monotonic. A frontend that sees `rev ≠ last + 1` (or fails to apply an
  op) sends `_oino/ui_resync{scope, rev}` and gets a fresh `UiSnapshot`.
- Session events: patches travel as the `ClientEvent` `{"type":"oino_ui_patch", …UiPatch}`, so
  they share the session stream's seq, ring buffer, resume and ordering guarantees
  (host.md §7.3–7.4). Host coalescing must not merge across `rev`s out of order; it may
  concatenate consecutive patches into one (rev = last).
- Host scope: host.md §7.4 `HubEvent` gains `Ui{snapshot? , patch?}` on `events_open`; the
  first event after open is a `snapshot`.

### 9.3 Frontend → core

| Command (RPC `_oino/*`; over the host: `Control::SessionCmd`, host.md §7.3) | Payload | Result |
|---|---|---|
| `extension_ui_response` (pi) | `{id, value? \| confirmed? \| cancelled?}` | — |
| `_oino/ui_action` | `{entryId, actionId, value?, values?, path?}` | `{}` once dispatched (v1 U8) |
| `_oino/ui_visible` | `{entryId, visible: bool}` | `{}` |
| `_oino/ui_snapshot` | `{scope?}` | `UiSnapshot` (v2 shape; replaces the v1 replay list) |
| `_oino/ui_resync` | `{scope, rev}` | `UiSnapshot` |
| `_oino/render` | v1 `RenderRow` + `target: "turnFooter"`, `turnEntryId` | `{node: Node \| null}` |
| `_oino/hello` | `{…, ui: UiCapabilities}` (§10) | — |

Host-scope actions (reserved entries have none; settings use their own commands) go through the
same `_oino/ui_action` on the hub with `scope:"host"`.

### 9.4 Per transport

| Consumer | Receives | Notes |
|---|---|---|
| TUI (`oino-tui`, in-process) | `UiSnapshot`/`UiPatch` through `oino-view`'s registry mirror over the local client channel | no JSON hop needed; same types |
| `oino --mode rpc`, Oino client (`_oino/hello` with `ui.protocol: 2`) | `_oino/ui_patch` events + pi `extension_ui_request` for dialogs | |
| pi RPC client (no hello) | pi records only (§10) | |
| ACP | dialogs → permission requests (unchanged acp mapping); slots dropped except `setPlan` | |
| unipi-host → app | session: `SessionSnapshot.ui` + `oino_ui_patch` events; host: `HubEvent::Ui` | host forwards verbatim, never interprets nodes |

---

## 10. Capabilities and degradation

`UiCapabilities` (sent in `_oino/hello`; the host sends the app's at stream open, host.md §7.3):

```json
{"protocol": 2,
 "dialogs": ["select","confirm","input","editor","multiSelect","form","approval","custom",
             "clipboardReadImage","editExternally"],
 "slots": ["overlay","sidePane","bottomPane","editorReplace","fullscreen","widget",
           "widgetBelow","status","turnFooter","header","footer","settings"],
 "reserved": ["board","cron","files","git","browser","terminal","sessionInfo","lhStatus","turnFiles"],
 "nodes": "all", "images": true, "osNotify": true, "clipboard": true}
```

`ReservedComponent` names: `board, cron, files, git, browser, terminal, sessionInfo, lhStatus,
turnFiles` (one per §8 row). The TUI declares `reserved: []`.

Degradation, per frontend (core does it; frontends never see what they did not declare):

| Missing capability | Core behaviour |
|---|---|
| a slot | entries of that slot are omitted for that frontend; `overlay` missing ⇒ `custom` dialogs degrade as below |
| a reserved component | `fallback` tree if the owner set one, else omitted (§8 rule 3) |
| `nodes` (no v2 renderer) | trees rendered to text with `oino_ui::plain::to_lines(node, 80)` and sent as `widgetLines`; panels omitted |
| Oino dialogs | v1 §10 rules kept: `custom` → `select` over its Buttons; `form` → sequential `select`/`input`; `multiSelect` → repeated `select` + "Done"; `approval` → `select` over option labels |
| pi client (no hello) | pi methods only; Oino-only dialogs resolve cancelled; Oino fields stripped (v1 U9) |

---

## 11. Guest builder crate `oino-ui`

**Crate:** `oino-ui` at `crates/oino-ui` (root workspace, core.md §2.4 layout: it joins the
blank-core crate set). Dependencies: `serde`, `serde_json` only (+ optional `schemars` behind
feature `schema`). No ratatui, no tokio → builds for `wasm32-wasip2` and stays small in guests.
The extensions workspace path-depends on it like `oino-ai` (`extensions/Cargo.toml`
`[workspace.dependencies] oino-ui = { path = "../crates/oino-ui" }`); `oino-extension`
re-exports it as `oino_extension::ui::widgets` and adds the slot calls.

### 11.1 Module layout

```text
crates/oino-ui/src/
  lib.rs            prelude; Node; IntoNode; Unknown
  style.rs          Color (token enum §5), Modifier (bitflags-like set), Style, Stylize trait
  text.rs           Span, Line, Text, Alignment (+ From<&str>/String/Vec<_>)
  layout.rs         Layout, Direction, Constraint, Flex, Margin (data only; no solver)
  widgets/          block.rs (Block, Borders, BorderType, Padding, Title) paragraph.rs (Paragraph, Wrap)
                    list.rs (List, ListItem, ListState, HighlightSpacing) table.rs (Table, Row, Cell, TableState)
                    tabs.rs gauge.rs (Gauge, LineGauge) sparkline.rs barchart.rs (BarChart, BarGroup, Bar)
                    chart.rs (Chart, Dataset, Axis, GraphType, Marker, LegendPosition) scrollbar.rs
  extras/           markdown.rs code.rs diff.rs image.rs log.rs tree.rs key_hints.rs input.rs
                    toggle.rs select.rs button.rs spinner.rs empty.rs icon.rs
  node.rs           enum Node (serde tag "t"), common props (key, style, block, fallback)
  patch.rs          NodeOp, diff(old, new), apply(&mut tree, ops)
  slot.rs           Slot, Placement, Panel (openPanel/updatePanel JSON), UiEntry, RegistryOp,
                    UiSnapshot, UiPatch, ReservedComponent, UiCapabilities, Action event type
  settings.rs       SettingsNamespace, Section, Field, FieldType, Cond, resolve(), simplify()
  limits.rs         Limits, validate(&Node) -> Result<(), Vec<Diagnostic>>, v1 gate
  plain.rs          to_lines(&Node, width) — text fallback for clients without a renderer
  schema.rs         (feature "schema") JSON Schema export; test asserts parity with Appendix A
```

### 11.2 API mirrors ratatui

| ratatui | `oino-ui` | Difference |
|---|---|---|
| `Block::bordered().title("x").border_type(BorderType::Rounded).padding(Padding::uniform(1))` | identical | `.child(node)` and `.collapsible(bool)`, `.rail(Color)` extras |
| `Paragraph::new(text).wrap(Wrap { trim: true }).block(b)` | identical | — |
| `"txt".bold().fg(Color::Red)` (`Stylize`) | `"txt".bold().fg(Color::Error)` | colours are tokens (`Color::Accent`, `Color::C(7)`, …) |
| `List::new(items).highlight_symbol("› ")` + `ListState` | identical + `.state(ListState::default().with_selected("k"))` | selection by **key**; `.on_select("id")`, `.on_activate("id")` |
| `Table::new(rows, widths).header(Row::new([...]))` | identical | `.on_select`, row `.key(...)` |
| `Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).split(area)` | `Layout::vertical([..]).children([a, b])` | data, not geometry: no `split`; children attached |
| `frame.render_widget(w, area)` | `w.into()` → `Node` | the frontend renders |

### 11.3 Example

```rust
use oino_extension::ui::{self, widgets::prelude::*};

fn board_panel(tasks: &[Task]) -> Node {
    let header = Paragraph::new(Line::from(vec![
        "Board ".bold(),
        Span::from(format!("{} open", tasks.len())).fg(Color::Muted),
    ]));
    let list = List::new(tasks.iter().map(|t| {
        ListItem::new(t.title.as_str())
            .key(&t.id)
            .detail(Line::from(t.status.as_str()).fg(Color::Dim))
            .badge(Span::from(t.priority.as_str()).bg(Color::WarningBg))
    }))
    .highlight_symbol("› ")
    .on_select("select-task")
    .on_activate("open-task")
    .block(Block::bordered().title("Tasks").border_type(BorderType::Rounded));
    Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)])
        .children([
            header.into(),
            list.into(),
            KeyHints::new([("enter", "open"), ("n", "new")]).hint_action("n", "new-task").into(),
        ])
        .into()
}

// open / update / close (oino-extension::ui):
let id = ui::Panel::new("tasks").placement(Placement::SidePane).title("Tasks")
    .icon(Icon::List).spec(board_panel(&tasks)).focus(true).open()?;
ui::Panel::update(&id).spec(board_panel(&tasks)).send()?;           // whole tree, core diffs
ui::Panel::update(&id).patch([NodeOp::append(["log"], "lines", lines)]).send()?;  // hot path
ui::widget("progress", Some(Gauge::default().ratio(0.4).label("40%").into()))?;
ui::status("mode", Some(Line::from("Graph 3/7").fg(Color::Info)))?;
ui::register_turn_footer("files")?;                                 // + guest render export
```

`oino-extension::ui` also keeps the pi-shaped helpers (`select`, `confirm`, `input`, `notify`,
`status(key, &str)`, `widget_lines`) unchanged.

---

## 12. Migration from OUS v1 and TUI renderer plan

### 12.1 Node mapping (old → new)

| OUS v1 `t` | v2 | Prop mapping |
|---|---|---|
| `text` | `Paragraph` | `value: Rich` → `text: Text` (Rich span `{text,tone,bold,italic,dim,underline,code,link,strike}` → `Span{content, style:{fg:tone, addModifier:[BOLD,ITALIC,DIM,UNDERLINED,CROSSED_OUT]}, link}`; `code` → `fg:"code"`); `wrap:false` → `wrap:false` |
| `markdown` | `Markdown` | `value` → `source` |
| `code` | `Code` | `value` → `source`; `language`, `lineNumbers` kept; `language:"mermaid"` kept |
| `diff` | `Diff` | `patch` / `{oldText,newText,path}` → `patch` / `old,new,path` |
| `image` | `Image` | `data+mimeType` → `src:{data,mimeType}`; `path` → `src:{path}`; `maxWidth` kept |
| `log` | `Log` | `lines: string[]` kept; `tail` → `follow`; `maxLines` kept |
| `kv` | `Table` | 2 columns, no header, `widths:[{"Max":24},{"Fill":1}]`, key cell `fg:"muted"` |
| `table` | `Table` | `columns[].title` → `header`; `align` → cell `Line.alignment`; `width auto\|fill\|N` → `Min(0)\|Fill(1)\|Length(N)`; `selectable: id` → `onSelect: id` |
| `list` | `List` | `items[{id,label,detail,badge,tone}]` → `ListItem{key,content,detail,badge,style.fg}`; `selectable` → `onSelect`; `selected` → `state.selected`; `multi` kept; `columns:"auto"` (U6) dropped |
| `tree` | `Tree` | `nodes[{id,label,children,expanded}]` → `{key,label,children,expanded}`; `selectable` → `onSelect` |
| `tabs` | `Tabs` | `tabs[{id,title,body}]` → `titles[{key,title}]` + `panes{key: body}`; `active` → `selected`; `onChange` → `onSelect` |
| `split` | `Layout` | `direction horizontal\|vertical` → `Horizontal\|Vertical`; `ratios` → `Ratio(r_i, Σr)` |
| `stack` | `Layout` | `children` kept, constraints = `Fill(1)` each (auto mode = stack), `gap` → `spacing` |
| `section` | `Block` | `title` → `title`, `body` → `child`, `collapsible`/`collapsed` kept, `borders:["TOP"]` |
| `card` | `Block` | `Block::bordered().border_type(Rounded)`, `rail` → `rail`, `source` → bottom title `{position:"Bottom", content: source}`; `effect` (U6) dropped |
| `badge` | `Span` | inside a `Paragraph`: `{content:" x ", style:{bg:"<tone>Bg", fg:tone}}` (chip in the app) |
| `meter` | `LineGauge` | `value/max` → `ratio`; `label`; `tone` → `filledStyle.fg`; `effect`/`durationMs` dropped |
| `gauge` | `Gauge` | `value` → `ratio`, `label` |
| `sparkline` | `Sparkline` | `values` → `data`; `label` → `block.title` |
| `barChart` | `BarChart` | `bars[{label,value,tone}]` → one `BarGroup{bars:[Bar{label,value,style.fg}]}`; `columns` → `direction` |
| `area` | `Chart` | `series[{label,values}]` → `datasets[{name, data:[[i, v]], graphType:"Line", marker:"Braille"}]`, axes from data bounds |
| `spinner` | `Spinner` | `label` |
| `divider` | `Block` | `Block{borders:["TOP"], title: label}` with no child, `Length(1)` |
| `keyHints` | `KeyHints` | kept (+ optional `action`) |
| `input` | `Input` | `id` → `key`; `onSubmit` kept; `multiline` kept |
| `toggle` | `Toggle` | `id` → `key`; `value`, `onChange` kept |
| `select` | `Select` | `id` → `key`; `options`, `value`, `onChange` kept |
| `button` | `Button` | `actionId` → `action`; `primary:true` → `variant:"Primary"`; `tone:"error"` → `variant:"Danger"` |
| `empty` | `Empty` | `message` kept |
| `responsive` (U1) | `Layout.collapse` | variants → one horizontal Layout with `collapse` (or the extension picks by its own width hint) |

Panel / method mapping: `openPanel` fields keep their names (`panelId`, `placement`, `title`,
`spec`, `keys`, `focus`, `closeOnEscape`); `keys[].actionId` → `keys[].action`; `_oino/ui_snapshot`
now returns `UiSnapshot` (§9.2) instead of a request list; `extension_ui_action{panelId, actionId,
value}` → `_oino/ui_action{entryId, actionId, value, values, path}` (event payload keeps
`panelId`). Settings v1 namespaces are valid OSS-2 unchanged.

### 12.2 Crates that change

| Crate / file | Change |
|---|---|
| `crates/oino-ui` (new) | WP-U1/U2 |
| `crates/oino-core/src/ui.rs` → `ui/{mod.rs, bridge.rs, registry.rs}` | `UiBridge` keeps dialogs; `retention_key` replay replaced by `UiRegistry` (entries, seq/order, rev, diff, coalescing, dispose hooks, `$show/$hide` aggregation) |
| `crates/oino-core/src/session/{frontend.rs, feeds.rs}` | `ui_action` v2 payload; `ui_snapshot` → `UiSnapshot`; `ui_visible`, `ui_resync`; turnFooter `render` dispatch |
| `crates/oino-core/src/extensions/settings.rs` | OSS-2 types (`list`/`dict`/`json`/`visibleIf`/`restart`), `resolve`/`simplify` from `oino_ui::settings`; `register-settings` |
| `crates/oino-extension-api` (`ops`, `registry`) | `RendererTarget::TurnFooter`, `RegisterSettings`, manifest `ui` section |
| `crates/oino-wasm-host/src/{imports/runtime.rs, core_bridge.rs}` | panel calls → registry; patch form of `panel-update` |
| `crates/oino-rpc/src/{wire.rs, backend.rs, lib.rs}` | `_oino/ui_patch` event, `_oino/ui_{action,visible,resync}`, hello `ui` capabilities, degradation |
| `crates/oino-view/src/{ous.rs (delete), ui_state.rs, settings_hub.rs, lib.rs}` | `ui_state` becomes a registry mirror (apply snapshot/patch) + dialog queue; settings hub on OSS-2 |
| `crates/oino-tui/src/render/{ous.rs → ui/, overlays.rs, transcript.rs, chrome.rs, mod.rs}`, `app/mod.rs` | §12.4 |
| `crates/oino/src/faux.rs`, tests in `crates/oino/tests/{frontend,rpc}.rs`, `crates/oino-tui/tests/*` | fixtures to v2 |
| `wit/oino-extension/{ui.wit, registry.wit, guest.wit}` | §9.1 (doc comments + `turn-footer` + `register-settings`) |

### 12.3 Extensions that change

All OUS emitters (found by `rg '"t"\s*:' extensions --glob '*.rs'` + `unipi_kit::ous` callers):

| Group | Files |
|---|---|
| SDK | `sdk/oino-extension/src/ui.rs` (Panel builder, re-exports), `sdk/unipi-kit/src/{ous.rs (delete → oino-ui), tray.rs, jev.rs}`, `sdk/unipi-kit/src/settings/*` (OSS-2) |
| Panels | `unipi/info-screen/src/{lib,pages,kit,splash,stats,usage}.rs` (+ reserved `dashboard`), `unipi/kanboard/src/{panel,lib}.rs` (reserved board + fallback), `unipi/long-horizon/src/lib.rs` (reserved `visualize`), `unipi/btw/src/lib.rs`, `unipi/core/src/{tray,lib}.rs`, `unipi/updater/src/view.rs`, `unipi/skill-registry/src/editor.rs`, `unipi/mcp/src/lib.rs`, `unipi/utility/src/{answer/mod,rename/mod,attach}.rs`, `unipi/fusion/src/lib.rs`, `unipi/workflow/src/{lib,plan}.rs`, `unipi/dream/src/{lib,tray}.rs`, `unipi/memory/src/lib.rs` |
| Render specs / cards | `unipi/utility/src/render/{mod,tools,harness}.rs`, `unipi/background-tasks/src/{cards,pane}.rs`, `unipi/compactor/src/{card,blocks,cut,source,lib}.rs`, `unipi/subagents/src/{lib,view,child}.rs`, `unipi/ask-user/src/render.rs`, `unipi/notify/src/messages.rs` |
| Others | `oino/browser/src/lib.rs`, `fixtures/ui-probe/src/lib.rs` (probe every node + slot), harness tests in `unipi/*/harness/tests/*.rs` |

### 12.4 TUI renderer update plan

1. **New module `crates/oino-tui/src/render/ui/`**: `mod.rs` (`render(node, area, buf, cx)`),
   `layout.rs` (ratatui `Layout` with the node's constraints/flex/spacing/margin + `collapse`),
   `widgets.rs` (map to real ratatui `Block`, `Paragraph`, `List`+`ListState`, `Table`+
   `TableState`, `Tabs`, `Gauge`, `LineGauge`, `Sparkline`, `BarChart`, `Chart`, `Scrollbar`),
   `extras.rs` (Markdown/Code/Diff via existing `markdown.rs`/`highlight.rs`, Image, Log, Tree,
   KeyHints, Input, Toggle, Select, Button, Spinner, Empty), `measure.rs` (§4.4 intrinsic
   heights), `focus.rs` (focus order, key routing, local state map), `style.rs` (token → `Color`).
2. **Bridge to the line-based compositor**: the TUI composes screens from `Block{lines}`
   (`render/mod.rs`). Each entry renders into an off-screen ratatui `Buffer` of its `Rect`
   (definite: slot size; auto: `measure()` height), then `buffer_to_lines(&Buffer) -> Vec<OLine>`
   merges same-style cell runs into `Span`s. Overlays/side panes keep `overlays.rs` framing.
3. **Caching**: buffer per `(entryId, entry.rev, rect, theme, focus-state)`; only entries with a
   `Spinner` re-render on ticks; a patch invalidates only its entry.
4. **Slots**: §6.2 TUI column (sidePane → overlay with tab strip; bottomPane → editorReplace
   region; fullscreen → transcript takeover; turnFooter → after the turn's last row via
   `_oino/render`).
5. **Delete** `render/ous.rs` and `oino-view/src/ous.rs` once all emitters are migrated (the v1
   gate in core keeps stragglers visible).
6. **Golden tests**: `crates/oino-tui/tests/ui_nodes.rs` renders every fixture in
   `crates/oino-ui/fixtures/nodes/*.json` at 40 and 100 columns into a `TestBackend` and
   snapshots it with `insta` (style markers included); `ui-probe` exercises every slot.

---

## 13. SolidJS renderer in the app

### 13.1 Module layout (`apps/unipi/apps/mobile/src/ui/`)

```text
ui/
  types.ts           TS mirror of oino-ui wire types (generated from Appendix A via json-schema-to-typescript, checked in)
  envelope.ts        zod for UiSnapshot / UiPatch / RegistryOp envelopes only (nodes not zod-parsed, D-U14)
  registry.ts        per-scope store: createStore<{rev, entries: Record<id, UiEntry>, dialogs, meta}>;
                     applySnapshot (reconcile by id), applyPatch (produce), resync on gap; slot selectors (§6.4 sort)
  tree.ts            NodeOp apply on a Solid store (path → keyed lookup maps per collection)
  actions.ts         sendAction / sendVisible / values collection (keyed Input/Toggle/Select registry per entry)
  localState.ts      selection/scroll/expansion keyed by (entryId, path, itemKey); "prop changed ⇒ adopt" rule
  layout/solve.ts    ratatui-formulation solver on @lume/kiwi (§4.3), memoized
  layout/units.ts    --ui-col / --ui-row measurement, px ⇄ units
  theme.ts + ui.css  token → CSS var map (§5), style → class/inline style
  NodeView.tsx       <Switch> on node.t → component; Unknown → fallback; ErrorBoundary per entry
  nodes/             Block, Paragraph, List, Table, Tabs, Gauge, LineGauge, Sparkline, BarChart, Chart,
                     Scrollbar, Markdown, Code, Diff, Image, Log, Tree, KeyHints, Input, Toggle, Select,
                     Button, Spinner, Empty, Layout, Text (Span/Line/Text helpers)  — one file each
  slots/             SlotModal, SidePaneTabs (contributes to the Z-5 container), BottomPanelTabs, ComposerTray,
                     ExtensionPage (sidebar route /x/:id), WidgetStack, StatusSegments, TurnFooters, HeaderBand,
                     SettingsSections, ReservedHost (id → native component + gate)
  dialogs/           pi + Oino dialogs (reuse/extend components/chat/DialogCard.tsx; custom → SlotModal)
  __tests__/         golden per node (§13.7)
```

### 13.2 Data flow

1. Session stream `SessionSnapshot.ui` → `registry.applySnapshot`; `oino_ui_patch` events →
   `applyPatch` (batched in the chat reducer's existing rAF flush). Hub `HubEvent::Ui` → the
   host-scope store.
2. Slots read **selectors** (`entriesFor(slot)`) — memoized, sorted by `(order desc, seq asc)`.
3. `NodeView` receives a store-backed node proxy; Solid's fine-grained reactivity means a `set`
   op on one node re-renders only the components reading that prop. Collections render with
   `<For>` keyed by item key (identity via the store's stable objects).
4. Actions: interactive components call `actions.send(entryId, actionId, value)` →
   `SessionCmd{_oino/ui_action}`; optimistic local state (e.g. selection) applies immediately.

### 13.3 Component rules

- **One component per node kind**, props = the node proxy + `ctx` (entryId, path, sizing mode,
  definite size). No component reads the whole tree.
- Sizing: a `Layout` in definite mode measures its box with one shared `ResizeObserver`
  (batched per frame), solves (§4.3), and sets each child's main-axis size in px; in auto mode
  it is a flex column/row with `Length` → fixed size and the rest natural size.
- Text: `Span`s are `<span>` with classes for modifiers and inline `color: var(--ui-…)`; no
  `innerHTML` except `Markdown` (`renderChatMarkdown`, DOMPurify) — enforced by a lint rule
  (`no-inner-html` outside `nodes/Markdown.tsx`).
- Charts (`Sparkline`, `BarChart`, `Chart`, `Gauge`): SVG, `vector-effect: non-scaling-stroke`,
  data downsampled (LTTB) to ≤ 2 × pixel width.
- `Spinner`: CSS animation only; `prefers-reduced-motion` → static.
- Every entry is wrapped in an `ErrorBoundary` showing a compact `EmptyState` "This panel failed
  to render" + "Report" (sends an `extension_error` notice); one bad tree never breaks the shell.

### 13.4 Virtualization

| Node | Threshold | Strategy |
|---|---|---|
| `List` | > 100 items | `@tanstack/solid-virtual`, estimate 32 px desktop / 44 px phone (56 with `detail`), measured |
| `Table` | > 100 rows | virtual rows, fixed `row.height × --ui-row`, sticky header |
| `Log` | always | virtual, fixed 20 px rows (`wrap:false`), measured when `wrap:true`; ring of `maxLines`; append batched per rAF; follow = single bottom-lock state (DIRECTION §4.2.3) |
| `Tree` | > 100 visible rows | flattened visible rows, virtual |
| `Markdown`/`Code`/`Diff` > 2 000 lines | — | rendered in chunks of 500 lines with `content-visibility: auto` |

Virtualized nodes in **auto** slots get a max height (slot max) so the virtualizer has a
viewport; turn footers never virtualize internally (≤ 160 px).

### 13.5 Performance budget (measured in WP-U9 on a mid-range Android phone, Pixel 7a class, and desktop)

| Metric | Desktop | Phone |
|---|---|---|
| First render of a 1 000-node panel | ≤ 16 ms | ≤ 50 ms |
| Apply a 100-op patch | ≤ 2 ms | ≤ 6 ms |
| Log append, 200 lines/s sustained with follow | 60 fps, no long task > 50 ms | ≥ 50 fps |
| Scroll a 5 000-row Table / List | 60 fps | ≥ 50 fps |
| Layout solve (≤ 16 children) | ≤ 0.2 ms | ≤ 0.6 ms |
| JS added to the bundle (`ui/` + kiwi) | ≤ 45 KB min+gz | — |
| Memory per open 5 000-line Log | ≤ 4 MB | ≤ 4 MB |
| Registry snapshot apply (64 entries) | ≤ 5 ms | ≤ 15 ms |

Rules: no work for hidden slots (unmounted tabs render nothing; `$hide` lets extensions stop
pushing); `turnFooter` requests only for visible turns; images lazy (`loading="lazy"`, blob
fetched on intersect).

### 13.6 Phone layout rules (`html:not([data-shell=desktop])`)

1. Horizontal `Layout` collapses to vertical auto mode when width < `collapse` × `--ui-col`
   (default 60 cols = 480 px).
2. Touch targets ≥ 44 px for every interactive node (DESIGN §1.4); List/Table/Tree rows 44 px.
3. `Table`: horizontal scroll inside its block, first column sticky; no column hiding.
4. `Tabs`: horizontally scrollable strip, 38 px chips.
5. `KeyHints`: hidden unless a hint has `action` (then tappable chips).
6. Slots: `overlay` → bottom sheet; `sidePane`/`bottomPane` → full-screen layers from the chat ⋯
   "Panes" group (Z-5 phone rec, final placement per Phase A rulings); `fullscreen` → page from
   the Chat-tab "More" menu; `widget` > 3 rows collapsed; `status` ≤ 4 segments.
7. `Diff` always unified; `Chart` min height 160 px; `Image` max width 100 %.
8. Safe areas respected by slot hosts, not by nodes.

### 13.7 Tests

- `src/ui/__tests__/nodes.golden.test.tsx`: for **every** fixture in
  `crates/oino-ui/fixtures/nodes/*.json` (the same corpus the TUI snapshots), render with
  `@solidjs/testing-library` in jsdom at desktop and phone shell and `toMatchSnapshot()` the
  serialized DOM (classes, styles, aria). Interactive fixtures also assert the emitted action.
- `src/ui/__tests__/solve.test.ts`: all `layout_vectors.json` cases at unit scale (±1).
- `src/ui/__tests__/registry.test.ts`: snapshot/patch/resync/ordering/dispose cases mirrored
  from core's registry tests (shared JSON fixtures `crates/oino-ui/fixtures/registry/*.json`).
- Visual: `vite.config.ui-capture.ts` (mock-IPC pattern of `vite.config.chat-capture.ts`, port
  1442) serves `/__ui-gallery` rendering every node fixture and every slot with mock entries;
  QA captures desktop (1280×820) and phone (412×915) screenshots with `agent-browser` and checks
  them against the design tokens (opus-ui-qa).
- Perf: `src/ui/__tests__/perf.bench.ts` (vitest bench) for the §13.5 rows that run in jsdom;
  phone numbers from a capture-page run on the plugged-in Android device (DIRECTION §8).

---

## 14. Work packages

Order: U1 → U2 → (U3, U5, U6 parallel) → U4 → U7a/U7b/U8 → app U9 → U10 → U11/U12 → U13.
The Rust side (U1–U8) can land before DIRECTION §9 step 8; the app side (U9–U12) needs the
side-pane container (Z-5) and bottom panel from the screens / terminal phases (host.md §9.3).
Commit prefixes `[ui-u<n>]`. Every WP ends green: `CARGO_BUILD_JOBS=8 cargo check --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace -j 8`,
`cargo fmt --all --check`, `scripts/docs-check.sh`; app WPs also `pnpm -C apps/unipi/apps/mobile
typecheck && pnpm -C apps/unipi/apps/mobile test`.

| WP | Scope | Files | Depends | Acceptance |
|---|---|---|---|---|
| **WP-U1** `oino-ui` types + builder | Wire types (§4, §6.1, §9.2), ratatui-mirroring builder (§11), `Stylize`, validation + limits + v1 gate (§4.8), `plain::to_lines`, JSON Schema export (feature `schema`) | `crates/oino-ui/**`, root `Cargo.toml` (workspace dep), `extensions/Cargo.toml` (path dep) | — | serde round-trip for every node; `schema.rs` test: exported schema ≡ Appendix A (normalized); fixture corpus `fixtures/nodes/<Kind>[-variant].json` (≥ 1 per node, 60+ total) validates; §11.3 example compiles for `wasm32-wasip2`; validate() ≤ 1 ms on a 256 KiB tree (bench) |
| **WP-U2** diff/patch + layout vectors | `patch.rs` diff/apply (§4.7); `layout_vectors.json` generator (test in `oino-tui` using ratatui 0.29) | `crates/oino-ui/src/patch.rs`, `crates/oino-ui/tests/{patch.rs,layout_vectors.json}`, `crates/oino-tui/tests/gen_layout_vectors.rs` | U1 | proptest `apply(old, diff(old,new)) == new` (10k cases); fallback-to-replace rule tested; ≥ 200 layout vectors covering every constraint × flex × spacing; regenerating vectors is a no-op diff |
| **WP-U3** core registry + wire | `UiRegistry` (§6), coalescing, rev, dispose hooks (§6.6), `$show/$hide`, reserved-id ownership (§8), turnFooter render dispatch (§6.7), WIT/manifest deltas (§9.1) on the ext-abi 2.0.0 cut, RPC messages + capabilities + degradation (§9–§10) | §12.2 rows for `oino-core`, `oino-extension-api`, `oino-wasm-host`, `oino-rpc`, `wit/` | U1, U2, ext-abi WP-X6 | registry tests: ordering, stack slots, dispose on instance drop / crash / disable / close, foreign-id rejection, reserved gate, resync on gap; rpc tests: v2 hello gets `_oino/ui_patch`, pi client gets pi records only, degradation table rows; pi `rpc-extension-ui.md` examples still round-trip byte-for-byte |
| **WP-U4** TUI renderer | §12.4 | `crates/oino-tui/src/render/ui/**`, `overlays.rs`, `transcript.rs`, `chrome.rs`, `mod.rs`, `app/mod.rs`, tests | U3, U5 | **golden per node**: `tests/ui_nodes.rs` insta snapshot for every fixture at 40 and 100 cols; slot tests per §6.2 TUI column; focus/keys tests (Tab, arrows, Enter, Esc, entry keys); render cost of a 1 000-node panel ≤ 2 ms per frame (bench); `ui-probe` passes |
| **WP-U5** `oino-view` | registry mirror, dialog queue, settings hub OSS-2; delete `ous.rs` (after U7) | `crates/oino-view/src/{ui_state.rs, settings_hub.rs, lib.rs}` | U1, U3 | reducer tests driven by `fixtures/registry/*.json` (shared with the app) |
| **WP-U6** SDK | `oino-extension::ui` Panel builder, `widget`/`status`/`register_turn_footer`/`register_settings`, re-exports; `unipi-kit` OSS-2 builder, delete `unipi-kit/src/ous.rs` | `extensions/sdk/{oino-extension,unipi-kit}/**` | U1 | doc-tests for §11.3; `cargo xtask wasm` builds all extensions |
| **WP-U7a** migrate panels | §12.3 "Panels" group; reserved ids for info-screen / kanboard / long-horizon with fallbacks | listed files + harness tests | U6, U3 | `rg -n '"t"\s*:\s*"[a-z]' extensions --glob '*.rs'` empty (CI gate script `scripts/ui-v1-gate.sh`); each extension's harness tests green; no panel duplicates a reserved surface (review checklist) |
| **WP-U7b** migrate render specs / cards / fixtures | §12.3 "Render specs", "Others" | listed files | U6, U3 | same gate; `utility` render rows snapshot-equal in the TUI (before/after text identical modulo styling); `ui-probe` emits every node and slot |
| **WP-U8** settings OSS-2 | §7 in core + unipi-kit + TUI hub; core `oino.*` namespaces for pi settings | `oino-core/src/extensions/settings.rs`, `oino_ui::settings`, `oino-view/settings_hub.rs`, `extensions/sdk/unipi-kit/src/settings/*` | U1 | resolve/simplify table tests (layers, invalid → fallback + diagnostic, default write = unset); v1 namespaces of all unipi extensions load unchanged |
| **WP-U9** app renderer core | `src/ui/` (§13.1) minus slots: registry, tree ops, NodeView + all node components, theme tokens, solver, virtualization, actions, local state | `apps/unipi/apps/mobile/src/ui/**`, `package.json` (`@lume/kiwi`) | U1, U2, host session stream | **golden per node** (§13.7) desktop + phone; solver vectors; perf budget rows §13.5 (bench + device run); token contrast check; gallery capture screenshots reviewed (opus-ui-qa) |
| **WP-U10** app slots + dialogs | slot hosts (§6.2 app columns), dialogs/effects (§3.2), status bar, turn footers in the virtualized transcript, phone rules (§13.6), command-palette entries "Open <title>" per sidePane/bottomPane/fullscreen entry | `src/ui/slots/**`, `src/ui/dialogs/**`, shell / chat integration points (`ChatView.tsx`, `Transcript.tsx`, `components/shell/*`) | U9, Z-5 container, bottom panel | scenario tests per slot (open/update/close/dispose/order/two contributors); G-1 and G-4 of zcode-audit closed (every v1 panel of unipi renders); no slot creates a second surface for an existing feature (QA checklist) |
| **WP-U11** app settings form | OSS-2 form engine (§7) in the one Settings screen, extension enable toggle per section | `src/ui/slots/SettingsSections.tsx`, `src/ui/settings/**`, `screens/Settings.tsx` | U8, U9, host.md §9.9 | each field type golden (desktop/phone); scope layering, reset, search, visibleIf; per-extension toggle round-trips `_oino/set_extension_enabled` |
| **WP-U12** reserved routing | `ReservedHost`: id → native component, gates (§8), fallback trees elsewhere, nav items/tabs appear/disappear with owner state | `src/ui/slots/ReservedHost.tsx`, shell nav | U10; native screens from their phases | disabling each owner removes its surface live (test per row); `openPanel` on a reserved id focuses the native view; TUI gets fallbacks |
| **WP-U13** parity QA + docs | cross-renderer parity pass on the gallery (TUI vs app), author guide `docs/dev/v2/ui-authoring.md`, pointer at top of v1 `ui-protocol.md` | docs, QA report | all | opus-ui-qa report with zero P0/P1; docs-check green |

---

## 15. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| App solver drifts from ratatui (cassowary rounding, kiwi strengths) | layouts differ between TUI and app | vectors generated from ratatui itself; ±1 cell tolerance; app solves in px so 1-cell drift is invisible |
| Cell units look wrong in a proportional UI (Length rows vs real text height) | clipped or loose panels | chrome rule §4.3; auto mode for inline slots; gallery review per node |
| Big trees / high update rates over iroh to a phone | jank, bandwidth | limits §4.8, core diff + 33 ms coalescing, `$hide` pausing, host batching (host.md §7.4), virtualization |
| Migration volume (~45 files, ~300 node sites) | long tail, v1 stragglers | mechanical mapping table §12.1, builder, CI v1 gate, two WPs in parallel |
| turnFooter pull needs the guest `render` within 50 ms with kv reads | footers missing on slow sessions | cache by `(turn, rev)`, render only visible turns, `null` ⇒ nothing (not an error) |
| Keyboard conflicts (panel keys vs composer, xterm, palette) | lost keystrokes | entry keys bound only while focus is inside the entry; reserved app bindings win (v1 §8 conflict rules) |
| Slot placement on phone not ruled yet (Z-5/Z-95) | rework of slot hosts | slot hosts isolated in `ui/slots/*`; mapping table §6.2 is the only place to change |
| WIT churn collides with ext-abi 2.0.0 cut | double rebuild | U3's WIT deltas ride WP-X6 (all additive within 2.0.0) |
| `@lume/kiwi` dependency | supply chain, size | MIT, ~13 KB, pinned; replaceable by any vector-passing solver |

---

## 16. Owner questions

Defaults below are chosen so work can start; none blocks U1–U8.

1. **Third-party sidebar pages.** DIRECTION says third-party extensions "get widgets only";
   this spec reads that as *node trees in any slot, never native components*, and caps them at
   one `fullscreen` page each. Default: allow (cap 1). Alternative: third parties limited to
   `overlay`/`sidePane`/`widget`/`status`/`turnFooter`.
2. **`accent` colour in the app.** Default `claw` (#F07818) so shell red stays reserved for
   primary actions and focus (DESIGN §1.6). Alternative: `shell`.

---

## Appendix A — JSON Schema (normative)

JSON Schema draft 2020-12 for every node kind. It describes what producers **must** emit;
parsers stay tolerant (unknown props ignored, unknown `t` → `fallback`, D-U14). `oino-ui`'s
`schema` feature exports the same schema (WP-U1 asserts equivalence after normalization); the
app's `ui/types.ts` is generated from it. Registry envelopes (`UiEntry`, `UiSnapshot`,
`UiPatch`, `RegistryOp`, `NodeOp`) are specified by §6.1, §4.7 and §9.2 and exported by
`oino-ui` as `registry.schema.json` (WP-U1). Limits that JSON Schema cannot express (depth,
total nodes, encoded size, sibling-key uniqueness) are §4.8.

```json
{"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://oino.dev/schemas/ui/2/node.schema.json", "title": "Oino UI protocol v2 node", "$ref": "#/$defs/Node",
 "$defs": {
  "Key": {"type": "string", "minLength": 1, "maxLength": 128},
  "ActionId": {"type": "string", "pattern": "^[A-Za-z0-9_.:/-]{1,128}$"},
  "Color": {"enum": ["reset", "text", "muted", "dim", "accent", "primary", "info", "link", "success", "warning", "error", "harness", "thinking", "tool", "user", "assistant", "code", "border", "borderAccent", "borderMuted", "c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9", "c10", "c11", "c12", "surface1", "surface2", "surface3", "selectedBg", "successBg", "errorBg", "warningBg", "infoBg", "accentBg", "diffAdded", "diffRemoved", "diffContext"]},
  "Icon": {"enum": ["check", "x", "dot", "warn", "info", "clock", "gear", "branch", "bolt", "bot", "folder", "file", "search", "cloud", "lock", "board", "terminal", "globe", "diff", "list", "chart", "play", "pause", "stop", "refresh", "plus", "trash"]},
  "Modifier": {"enum": ["BOLD", "DIM", "ITALIC", "UNDERLINED", "SLOW_BLINK", "RAPID_BLINK", "REVERSED", "HIDDEN", "CROSSED_OUT"]},
  "Alignment": {"enum": ["Left", "Center", "Right"]},
  "Style": {"type": "object", "properties": {"fg": {"$ref": "#/$defs/Color"}, "bg": {"$ref": "#/$defs/Color"}, "underlineColor": {"$ref": "#/$defs/Color"}, "addModifier": {"type": "array", "items": {"$ref": "#/$defs/Modifier"}}, "subModifier": {"type": "array", "items": {"$ref": "#/$defs/Modifier"}}}},
  "Span": {"oneOf": [{"type": "string"}, {"type": "object", "properties": {"content": {"type": "string"}, "style": {"$ref": "#/$defs/Style"}, "link": {"type": "string", "format": "uri"}}, "required": ["content"]}]},
  "Line": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"$ref": "#/$defs/Span"}}, {"type": "object", "properties": {"spans": {"type": "array", "items": {"$ref": "#/$defs/Span"}}, "style": {"$ref": "#/$defs/Style"}, "alignment": {"$ref": "#/$defs/Alignment"}}, "required": ["spans"]}]},
  "Text": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"$ref": "#/$defs/Line"}}, {"type": "object", "properties": {"lines": {"type": "array", "items": {"$ref": "#/$defs/Line"}}, "style": {"$ref": "#/$defs/Style"}, "alignment": {"$ref": "#/$defs/Alignment"}}, "required": ["lines"]}]},
  "Constraint": {"oneOf": [{"type": "integer", "minimum": 0, "maximum": 65535}, {"type": "object", "properties": {"Length": {"type": "integer", "minimum": 0, "maximum": 65535}}, "required": ["Length"], "additionalProperties": false}, {"type": "object", "properties": {"Percentage": {"type": "integer", "minimum": 0, "maximum": 100}}, "required": ["Percentage"], "additionalProperties": false}, {"type": "object", "properties": {"Ratio": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 4294967295}, "minItems": 2, "maxItems": 2}}, "required": ["Ratio"], "additionalProperties": false}, {"type": "object", "properties": {"Min": {"type": "integer", "minimum": 0, "maximum": 65535}}, "required": ["Min"], "additionalProperties": false}, {"type": "object", "properties": {"Max": {"type": "integer", "minimum": 0, "maximum": 65535}}, "required": ["Max"], "additionalProperties": false}, {"type": "object", "properties": {"Fill": {"type": "integer", "minimum": 0, "maximum": 65535}}, "required": ["Fill"], "additionalProperties": false}]},
  "Padding": {"oneOf": [{"type": "integer", "minimum": 0, "maximum": 65535}, {"type": "object", "properties": {"left": {"type": "integer", "minimum": 0, "maximum": 65535}, "right": {"type": "integer", "minimum": 0, "maximum": 65535}, "top": {"type": "integer", "minimum": 0, "maximum": 65535}, "bottom": {"type": "integer", "minimum": 0, "maximum": 65535}}}]},
  "Borders": {"oneOf": [{"enum": ["ALL", "NONE"]}, {"type": "array", "items": {"enum": ["TOP", "RIGHT", "BOTTOM", "LEFT"]}, "uniqueItems": true}]},
  "Title": {"oneOf": [{"$ref": "#/$defs/Line"}, {"type": "object", "properties": {"content": {"$ref": "#/$defs/Line"}, "alignment": {"$ref": "#/$defs/Alignment"}, "position": {"enum": ["Top", "Bottom"]}}, "required": ["content"]}]},
  "BlockProps": {"type": "object", "properties": {"title": {"oneOf": [{"$ref": "#/$defs/Line"}, {"type": "array", "items": {"$ref": "#/$defs/Title"}}]}, "titleAlignment": {"$ref": "#/$defs/Alignment"}, "borders": {"$ref": "#/$defs/Borders"}, "borderType": {"enum": ["Plain", "Rounded", "Double", "Thick"]}, "borderStyle": {"$ref": "#/$defs/Style"}, "titleStyle": {"$ref": "#/$defs/Style"}, "padding": {"$ref": "#/$defs/Padding"}, "collapsible": {"type": "boolean"}, "collapsed": {"type": "boolean"}, "rail": {"$ref": "#/$defs/Color"}, "style": {"$ref": "#/$defs/Style"}}},
  "Common": {"type": "object", "properties": {"key": {"$ref": "#/$defs/Key"}, "style": {"$ref": "#/$defs/Style"}, "block": {"$ref": "#/$defs/BlockProps"}, "fallback": {"$ref": "#/$defs/Node"}}},
  "ImageSource": {"oneOf": [{"type": "object", "properties": {"data": {"type": "string", "contentEncoding": "base64"}, "mimeType": {"type": "string"}}, "required": ["data", "mimeType"]}, {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}, {"type": "object", "properties": {"blob": {"type": "string"}}, "required": ["blob"]}]},
  "ListItem": {"oneOf": [{"$ref": "#/$defs/Text"}, {"type": "object", "properties": {"key": {"$ref": "#/$defs/Key"}, "content": {"$ref": "#/$defs/Text"}, "style": {"$ref": "#/$defs/Style"}, "detail": {"$ref": "#/$defs/Line"}, "badge": {"$ref": "#/$defs/Line"}, "icon": {"$ref": "#/$defs/Icon"}, "disabled": {"type": "boolean"}}, "required": ["content"]}]},
  "Cell": {"oneOf": [{"$ref": "#/$defs/Text"}, {"type": "object", "properties": {"content": {"$ref": "#/$defs/Text"}, "style": {"$ref": "#/$defs/Style"}}, "required": ["content"]}]},
  "Row": {"type": "object", "properties": {"key": {"$ref": "#/$defs/Key"}, "cells": {"type": "array", "items": {"$ref": "#/$defs/Cell"}}, "height": {"type": "integer", "minimum": 0, "maximum": 65535}, "bottomMargin": {"type": "integer", "minimum": 0, "maximum": 65535}, "style": {"$ref": "#/$defs/Style"}}, "required": ["cells"]},
  "TreeNode": {"type": "object", "properties": {"key": {"$ref": "#/$defs/Key"}, "label": {"$ref": "#/$defs/Line"}, "icon": {"$ref": "#/$defs/Icon"}, "badge": {"$ref": "#/$defs/Line"}, "children": {"type": "array", "items": {"$ref": "#/$defs/TreeNode"}}, "expanded": {"type": "boolean"}, "lazy": {"type": "boolean"}}, "required": ["key", "label"]},
  "Axis": {"type": "object", "properties": {"title": {"$ref": "#/$defs/Line"}, "bounds": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}, "labels": {"type": "array", "items": {"$ref": "#/$defs/Span"}}, "labelsAlignment": {"$ref": "#/$defs/Alignment"}, "style": {"$ref": "#/$defs/Style"}}, "required": ["bounds"]},
  "Dataset": {"type": "object", "properties": {"name": {"$ref": "#/$defs/Line"}, "data": {"type": "array", "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}}, "marker": {"enum": ["Dot", "Braille", "Block", "Bar", "HalfBlock"]}, "graphType": {"enum": ["Scatter", "Line", "Bar"]}, "style": {"$ref": "#/$defs/Style"}}, "required": ["data"]},
  "Bar": {"type": "object", "properties": {"value": {"type": "integer", "minimum": 0}, "label": {"$ref": "#/$defs/Line"}, "textValue": {"type": "string"}, "style": {"$ref": "#/$defs/Style"}, "valueStyle": {"$ref": "#/$defs/Style"}}, "required": ["value"]},
  "BarGroup": {"type": "object", "properties": {"label": {"$ref": "#/$defs/Line"}, "bars": {"type": "array", "items": {"$ref": "#/$defs/Bar"}}}, "required": ["bars"]},
  "Hint": {"type": "object", "properties": {"keys": {"type": "string"}, "label": {"type": "string"}, "action": {"$ref": "#/$defs/ActionId"}}, "required": ["keys", "label"]},
  "Option": {"type": "object", "properties": {"value": {"type": "string"}, "label": {"$ref": "#/$defs/Line"}, "description": {"type": "string"}}, "required": ["value", "label"]},
  "ListState": {"type": "object", "properties": {"selected": {"oneOf": [{"$ref": "#/$defs/Key"}, {"type": "array", "items": {"$ref": "#/$defs/Key"}}]}, "offset": {"type": "integer", "minimum": 0, "maximum": 4294967295}}},
  "Layout": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Layout"}, "direction": {"enum": ["Vertical", "Horizontal"]}, "constraints": {"type": "array", "items": {"$ref": "#/$defs/Constraint"}, "maxItems": 64}, "children": {"type": "array", "items": {"$ref": "#/$defs/Node"}, "maxItems": 64}, "spacing": {"type": "integer", "minimum": 0, "maximum": 65535}, "margin": {"oneOf": [{"type": "integer", "minimum": 0, "maximum": 65535}, {"type": "object", "properties": {"horizontal": {"type": "integer", "minimum": 0, "maximum": 65535}, "vertical": {"type": "integer", "minimum": 0, "maximum": 65535}}}]}, "flex": {"enum": ["Legacy", "Start", "End", "Center", "SpaceBetween", "SpaceAround"]}, "collapse": {"type": "integer", "minimum": 0, "maximum": 65535}}, "required": ["t", "children"]}]},
  "Block": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Block"}, "title": {"oneOf": [{"$ref": "#/$defs/Line"}, {"type": "array", "items": {"$ref": "#/$defs/Title"}}]}, "titleAlignment": {"$ref": "#/$defs/Alignment"}, "borders": {"$ref": "#/$defs/Borders"}, "borderType": {"enum": ["Plain", "Rounded", "Double", "Thick"]}, "borderStyle": {"$ref": "#/$defs/Style"}, "titleStyle": {"$ref": "#/$defs/Style"}, "padding": {"$ref": "#/$defs/Padding"}, "collapsible": {"type": "boolean"}, "collapsed": {"type": "boolean"}, "rail": {"$ref": "#/$defs/Color"}, "style": {"$ref": "#/$defs/Style"}, "child": {"$ref": "#/$defs/Node"}, "onToggle": {"$ref": "#/$defs/ActionId"}}, "required": ["t"]}]},
  "Paragraph": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Paragraph"}, "text": {"$ref": "#/$defs/Text"}, "wrap": {"oneOf": [{"const": false}, {"type": "object", "properties": {"trim": {"type": "boolean"}}}]}, "scroll": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 65535}, "minItems": 2, "maxItems": 2}, "alignment": {"$ref": "#/$defs/Alignment"}, "scrollbar": {"type": "boolean"}}, "required": ["t", "text"]}]},
  "List": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "List"}, "items": {"type": "array", "items": {"$ref": "#/$defs/ListItem"}, "maxItems": 5000}, "state": {"$ref": "#/$defs/ListState"}, "highlightStyle": {"$ref": "#/$defs/Style"}, "highlightSymbol": {"type": "string"}, "repeatHighlightSymbol": {"type": "boolean"}, "highlightSpacing": {"enum": ["Always", "WhenSelected", "Never"]}, "direction": {"enum": ["TopToBottom", "BottomToTop"]}, "scrollPadding": {"type": "integer", "minimum": 0, "maximum": 65535}, "multi": {"type": "boolean"}, "scrollbar": {"type": "boolean"}, "onSelect": {"$ref": "#/$defs/ActionId"}, "onActivate": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "items"]}]},
  "Table": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Table"}, "header": {"$ref": "#/$defs/Row"}, "rows": {"type": "array", "items": {"$ref": "#/$defs/Row"}, "maxItems": 5000}, "footer": {"$ref": "#/$defs/Row"}, "widths": {"type": "array", "items": {"$ref": "#/$defs/Constraint"}}, "columnSpacing": {"type": "integer", "minimum": 0, "maximum": 65535}, "rowHighlightStyle": {"$ref": "#/$defs/Style"}, "highlightSymbol": {"type": "string"}, "highlightSpacing": {"enum": ["Always", "WhenSelected", "Never"]}, "state": {"$ref": "#/$defs/ListState"}, "sticky": {"enum": ["FirstColumn"]}, "scrollbar": {"type": "boolean"}, "onSelect": {"$ref": "#/$defs/ActionId"}, "onActivate": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "rows", "widths"]}]},
  "Tabs": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Tabs"}, "titles": {"type": "array", "items": {"type": "object", "properties": {"key": {"$ref": "#/$defs/Key"}, "title": {"$ref": "#/$defs/Line"}, "badge": {"$ref": "#/$defs/Line"}}, "required": ["key", "title"]}}, "selected": {"$ref": "#/$defs/Key"}, "divider": {"$ref": "#/$defs/Span"}, "padding": {"type": "array", "items": {"$ref": "#/$defs/Line"}, "minItems": 2, "maxItems": 2}, "highlightStyle": {"$ref": "#/$defs/Style"}, "panes": {"type": "object", "additionalProperties": {"$ref": "#/$defs/Node"}}, "onSelect": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "titles"]}]},
  "Gauge": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Gauge"}, "ratio": {"type": "number", "minimum": 0, "maximum": 1}, "percent": {"type": "integer", "minimum": 0, "maximum": 100}, "label": {"$ref": "#/$defs/Span"}, "gaugeStyle": {"$ref": "#/$defs/Style"}, "useUnicode": {"type": "boolean"}}, "required": ["t"]}]},
  "LineGauge": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "LineGauge"}, "ratio": {"type": "number", "minimum": 0, "maximum": 1}, "label": {"$ref": "#/$defs/Line"}, "filledStyle": {"$ref": "#/$defs/Style"}, "unfilledStyle": {"$ref": "#/$defs/Style"}, "lineSet": {"enum": ["Normal", "Thick", "Double"]}}, "required": ["t", "ratio"]}]},
  "Sparkline": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Sparkline"}, "data": {"type": "array", "items": {"type": ["integer", "null"], "minimum": 0}, "maxItems": 1000}, "max": {"type": "integer", "minimum": 0}, "direction": {"enum": ["LeftToRight", "RightToLeft"]}, "barSet": {"enum": ["Nine", "Three"]}, "absentValueSymbol": {"type": "string"}}, "required": ["t", "data"]}]},
  "BarChart": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "BarChart"}, "data": {"type": "array", "items": {"$ref": "#/$defs/BarGroup"}}, "barWidth": {"type": "integer", "minimum": 0, "maximum": 65535}, "barGap": {"type": "integer", "minimum": 0, "maximum": 65535}, "groupGap": {"type": "integer", "minimum": 0, "maximum": 65535}, "direction": {"enum": ["Vertical", "Horizontal"]}, "max": {"type": "integer", "minimum": 0}, "barStyle": {"$ref": "#/$defs/Style"}, "valueStyle": {"$ref": "#/$defs/Style"}, "labelStyle": {"$ref": "#/$defs/Style"}, "onSelect": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "data"]}]},
  "Chart": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Chart"}, "datasets": {"type": "array", "items": {"$ref": "#/$defs/Dataset"}}, "xAxis": {"$ref": "#/$defs/Axis"}, "yAxis": {"$ref": "#/$defs/Axis"}, "legendPosition": {"enum": ["Top", "TopRight", "TopLeft", "Left", "Right", "Bottom", "BottomRight", "BottomLeft", null]}}, "required": ["t", "datasets", "xAxis", "yAxis"]}]},
  "Scrollbar": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Scrollbar"}, "orientation": {"enum": ["VerticalRight", "VerticalLeft", "HorizontalBottom", "HorizontalTop"]}, "state": {"type": "object", "properties": {"contentLength": {"type": "integer", "minimum": 0, "maximum": 4294967295}, "position": {"type": "integer", "minimum": 0, "maximum": 4294967295}, "viewportContentLength": {"type": "integer", "minimum": 0, "maximum": 4294967295}}, "required": ["contentLength", "position"]}, "thumbStyle": {"$ref": "#/$defs/Style"}, "trackStyle": {"$ref": "#/$defs/Style"}, "beginSymbol": {"type": ["string", "null"]}, "endSymbol": {"type": ["string", "null"]}}, "required": ["t", "state"]}]},
  "Markdown": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Markdown"}, "source": {"type": "string", "maxLength": 262144}}, "required": ["t", "source"]}]},
  "Code": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Code"}, "source": {"type": "string"}, "language": {"type": "string"}, "lineNumbers": {"type": "boolean"}, "startLine": {"type": "integer", "minimum": 0, "maximum": 4294967295}, "highlightLines": {"type": "array", "items": {"type": "integer", "minimum": 0, "maximum": 4294967295}}, "wrap": {"type": "boolean"}}, "required": ["t", "source"]}]},
  "Diff": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Diff"}, "patch": {"type": "string"}, "old": {"type": "string"}, "new": {"type": "string"}, "path": {"type": "string"}, "context": {"type": "integer", "minimum": 0, "maximum": 65535}, "mode": {"enum": ["Unified", "Split"]}}, "required": ["t"]}, {"anyOf": [{"required": ["patch"]}, {"required": ["old", "new"]}]}]},
  "Image": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Image"}, "src": {"$ref": "#/$defs/ImageSource"}, "alt": {"type": "string"}, "fit": {"enum": ["Contain", "Cover"]}, "maxWidth": {"type": "integer", "minimum": 0, "maximum": 65535}, "maxHeight": {"type": "integer", "minimum": 0, "maximum": 65535}}, "required": ["t", "src", "alt"]}]},
  "Log": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Log"}, "lines": {"type": "array", "items": {"$ref": "#/$defs/Line"}, "maxItems": 5000}, "maxLines": {"type": "integer", "minimum": 1, "maximum": 5000}, "follow": {"type": "boolean"}, "wrap": {"type": "boolean"}, "lineNumbers": {"type": "boolean"}}, "required": ["t", "lines"]}]},
  "Tree": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Tree"}, "nodes": {"type": "array", "items": {"$ref": "#/$defs/TreeNode"}}, "state": {"type": "object", "properties": {"selected": {"$ref": "#/$defs/Key"}}}, "onSelect": {"$ref": "#/$defs/ActionId"}, "onActivate": {"$ref": "#/$defs/ActionId"}, "onToggle": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "nodes"]}]},
  "KeyHints": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "KeyHints"}, "hints": {"type": "array", "items": {"$ref": "#/$defs/Hint"}}}, "required": ["t", "hints"]}]},
  "Input": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Input"}, "value": {"type": "string"}, "placeholder": {"type": "string"}, "label": {"$ref": "#/$defs/Line"}, "multiline": {"type": "boolean"}, "rows": {"type": "integer", "minimum": 0, "maximum": 65535}, "secret": {"type": "boolean"}, "maxLength": {"type": "integer", "minimum": 0, "maximum": 4294967295}, "disabled": {"type": "boolean"}, "onSubmit": {"$ref": "#/$defs/ActionId"}, "onChange": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "key"]}]},
  "Toggle": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Toggle"}, "label": {"$ref": "#/$defs/Line"}, "value": {"type": "boolean"}, "disabled": {"type": "boolean"}, "onChange": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "key", "label", "value"]}]},
  "Select": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Select"}, "options": {"type": "array", "items": {"$ref": "#/$defs/Option"}}, "value": {"type": "string"}, "placeholder": {"type": "string"}, "disabled": {"type": "boolean"}, "onChange": {"$ref": "#/$defs/ActionId"}}, "required": ["t", "key", "options"]}]},
  "Button": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Button"}, "label": {"$ref": "#/$defs/Line"}, "action": {"$ref": "#/$defs/ActionId"}, "value": {}, "variant": {"enum": ["Primary", "Secondary", "Ghost", "Danger"]}, "icon": {"$ref": "#/$defs/Icon"}, "disabled": {"type": "boolean"}, "confirm": {"type": "string"}}, "required": ["t", "label", "action"]}]},
  "Spinner": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Spinner"}, "label": {"$ref": "#/$defs/Line"}}, "required": ["t"]}]},
  "Empty": {"allOf": [{"$ref": "#/$defs/Common"}, {"type": "object", "properties": {"t": {"const": "Empty"}, "title": {"type": "string"}, "message": {"type": "string"}, "icon": {"$ref": "#/$defs/Icon"}, "action": {"$ref": "#/$defs/Button"}}, "required": ["t"]}]},
  "Node": {"type": "object", "required": ["t"], "oneOf": [{"$ref": "#/$defs/Layout"}, {"$ref": "#/$defs/Block"}, {"$ref": "#/$defs/Paragraph"}, {"$ref": "#/$defs/List"}, {"$ref": "#/$defs/Table"}, {"$ref": "#/$defs/Tabs"}, {"$ref": "#/$defs/Gauge"}, {"$ref": "#/$defs/LineGauge"}, {"$ref": "#/$defs/Sparkline"}, {"$ref": "#/$defs/BarChart"}, {"$ref": "#/$defs/Chart"}, {"$ref": "#/$defs/Scrollbar"}, {"$ref": "#/$defs/Markdown"}, {"$ref": "#/$defs/Code"}, {"$ref": "#/$defs/Diff"}, {"$ref": "#/$defs/Image"}, {"$ref": "#/$defs/Log"}, {"$ref": "#/$defs/Tree"}, {"$ref": "#/$defs/KeyHints"}, {"$ref": "#/$defs/Input"}, {"$ref": "#/$defs/Toggle"}, {"$ref": "#/$defs/Select"}, {"$ref": "#/$defs/Button"}, {"$ref": "#/$defs/Spinner"}, {"$ref": "#/$defs/Empty"}]}
 }
}
```

## Appendix B — layout vectors

`crates/oino-ui/tests/layout_vectors.json` is generated by running ratatui 0.29
`Layout::horizontal(constraints).flex(flex).spacing(spacing).split(Rect::new(0, 0, width, 1))`
(vertical is the same solver on the other axis) and recording `[x, width]` per segment:

```json
{"ratatui": "0.29.0", "cases": [
  {"name": "fills 1:2:3", "size": 50, "flex": "Start", "spacing": 0, "margin": 0,
   "constraints": [{"Fill": 1}, {"Fill": 2}, {"Fill": 3}], "segments": [[0, 8], [8, 17], [25, 25]]}
]}
```

Sample cases (sizes only; produced with ratatui 0.29.0 while writing this spec — the
generated file is authoritative):

| Case | Size | Constraints | Flex | Spacing | Result sizes |
|---|---|---|---|---|---|
| lengths | 80 | `[Length(20), Length(20)]` | Start | 0 | [20, 20] (60 trailing) |
| length + fill | 80 | `[Length(20), Fill(1)]` | Start | 0 | [20, 60] |
| fills 1:2:3 | 50 | `[Fill(1), Fill(2), Fill(3)]` | Start | 0 | [8, 17, 25] |
| fill / pct / fill | 50 | `[Fill(1), Percentage(50), Fill(1)]` | Start | 0 | [13, 25, 12] |
| pct + min | 50 | `[Percentage(100), Min(20)]` | Start | 0 | [30, 20] |
| min grows (non-Legacy) | 80 | `[Min(20), Max(20)]` | Start | 0 | [60, 20] |
| max + max | 80 | `[Max(20), Max(20)]` | Start | 0 | [20, 20] |
| ratio quarters | 50 | `[Ratio(1,4) ×4]` | Start | 0 | [13, 12, 13, 12] |
| legacy: last takes excess | 80 | `[Length(20) ×3]` | Legacy | 0 | [20, 20, 40] |
| center | 80 | `[Length(20), Length(20)]` | Center | 0 | [20, 20] at x = 20, 40 |
| space-between | 80 | `[Length(20), Length(20)]` | SpaceBetween | 0 | [20, 20] at x = 0, 60 |
| spacing | 40 | `[Fill(1), Fill(1)]` | Start | 2 | [19, 19] |
| over-full: Min beats Length | 30 | `[Length(20), Min(15)]` | Start | 0 | [15, 15] |
| header / body / footer | 24 | `[Length(3), Fill(1), Length(1)]` | Start | 0 | [3, 20, 1] |

Coverage required in WP-U2 (≥ 200 cases): every constraint alone and paired with every other,
each `Flex`, spacing 0/1/3, margins 0/1, sizes 0, 1, 7, 24, 80, 200, over-full and under-full
areas, `Fill(0)`, `Ratio(_, 0)`, `Percentage(0|100)`.
