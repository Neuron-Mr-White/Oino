# Round 5 — reference-app scout (ZCode + unipi)

Read-only UX/UI survey backing `docs/dev/v1/round5-feedback.md` items. Sources:

- **ZCode** `/tmp/zcode-src` (v3.16.2; React/Tailwind, design system `packages/ui/src/DESIGN.md`,
  tokens in `packages/ui/src/styles.css`). Chat UI = `packages/ui/src/v4/`, composer =
  `packages/ui/src/prompt-editor/` + `v4/composer/`, settings = `packages/ui/src/settings/`.
- **Unipi** `/home/oi/Projects/Personal/archived/unipi/`: app UI `unipi-app/apps/mobile/src`
  (SolidJS + CSS; `data-shell=desktop` ≥900px switches to the PC layout, see
  `components/shell/shell.css:6-7`), board Rust core `crates/kanboard/`.

All paths below are relative to those two roots. Line numbers verified 2026-10-30.

---

## PART A — ZCode (follow for layout/UX/motion)

### A. Design tokens worth copying first

- Base UI font size `--ui-font-size: 14px` (`packages/ui/src/styles.css:76`); the mandatory
  text scale `styles.css:150-162`: `text-ui-xl` 18 / `lg` 16 / `base` 14 / `caption` 13 /
  `sm` 12 / `xs` 10, settings title `+10px` (24) and `+16px` lg (30). Everything in the app
  uses this scale — no ad-hoc px sizes (DESIGN.md "Highest-priority UI constraint").
- Motion tokens `.wf-motion` `styles.css:1510-1520`: `--wf-t-fast: 120ms`, `--wf-t-base: 160ms`,
  `--wf-t-enter: 200ms`, `--wf-t-ink: 320ms`, ease `cubic-bezier(0.22,0.61,0.36,1)`,
  heartbeat `--wf-beat: 1.6s`. Collapsible expand/collapse `300ms ease-in-out`
  (`styles.css:1113`). Every animated block has a `prefers-reduced-motion: reduce` off-switch
  (e.g. `styles.css:975,1011,1048,1076,1140`).
- GPUI mapping: encode the six-step text scale + 5 motion durations + 1 ease curve as theme
  constants; forbid raw font sizes in app code.

### A(#6). New-chat page (draft home)

ZCode has no "new chat dialog" — a new task opens the **draft state of the chat page itself**:

- Draft empty state = time-of-day greeting + Z logo, centered
  (`packages/ui/src/v4/ConversationDraftEmptyState.tsx:203`: `mb-10 flex w-full max-w-2xl
  flex-col items-center justify-center gap-6`; logo `@/assets/Z.svg` line 15; greeting font
  adapts 20→30px to width, lines 24-26, 149-166; greeting buckets at hours 5/9/12/14/18/23,
  lines 27-45).
- Composer is centered and narrowed in draft mode: `centered && "max-w-2xl"`
  (`v4/ConversationComposer.tsx:2538`, comment at 2524 "居中草稿布局（m5）"); the timeline
  centers the empty state with the dock pinned to the bottom
  (`v4/SessionPane.tsx:5930` `centerEmptyStateWithDock={isDraft && …}`).
- Suggested prompts (rotating) render under the greeting
  (`v4/ConversationDraftSuggestedPromptsContainer.tsx`, `v4/ConversationDraftSuggestedPrompts.tsx`).
- **Project selection lives in the sidebar, not the page**: each project section header has a
  "+" (new project) and a `MessageCirclePlus` "New task in group"
  (`project-sidebar/PurposeSections.tsx:118-140`; i18n `taskGroup.newTask`,
  `project.createTask` in `i18n/locales/en-US.ts:93,142`). Top of sidebar a single
  full-width "New task" button with shortcut hint
  (`NewTaskButtonGroup.tsx:27-36`: `h-8 rounded-lg`, `MessageCirclePlus` 16px, shortcut label
  right-aligned `text-ui-xs`).
- "Outside a project" = the Home workspace: home-only workspace rows
  (`project-sidebar/HomeOnlyWorkspaceRows.tsx:29+`, `useHomeOnlyWorkspaces.ts`); remote
  no-project conversations are data-only (`conversation-workspace-target/purpose.ts:3-10`).

GPUI sketch: new-session opens the chat page in draft mode (greeting + centered composer +
suggestions); project choosing happens in the sidebar per-project "+" and a default "no
project" bucket; pressing "+" repeatedly focuses/reuses the existing draft instead of
stacking sessions (oino issue 4).

### A(#14). Settings layout & density

- Page shell: left nav rail + content grid `SettingsPage.tsx:1515`:
  `grid h-screen grid-cols-[68px_1fr] lg:grid-cols-[268px_1fr]` — icon-only rail on narrow,
  268px labelled rail on wide. Nav `px-2 pb-3` scrollable (`:1584-1586`); content
  `main` scrolls with `scrollbar-gutter: stable` (`:1775`).
- Section title `text-ui-settings-title` 24px/30px semibold tracking-tight
  (`SettingsPage.tsx:1789`; token `styles.css:150-153`).
- Nav buttons: `SettingsSidebarButton` (`SettingsPage.tsx:269-293`): `h-8 w-full gap-2
  rounded-xl px-2.5`, icon 16px, active `bg-surface-hover text-foreground`, muted otherwise;
  collapses to 40px icon buttons under lg (`max-lg:size-10 max-lg:justify-center`).
- Nav content model (`settings/settingsPageConfig.ts:39-176`): grouped sections
  (basics / agent capabilities / data & stats) each with a lucide icon — General `Settings2`,
  Appearance `Palette`, Model provider `Package`, Memory `Brain`, Subagents `Bot`, Plugins
  `Blocks`, MCP `Cable`, Skills `WandSparkles`, Commands `Terminal`, Automations `AlarmClock`,
  Hooks `Anchor`, Browser `Globe2`, Remote devices `HardDrive`, Computer use `Monitor`,
  Shortcuts `Keyboard`, File search `FileSearch`, Usage `BarChart3`, Archives `Archive`.
- Row pattern `settings/SettingsPageParts.tsx:109-146` `SettingsRow`:
  `border-t border-border px-4 py-3 first:border-t-0` (hairline-divided rows, no zebra);
  label `text-ui-base font-medium`; description `text-ui-base leading-6 text-foreground-subtle`
  (mt-1); control right-aligned (`justify-end gap-2`), `controlLayout: "wide"` moves the
  control to its own row. Rows are grouped in `SettingsGroupCard` (`:148-154`):
  `Card rounded-xl border border-border bg-card shadow-none`, content `px-0` — the card
  supplies only the outer border/radius, rows the separators.
- Slider control: `w-[280px]` row with `h-1.5 rounded-full bg-secondary` track (`:164-193`).
- Density feel: 14px base text, 12px descriptions-in-rows are NOT smaller (same 14, subtle
  colour), py-3 (12px) vertical rhythm, 8px nav item height. No Global/Project tabs in the
  ZCode page — scope is shown via badges (`settings/SettingsScopeBadge.tsx`) — for oino #19
  the Project tab just needs a label; keep ZCode's rail+rows structure.

### A(#2/#3). Composer: shell, inner footer, context-in-input, mode switcher

Shell (`prompt-editor/ChatPromptEditor.tsx:409-411`): one rounded card containing everything
(text, attachments, toolbar):
`relative flex flex-col gap-3 overflow-hidden rounded-2xl border border-input-border bg-input p-3
transition-colors hover:border-input-border-hover focus-within:!border-input-border-focused
focus-within:bg-input-focused` — 16px radius, 12px padding, border+bg brighten on focus.
Drag-over state: `border-brand ring-1 ring-brand/30` (`:414`). Inline (compact) variant is
`flex-row flex-wrap items-end gap-2 p-2` (`:412`).

Inner footer row (`:441-461`): `flex items-end gap-3`; left cluster =
"+" attachment menu + leading actions + **mode switch**; right cluster (`ml-auto gap-1.5`) =
model/context controls + stop/send. oino issue 3 maps 1:1: no footer *below* the input — the
row lives inside the same rounded card.

- **Mode switcher** (always visible, left of footer; `v4/composer/V4ComposerModeControls.tsx`):
  trigger = ghost button `h-7 gap-1 rounded-lg px-2 text-ui-base` with mode icon 16px + label +
  chevron 14px (`:125-146`); per-mode colours: yolo-mode text `--color-warning`, guarded mode
  `--color-icon-blue` (`:127-130`). Menu (`:140-191`): opens **upward** (`side="top"
  sideOffset={4}`), `w-64` (256px); first a Plan checkbox row (`min-h-13 items-start gap-3
  py-2`, lightbulb 18px + name + `text-ui-sm` description), separator, then radio rows with
  icon + name + description each. Cycle shortcut Ctrl+Shift+M (`toolbarShortcuts.ts`); label
  uses a rolling/vertical-slide animation on change (`chat-input-toolbar/RollingToolbarLabel.tsx`).
  When the composer is narrow the label+chevron collapse to a 28px square icon-only trigger
  (`data-[composer-compact=true]:w-7 px-0`, container query `@xl/composer` restores them).
- **Context usage inside the input** (`chat-input-toolbar/contextUsage.tsx:930-1103`, ring
  icon `components/ai-elements/context.tsx:77-109`): trigger is a ghost `icon-md` button whose
  icon is a **14px SVG ring gauge** (viewBox 24, r=10, stroke-width 4; track
  `opacity:.25`, arc `opacity:.7`, `strokeDashoffset = C·(1−used%)`, rotated −90°) —
  `context.tsx:21-24,81-109`. Turns into a spinner while quota reset runs (`:131-134`).
  Tones: normal `text-foreground-subtle`, healthy `text-success`, urgent `bg-warning/10
  text-warning` (`contextUsage.tsx:969-974`). Hover card opens above (`side="top"
  sideOffset={2}`, `!w-64 rounded-xl shadow-md`, `:995-999`) containing: title row
  "Context usage" + mono compact token label right (`:1017-1024`); a `h-2` multisegment
  progress bar (segment tones derived from `--color-usage-chart-1` mixed with surface at
  78/58/42/28%, `:56-61`); per-source breakdown rows (8px colour square, label, mono %
  right-aligned, `text-ui-sm`, `:1030-1070`); optional cache-hit-rate row
  (`:1071-1082`); compression command lives in this panel (`contextPanelAction.ts`).
- Model + thinking controls sit next to it in the same footer cluster
  (`v4/composer/V4ComposerToolbar.tsx:1256-1382`): `ChatContextUsage` → `ModelConfigSelect`
  (ghost trigger `h-7 px-2 text-ui-sm`, chevron, tooltip = full provider/model name, shortcut
  label; menu aligns center; submenu `w-40 min-w-0` capped to viewport on mobile `:1250-1253`)
  → `ThoughtLevelCycleControl` (cycles with Ctrl+T, `:1360-1382`). Keyboard: Ctrl+M opens
  model menu (`:1233-1243`).
- Send/stop (`v4/ConversationComposer.tsx:2312-2436`): right side = stop **or** send.
  Stop: `variant="secondary" size="icon-md"` square icon button, filled `SquareIcon`
  `size-4 fill-current`, tooltip "Stop · Esc" (`:1901,2386-2396`). Send: `size="icon-md"`,
  `rounded-lg bg-brand text-foreground-inverse hover:bg-brand/80`, `ArrowUpIcon` (or spinner),
  tooltip with Enter shortcut (`:2397-2411`). Show stop when running **and** draft empty
  (`:1336-1337`); with a draft typed, send queues a follow-up instead.
- Placeholder: "Ask ZCode anything, @ to add context, / for commands or capabilities"
  (`i18n/locales/en-US.ts:5635-5637`).

### A(#15). Esc-to-stop

ZCode: **single Esc** stops, heavily guarded (`v4/SessionPane.tsx:4608-4624`):
window keydown listener active only while `focused && !readOnly && canStop`; ignores events
not containing the pane in `composedPath()` unless `allowEscapeStopOutsidePane` (shell grants
it when no overlay owns Esc); `shouldIgnoreEscapeForStopGeneration`
(`v4/composer/escapeStop.ts:3-16`) skips events that were already handled (`defaultPrevented`)
or that passed through a dialog (`data-slot="dialog-content"` or `role="dialog"` in the event
path) — so closing a dialog with Esc never stops the task. Stop tooltip advertises the
shortcut (`ControlHintTooltip title stopTooltipTitle shortcut="Esc"`,
`ConversationComposer.tsx:2386-2390`). There is **no double-Esc affordance in ZCode** — the
"press Esc again" hint (oino #15) is an oino addition; implement first-Esc = show inline
hint/toast, second-Esc within a short window = stop, using the same dialog-path guard.

### A(#5). Sidebar icons (lucide) + nav shape

- Top-level nav above the project tree (`project-sidebar/ProjectSidebarNavigation.tsx:29-52`):
  Automations = `CalendarClock` `size-4`; Plugins = `Blocks` `size-4`; full-width ghost
  buttons `size="lg" justify-start gap-2`, active `bg-selected text-foreground`.
  (oino #5/#13: Automations → `CalendarClock`; settings Automations section uses
  `AlarmClock` — `settings/settingsPageConfig.ts:115-117` — either is fine, pick one.)
- Sections (`PurposeSections.tsx:96-140`): "Projects" and "Tasks" collapsible sections,
  headers have collapse-all (`ListChevronsDownUp`/`ListChevronsUpDown` 14px) and add
  (`Plus` / `MessageCirclePlus` 14px) icon buttons; sections drag-reorder (dnd-kit, vertical).
- Project rows use coloured folder icons (`project-sidebar/ProjectFolderIcon.tsx`:
  Folder/FolderOpen/Folders base); remote/host rows carry a connection-state dot
  (`remote-connection/RemoteConnectionStateIcon.tsx`).

### A(#12). Host/remote switching — centralized

ZCode has **no per-page host picker**. Identity of "where am I running" is shown once:
- Sidebar top: device header for remote-control sessions
  (`ProjectSidebarNavigation.tsx:20` `WebRemoteControlDeviceHeader`).
- Remote hosts appear as sidebar sections listing their projects
  (`project-sidebar/RemoteHostSection.tsx:3-8` labels hosts `user@host`;
  `RemoteProjectPicker.tsx` / `RemoteProjectList.tsx` list per-host projects; connecting
  state icons per row).
- Settings has one "Remote devices" section (`settingsPageConfig.ts:135-137`, icon
  `HardDrive`); connecting/SSH flows are dialogs
  (`SSHDialog.tsx`, `remote-connection-flow/`, `RemoteConnectionWizardChrome.tsx`).
GPUI mapping: one host switcher in the sidebar (top) + settings section; pages never pick hosts.

### A(motion). Animations worth mirroring

- Rolling label swap on toolbar chips (`RollingToolbarLabel.tsx`) — vertical slide.
- Collapsible sections 300ms ease-in-out (`styles.css:1113-1135`).
- Indeterminate progress `1.2s ease-in-out infinite` (`styles.css:998`); ring/breathe
  effects 1.8s/900s heartbeat (`:1280,1348`); alarm ring 600ms (`:1178`).
- Result highlight flash `1.2s ease-out` (`:1396`), fork pulse `1.6s` (`:1415`).
- Pause all CSS animation while window hidden (`html[data-document-hidden]`,
  `styles.css:88-94`) — relevant for oino background streaming (memory).
- Reduced-motion: every effect gated (`styles.css:975+`).

---

## PART B — unipi (copy for harness features)

Base palette (`unipi-app/apps/mobile/src/style.css:16-18`): shell/accent `#F03018`
(shell-hi `#F01818`, claw `#F07818`, amber `#F0A818`, gold `#F0C030`, cream `#F0F0A8`,
face `#181818`), wm-red `#F03C32`, wm-green `#5AD25A`, wm-cyan `#28BEF0`, wm-yellow `#FAC828`;
bg `#0A0908`, surface-1 `#141211`, surface-2 `#1C1917`, surface-3 `#26211F`, line `#2E2825`,
text `#F5EFE6`, text-muted `#A39A90`, text-dim `#6E655D`. Fonts: Inter Variable /
JetBrains Mono Variable (`style.css:19`). Global metrics `ui.css:14`: page gutter 16,
radius-float 16, button 44, chip 38.

### B(#1). Model picker modal

Component: `ModelPickerSheet` in
`unipi-app/apps/mobile/src/components/chat/ControlCentre.tsx:422-661`; container
`BoardSheet` (`components/board/BoardSheet.tsx:69-121`) = corvu bottom Drawer:
`.sheet-overlay` = fullscreen `rgb(0 0 0 / 50%)` z-20 (`style.css:48`); sheet surface
`background: var(--color-surface-1)`, `padding: 20px 20px calc(20px + safe-area)`
(`style.css:137`), phone radius `24px 24px 0 0` + drag handle `36×4px` bar
(`style.css:125,138`), desktop shell: `border-radius: 16px` (`--radius-float`,
`ui.css:14`, `shell.css:97-104`), width unconstrained (full sheet). Header row
`min-height:44px; padding:0 12px 6px 20px`, title 20px/26 semibold, ✕ 20px icon
(`screens/board.css:249-251`); body scrollable `padding: 4px 20px 20px` (`:252`);
sticky footer `padding:12px 20px 14px`, hairline top `rgb(46 40 37 / 60%)`,
bg surface-1 (`:253`).

Picker content (classes in `screens/chat.css` + `screens/board.css`):
- "Now" row: `.mp-current` 13px muted + `.mp-current-label` uppercase pill 10.5px/18px on
  surface-3 (`chat.css:445-446`).
- Single/Fusion segmented radiogroup `.think-levels/.think-level`: pills `min-height:36px;
  padding:0 14px; radius 9999px; bg surface-2; 14px`; selected `bg var(--color-shell);
  color var(--color-face); weight 600` (`chat.css:306-308`).
- Search field `.model-search` — rendered **only when >6 models** (ControlCentre.tsx:619-621):
  `height:42px; padding 0 14px; radius 12px; bg surface-2`, placeholder
  "Search all N models"; matches span full list, curated hits float first
  (`pickerModels`, ControlCentre.tsx:32-58).
- Model list `.model-list` = one rounded-16 surface-2 block with hairline separators
  between rows (`chat.css:310-312`); row `.model-row` `min-height:54px; padding:8px 14px;
  gap:10px; 15px` with name (truncate) + mono sub `provider/id` 11.5px dim
  (`.model-row-sub`, `chat.css:314`); current row `background: rgb(240 48 24 / 7%)`
  (`chat.css:313`); tapped-but-unapplied row `data-picked`: `rgb(240 48 24 / 14%)` +
  3px inset left accent bar (`chat.css:450`); "current" tag pill 10.5px green
  (`.mp-tag`, `chat.css:451`); check icon 18px accent. List scrolls
  `max-height: min(44vh,380px)` with a fade-out mask on the bottom 24px
  (`chat.css:448`, `ui.css:233`); renders max 120 rows (ControlCentre.tsx:634).
- Fusion tab (`ControlCentre.tsx:541-600`): two columns `.fusion-pair` (flex gap 10px,
  `chat.css:381-382`), Lead and Sidekick lists each `max-height: min(30vh,220px)`
  (`.cc-fusion-list`, `chat.css:383,449`), row names clamped to 2 lines 14px/19px
  (`.fusion-model-name`, `chat.css:384`); per-side effort radiogroups low/medium/high
  (`ControlCentre.tsx:577-598`). Pending picks highlight locally; the footer button applies
  ("Use this pair" / "Update pair"; busy "Applying…" with 15px spinner `.mp-spinner`
  0.8s linear, `chat.css:457`).
- Footer (`.mp-foot`, `chat.css:452-455`): Single mode = inline hint 13px dim ("Click a model
  to switch.") until a pick exists, then Cancel (`.pair-secondary`, flex-1) + primary action
  (`.pair-primary`, flex-1.6); both pill buttons `height:44px radius 9999px 15px`
  (`ui.css:85`), primary `bg var(--color-shell) color var(--color-face)`
  (`ui.css:77-80`), disabled opacity .5; errors inline `rgb(240 48 24 / 12%)` red block
  (`.mp-error`, `chat.css:456`).
- Fusion header label format: `Fusion · {lead} ◆ {sidekick}` (`modelHeaderLabel`,
  ControlCentre.tsx:105-110); provider prefix stripped from names in narrow columns
  (`shortModelName`, :73-77).
- **oino deltas to keep**: unipi's fusion tabs list only curated leads/sidekicks from the
  session state — oino must let the user pick *any* model as custom lead/sidekick (feedback #1)
  and support pinning models to the top (Devin-style; no unipi/ZCode reference — new design).

### B(#10). Kanboard layouts

Phone (`screens/board.css`, `components/board/BoardLanes.tsx`):
- Fixed header: `.pc-chip` host pill `h:34px; radius:9999px; bg surface-2; 13px` with 8px
  status dot (green live + halo `0 0 0 3px rgb(90 210 90 / 18%)`, pulse `dot-pulse 1.2s`
  while connecting) (`board.css:49-62`); project title row 36px + chevron (`:70-77`);
  running ring on project tile: conic-gradient gold ring, `ring-spin 2.4s linear`
  (`:82-88`).
- Toolbar `.board-toolbar` `padding:12px 16px 10px; gap:8px` (`:92-94`): view toggle pill
  `padding:3px; radius:9999px; bg surface-2`, buttons `40×32px`, checked =
  `bg surface-3 + shadow 0 1px 4px rgb(0 0 0 / 35%)` (`:95-101`); `.tool-chip` chips
  `h:38px; padding 0 14px; radius 9999px; bg surface-2; 14px`, active filter chip
  `bg rgb(240 48 24 / 14%) + inset ring rgb(240 48 24 / 38%)`, count badge 18px round
  shell-red (`:102-110`). Toolbar contents (`screens/Board.tsx:498-552`): view toggle
  (Columns3/List), scope chip, search chip, Filter, Display, Clear, and on desktop shell a
  right-aligned primary "New task" button (`.board-new-task`).
- Lane strip `.lane-strip` `gap:8px; padding:2px 16px 12px` (`board.css:107-108`); pills
  `.lane-pill` `h:40px; padding 0 10px 0 13px; radius:9999px; bg surface-1; inset ring
  rgb(46 40 37 / 70%); 14px` with StatusGlyph 14px + count badge 22px; selected pill =
  `color-mix(lane 15%, surface-1)` bg + `1.5px` lane-coloured inset ring; count badge tints
  with the lane (`board.css:109-115`); strip auto-scrolls active pill to centre
  (BoardLanes.tsx:57-66, smooth).
- Lane pages `.lane-pager` horizontal `scroll-snap-type: x mandatory` (one swipe = one lane,
  `scroll-snap-stop: always`), page = `flex 0 0 100%`, own vertical scroll, `padding:
  0 16px 108px` (FAB clearance) (`board.css:119-123`); snap settle after `scrollend` or
  140ms fallback, haptic tick on lane change (BoardLanes.tsx:236-260). Lane head: info text
  12.5px/17 dim clamped to 2 lines + quick-action pills 34px tinted with lane colour
  (`.lane-quick`, `board.css:125-129`) + ⋮ 18px.
- Cards `.bcard`: `padding:13px 14px; radius:16px; bg surface-1; inset ring
  rgb(46 40 37 / 75%); gap 6px; press scale .985`; title 15.5px/21 weight 550 clamped 3
  lines; mono id 12px; excerpt 13px/18 muted 1 line; blocked reason callout
  `border-left:2px solid red; bg rgb(240 60 50 / 10%); #f6a39d; 13px/18` 2 lines
  (`board.css:142-160`); running card amber ring `rgb(240 168 24 / 45%)` (`:146`);
  final/done cards `bg rgb(20 18 17 / 60%)`, muted title; cancelled = line-through
  (`:147-149`); update flash `card-flash 1.6s` (ring → 4px red glow) (`:153-154`).
  Tags `.btag` `h:22px; radius:6px; 12px` surface-2 + inset line, label tags carry a 3px
  left colour bar (`board.css:191-207`); agent chip `h:22px; radius:11px; amber/15% bg` with
  6px pulsing dot (`pulse-ring 1.8s ease-out`) (`board.css:213-219`).
- Dependency chains: left rail joining same-lane blocks — 8px nodes at `left:-14px`, 1.5px
  vertical connector, chain colour = lane accent; locked chain grey (`board.css:165-172`).
- FAB 58×58 radius 20 shell-red with red glow shadow, `right:16px; bottom:18px`
  (`board.css:434-437`); toasts bottom-left `min-height:52px; radius:16px; bg surface-3;
  shadow 0 10px 30px rgb(0 0 0 / 55%)`, slide-in `220ms cubic-bezier(.2,.8,.2,1)`
  (`board.css:439-444`).
- Lane status colours (`board.css:6-21`): backlog `#8a8178`, todo `#c9bfb3`,
  in_progress `amber #F0A818`, blocked `wm-red #F03C32`, in_review `wm-cyan #28BEF0`,
  done `wm-green #5AD25A`; priority urgent `#F03018`, high `#F07818`, medium `#F0C030`,
  low muted. StatusGlyphs are 14px SVGs (dashed circle = backlog, half-wedge = in-progress,
  full dot + check = done, X = cancelled, archive box; `components/board/Glyphs.tsx:19-84`).

PC (`data-shell=desktop`, `components/board/board-desktop.css`) — the layout the owner wants
"as exactly as possible":
- Tokens `:5-9`: `--lane-w: 288px; --lane-gap: 10px; --lane-strip-w: 36px;`
  `--lane-bg: color-mix(in srgb, surface-1 62%, bg)`. Lane strip hidden; columns
  `.lane-page` = `flex 0 0 288px; radius:14px; bg lane-bg; inset ring rgb(46 40 37 / 45%)`,
  horizontal strip scrolls (`:11-23`).
- Column head `.lane-head-desk`: sticky, `height:42px; padding:0 4px 0 12px; gap:6px`,
  same lane-bg; content = glyph 13px + name 13px/600 + count 13px muted + ⓘ (20px, hover
  muted, title tooltip = lane about-text) + spacer + quick action (26px pill, 12.5px) + ⋮
  (32px square) (`:26-40`; JSX BoardLanes.tsx:471-507). The lane "about" text moves into the
  ⓘ tooltip and the ⋮ menu on desktop (`BoardLanes.tsx` LANE_INFO + `lane-menu-info`
  12.5px/18 in menu, `board-desktop.css:41`).
- Cards in columns: `gap:8px; padding 0 8px`; card `padding:10px 12px 11px; radius:12px;
  title 14px/20; id 11px; excerpt 12.5px/17; tags 20px/11.5px`; card transitions
  `transform/opacity/background/box-shadow 140ms ease-out` (`:44-53`); compact density
  `padding:8px 10px` (`:47`); "Add task" inline row `h:32px; radius:8px; 13px`,
  hover `rgb(245 239 230 / 5%)` (`:59-63`); empty state `padding:22px 12px; radius:12px`
  (`:58`).
- Detail view: when a task page is open the list column narrows to
  `--lane-w: min(288px, list-w − 2·gutter)` (`:104-105`); app shell columns
  `224px sidebar | list | detail` (`shell.css:30-34`).
- Task page (card detail, desktop): header/ⓘ buttons 40px, title 22px/28 semibold,
  section counts plain 12px text, composer buttons 40px (`:107-114`).

### B(#8). Drag animation + unrelated columns collapse

Logic `components/board/cardDrag.ts` (pointer-events, no HTML5 DnD):
- Pick-up threshold `DRAG_THRESHOLD = 5px` (a click still opens) (`:24`); blocked drags
  (running task) toast instead (`:212-218`).
- Ghost: clone of the card in `.bcard-ghost` fixed overlay following the pointer
  (`translate3d`), `z-index:1000`, source card stays in place at `opacity:.35; scale(.98)`
  (`cardDrag.ts:219-231`; CSS `board-desktop.css:66-77`). Lift animation on next frame:
  `rotate(1.5deg) scale(1.03)` + `box-shadow 0 18px 40px rgb(0 0 0 / 55%), 0 4px 12px
  rgb(0 0 0 / 40%)`, transform-origin `30% 30%`, transitions `140ms ease-out`
  (`board-desktop.css:69,77`).
- **Unrelated columns collapse**: each lane gets `data-drop` = `ok | allowed | refused`.
  `refused` (drop not allowed by lane rules) collapses the column to a 36px vertical strip:
  `flex-basis: var(--lane-strip-w)`, cards/counts/quick actions hidden, head becomes centred
  vertical title (`writing-mode: vertical-rl; rotate(180deg)`), bg darkened
  `color-mix(lane-bg 60%, bg)` (`board-desktop.css:91-95`). Column width transition
  `flex-basis/width 180ms ease-out`, opacity 160ms, colours 140ms, gated by
  `prefers-reduced-motion` (`:97-99`). Collapse constant `PIN_MS = 400ms` slack for the
  re-pin animation (`cardDrag.ts:148`).
- Hovered valid target: `inset 0 0 0 1.5px` shell-red ring + `color-mix(shell 3%, lane-bg)`
  tint, head ring drawn with corner inset shadows (`board-desktop.css:84-85`).
- Insert indicator `.drop-line`: `height:2px; margin:-5px 4px; radius:2px; bg
  var(--color-shell)` with an 8px hollow cap on the left (`::before` circle, 2px inset
  ring) (`:80-81`).
- Autoscroll: 80px edge zones, max 1320 px/s (22px/frame @60Hz), time-based; scrolls the
  lane strip horizontally and the hovered column vertically (`cardDrag.ts:25-44,299-316`).
- Source lane stays pinned under the pointer while other lanes collapse/expand: measure
  anchor left edge before/after the change, compensate with `scrollLeft` (+ `--drag-room`
  spacer at strip end so the strip can grow), for 400ms via rAF (`pinWhile`,
  `cardDrag.ts:148-177`); spacer `.lane-pager::after` (`board-desktop.css:88-89`).
- Esc cancels mid-drag; the click after a drop is swallowed for 250ms (`cardDrag.ts:349-363`).
- Drop legality per lane: `lib/board/dnd.ts` (`dropAllowed`, `planDrop`), final lanes
  (done/cancelled/archived) don't drag (`FINAL_LANES`).

### B(#9). Card modal formatting (TaskDetail)

Unipi's card detail is a **full-screen pushed page**, not a sheet
(`components/board/TaskDetail.tsx:1-19` rationale; container `.board-page` = fixed inset-0
z-18, bg `--color-bg`, entry animation `page-in 240ms cubic-bezier(.2,.8,.2,1)` from
`translateX(24%)` + fade, `screens/board.css:333-335`). Structure (all `screens/board.css`):
- Header `.task-head` `min-height:56px`, back button 44px; lane crumb `.task-crumb` pill
  `h:38px; radius:9999px`, tinted `color-mix(lane 12%, surface-1)` with `34%` lane ring
  (`:337-341`); ⋮ menu.
- Title `.task-title-edit`: in-place auto-growing textarea `23px/30 weight 650
  letter-spacing -.01em`; focus = surface-1 bg + 6px halo (`:343-345`); desktop override
  22px/28 (`board-desktop.css:110`).
- Chips row `.task-chips` `gap:8px; margin:12px 0 2px`: status/priority/labels as
  `.prop-chip` pills `h:38px; padding 0 10px 0 13px; 14px`, lane chip tinted with lane mix
  (`:347-352`).
- Report card (In Review / Blocked): `.report-card` `padding:14px 14px 12px; radius:18px;
  bg color-mix(tone 9%, surface-1); inset ring tone 34%`, tone = cyan for review / red for
  blocked (`:356-360`); body preview clamped to 210px with a fade mask
  (`-webkit-mask … #000 160px → transparent 210px`, `:361-363`) + "Read" expander; action
  buttons `.report-button` `h:42px; radius:9999px; bg surface-3; 14.5px/600`, primary =
  shell-red (review primary = done-green with `#071407` text) (`:366-369`).
- Run card `.run-card`: `radius:18px; bg surface-1; amber ring 32%`; stale = red tint +
  red chip (`:369-377`); run grid `dt` dim / `dd` text, 13.5px/19 (`:378-383`).
- Sections `.task-section`: `margin-top:14px; padding-top:12px; hairline top`; heads =
  uppercase 12px/500 `letter-spacing .1em` dim + count pill 11.5px + gold text action
  (`:386-395`); description body `.body-card` 15.5px/23 `#e9e1d6` rendered as markdown
  (`:400`).
- Dependencies `.dep-row` `radius:14px; bg surface-1; inset ring` with 52px open rows and
  state chips (ready green `rgb(90 210 90 / 12%)`, waiting amber, locked/missing red)
  (`:396-407`); lock note red-tinted callout (`:407`).
- Attachments: 3-col thumbnail grid, square 12px-radius tiles with inset line ring
  (`:408-411`); file rows 52px like dep rows (`:412-416`).
- Note composer pinned above the keyboard (footer `padding:12px 20px 14px+safe-area`,
  hairline, surface-1 — shared `.board-sheet-foot`, `:253`).

### B(#11). Overlay full text

Image 3 unavailable. Truncation points found in the board overlays — one of these is the
culprit:
- Menu overlay rows truncate: `AnchoredMenu.tsx:102` `<span class="min-w-0 flex-1 truncate
  text-left">{props.label}</span>` (240px default width, 288px for lane menus) — long menu
  labels ellipsize despite free vertical space. Fix: wrap instead of truncate.
- Phone lane about-text is clamped to 2 lines: `.lane-info` `-webkit-line-clamp: 2`
  (`board.css:126`) — but on desktop it shows in tooltips/menu instead
  (`board-desktop.css:41`).
- Other suspects (dashboard only): `.dash-row-title`, `.dash-activity-text` truncate
  (`components/board/Dashboard.tsx:335,437`).
Report what image 3 shows before implementing; the menu-row fix is the most likely intent.

### B(#17). Markdown colour scheme (shared `.md` stylesheet)

Tokens `style.css:22-26`: body 15px/24 `--color-text`; **strong `#FFF8EC` weight 680**;
inline code = surface-3 bg + **cream `#F0F0A8`** text, mono, 0.86em, `padding:1px 5px;
radius:5px`; links = **wm-cyan `#28BEF0`** underlined (offset 2px); list markers =
**claw orange `#F07818`**; blockquote = 2px `--color-line` left bar + muted text;
del = muted. Tables: 13.5px/19, head bg surface-2 muted 600, borders `rgb(46 40 37 / 70%)`,
sticky first column, cell padding `6px 10px` (`style.css:322-330`). Code blocks `.cb`:
`radius:14px; bg surface-1; inset ring rgb(46 40 37 / 80%)`, sticky head surface-2 with mono
lang label 11px dim + copy button; code 12.5px/19; optional line numbers at 2.6em dim
(`style.css:333-346`). Syntax highlighting (CodeMirror/Lezer classes,
`style.css:348-358`): keywords/tags = wm-red `#F03C32`; strings = wm-green `#5AD25A`;
numbers/bools = gold `#F0C030`; comments = dim italic; functions = wm-cyan; types/classes =
amber `#F0A818`; properties/attributes = wm-yellow `#FAC828`; punctuation = muted.
Checkboxes: 15px, `accent-color: var(--color-wm-green)` (`style.css:310`).
Headings h1–h4: 1.3/1.18/1.08/1em, weight 650, `margin:14px 0 6px` (`style.css:311-315`).

### B(#26). Real unipi logo assets

- **Logo PNG**: `/home/oi/Projects/Personal/archived/unipi/docs/assets/unipi-logo.png`
  (512×512 RGBA, 84 KB-class). Co-located: `unicrab-pixel.png` (crab mascot render),
  `unicrab-real.jpg`, `screenshots/`.
- In-app crab: `unipi-app/apps/mobile/src/assets/unicrab-pixel.png` (1254×1254 RGBA)
  imported as `<img>` brand in the desktop shell
  (`components/shell/DesktopShell.tsx:14,43`), rendered `26×26px; image-rendering: pixelated`
  (`components/shell/shell.css:46`); used on setup/empty/pairing screens too.
- Animated mascot: SVG rects from `apps/mobile/src/assets/unicrab/unicrab.sprite.json`
  via `components/Unicrab.tsx` + `lib/unicrab.ts` (crisp-edge pixel rig; DESIGN.md §1.5.1).
- App icons (Tauri): `unipi-app/apps/mobile/src-tauri/icons/icon.png` 512×512 (+icns/ico).
- Wordmark colours (for a text logo): `renderWordmark()`
  `packages/core/src/hints/index.ts:314-324` — "U N I P I" letters in white, cyan
  `rgb(40,190,240)`, green `rgb(90,210,90)`, yellow `rgb(250,200,40)`, red `rgb(240,60,50)`.
- Oino action: ship `unipi-logo.png` (or a trace of it) as a GPUI raster asset instead of the
  current grey placeholder icon (feedback #26); crab pixel art is a secondary asset if
  wanted.

---

## Verified-uncertain / open

- **#11 overlay**: screenshot unavailable; three truncation candidates listed above — confirm
  which overlay image 3 shows.
- ZCode Esc-stop is single-Esc with guards; the double-Esc + hint is oino's own design
  (feedback #15) — no reference implementation exists in either app.
- ZCode's model picker has no pin-to-top (Devin-only per feedback #1); unipi's fusion lists
  are curated-only — both "pin" and "custom lead/sidekick" need fresh design.
- Unipi PC board metrics measured in CSS px at the WebView scale; GPUI should treat them 1:1
  as logical px.
- unipi-app has no separate desktop app sources in the archive (the "desktop" is the same
  SolidJS app under `data-shell=desktop`); `unipi/` (older Tauri shell) and
  `packages/*` (pi extensions) were not needed for these UI items.
