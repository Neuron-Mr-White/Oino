# ZCode vs unipi-app UX audit (v2 phase A — for owner rulings)

Status: **awaiting owner rulings.** Work id `[zcode-audit]`, branch `v2`, 2026-10-31. Read-only audit;
nothing is implemented from it until each `Owner ruling:` line is filled
([DIRECTION.md](DIRECTION.md) §4.2 rule 3, §9 phase A).

Sources:
- **unipi-app** (base we keep): `apps/unipi/apps/mobile/src` (imported verbatim from
  `archived/unipi/unipi-app` @ `336b60a`, release 1.0.0) + `apps/unipi/DESIGN.md`; design docs and
  screenshots in the archive's `docs/m5..m9/` (e.g. `docs/m5/design/0*-chat-*.png`). Paths below
  are relative to `apps/unipi/apps/mobile/src` unless they start with `apps/` or `DESIGN`.
- **ZCode** (reference): `/tmp/zcode-src` @ `3c5b2276` (`git clone --depth 1
  https://github.com/zai-org/ZCode`): `DESIGN.md`, `docs/ui/*.md`, `packages/ui/src`. Paths below
  starting with `ui/` mean `packages/ui/src/`, `docs/ui/` means the spec of that name.
- oino's earlier ZCode pass: [../v1/zcode-details.md](../v1/zcode-details.md) (M/T/C/N/F rows) and the
  owner's alpha.5 QA: [../v1/round5-qa2.md](../v1/round5-qa2.md) (`QA#n` below).
- unipi extensions: `extensions/unipi/*/src/lib.rs` module tables.

Owner rules applied: (1) every harness feature reachable; (2) no redundant UI; (3) ZCode
differences are asked, not decided; (4) ZCode chat optimisations preferred; (5) unipi keeps
Unicrab look, palette, mascot. "Take ZCode" always means *ZCode behaviour in the unipi look*.

Item format: **ZCode** (ref) · **unipi** (ref) · **Rec** (take ZCode / keep unipi / hybrid) — reason ·
**Effort** S (≤1 day) / M (≤1 week) / L (more) · `Owner ruling:` (blank).

---

## Summary

- **116 items** (Z-1..Z-116) in 22 areas, **21 gaps** (G-1..G-21) in the reachability table,
  **24 already-decided** points (D-1..D-24) listed for completeness, not asked.
- unipi-app 1.0.0 is a strong *phone-first remote* for pi-in-herdr: rich harness surfaces
  (working-mode chip, Progress/Dream/BTW sheets, Session modal, ask_user cards, Board, Cron,
  stealth browser, push, mascot). ZCode is a *desktop-first workspace*: one sidebar of
  projects→sessions, a conversation pane with a turn-based virtual timeline, a right **side pane
  of tabs** (git review, browser, terminal, files, previews, side chats, subagents), a bottom
  terminal, and a top-right **status panel**.
- The biggest structural questions: adopt ZCode's **side-pane-of-tabs** model as the one place
  for git/browser/terminal/files/preview/subagents (Z-5), merge unipi's per-screen **PC chip**
  into a sidebar of hosts→projects→sessions (Z-1/Z-8), take ZCode's **turn fold "Worked for N s"**
  and **turn-tail actions** (Z-21/Z-27), move dialogs from *replacing* the composer to *docking
  above* it (Z-46), and decide whether the ZCode status panel (todos, background work, subagents)
  absorbs the Session modal's Running tab (Z-95, redundancy).
- Reachability: unipi-app reaches the harness mostly through `/` commands and app-bridge
  specials. Under oino every extension panel, widget, status and settings namespace needs the
  §5 UI v2 slots; today 4 dialog kinds, all `custom`/overlay/sidePane panels, all widgets, all
  statuses and all 21 settings namespaces are unreachable (G-1..G-5). MCP, memory browsing,
  skills exposure, subagent profiles and permission/plan mode have **no UI at all** (G-6..G-11).

Counts per area:

| Area | Items |
|---|---|
| A App shell & navigation | Z-1..Z-8 (8) |
| B Sessions list & session actions | Z-9..Z-20 (12) |
| C Chat timeline | Z-21..Z-36 (16) |
| D Tool rows, thinking, diffs | Z-37..Z-45 (9) |
| E Dialogs, errors, retry | Z-46..Z-51 (6) |
| F Queue, steering, interruption | Z-52..Z-56 (5) |
| G Composer | Z-57..Z-68 (12) |
| H Model, thinking, context | Z-69..Z-76 (8) |
| I Command palette & search | Z-77..Z-81 (5) |
| J Notifications / needs-you / toasts | Z-82..Z-86 (5) |
| K Empty / loading / error states | Z-87..Z-90 (4) |
| L Motion | Z-91..Z-93 (3) |
| M Keyboard shortcuts | Z-94 (1, table) |
| N Git pane & status panel | Z-95..Z-100 (6) |
| O Files, editor, LSP, preview | Z-101..Z-104 (4) |
| P Terminal | Z-105..Z-107 (3) |
| Q Browser | Z-108..Z-110 (3) |
| R Board | Z-111 (1) |
| S Cron | Z-112 (1) |
| T Settings | Z-113..Z-114 (2) |
| U Mobile layout | Z-115 (1) |
| V Accessibility | Z-116 (1) |
| (Reachability) | G-1..G-21 |

---

## Already decided (listed, not asked)

From the owner's brief and [DIRECTION.md](DIRECTION.md):

- **D-1** Per-turn file summary with undo, as an **extension** (`turnFooter` slot): collapsed "N files +X/−Y", expand, undo per file / per set, snapshots ≤ 5 MB, bash not captured (ZCode `ui/v4/ConversationFileSummaryPanel.tsx`, `docs/ui/task-file-change-summary.md`). Replaces unipi's "Changes this turn" card (`components/chat/Transcript.tsx` `ChangesCard`).
- **D-2** Git pane like ZCode (review-oriented, last-turn scope, lazy per-file diffs) that **contains long-horizon status**, plus a small capsule visible when the pane is collapsed (replaces the top-right capsule).
- **D-3** Multi-tab terminal like ZCode: bottom panel + side-pane tabs, xterm.js, PTY on the host (works from the phone). Agent bash output stays in chat tool rows.
- **D-4** ZCode browser UX on top of oino-browser: tab strip + URL bar in the side pane, element picker → composer chip, user screenshot → composer.
- **D-5** Image annotation: add an **ellipse/circle** tool to `components/chat/ImageEditor.tsx`.
- **D-6** Fix pending images vanishing on chat switch: move `Composer.tsx`'s `images` signal into a per-chat store like `pendingFilesFor` (`lib/chat/attachments.ts`).
- **D-7** Info-screen **Session modal** with Context / Running / Info tabs (`components/chat/ControlCentre.tsx` `ControlCentreSheet`) stays.
- **D-8** Chat performance = ZCode's: TanStack virtual with a **whole turn** as the virtual unit, LRU row-height cache keyed by turn id, memoized rows + lazy detail render, a single bottom-lock follow state, per-session scroll memory (DIRECTION §4.2; ZCode `docs/ui/chat-virtual-scroll.md`, `ui/v4/timelineRowHeightCache.ts`). unipi today virtualizes per **row** (`ChatView.tsx` `createVirtualizer`, `estimate()`).
- **D-9** Sessions are oino sessions **grouped by project**, many running at once (replaces herdr workspace groups in `screens/Chat.tsx`).
- **D-10** Files = tree + CodeMirror editor + **LSP like Zed** (replaces the breadcrumb list `screens/Files.tsx`; unipi has no LSP).
- **D-11** **One Settings screen** for pi agent `settings.json` + app/host settings, JSON-backed, with schema-driven extension sections.
- **D-12** Board, Cron, Settings, SetupWizard, Preview, Editor screens kept.
- **D-13** Host features kept: push (ntfy/UnifiedPush, E2E), port forwarding, sharing/blob transfer, QR pairing, relay settings, host supervisor, app updater.
- **D-14** herdr/tmux-only screens removed: `screens/Sessions.tsx` (Ongoing/Recent/herdr picker), herdr parts of `screens/Terminal.tsx`, `NewChatSheet` herdr/tmux "Where" step, "Close chat… stop pi and close tab".
- **D-15** Desktop streaming view (`screens/DesktopView.tsx`) deferred.
- **D-16** `/answer` (utility) removed; info-screen kept; other TUI-only extension UI removed or ported when useful.
- **D-17** Extension UI v2: ratatui vocabulary, slots (overlay→modal, sidePane→right-pane tab, bottomPane→bottom-panel tab, editorReplace→composer tray, fullscreen→sidebar page, setWidget→above composer, setStatus→status bar, turnFooter→end of turn, settings schema→Settings section). No extension JS/iframes.
- **D-18** First-party rich screens (Board, Cron, Files, Git, Browser, Terminal, Info) open via reserved panel ids, shown only when the owning extension is enabled.
- **D-19** A crashed/trapping extension shows a notice; safe-mode banner names the suspect; per-extension toggle in the app (DIRECTION §3.3).
- **D-20** Tray: Show/Hide/Quit; ask once on window close (close vs minimize), changeable in Settings; start-at-login setting.
- **D-21** Browser engines/stealth per DIRECTION §6; pane and phone use `Page.startScreencast`.
- **D-22** Unicrab brand, palette, mascot, floating mascot and lore kept (owner rule 5).
- **D-23** Desktop and phone share one codebase.
- **D-24** Ask-once host lifetime + "This computer" + paired hosts over iroh (affects the host switcher, Z-8).

---

## A. App shell & navigation

**Z-1 Primary navigation model**
- ZCode: left sidebar = workspaces → tasks (the work list *is* the nav); Automations and Plugins entries on top; settings in footer; no screen tabs (`docs/ui/workspace-sidebar-navigation.md`, `ui/WorkspaceSidebar.tsx`).
- unipi: sidebar of screens Sessions·Chat·Board·Files·Cron·Preview·Settings; each screen has its own list | detail (`components/shell/DesktopShell.tsx` `NAV`); phone: bottom tab bar.
- Rec: **hybrid** — sidebar shows projects→sessions (ZCode) with a compact screen rail/footer for Board, Cron, Settings (unipi); Files/Git/Terminal/Browser move into the side pane (Z-5) so they are not top-level screens. Effort L.
- Owner ruling:

**Z-2 Workspace header (conversation top bar)**
- ZCode: task title + project as secondary, project/branch hover card (Folder/Cloud icon), "open in editor" split button, terminal toggle, side-pane toggle; ⋯ menu: open in file manager, copy path / task id / session id / log path (`docs/ui/workspace-header-structure.md`, `docs/ui/header-git-branch-switcher.md`).
- unipi: 56 px detail header: back, title + "PC · workspace" sub-line (title opens the model picker), working-mode chip, session strip (model·ctx·cost), ⋯ menu, globe (browser), terminal switch (`components/chat/ChatView.tsx` header, DESIGN §1.6 detail header).
- Rec: **hybrid** — keep the 56 px detail header and mode chip; make the right side ZCode's icon set (terminal icon, side-pane toggle — QA#17); title click should not open the model picker (redundant with the composer chip, Z-70); add copy-session-id/path to ⋯. Effort M.
- Owner ruling:

**Z-3 Sidebar collapse & width**
- ZCode: collapsible sidebar (Cmd+B), reopen button stays top-left with New task; default/min 264 px; auto-collapse side pane when chat < 480 px and sidebar when < 360 px on window resize (`docs/ui/workspace-layout.md`, `docs/ui/workspace-resize-auto-collapse.md`).
- unipi: fixed sidebar width (`lib/layout.ts SIDEBAR_WIDTH`), list | detail splitter remembered per screen; chat list folds away when list+chat+browser don't fit (DESIGN §5.8).
- Rec: **take ZCode** — collapse + resize auto-collapse rules are cheap and protect the chat column. Effort S.
- Owner ruling:

**Z-4 Independent frames & resize handles**
- ZCode: conversation, bottom terminal and side pane are separate framed panels with 4 px gaps; handles show a 2 px line on hover/drag (`DESIGN.md` "Workspace layout").
- unipi: flush panes with 1 px rules; splitter between list and detail (`components/shell/shell.css`).
- Rec: **keep unipi** look (warm flush surfaces) but take ZCode's handle affordance (hover line, 4 px hit). Effort S.
- Owner ruling:

**Z-5 Right side pane = tabs container**
- ZCode: one right pane with tabs (browser ×n, terminal ×n, review/git, files, code viewer, diff, plan, subagent session, side chat, workflow …), `+` launcher, tab overview with search, close current/others/all, recently closed + reopen, tabs scoped per workspace+task (`docs/ui/side-pane-tabs.md`, `ui/app-shell/AnimatedSidePanePanel.tsx`).
- unipi: only the browser can sit beside the chat (`components/chat/BrowserSide.tsx`); git, files, editor, terminal are separate screens/layers.
- Rec: **take ZCode** — it is the natural host for D-2/D-3/D-4/D-10 and the `sidePane` slot (D-17); avoids separate screens for the same things (rule 2). Effort L.
- Owner ruling:

**Z-6 Side pane "full view"**
- ZCode: Cmd+Shift+F expands the side pane over the conversation; chat becomes the first tab and a floating composer stays over other tabs (`docs/ui/side-pane-full-view.md`).
- unipi: none (full-screen layers on phone only).
- Rec: **take ZCode** (desktop only) — useful for git review and browser; low risk once Z-5 exists. Effort M.
- Owner ruling:

**Z-7 Split panes / multiple conversations**
- ZCode: workbench split panes (`ui/v4/WorkbenchPane.tsx`, `paneLayoutStore.ts`) — several sessions side by side.
- unipi: one open chat at a time; Alt+↑/↓ switches.
- Rec: **ask** — not needed for v2 parity; recommend **defer**. Effort L.
- Owner ruling:

**Z-8 Host switcher**
- ZCode: remote workspaces (SSH/WSL/Docker) sit in the same sidebar as local ones, Cloud icon + hover card with host details (`docs/ui/header-git-branch-switcher.md`, `ui/WorkspaceSidebar/RemoteProjectDetailsPopover.tsx`).
- unipi: a `PcChip` + "Choose a PC" sheet per screen (Chat, Board, Files, Preview), remembered per screen (`components/PcChip.tsx`, `screens/Chat.tsx`).
- Rec: **hybrid** — sidebar groups by host (This computer first, then paired PCs, collapsible) → projects → sessions; keep the PC chip only on Board/Cron where content is per-PC. Effort M.
- Owner ruling:

## B. Sessions list & session actions

**Z-9 Grouping & views**
- ZCode: views "By project / By group / Recently updated"; running + background-work sessions always on top inside a project (`docs/ui/sidebar-task-view-switcher.md`, `docs/ui/task-grouped-view.md`).
- unipi: groups per herdr workspace, needs-you first (`lib/chat/list.ts needsYouFirst`).
- Rec: **hybrid** — by project (D-9) with running-first + needs-you-first ordering; ask whether a "Recently updated" flat view is wanted; skip user-defined groups. Effort M.
- Owner ruling:

**Z-10 Time groups**
- ZCode: timeline view grouped Today / Yesterday / 2–3 days ago / This week / Last week / This month / Last month / Older (`docs/ui/sidebar-task-timeline-groups.md`).
- unipi: none (v1 deferred as D9 in zcode-details).
- Rec: **ask** — only meaningful if the flat "Recently updated" view (Z-9) is taken; inside projects recommend relative time on the row instead. Effort S.
- Owner ruling:

**Z-11 Paging long lists**
- ZCode: 5 sessions per project, "Show more" adds 5 (`docs/ui/workspace-sidebar-navigation.md`).
- unipi: all rows rendered.
- Rec: **take ZCode** — needed once archive (Z-13) isn't enough with many oino sessions. Effort S.
- Owner ruling:

**Z-12 Row content**
- ZCode: title (fade mask + 1 s hover marquee, no ellipsis), relative time, change summary +N −M, permission/interaction badge, running spinner, red error dot, blue unread dot (`docs/ui/task-item-title-overflow.md`, `ui/TaskListItem.tsx`, `ui/TaskInteractionBadge.tsx`).
- unipi: π glyph tinted by state, title, "Needs you" badge, working-mode badge, sub-line status word + cwd or pending-work text (`screens/Chat.tsx`).
- Rec: **hybrid** — keep unipi state glyph/words/mode badge (DESIGN §1.3: colour+icon+word); add ZCode relative time, +N −M and unread dot; marquee optional (ask). Effort M.
- Owner ruling:

**Z-13 Archive** (QA#1)
- ZCode: Archive from context menu or inline hover button with a confirm state; archived list reachable; Delete-all-archived (`docs/ui/task-list-context-menu.md`, `ui/DeleteAllArchivedTasksButton.tsx`).
- unipi: "Just hide from list" per device + "n hidden · Show" (`lib/chat/newChat.ts hideChat`).
- Rec: **take ZCode** — owner asked for archive + a place to reopen; store in session `custom` entry/host index so all devices agree. Effort M.
- Owner ruling:

**Z-14 Pin**
- ZCode: Pin/Unpin, separate "Pinned" group on top (`docs/ui/pinned-task-storage.md`).
- unipi: none.
- Rec: **take ZCode**. Effort S.
- Owner ruling:

**Z-15 Rename**
- ZCode: Rename dialog from context menu, Enter submits; manual title wins over auto title (`docs/ui/task-title-authority.md`).
- unipi: ⋯ → Rename… sheet, `/name` (`components/chat/SessionMenus.tsx RenameSheet`); utility auto-rename runs silently.
- Rec: **hybrid** — add Rename to the sidebar context menu; keep `/name`; manual name must lock out auto-rename. Effort S.
- Owner ruling:

**Z-16 Session context menu**
- ZCode: Pin, Rename, Archive, Mark as unread, Open in file manager, Copy path, Copy log path, Go to config, Delete (confirm) (`docs/ui/task-list-context-menu.md`).
- unipi: no row menu on the chat list; header ⋯ has New session, New chat in workspace, Resume…, Search, Tree, Files here, Git, Kanboard, Add task, Rename, Close chat (`SessionMenus.tsx ChatHeaderMenu`).
- Rec: **hybrid** — right-click/long-press row menu with Pin, Rename, Archive, Mark unread, Fork, Copy id/path, Reveal folder, Delete; header ⋯ keeps chat-scoped actions only. Effort M.
- Owner ruling:

**Z-17 Fork**
- ZCode: Fork action on the assistant turn actions; fork child sorts above parent while running (`ui/v4/ConversationRowView.tsx ConversationAssistantTextActions`).
- unipi: Fork from here on any user/prose row action bar and in the message menu; `/fork` (`ChatView.tsx doFork`).
- Rec: **hybrid** — keep "fork from any message" (pi supports it) but surface it in the turn-tail action bar (Z-27). Effort S.
- Owner ruling:

**Z-18 Tree / branches**
- ZCode: no tree UI (edit = new branch; fork = new task).
- unipi: Session tree sheet with filters, "Go to this point", "Start a new branch here", jump restores the message into the composer (`SessionMenus.tsx TreeSheet`, `ChatView.tsx doTreeGo`).
- Rec: **keep unipi** — pi-specific power feature; open it as a side-pane tab on desktop instead of a sheet (ask). Effort S.
- Owner ruling:

**Z-19 New session / resume flow**
- ZCode: Cmd+N new task in the last active workspace; draft composer with project picker; empty draft shows greeting + suggested prompts (`docs/ui/workspace-sidebar-navigation.md`, `docs/ui/project-local-directory-picker.md`).
- unipi: New chat sheet (PC → where (herdr/tmux) → folder → first message); "New session" (restart this pi), "New chat in this workspace", "Resume…" sheet, WelcomeState lists recent sessions (`components/chat/NewChatSheet.tsx`, `SessionMenus.tsx ResumeSheet/WelcomeState`).
- Rec: **take ZCode** — one "New chat" (+ per-project `+`) opening a draft composer with host+folder pickers; drop "New session"/"New chat here" duplication and the Resume sheet (the sidebar *is* resume; rule 2). Effort M.
- Owner ruling:

**Z-20 Export / share / delete**
- ZCode: conversation share link with permission picker and selection mode; delete task with confirm (`ui/ConversationShareMenu.tsx`, `docs/ui/confirm-dialog.md`).
- unipi: `/export`, `/share`, `/import`, `/clone` marked terminal-only (`lib/chat/commands.ts TERMINAL_ONLY`); message Share (OS share text); no delete.
- Rec: **ask** — recommend Export (HTML/JSONL via pi) + Delete (confirm) in the row menu; skip ZCode web share links (needs a service). See G-15. Effort M.
- Owner ruling:

## C. Chat timeline

**Z-21 Turn fold**
- ZCode: per work segment, everything before the latest assistant text folds into one "Worked for 1m 18s" line; forced open while running and on interrupted/failed/error; "Working for N s" live; "Stopped" without time when interrupted (`docs/ui/v4-chat-turn-render-units.md`).
- unipi: each run of thinking+tools between prose blocks folds into its own process group ("Read files, edited files · 1m 3s", live shimmer line); intermediate prose stays visible (`Transcript.tsx ProcessGroup`, `lib/chat/model.ts processSummary`).
- Rec: **take ZCode** (owner rule 4) keeping unipi's summary wording inside the fold ("Worked for 1m 3s · read files, edited 2"). Effort M.
- Owner ruling:

**Z-22 Live tail while running**
- ZCode: the running turn renders expanded as a live tail outside the virtualized history, then migrates in (`ui/v4/conversationTimelineLiveTail.ts`).
- unipi: a separate RunningRow ("Working… · cmd" + clock) below the list unless the last row is live (`ChatView.tsx RunningRow`, `lastRowLive`).
- Rec: **hybrid** — ZCode live tail; keep the unipi clock text in the "Working for" label so there's one live indicator. Effort M.
- Owner ruling:

**Z-23 Follow threshold & jump button**
- ZCode: detach at > 48 px, user intent beats layout, button only when detached *and* overflowing, gradient mask behind it, `scrollbar-gutter: stable` (`docs/ui/chat-scroll-bottom-pinning.md`).
- unipi: FOLLOW = 32 px, `.jump-btn` whenever not following (`ChatView.tsx`).
- Rec: **take ZCode** (part of D-8). Effort S.
- Owner ruling:

**Z-24 Unread marker on jump button**
- ZCode: none beyond the button.
- unipi: none. (v1 oino added an unread dot while busy.)
- Rec: **ask** — small dot on the jump button when new content arrived while detached. Effort S.
- Owner ruling:

**Z-25 Query navigator rail**
- ZCode: compact rail left of the timeline, one bar per user query (≥ 2 queries, wide screens), tooltip = query + answer excerpt, click jumps exactly (`docs/ui/conversation-turn-navigator.md`, `ui/v4/ConversationTurnNavigator.tsx`).
- unipi: none.
- Rec: **take ZCode** (desktop only). Effort M.
- Owner ruling:

**Z-26 Timestamps**
- ZCode: assistant turn actions end with the time: `HH:MM` today, `Yesterday HH:MM`, `M/D HH:MM`, `Y/M/D HH:MM` (`ui/v4/messageTimeLabel.ts`).
- unipi: no message times; turn end "Done in 1m 34s · $0.003" (`Transcript.tsx TurnEnd`).
- Rec: **hybrid** — keep "Done in · $cost" and add the ZCode time label to the same line. Effort S.
- Owner ruling:

**Z-27 Message actions placement** (QA#14)
- ZCode: actions only at the **tail of each turn**, bottom-left, hover-revealed (always visible on phone): Copy (whole turn text, glyph→check 1.2 s), like/dislike, Fork, hook details, time. User row: Copy + inline Edit (`ConversationRowView.tsx` 1478–1650).
- unipi: hover bar on **every** user and prose row: Copy, Quote, Start from here, Fork, Retry, More (Share); phone: tap a row toggles the bar; swipe right quotes (`ChatView.tsx actionBar`).
- Rec: **hybrid** — ZCode placement (turn tail, bottom-left) with unipi's set (Copy, Quote, Fork, Retry, More); drop like/dislike (no telemetry); per-message actions stay in right-click/long-press menu. Effort M.
- Owner ruling:

**Z-28 Edit a sent message**
- ZCode: inline edit of the user bubble (text + attachments), submit as new branch, optional "reset with files" (FileClock) that rewinds the workspace too (`docs/ui/user-message-plain-text.md`).
- unipi: "Start from here" = tree jump that puts the message back into the composer (`ChatView.tsx restoreIntoDraft`).
- Rec: **hybrid** — Edit icon on the user bubble that does unipi's tree-jump into the composer; "reset with files" ties into D-1 snapshots (ask). Effort M.
- Owner ruling:

**Z-29 User bubble fold**
- ZCode: max 120 px, bottom fade mask, centred expand/collapse button, 300 ms height transition, chips/attachments stay outside the fold (`docs/ui/v4-user-message-display.md`).
- unipi: clamps > 600 chars or > 12 lines, tap toggles, no button (`Transcript.tsx UserBubble data-clamped`).
- Rec: **take ZCode** — explicit button is clearer and works with tap-to-select. Effort S.
- Owner ruling:

**Z-30 Text-selection actions**
- ZCode: select text in the timeline → "Add to current task" (quote chip in composer) / "Ask in side chat" (`docs/ui/conversation-selection-side-chat.md`, `ui/v4/SelectionActionMenu.tsx`).
- unipi: Quote inserts the whole message as `> text`; swipe-to-quote on phone.
- Rec: **take ZCode** for desktop (selection → quote chip); keep swipe-to-quote on phone. Effort M.
- Owner ruling:

**Z-31 Side chat (BTW)**
- ZCode: `/side` and `/btw` open a *multi-turn* side chat as a side-pane tab (several allowed); selection can seed it (`docs/ui/conversation-selection-side-chat.md`).
- unipi: BtwSheet: one-question pages, streamed answer, history, "Ask in main chat" (`components/chat/BtwSheet.tsx`); oino btw ext is single-answer `editorReplace` panel.
- Rec: **hybrid** — keep btw semantics (read-only, never touches main session) but open it as a side-pane tab on desktop, sheet on phone. Effort M.
- Owner ruling:

**Z-32 Assistant preview cards**
- ZCode: media/file paths in the final answer become preview cards (≤ 10) that open in the preview pane (`docs/ui/assistant-media-preview.md`, `docs/ui/assistant-preview-cards.md`).
- unipi: verified file refs become tappable links with a menu (`lib/chat/fileRefs.ts`, `FileRefText.tsx`); tool attachments as chips; mermaid viewer.
- Rec: **keep unipi** file refs; ask whether image/video refs should get inline cards. Effort M.
- Owner ruling:

**Z-33 Display settings for the stream**
- ZCode: Settings toggles "show all reasoning" (first reasoning per turn always shown) and "show Todo cards" (`docs/ui/message-stream-display-settings.md`).
- unipi: none.
- Rec: **ask** — low value if Z-21 folds everything; recommend skip. Effort S.
- Owner ruling:

**Z-34 Streaming text animation**
- ZCode: new streamed text fades in (900 ms per chunk), completed markdown rendered static (`ui/styles.css zcode-stream-text-in`, `docs/ui/chat-virtual-scroll.md`).
- unipi: blocks re-render as they stream (`renderBlocks(text, live)`).
- Rec: **take ZCode**'s "static once complete" render rule (perf); fade-in optional (ask). Effort S.
- Owner ruling:

**Z-35 Compaction marker**
- ZCode: manual `/compact` shows no user bubble, only a compact timeline marker (`docs/ui/v4-chat-turn-render-units.md`).
- unipi: "Context compacted · Nk tokens before" divider, expandable summary (`Transcript.tsx Compaction`) + compactor card.
- Rec: **keep unipi** divider; hide the `/compact` echo bubble like ZCode. Effort S.
- Owner ruling:

**Z-36 Harness / command echo rows**
- ZCode: control-only turns (e.g. model change) render as light boundary rows; `/goal` renders as a goal tag in the bubble; cron-sent prompts show "Sent by scheduled task" above the bubble (`docs/ui/scheduled-tasks-main-view.md`).
- unipi: harness messages as cards with source/title; `/command` echo = own bubble with command chip + injected-text chip (`Transcript.tsx UserBubble harness/command`, `Cards.tsx`).
- Rec: **keep unipi** and add the cron source label (Z-112). Effort S.
- Owner ruling:

## D. Tool rows, thinking, diffs

**Z-37 Tool row anatomy**
- ZCode: status + title + summary, static kind icon, chevron only on hover when done, shimmer while running, progress indicator bar (`docs/ui/tool-inline-diff.md`, `ui/ToolCallBlocks/ToolSummaryRow.tsx`).
- unipi: icon by category, tool name (shimmer when busy), subject (tappable verified path), +N −M, error first line in red (`Transcript.tsx ToolRow`).
- Rec: **keep unipi** row (QA#13 preferred it); take ZCode hover-only chevron. Effort S.
- Owner ruling:

**Z-38 Tool grouping**
- ZCode: consecutive non-read-only shell calls grouped into one "Execute" parent; explore (reads/searches) and changes groups (`docs/ui/tool-display-rendering.md`, `renderers/execute-group.tsx`, `explore.tsx`, `changes-group.tsx`).
- unipi: all tools listed flat inside the process fold.
- Rec: **take ZCode** grouping inside the turn fold. Effort M.
- Owner ruling:

**Z-39 Expanded tool views**
- ZCode: per-tool renderers (execute, read, search, web-fetch, todo, mcp, agent, ask-question, skill …); write `.md` previews as markdown; generic In/Out fallback (`ui/ToolCallBlocks/renderers/*`, `docs/ui/write-tool-markdown-preview.md`).
- unipi: run → `$ cmd` + output tail; edit → diff; write → all-additions diff; read/search/web/memory → output head; fallback In/Out JSON (`Transcript.tsx ToolBody`).
- Rec: **hybrid** — keep unipi bodies; add markdown preview for `.md` writes, todo and mcp renderers; tool renderers from extensions come through UI v2. Effort M.
- Owner ruling:

**Z-40 Inline diff vs full diff**
- ZCode: lightweight highlighted unified diff inline; "View diff" opens the full Pierre diff viewer in the side pane (`docs/ui/diff-viewer.md`).
- unipi: `renderDiffHtml` inline; Review opens a full-screen read-only viewer; Open in editor jumps to the line (`components/chat/ReviewViewer.tsx`).
- Rec: **hybrid** — unipi inline diff; Review opens a side-pane diff tab on desktop (Z-5). Effort S.
- Owner ruling:

**Z-41 Thinking display**
- ZCode: Reasoning collapsed by default even while streaming; label "Thinking…" (shimmer) → "Thought for 12s"; hidden when empty (`ConversationRowView.tsx ReasoningRowView`).
- unipi: ThinkingRow inside the fold, preview = latest finished paragraph, "Thinking" label (`Transcript.tsx ThinkingRow`).
- Rec: **hybrid** — ZCode labels with duration + unipi's one-line preview. Effort S.
- Owner ruling:

**Z-42 Subagent / sidekick rows**
- ZCode: subagent row opens the child session in a side-pane tab (drilldown) with its own transcript (`ui/v4/SubagentDrilldown.tsx`, `ui/app-shell/SubagentSessionSidePane.tsx`).
- unipi: completion cards; Running tab detail (report text, stop/re-run/background); Progress sheet "Open session" (`Cards.tsx`, `ControlCentre.tsx WorkDetail`).
- Rec: **take ZCode** — open subagent transcripts as side-pane tabs; one place instead of two (rule 2). Effort M.
- Owner ruling:

**Z-43 Todo / plan rendering**
- ZCode: plan & todos live in the status panel (Z-95); inline todo cards optional (`docs/ui/plan-mode-panel.md`).
- unipi: no todo concept beyond harness cards; plan review is a `custom` dialog (unsupported, G-2).
- Rec: **ask** together with Z-95. Effort M.
- Owner ruling:

**Z-44 Images in tool output**
- ZCode: image grids (node-repl, CUA screenshots), media cards.
- unipi: `AttachmentChipRow` + previewer (`ImageChip.tsx`).
- Rec: **keep unipi**. Effort —.
- Owner ruling:

**Z-45 Code blocks**
- ZCode: Streamdown; sticky header with language + copy (`DESIGN.md` Chat).
- unipi: sticky language + Copy header, Copy→Copied 1.2 s, wrap toggle (`MdBlock.tsx`).
- Rec: **keep unipi**. Effort —.
- Owner ruling:

## E. Dialogs, errors, retry

**Z-46 Where pending dialogs render**
- ZCode: permission / question / elicitation panels dock **above** the composer in the same bottom dock; composer stays usable; mobile can collapse the question panel (`ui/v4/SessionPane.tsx` bottom dock, `docs/ui/ask-user-question-header-title.md`).
- unipi: dialogs **replace** the composer slot (`ChatView.tsx DialogStack`).
- Rec: **take ZCode** — keeps typing/steering possible while a question waits; add collapse on phone. Effort M.
- Owner ruling:

**Z-47 Permission prompt**
- ZCode: inline permission block with origin badge and workflow holes (`ui/PermissionDialog.tsx`, `ui/WorkflowPermissionBlock.tsx`).
- unipi: no `approval` kind; oino workflow sends `approval` (Allow once / Always / Deny / Deny with note) (G-2).
- Rec: **take ZCode** layout in unipi style. Effort M.
- Owner ruling:

**Z-48 Error rows**
- ZCode: error banner with trace id and a details dialog; provider/business error mapping (`ui/ChatErrorBanner.tsx`).
- unipi: red error row; Retry button under the last error when `/unipi:continue` exists (`ChatView.tsx canRetry`).
- Rec: **hybrid** — unipi row + a "Details" disclosure with the raw error. Effort S.
- Owner ruling:

**Z-49 Retry**
- ZCode: Retry is an assistant turn action (`retryTurn`), always available on finished turns (`docs/ui/chat-user-message-retry.md`).
- unipi: Retry only after an error (`/unipi:continue`).
- Rec: **ask** — recommend Retry in the turn-tail bar for the last turn (re-run from its user message via tree jump). Effort S.
- Owner ruling:

**Z-50 Connection / subscription errors**
- ZCode: session subscription error panel (Reconnect + feedback); pending command recovery banner (Resend / Dismiss a discarded command) (`ui/v4/SessionSubscriptionErrorPanel.tsx`, `PendingCommandRecoveryBanner.tsx`).
- unipi: chat banner "Lost the connection · Retry"; outbox bubbles "Waiting for connection… ✕" (`ChatView.tsx chat-banner`, `Transcript.tsx`).
- Rec: **keep unipi** (outbox already covers resend). Effort —.
- Owner ruling:

**Z-51 Session lock taken elsewhere**
- ZCode: n/a.
- unipi: n/a. DIRECTION §3.1.5: opening a session running elsewhere closes the other runner.
- Rec: **ask** — show "Opened here; the TUI copy was closed" notice and a read-only banner in the losing client. Effort S.
- Owner ruling:

## F. Queue, steering, interruption

**Z-52 Default while running**
- ZCode: global setting follow-up mode = queue or guide (steer); Enter does the default, Cmd/Ctrl+Enter the opposite; placeholder "Keep typing to queue follow-up changes" (`ui/v4/composer/followupModeSettings.ts`, `docs/ui/chat-composer-placeholder.md`).
- unipi: Enter = this chat's last Steer/Queue choice (Steer first); Ctrl/⌘+Enter opens an inline Steer/Queue choice (S/Q); chevron menu: Steer / Send now / After it ends (`Composer.tsx`, `lib/chat/sendChoice.ts`).
- Rec: **hybrid** — ZCode's global default setting + modifier for the opposite (simpler), keep "Send now (stops run)" in the chevron menu. Effort S.
- Owner ruling:

**Z-53 Queue panel**
- ZCode: queue above composer; drag to reorder, Edit = recall into composer (refused if a draft exists), Remove, Send now; held queue after interrupt with "Resume" and auto-drain (`docs/ui/chat-message-queue.md`, `ui/v4/ConversationQueuePanel.tsx`).
- unipi: queue strip with mode label, inline edit, remove, ↑ move, ⋯ Steer now / Send now / Move down (`Composer.tsx QueueRow`).
- Rec: **hybrid** — ZCode recall-to-composer edit + draft guard and held-queue Resume; drag reorder on desktop, keep arrows on phone. Effort M.
- Owner ruling:

**Z-54 Steered messages in the timeline**
- ZCode: an accepted guide opens a new work segment inside the same turn, own fold and own navigator entry (`docs/ui/v4-chat-turn-render-units.md`).
- unipi: steer shows as a normal user bubble when pi delivers it.
- Rec: **take ZCode** (falls out of Z-21). Effort S.
- Owner ruling:

**Z-55 Stop**
- ZCode: Esc stops unless a dialog/popup consumed it; Send button becomes Stop (`ui/v4/composer/escapeStop.ts`).
- unipi: Esc in composer while running stops; Send ↔ Stop (`lib/desktop.ts composerKey`).
- Rec: **take ZCode** guard (Esc from any focus in the chat, ignoring consumed events). Effort S.
- Owner ruling:

**Z-56 Interrupted turn display**
- ZCode: interrupted segment forced open, label "Stopped" (no duration) (`docs/ui/v4-chat-turn-render-units.md`).
- unipi: "Stopped" tag in prose + "Stopped" turn end.
- Rec: **take ZCode** (one "Stopped" label at the fold). Effort S.
- Owner ruling:

## G. Composer

**Z-57 Placeholder by state**
- ZCode: empty session → capabilities hint (`@ files, / commands, $ skills, # conversations`); has history → "Ask for follow-up changes"; running → "Keep typing to queue…"; phone short variant (`docs/ui/chat-composer-placeholder.md`).
- unipi: "Message pi, / commands, @ files" / "Steer pi… (Ctrl+Enter: steer or queue)" (`Composer.tsx` placeholder).
- Rec: **take ZCode** three-state copy (say "Oino"/"pi" per owner). Effort S.
- Owner ruling:

**Z-58 Prompt history ↑/↓**
- ZCode: ↑/↓ on empty composer walks sent prompts, per workspace, 30 max, consecutive duplicates skipped (`docs/ui/composer-prompt-history.md`).
- unipi: none in chat (terminal composer has per-host history); chat has undo/redo per chat (`lib/chat/undo.ts`).
- Rec: **take ZCode**. Effort S.
- Owner ruling:

**Z-59 Draft persistence**
- ZCode: per-session drafts incl. attachments in memory; text drafts persisted across restarts (text only, removed when emptied) (`docs/ui/task-scoped-composer-drafts.md`, `composer-text-draft-persistence.md`).
- unipi: drafts in an in-memory signal per chat key; lost on restart (`ChatView.tsx drafts`).
- Rec: **take ZCode** (with D-6 for images). Effort S.
- Owner ruling:

**Z-60 Attachment states**
- ZCode: pre-upload on attach, states queued→uploading→ready with progress %, auto-retry once, manual retry, send disabled until ready, max 8 (`docs/ui/chat-file-attachments.md`).
- unipi: images shrunk and sent inline (or blob upload); files uploaded to the host, chip `[file: name]@path` with spinner; 1 GB cap (`lib/chat/attachments.ts`).
- Rec: **hybrid** — keep unipi paths; add progress %, failed+retry state and the send gate. Effort S.
- Owner ruling:

**Z-61 Long paste → text attachment**
- ZCode: plain-text paste ≥ 5120 chars becomes a "Pasted text · 128 lines" chip saved to a temp file (`docs/ui/chat-long-paste-text-attachment.md`).
- unipi: long text stays inline.
- Rec: **ask** — useful for logs; pi parity sends text inline. Effort S.
- Owner ruling:

**Z-62 Mention triggers**
- ZCode: `@` plugins/files+folders/conversations, `/` commands+skills+subagents grouped, `$` skills, `#` conversations; chips with token colours (`docs/ui/mention-panel-layout.md`, `docs/ui/slash-command-skill-suggestions.md`).
- unipi: `/` commands/prompt templates/skills with source labels; `@` files via pi's finder; chips (`lib/chat/commands.ts`, `ChipEditor.tsx`).
- Rec: **hybrid** — keep `/` and `@`; ask for `#` session refs and `$` skills alias. Effort M.
- Owner ruling:

**Z-63 "+" menu**
- ZCode: `+` popover (composer width): Attach, Goal/Workflow starters, Plugins, Files, Conversations (`docs/ui/chat-composer-action-menu.md`).
- unipi: `+` menu: Photo, Camera, File, Side question, Board task…, New session, Resume… (`SessionMenus.tsx PlusMenu`).
- Rec: **hybrid** — keep attach + side question + board task; move New session/Resume out (Z-19); add mode starters (Goal/Ralph/Graph) from the mode chip (Z-74). Effort S.
- Owner ruling:

**Z-64 Composer toolbar contents**
- ZCode: mode control, plan toggle, model, thought level, context, background-works count, send/stop; seven-step compression on narrow widths (`docs/ui/chat-model-select-menu.md`, `ui/v4/composer/V4ComposerToolbar.tsx`).
- unipi: `+`, model chip (ring + name + thinking text), send chevron, Send/Stop (`Composer.tsx`).
- Rec: **hybrid** — add permission/plan mode control (G-6) and separate thinking control (Z-71); keep one model chip; adopt the compression order. Effort M.
- Owner ruling:

**Z-65 Dictation**
- ZCode: none.
- unipi: planned in DESIGN §3.5 (Android SpeechRecognizer), not built.
- Rec: **ask** (phone). Effort M.
- Owner ruling:

**Z-66 Composer spacing** (QA#16)
- ZCode: composer sticky inside the scroll viewport with a gradient fade above (`docs/ui/chat-virtual-scroll.md`).
- unipi: composer in `.chat-bottom` below the scroller.
- Rec: **take ZCode** (also fixes "last message too close to input"). Effort S.
- Owner ruling:

**Z-67 Web element / screenshot chips**
- ZCode: element-picker chip and screenshot attachment in composer (`docs/ui/web-element-context.md`).
- unipi: none. (Decided D-4.)
- Rec: decided — listed for completeness. Effort M.
- Owner ruling: (decided D-4)

**Z-68 Draft suggested prompts**
- ZCode: under an empty draft, rotating suggested prompts that prefill the composer (`docs/ui/conversation-draft-suggested-prompts.md`).
- unipi: WelcomeState lists recent sessions (`SessionMenus.tsx WelcomeState`).
- Rec: **ask** — recommend skip (sidebar covers recents); maybe Unicrab lore line instead. Effort S.
- Owner ruling:

## H. Model, thinking, context

**Z-69 Model picker form**
- ZCode: dropdown menu from the composer; provider level → submenu when several providers; Vision badge; checked current; "Manage models"; Ctrl+M opens; rolling label animation; spinner after the name while switching (`docs/ui/chat-model-select-menu.md`, `ui/ModelConfigSelect.tsx`).
- unipi: centred sheet/dialog: "Now …" line, Single|Fusion tabs, thinking levels, search over > 6 models, list with provider/id sub-line, "current" tag, sticky Cancel / "Switch model" footer (`ControlCentre.tsx ModelPickerSheet`).
- Rec: **hybrid** — keep unipi sheet (Fusion needs room) but: wider dialog, search shared by Single and Fusion columns (QA#4), Now label in sentence case (QA#3a), compact right-aligned buttons (QA#3c), Vision/context badges, Ctrl+M. Effort M.
- Owner ruling:

**Z-70 Model picker entry points**
- ZCode: composer only (+ Ctrl+M).
- unipi: composer chip, chat title button, tray "Model" row, `/model`, `/thinking` (`ChatView.tsx` header, `FooterTray`).
- Rec: **take ZCode** — composer chip + Ctrl+M + `/model`; drop title and tray entries (rule 2). Effort S.
- Owner ruling:

**Z-71 Thinking control**
- ZCode: separate composer control: brain icon + level label + ladder bar; click = list; Ctrl+T cycles without opening (`docs/ui/chat-thought-level-control.md`).
- unipi: level radio inside the model sheet; level text on the model chip.
- Rec: **hybrid** — separate brain chip (lucide, QA#10) with level text, Ctrl+T cycle; no ladder bar unless asked. Effort S.
- Owner ruling:

**Z-72 Model switch feedback**
- ZCode: toast "Switched from X to Y" (`docs/ui/model-switch-toast.md`).
- unipi: sheet shows pending → closes; transcript `model_change` card (`Cards.tsx`).
- Rec: **take ZCode** toast; keep the card. Effort S.
- Owner ruling:

**Z-73 Context indicator**
- ZCode: Context trigger with hover card: used/total tokens, breakdown bars, cache-hit rate (≥ 78 %) (`docs/ui/chat-input-context.md`).
- unipi: ring + "1%" in model chip; tray Context row; Session modal Context tab (`Composer.tsx ContextRing`, D-7).
- Rec: **hybrid** — ring stays (fix 0 % visibility, QA#8); hover/click opens Session modal → Context (no second breakdown surface). Effort S.
- Owner ruling:

**Z-74 Working mode (goal/ralph/swarm/graph)**
- ZCode: mode control in composer (Ctrl+Shift+M cycles), goal banner, goal in status panel with pause/resume (`ui/v4/composer/V4ComposerModeControls.tsx`, `ConversationGoalBanner.tsx`).
- unipi: header ModeChip with live count + popover (what it does, state, Show progress, switch mode); ⋯ → Working mode; Progress sheet (`components/chat/ModeChip.tsx`, `ProgressSheet.tsx`).
- Rec: **ask** — with D-2 the LH status moves into the git pane; keep the mode chip as the *switch* (header or composer?) and the Progress view as the pane section. Effort M.
- Owner ruling:

**Z-75 Session strip / tray**
- ZCode: no header strip; cost not shown in chat.
- unipi: header strip "model · ctx · cost" + "N running" badge, tap expands a tray (Model, Context, Cost, Running) (`ControlCentre.tsx FooterStrip/FooterTray`).
- Rec: **ask** — redundant with composer chip + ring + Session modal (rule 2); recommend drop the tray and keep a one-line cost in the turn end. Effort S.
- Owner ruling:

**Z-76 Fusion pairing UI**
- ZCode: n/a (single model).
- unipi: Fusion tab with Lead/Sidekick lists and effort pills (`ModelPickerSheet`).
- Rec: **keep unipi** with QA#4 fixes (search above both columns, wider). Effort S.
- Owner ruling:

## I. Command palette & search

**Z-77 Command center scope**
- ZCode: Cmd+K (and Cmd+Shift+P): tabs All / Actions / Tasks / Files; prefixes `>` `#` `@`; empty state = Recent changes (3), Recent tasks (3), grouped commands (Suggested, Chat, Navigation, Panels, Configure, App); ≤ 3 per group + "show more" (`docs/ui/command-center-cmd-k.md`, `docs/ui/quickpick-command-palette.md`).
- unipi: Ctrl+K palette of registered commands only; ranked; group headers on empty query (`components/shell/CommandPalette.tsx`).
- Rec: **take ZCode** scopes (sessions + files + commands) with unipi's command registry. Effort M.
- Owner ruling:

**Z-78 Search history**
- ZCode: per-workspace history chips (20, dedup) under the input (`docs/ui/quickpick-command-palette.md`).
- unipi: none.
- Rec: **take ZCode**. Effort S.
- Owner ruling:

**Z-79 Find in chat**
- ZCode: Cmd+F find bar centred over the conversation; highlights all matches in the virtual timeline; ↑/↓/Enter/Shift+Enter wrap; scope switch Conversation ↔ File changes (`docs/ui/chat-find-placement.md`, `docs/ui/task-find-dialog.md`).
- unipi: Ctrl+F opens a Search sheet: whole-session fuzzy search via pi, results list, jump + flash (`components/chat/SearchSheet.tsx`).
- Rec: **hybrid** — ZCode in-place find bar for loaded text; "Search whole session" inside it falls back to unipi's host search. One surface (rule 2). Effort M.
- Owner ruling:

**Z-80 Extension commands in the palette**
- ZCode: commands from a static registry + plugins.
- unipi: extension commands only via composer `/`.
- Rec: **ask** — list session `/` commands in the palette's Actions scope (running inserts `/cmd` into the composer). Effort S.
- Owner ruling:

**Z-81 Quick open file**
- ZCode: Cmd+P routes into the command center Files scope.
- unipi: Files screen search.
- Rec: **take ZCode**. Effort S.
- Owner ruling:

## J. Notifications, needs-you, toasts

**Z-82 Unread state**
- ZCode: persisted blue unread dot, set when a background session finishes unseen or via "Mark as unread", cleared on open; dock badge (`docs/ui/task-unread-state.md`).
- unipi: "Needs you" badge + status words; Needs-you drawer marks read (`lib/needsYou.ts`).
- Rec: **hybrid** — add persisted unread dot + Mark as unread; drawer stays. Effort S.
- Owner ruling:

**Z-83 Needs-you surface**
- ZCode: none central; badges on task rows + OS notifications.
- unipi: animated Unicrab (floating crab on phone, header crab on desktop, optional mascot window, Android overlay) opening a drawer grouped Waiting / Finished / Other (`components/NeedsYouBell.tsx`, `FloatingMascot.tsx`, DESIGN §1.5.1).
- Rec: **keep unipi** (brand). Effort —.
- Owner ruling:

**Z-84 OS notifications**
- ZCode: notifications from projections on completion/error/permission, suppressed when that task is focused; cold snapshots never replay old ones (`docs/ui/task-notification-routing.md`).
- unipi: desktop polls every 20 s, notifies for new needs-you/finished while unfocused, first look seeds, max 3 (`components/shell/DesktopNative.tsx`).
- Rec: **take ZCode** event-driven routing (host pushes) with unipi's seed + max-3 rules; drop polling. Effort M.
- Owner ruling:

**Z-85 Toasts**
- ZCode: top-centre toasts (sonner) for confirmations (model switch, feedback).
- unipi: one chat toast string at a time; Board toasts with one action (Undo/Open) (`ChatView.tsx chat-toast`, `components/board/BoardToasts.tsx`).
- Rec: **hybrid** — one app-wide toast stack in the BoardToasts style (action, exit fade, hover pause, v1 F1 timings). Effort S.
- Owner ruling:

**Z-86 Background-session completion while focused elsewhere**
- ZCode: completion seen in foreground isn't marked unread; minimizing right after marks it (`docs/ui/task-unread-state.md`).
- unipi: n/a.
- Rec: **take ZCode** rule. Effort S.
- Owner ruling:

## K. Empty, loading, error states

**Z-87 New chat empty state**
- ZCode: time-of-day greeting, decorative logo behind the title, suggested prompts (`docs/ui/chat-empty-time-greeting.md`, `chat-empty-state-logo.md`).
- unipi: Sparkles + "New session" + recent sessions (`SessionMenus.tsx WelcomeState`); EmptyState with crab elsewhere.
- Rec: **hybrid** — Unicrab (not ZCode logo) + time greeting + short capability hint; no recents (Z-19). Effort S.
- Owner ruling:

**Z-88 Loading**
- ZCode: task restore placeholder; image shimmer skeleton; tool progress bar (`ui/styles.css markdown-image-loading-shimmer`).
- unipi: 3-line skeleton while connecting; HistoryEdge "load older"; spinners (`ChatView.tsx chat-skeleton`).
- Rec: **keep unipi** + image skeleton. Effort S.
- Owner ruling:

**Z-89 Crash containment**
- ZCode: root React error boundary with Retry / Reload app; optional per-region boundaries (`docs/ui/react-error-boundary.md`).
- unipi: none.
- Rec: **take ZCode** — Solid `ErrorBoundary` at root + per pane (chat, side pane tab, extension panel). Effort S.
- Owner ruling:

**Z-90 Offline / no host / unavailable**
- ZCode: remote availability banner.
- unipi: `NoHostState`, `OfflineState` with retry + hint, `<Requires cap>` notices (`components/EmptyState.tsx`, `Requires.tsx`).
- Rec: **keep unipi**. Effort —.
- Owner ruling:

## L. Motion

**Z-91 Exit animations**
- ZCode: collapses animate out (`zcode-collapsible-up` + fade, 300 ms); toasts animate out (`ui/styles.css`).
- unipi: enter animations (popover-in, pane-in, send-choice-in), removals are instant; sheets spring 380 ms (`style.css`).
- Rec: **take ZCode** exits (≤ 200 ms, DESIGN §1.4). Effort S.
- Owner ruling:

**Z-92 Rolling labels**
- ZCode: model / mode / thinking labels roll vertically on change (`docs/ui/chat-model-select-menu.md`).
- unipi: instant swap.
- Rec: **ask** — nice, cheap; recommend take. Effort S.
- Owner ruling:

**Z-93 Micro-feedback**
- ZCode: copy glyph→check 1.2 s; reaction burst; draft prompt waterfall.
- unipi: code Copy→Copied 1.2 s; row flash after a jump; mode chip pulse; shimmer.
- Rec: **keep unipi** + copy→check on row actions. Effort S.
- Owner ruling:

## M. Keyboard shortcuts

**Z-94 Shortcut map and rebinding**
- ZCode (`packages/shared/src/shortcutCommands.ts`, rebindable in Settings, platform primary modifier): Cmd+K/Cmd+Shift+P palette · Cmd+, settings · Cmd+F find · Cmd+B sidebar · Cmd+J terminal · Cmd+Alt+B side pane · Cmd+Shift+F side-pane full view · Cmd+Shift+[ / ] prev/next conversation · Cmd+[ / ] back/forward · Ctrl+M model menu · Ctrl+Shift+M cycle mode · Ctrl+T cycle thinking · Cmd+N new task · Cmd+O open workspace · Cmd+W close context · Cmd+= / - / 0 zoom · Cmd+Shift+L theme.
- unipi (`lib/desktop.ts shortcuts()`, fixed): Ctrl+K palette · Ctrl+1…6 screens · Ctrl+/ shortcuts list · Esc back · Enter/Shift+Enter/Ctrl+Enter send/newline/steer-or-queue · Ctrl+L focus composer · Ctrl+Shift+B browser · Alt+↑/↓ prev/next chat · Ctrl+F chat search · terminal Ctrl+Shift+C/V, Ctrl+=/-/0, Ctrl+Shift+K · Files ↑↓/Enter/Backspace/F2/Delete · Editor Ctrl+S/F · Board C / J K B L R X ? · Git Space/Ctrl+Enter.
- Rec: **hybrid** — take ZCode's map for shell-level actions (B, J, Alt+B, N, F, M, T, Shift+[ ]) and rebinding; keep unipi screen/board/files keys and Ctrl+/ list; register extension shortcuts (Alt+M permission, Alt+P plan, Shift+↓ tray, Ctrl+B subagents to background → **conflicts with Cmd+B sidebar**, needs a ruling) (G-7). Effort M.
- Owner ruling:

## N. Git pane & status panel

**Z-95 Status panel (top-right of conversation)**
- ZCode: one aside that collapses to a capsule: sections Git tools (changes +/−, branch switcher, Commit/Push/Create branch), Goal (elapsed, pause/resume, iterations), Plans, Progress (todos, windowed), Terminals (background bash), Agents (subagents); inline-offsets chat by 168 px only when the container ≥ 1280 px, else mini capsule (`docs/ui/chat-summary-panel.md`, `ui/v4/ConversationStatusPanel.tsx`).
- unipi: ModeChip + Progress sheet (LH), Session modal Running tab (bg tasks, subagents, sidekick), Git via ⋯.
- Rec: **ask (redundancy)** — D-2 puts LH status in the git pane with a capsule. Does that pane also take ZCode's todos, background terminals and agents? If yes, the Session modal Running tab (D-7) duplicates it → pick one. Recommend: pane = Git + LH + Running; modal keeps Context + Info and links to the pane. Also QA#12: overlay vs push — ZCode only pushes at ≥ 1280 px. Effort L.
- Owner ruling:

**Z-96 Git review sources**
- ZCode: Git pane source select: Unstaged / Staged / Branch (vs base) / Last turn; virtualized file rows, lazy per-file diff, file-change find, reveal in tree, open in editor (`ui/GitPane.tsx`, `packages/shared/src/git.ts GitChangeSourceId`).
- unipi: GitView Changes tab grouped Conflicts / Staged / Changes / Untracked + History tab (`components/git/GitView.tsx`).
- Rec: **hybrid** — ZCode sources incl. Last turn (D-2) + unipi grouping and History tab. Effort M.
- Owner ruling:

**Z-97 Git write actions**
- ZCode: Commit / Push / Create branch from the status panel (`ui/GitActionMenu.tsx`); branch switcher in panel/composer area; no stage-per-file UI in the pane.
- unipi: full client: stage/unstage per file and all, discard/delete with confirm, commit composer with Amend, pull (ff/rebase), push, fetch, stash/pop, branch sheet with create/track (`GitView.tsx`, DESIGN §3.8.2).
- Rec: **keep unipi** actions inside the git pane (ZCode is thinner). Effort M.
- Owner ruling:

**Z-98 Git auto-refresh**
- ZCode: file watcher on git metadata (and worktree on macOS/Windows), 1 min debounce; no polling (`docs/ui/git-auto-refresh.md`).
- unipi: refresh on open/after ops + manual Refresh.
- Rec: **take ZCode** (host-side watcher). Effort S.
- Owner ruling:

**Z-99 Git graph**
- ZCode: commit graph pane/dialog (`ui/git-graph/*`).
- unipi: History list with ref pills.
- Rec: **ask** — recommend skip for v2. Effort M.
- Owner ruling:

**Z-100 Git entry points**
- ZCode: side-pane Review tab, status panel, Cmd+Alt+B.
- unipi: Files toolbar branch chip, Files ⋯ → Git…, chat ⋯ → Git, palette Git commands.
- Rec: **hybrid** — chat: capsule/side pane; Files: branch chip opens the same pane (no second git UI). Effort S.
- Owner ruling:

## O. Files, editor, LSP, preview

**Z-101 File tree**
- ZCode: tree in a side-pane Files tab (preview left, tree right, resizable), lazy readdir, virtualized, compact folders, dotfiles shown, fuzzy name search, git-changed filter, watchers on expanded dirs (`docs/ui/workspace-file-tree.md`).
- unipi: breadcrumb list with git badges, gitignore dim, search name/content, recent files, changed filter, hidden toggle, row menu (Rename/Delete/Copy path/Mention in chat), upload by drop (`screens/Files.tsx`).
- Rec: **hybrid** — D-10 tree with ZCode behaviours + unipi row menu, Mention in chat, upload, content search. Ask: Files as a sidebar screen, a side-pane tab, or both (rule 2 says one). Effort L.
- Owner ruling:

**Z-102 Editor**
- ZCode: read-only code viewer (Shiki, sticky line numbers, wrap toggle, 256 KB cap) + "open in external editor"; no in-app editing (`docs/ui/preview-pane.md`).
- unipi: CodeMirror 6 editor with save (baseHash conflict sheet), find, vim option, phone key bar (`screens/Editor.tsx`).
- Rec: **keep unipi** editor + LSP (D-10); add "Open in external editor" split button like ZCode header (`docs/ui/workspace-open-actions.md`). Effort M.
- Owner ruling:

**Z-103 Preview pane (file types)**
- ZCode: side-pane preview of text, patch, image (checkerboard, @2x aware), media, PDF, Office (xlsx/docx), PPTX, markdown/SVG preview↔source (`docs/ui/preview-pane.md`).
- unipi: full-screen Previewer/MediaPager for images, PDF, media; mermaid viewer (`components/media/*`).
- Rec: **hybrid** — Previewer becomes a side-pane tab on desktop; ask for Office/PPTX (recommend skip). Effort M.
- Owner ruling:

**Z-104 "Preview" screen naming / overlap**
- ZCode: "Preview" = file preview; web pages open in the browser tab.
- unipi: Preview screen = PC ports → port-forwarded web viewer, plus Browser and Desktop modes (`screens/Preview.tsx`).
- Rec: **ask (redundancy)** — the forwarded-port viewer and the side browser both show web pages. Recommend: ports list becomes a launcher inside the Browser tab ("Open localhost:5173"); phone keeps the forward viewer (D-13). Effort M.
- Owner ruling:

## P. Terminal

**Z-105 Terminal tab behaviour**
- ZCode: tab names = cwd leaf, no "1", then "name 2…", lowest free number reused; closing the only tab hides the panel but keeps the session; PTY exit closes its tab; last exit closes the panel; focus the visible xterm on open/switch; resize throttled while dragging (`docs/ui/terminal-sessions.md`).
- unipi: live connections as tabs over the terminal pane, ✕ detaches (herdr/tmux era) (`screens/Terminal.tsx`).
- Rec: **take ZCode** (D-3 details). Effort M.
- Owner ruling:

**Z-106 Terminal links**
- ZCode: http(s) links in xterm open in the side browser (`docs/ui/terminal-sessions.md`).
- unipi: none.
- Rec: **take ZCode**. Effort S.
- Owner ruling:

**Z-107 Phone terminal input**
- ZCode: no phone terminal UX (web remote only).
- unipi: Moshi-grade toolbar capsule (modifiers one-shot/lock, arrow drag pad, shortcuts, history), gestures (tap = click, drag scroll, pinch font), composer (`components/TermToolbar.tsx`, DESIGN §3.3).
- Rec: **keep unipi** on the phone for host PTYs. Effort M (re-point to PTY).
- Owner ruling:

## Q. Browser

**Z-108 Browser toolbar extras**
- ZCode: responsive viewport (width/height/zoom, fit), site permission prompts/settings, DevTools, load-error and guest-failed pages with Retry, open externally (`ui/browser-use/BrowserViewportToolbar.tsx`, `ui/EmbeddedBrowserPaneParts.tsx`).
- unipi: profile picker, Stealth / Not stealthy badge, back/forward/reload, keyboard button, open tabs list (`screens/BrowserView.tsx`).
- Rec: **hybrid** — keep profile + stealth badge; take load-error page, open externally, responsive viewport (ask); skip DevTools. Effort M.
- Owner ruling:

**Z-109 Agent/user handoff**
- ZCode: browser-use operation-active indicator.
- unipi: ember strip "Agent is using the browser · Take over" / cyan "You have control · Give back" (`BrowserView.tsx`, DIRECTION §6 `gate.rs`).
- Rec: **keep unipi**. Effort —.
- Owner ruling:

**Z-110 Browser on phone**
- ZCode: not supported on web remote (`docs/ui/web-embedded-browser-capability.md`).
- unipi: full-height sheet over the chat with touch mapping (`BrowserSide.tsx BrowserSheet`).
- Rec: **keep unipi**. Effort —.
- Owner ruling:

## R. Board

**Z-111 Board scope** (QA#20)
- ZCode: no kanban.
- unipi: full kanboard parity (lanes, list, overview dashboard, filters, DnD, keys, sheets) (`screens/Board.tsx`, DESIGN §3.9); oino v1 GPUI showed unexplained lane buttons.
- Rec: **keep unipi** Board, re-pointed to the `unipi-kanboard` CLI (DIRECTION §3.4.1); ask whether chat's "Add task…" + task refs stay in chat ⋯ or move to the `+` menu only (currently both — rule 2). Effort M.
- Owner ruling:

## S. Cron

**Z-112 Scheduled tasks**
- ZCode: Scheduled tasks are a workspace main view (not settings); Run now is a separate manual run; cron-sent prompts carry a clock "Sent by scheduled task" label in the bubble and task list (`docs/ui/scheduled-tasks-main-view.md`).
- unipi: Cron screen list → detail (prompt, folder, run history) → form with live schedule preview, Run now, Pause all (`screens/Cron.tsx`).
- Rec: **hybrid** — keep the Cron screen; add the "Sent by cron" label on bubbles and a clock badge on cron sessions in the sidebar. Effort S.
- Owner ruling:

## T. Settings

**Z-113 Settings layout**
- ZCode: full page, 264 px left nav (64 px icon rail below `lg`), groups (Basics / Agent capabilities / Data & stats), search inputs per list page, secondary pages with a standard Back, empty states, row actions (`docs/ui/settings-sidebar-layout.md`, `settings-secondary-navigation.md`).
- unipi: desktop two columns (section list | section), phone one long page; sections Hosts, Terminal, Input, Editor, Connection, Board, Desktop, Phone notifications, Unicrab, About (`screens/Settings.tsx`).
- Rec: **hybrid** — unipi two columns + ZCode grouping and a **global settings search** (needed once 21 extension namespaces land, D-11). Effort M.
- Owner ruling:

**Z-114 Appearance**
- ZCode: theme System/Light/Dark, UI font size 12–20 px via `--ui-font-size`, code theme/line numbers/wrap/code font size (`docs/ui/settings-appearance.md`).
- unipi: dark only; terminal and chat font sizes separate (DESIGN §1.4).
- Rec: **ask** — UI font size + code settings recommended; light theme recommend **no** (brand). Effort S.
- Owner ruling:

## U. Mobile layout

**Z-115 Phone shell**
- ZCode (web remote): single column, sidebar as drawer, side pane as overlay drawer, actions always visible (no hover), placeholder ≤ 2 lines (`ui/WebRemoteControlMobileShell.tsx`, `docs/ui/web-remote-control-mobile-task-timeline.md`).
- unipi: bottom tab bar, full-screen layers (chat, git, editor), corvu bottom sheets, floating crab, tap-to-select action bar, swipe-to-quote, Android Back closes top layer (DESIGN §2–3).
- Rec: **keep unipi** phone shell; map side-pane tabs (Z-5) to full-screen layers/sheets and the sidebar to the Chat tab list. Effort M.
- Owner ruling:

## V. Accessibility

**Z-116 Tooltips, labels, focus**
- ZCode: shared tooltip (300 ms, per-trigger timer) on every icon-only control incl. disabled ones; i18n-safe layouts; no added focus outlines in composites (`docs/ui/sidebar-tooltips.md`, `DESIGN.md` Accessibility).
- unipi: native `title` tooltips; 44 dp targets; 2 px shell focus ring; state = colour + icon + word; reduced-motion honoured globally (`ui.css`, DESIGN §1.3/1.6).
- Rec: **hybrid** — unipi targets/focus/state rules + ZCode shared tooltip component with plain-English text on every icon button, lucide icons only, no emoji (QA#10). Effort S.
- Owner ruling:

---

## Unipi feature reachability

Legend: ✅ reachable in unipi-app today · ◐ partly (text command only, or app-bridge-specific path
that the oino protocol must re-provide) · ❌ not reachable. "Today" = unipi-app 1.0.0 against
unipi 3.x over app-bridge; under oino every ◐ app-bridge path needs a v2 equivalent.

| Extension | User-facing features | Where unipi-app exposes it today | State |
|---|---|---|---|
| ask-user | tool `ask_user` (oino `form` dialog: pager, options, multi-select, free text); end_turn/new_session launcher (Compact & run / Run directly); settings `ask-user` | `DialogCard.tsx AskUser` (unipi `ask_user` kind); launcher as select dialogs | ◐ — `form` kind unsupported (G-2); settings ❌ (G-5) |
| background-tasks | tools `bg_run/status/logs/kill`; `/bg`, `/bg-tasks`; Ctrl+Alt+C clear; launch/completion cards; work-tray tab; wait source; settings toggle | cards (`Cards.tsx background-task-notification`); Session modal Running (log, Stop, Re-run, Send to background); `/` list | ◐ — shortcut ❌ (G-7), settings ❌ |
| btw | `/btw` editorReplace panel; services `btw.ask/list` | `BtwSheet.tsx` (+ menu, `/btw`) via bridge `btw_*` | ◐ — needs oino service path (G-12 note) |
| compactor | vcc/llm compaction; `/compact-then`, `/compact-vcc`, `-by-llm`, `session-recall`, `compact-stats`, `-doctor`, `-help`; compaction card; tools `session_recall`, `context_budget`; info group; settings | `/compact` mapped (`ChatView.onCommand`); Compaction divider + card; others via `/` text | ◐ — stats/doctor output panels (G-1), settings ❌ |
| core | work tray (editorReplace, Shift+↓); hint line; `decision-model` settings | — (Running tab replaces the tray) | ❌ tray/shortcut (G-1, G-7); settings ❌ |
| cron | `/cron` + add/edit/remove; `cron_*` tools; scheduler settings; skill | `screens/Cron.tsx` (host cron ops) | ◐ — must re-point to extension job files + host service (G-17) |
| dream | `/dream status/tray/report/run/stop/approve/reject`; work-tray Dream tab; report card; settings | `DreamSheet.tsx` (status, Run now, runs, approve/reject, stop) | ✅ (app-bridge) / settings ❌ |
| fusion | tool `sidekick`; `/fusion` picker; `/fusion-stats`; preset editor (hidden runner); wake widget; status topic; Alt+Enter default; settings | Model picker Fusion tab; sidekick cards; Running items | ◐ — wake widget (G-3), preset editor (G-1), stats, settings ❌ |
| info-screen | `/info [page]` overlay: Session / Usage / Tools / Skills / Modules + module groups; splash; settings | Session modal (D-7) Context / Running / Info groups | ◐ — Usage/Tools/Skills/Modules pages (G-13) |
| kanboard | `/kanboard open/close/onboard/status/show/doctor`, `-add`, `-do`, `-autowork`; bash guard; reminders/badges/notices/progress bar; in-chat board renderer; sidePane web board | Board screen; kanboard badge/notice cards; task refs; ⋯ Add task / Kanboard | ◐ — in-chat board + progress renderers (G-1), Board must use CLI (G-18) |
| long-horizon | `/goal`, `/ralph`, `/swarm`, `/graph`, `/regular`, `/visualize-progress`; tools; status slots; LH_STATE/PROGRESS | ModeChip + popover (switch mode), ProgressSheet (graph mermaid, steps, stop, history) | ✅ (app-bridge `lh_*`) / status slots → D-2 |
| mcp | `/mcp-status`; server list overlay; catalog browser + JSON editor; add/settings/sync/reload runners; info group | — | ❌ (G-8) |
| memory | tools `memory_*`, `global_memory_*`; `/memory`, `-process`, `-consolidate`, `-search`, `-forget`, `global-memory-list/search`; cards; status `unipi-memory`; settings | memory cards; `/` text | ◐ — no browse/search/forget UI (G-9), status (G-4), settings ❌ |
| notify | `notify_user` tool; event matrix; platforms OS/Gotify/Telegram/ntfy; `/notify-event`; setup wizards; recap; renotify | app's own Phone notifications (ntfy E2E, UnifiedPush) + desktop OS notifications | ◐ — oino notify dropped app push/E2E (G-12); ext config ❌ |
| skill-registry | exposure judged/all/off, pins, vault; `/skills` hub; skills-editor overlay; settings | `/` lists skills | ❌ management (G-10) |
| subagents | `run_subagent`, `read_subagent`; `/subagents` tray tab; `/agents` profile manager; Ctrl+B all to background; cards; settings | cards; Running tab (per item Stop/Background); Progress "Open session" | ◐ — profile manager ❌ (G-11), Ctrl+B ❌ (G-7) |
| updater | Oino release check overlay (changelog, how to update, skip); `/readme`; info group; settings | app's own updater (APK / desktop) + host update row | ◐ — overlap with app updater (G-14) |
| utility | `/continue`/`/retry`, `/cleanup`, `/doctor`, `/summarize`, auto-rename + rename-now, attachments, `image_generate/edit/recognize`, `/move`, settings `utility`/`image`; `/answer` (removed, D-16) | Retry button (`/unipi:continue`); `/` text; tool attachments chips | ◐ — doctor/cleanup output (G-1), settings ❌ |
| watchdog | jev watchdog, background/kill actions, warnings, `/bg-detach`, status `watchdog: N`, session caps flags `--max-turns/cost/wall/tool-errors` | watchdog cards (`Cards.tsx`) | ◐ — status (G-4), caps ❌ (G-16), settings ❌ |
| web-api | `web_search`, `multi_web_content_read`, `web_llm_summarize`; provider keys; cache clear runner; info group | web tool rows | ◐ — provider keys/settings ❌ (G-5), cache clear (G-1) |
| workflow | permission gate; `approval` dialog; `/permission ask/auto/full/status` + Alt+M; `/plan on/off/view/approve` + Alt+P + `plan_submit`; plan review overlay; WORKFLOW_STATUS; settings | `/` text only; plan/approval dialogs not rendered | ❌ (G-2, G-6) |
| (pi built-ins) | `/export`, `/share`, `/import`, `/clone`, `/scoped-models`, `/login`, `/logout`, `/settings`, `/changelog`, `/hotkeys`, `/trust`, `/bug`, `/quit` | `TERMINAL_ONLY` toast "only the terminal can show" (`lib/chat/commands.ts`) | ❌ (G-15) |

### Gaps

- **G-1** Extension `overlay` / `sidePane` / `editorReplace` / `fullscreen` panels and `custom` dialogs render nowhere ("This one can only be answered in the terminal", `DialogCard.tsx`). Blocks info pages, MCP list/catalog, skills editor, plan review, fusion preset editor, updater overlay, `/readme`, compactor stats/doctor, kanboard in-chat board, work tray, utility doctor/cleanup. → UI v2 renderer + slots (D-17).
- **G-2** Dialog kinds `multiSelect`, `form` (oino `ask_user`), `approval` (workflow), `clipboardReadImage`, `editExternally` are not handled by `DialogCard` (only select/confirm/input/editor/unipi `ask_user`).
- **G-3** `setWidget` (above composer) not rendered (fusion sidekick-wake widget, any third-party widget).
- **G-4** `setStatus` not rendered (long-horizon slots, `unipi-memory`, `watchdog: N`, workflow mode, kanboard status). Needs a status bar slot (D-17) — and a ruling on where it lives vs the git-pane capsule (Z-95).
- **G-5** No extension settings in the app: 21 namespaces (ask-user, background-tasks, compactor, core decision-model, cron, dream, fusion, info-screen, memory, notify, skills, subagents, updater, utility, image, watchdog, web-api, permission, long-horizon, kanboard, mcp). → schema-driven Settings sections (D-11).
- **G-6** Permission mode (ask/auto/full) and plan mode have no visible indicator or control; only typed `/permission`, `/plan`. Recommend a composer mode control (Z-64).
- **G-7** Extension keyboard shortcuts are not bound in the app: Alt+M, Alt+P, Shift+↓ (tray), Ctrl+Alt+C (bg clear), Ctrl+B (subagents to background), Alt+Enter (fusion default). Needs a shortcut registry fed by extensions + conflict ruling (Z-94).
- **G-8** MCP: no status, server list, enable/disable, add from catalog, edit JSON, reload.
- **G-9** Memory: no browse/search/forget/consolidate UI; only cards and typed commands.
- **G-10** Skills: no exposure mode, pins, vault or skill grid editor (composer `/` only lists them).
- **G-11** Subagents: no `/agents` profile manager; no "all running to background".
- **G-12** Notify: oino's notify port dropped app push encryption, UnifiedPush fan-out and `unipi://` deep links (unipi-port §2.5), but DIRECTION keeps E2E push and the app's ntfy path expects `unipi1:` payloads → must be re-specified (host-side). Notify's own event matrix / Gotify / Telegram setup has no UI. Also btw/dream/lh/fusion/work data reach the app through app-bridge messages (`btw_*`, `dream_status`, `lh_*`, `fusion`, `work`, `info`) that the oino session protocol must re-provide.
- **G-13** info-screen Usage / Tools / Skills / Modules pages not shown (Session modal has Context/Running/Info groups only).
- **G-14** Two updaters: the `updater` extension's Oino release overlay vs the app's own updater + host update row → ruling needed (recommend: app shows one Update card fed by the extension's check; extension overlay hidden in the app).
- **G-15** pi built-ins `/export`, `/share`, `/import`, `/clone`, `/scoped-models`, `/login`, `/logout`, `/settings`, `/changelog`, `/hotkeys`, `/trust`, `/bug` have no app equivalent (only a toast). Each needs an app surface or an explicit drop (Z-20, Z-113).
- **G-16** watchdog session caps (`--max-turns`, `--max-cost`, `--max-wall`, `--max-tool-errors`) have no per-session UI (new-chat options or Session modal).
- **G-17** Cron screen talks to unipi-host's cron store; v2 makes jobs files written by the cron extension and fired by unipi-host → screen must be re-pointed; `/cron` command output has no panel.
- **G-18** Board uses the kanboard daemon over iroh (`lib/board/transport.ts`); v2 removes the daemon → Board must call the `unipi-kanboard` CLI through the host (DIRECTION §3.4.1).
- **G-19** Extension crash / disabled notice and per-extension toggle (D-19) have no app surface yet.
- **G-20** Background-tasks / subagents "work tray" (`core` editorReplace + tabs) has no direct app equivalent other than the Running tab; extensions registering `work-tray.tab` (dream, subagents, bg) lose their tabs → ruling with Z-95.
- **G-21** Tool renderers registered by extensions (OUS renderers for cards like `unipi-dream-report-card`, memory cards, kanboard progress) are hard-coded in `Cards.tsx` for unipi 3.x content types; third-party renderers can't show → UI v2 renderer for message/entry renderers.

---

## Top 10 most impactful items (recommended order of ruling)

1. **Z-5** Side pane of tabs as the single home for git, browser, terminal, files, previews, subagents, side chats — unblocks D-2/D-3/D-4/D-10 and the `sidePane` slot.
2. **Z-21 / Z-27** Turn fold "Worked for N s" + turn-tail actions — the core chat reading model (owner rule 4, QA#14).
3. **Z-95** Status panel vs git pane vs Session modal Running — biggest redundancy risk.
4. **Z-1 / Z-8** Sidebar = hosts → projects → sessions; screen rail for Board/Cron/Settings.
5. **Z-46 / Z-47** Dialogs dock above the composer; approval/form kinds (with G-2).
6. **Z-19** One "New chat" draft flow; drop New session / New chat here / Resume sheet duplication.
7. **Z-77 / Z-79** Command center with sessions+files scopes, and one find surface.
8. **Z-13 / Z-16** Archive + row context menu (QA#1).
9. **Z-52 / Z-53** Queue/steer defaults and queue recall with draft guard.
10. **Z-69 / Z-70 / Z-71** Model picker fixes, single entry point, separate thinking control.
