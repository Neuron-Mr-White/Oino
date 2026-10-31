# Spec — UniPi app v2 (SolidJS + Tauri 2) on the new host

Status: **implementation spec**, work id `[spec-app]`, branch `v2`, 2026-11-01. Planning only.
Commit prefix for the work packages: `[app-a<n>]` (e.g. `[app-a6]`).

Binding inputs: [DIRECTION.md](../DIRECTION.md), [RULINGS.md](../RULINGS.md),
[zcode-audit.md](../zcode-audit.md) (every Z item ruled "take Rec", overrides Z-43/Z-61/Z-65/Z-95;
D-1..D-24 decided; G-1..G-21 must all close), sibling specs [host.md](host.md),
[ui-protocol.md](ui-protocol.md), [browser.md](browser.md), [dist.md](dist.md), [core.md](core.md),
[ext-abi.md](ext-abi.md). References: ZCode `/tmp/zcode-src` (`docs/ui/*.md`, `packages/ui/src`);
unipi-app design `/home/oi/Projects/Personal/archived/unipi/unipi-app/{DESIGN.md,docs/m5..m9}`
(read-only).

## Contents

0. Scope and ownership
1. Conventions (paths, gates, verification)
2. Current state (reuse / rewrite / delete)
3. Architecture
4. Rulings applied to "ask" items
5. Work packages WP-A1..A28
6. Order and dependency graph
7. Wave-3 list (reference)
8. Risks
9. Closure tables (Z → WP, G → WP, D → WP)
10. Owner questions

---

## 0. Scope and ownership

This spec covers everything the app must do for v2 **except**:

| Not here | Owner |
|---|---|
| Host process, session hub, session protocol, takeover, PTY/fs/git/push/cron services, tray/lifetime (Rust side) | [host.md](host.md) WP-H1..H10 |
| Extension UI renderer, node components, slot hosts, dialogs kinds, settings form engine, reserved routing | [ui-protocol.md](ui-protocol.md) WP-U9..U12 (app side), U1..U8 (Rust side) |
| Browser engine, stealth, host service; **browser pane UX** | [browser.md](browser.md) WP-B1..B9 (B8 = pane UX, wave 3) |
| Installers, updater channel, release | [dist.md](dist.md) WP-D1..D9 (wave 3) |
| Core (durability, lazy sessions, crash isolation, checkpoints) | [core.md](core.md) |
| Extension ABI (host-service, kanboard CLI, cron files, browser service) | [ext-abi.md](ext-abi.md) |

What this spec **does** own: the app shell and navigation, the side-pane-of-tabs container and the
bottom panel container, the sessions sidebar and session actions, the chat timeline rewrite on the
host session protocol (host WP-H4 `session.ts`) with ZCode performance (D-8), tool rows and diffs,
the dialog dock, queue/steer, composer, model/thinking/context controls, command center and find,
notifications/toasts, empty/loading/error states, motion, shortcuts, the git/status pane (Git + LH +
todos + Running summary, RULINGS Z-95), the Session modal, Files + CodeMirror + **LSP** (incl. the
host-side LSP service), the terminal pane **container** (pane UI itself is wave 3), Board / Cron /
Settings adaptation, phone layout, accessibility, the `unipi.file-summary` extension and its
turn-footer UI, and the small host deltas the app needs that host.md does not define (§3.14).

## 1. Conventions

- **App root**: `apps/unipi` (moved by WP-H1). Sources `apps/unipi/src`, Tauri `apps/unipi/src-tauri`
  (own `[workspace]`), TS protocol `apps/unipi/packages/protocol`. Paths in this spec are relative
  to `apps/unipi/src` unless they start with `apps/`, `crates/`, `extensions/` or `docs/`.
  ui-protocol.md still writes `apps/unipi/apps/mobile/src/ui/` — read it as `apps/unipi/src/ui/`.
- **Design system**: unipi-app tokens and rules (archived `DESIGN.md` §1, `src/ui.css`,
  `src/style.css`, `components/shell/shell.css`): Unicrab palette, shell red for primary/focus,
  `claw` accent, 44 px touch targets, 2 px focus ring, state = colour + icon + word, reduced motion
  honoured, lucide icons only, no emoji (QA#10). "Take ZCode" always means ZCode behaviour in the
  unipi look (audit rule).
- **Gates per WP** (all must be green before commit): `cd apps/unipi && npm run typecheck && npm test`
  (vitest, app + `packages/protocol`); for WPs touching Rust also
  `cargo fmt --all --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`,
  `cargo test -p <crate>`; for `src-tauri` changes `cargo check --manifest-path apps/unipi/src-tauri/Cargo.toml`
  (once H4/H6/H9 make it compile again; before that the WP notes it).
- **Component tests**: `@solidjs/testing-library` + vitest in jsdom (existing `src/test-setup.ts`),
  colocated `*.test.tsx`. Pure logic in `*.test.ts`. Perf: vitest bench (`*.bench.ts`).
- **Visual verification** (every UI WP): the capture harness of WP-A1 —
  `npx vite --config vite.config.capture.ts` (port **1450**) serves the real app with a **mock
  host** (`src/dev/mock/`) answering every host op and replaying scenario fixtures; a scenario is
  chosen by URL: `http://127.0.0.1:1450/#/…?scenario=<name>&shell=desktop|phone`. The worker
  captures with `agent-browser` (desktop 1280×820 and 1440×900 where widths matter, phone
  412×915), writes `docs/dev/v2/design/shots/app/<wp>-<name>-<shell>.png`, inspects each with
  `image_recognize` against the design rules and the referenced ZCode doc, and iterates. QA
  (`opus-ui-qa`) re-checks the same scenarios.
- **No duplicate surfaces** (DIRECTION §4.2.2): each WP lists what it deletes when it adds a
  surface. Never touch `~/.unipi`, `~/.pi`, or the owner's running unipi-host; the mock host never
  opens a socket.
- Only commit the files of the WP (explicit paths); other workers edit in parallel.

## 2. Current state

`apps/unipi/src` is unipi-app 1.0.0 (`@unipi/mobile`, ~58 k lines TS/TSX/CSS, 171 test files /
1615 tests green after H1). It speaks the **deleted** v1 protocol (herdr/tmux mux, pi app-bridge
`chat_*`, kanboard daemon, desktop streaming); the TS `packages/protocol` still exports those schemas
(H1 kept them so the app compiles). `src-tauri` does not compile until H4/H6/H9. Routing is a
`HashRouter` in `main.tsx` (`Root()`), desktop shell = `components/shell/DesktopShell.tsx`
(`NAV` of screens, Ctrl+1..6), phone = bottom tab bar + full-screen layers (`lib/back.ts`).

Verdicts (R = reuse as is or with small edits, W = rewrite, D = delete, M = move/re-home):

| Area | Files | Verdict | Notes |
|---|---|---|---|
| Chat transport + store | `lib/chat/transport.ts` (herdr `chat_list`/`chat_open`), `lib/chat/store.ts` (1366, bridge `Msg` reducer), `lib/chat/open.ts`, `lib/chat/list.ts`, `lib/chat/newChat.ts`, `lib/chat/closeTarget.ts`, `lib/chat/bridgeError.ts`, `lib/chat/boardPortal.ts` | **W/D** | replaced by `lib/host/*` + `lib/session/*` (§3.2–3.4) on host session streams. `customInsertIndex`, outbox/pending sweep logic (`sweepPending`, `PENDING_*`) and `treeGoRestoreOf` are ported. |
| Transcript model | `lib/chat/model.ts` (715: `Entry`, `Part`, `buildRows`, `processSummary`, `TurnFile`) | **W** | becomes `lib/session/turns.ts` (`buildTurns`, turn units, segments, fold summary); `textOf`, `imagesOf`, `attachmentsOf`, `STEP_TYPES`, `HIDDEN_COMMANDS` reused. `TurnFile`/`ChangesCard` deleted (D-1 replaces them). |
| App-bridge DTOs | `lib/chat/progress.ts`, `lib/chat/lhMode.ts`, `lib/chat/dream.ts` | **D** | LH/dream data now arrive as topics, UI v2 entries and reserved panels (§3.10). Pure helpers worth keeping (graph error text, mode labels) move into `components/status/*`. |
| Chat view | `components/chat/ChatView.tsx` (1234, per-row `createVirtualizer` at :415, `RunningRow`, `HistoryEdge`, action bar, `DialogStack`), `Transcript.tsx` (789: `UserBubble`, `ToolRow`, `ToolBody`, `ThinkingRow`, `ProcessGroup`, `TurnEnd`, `ChangesCard`, `Compaction`) | **W** | timeline rewritten as `components/timeline/*` (turn-unit virtualizer); `UserBubble`, `ToolRow/ToolBody`, `ThinkingRow`, `Compaction`, `Prose`, `TaskRefText`, `bubbleSegments` are moved and adapted, not re-invented. |
| Cards | `components/chat/Cards.tsx` (257, hard-coded unipi 3.x content types) | **W** | generic renderer cards via `_oino/render` + U9 `NodeView` (G-21); only `model_change`, compaction and harness card kept native. |
| Composer | `components/chat/Composer.tsx` (1030; `images` signal :404 = D-6 bug), `ChipEditor.tsx`, `ImageChip.tsx`, `ImageEditor.tsx` (no ellipse, D-5), `lib/chat/{attachments,chips,paste,undo,sendChoice,commands,mention}.ts`, `lib/imageEdit.ts`, `lib/imageShrink.ts` | **R+W** | split into `components/composer/*`; chip editor, undo, attachments upload, image shrink/editor reused; send-choice replaced (Z-52). |
| Dialogs | `components/chat/DialogCard.tsx` (295), `LauncherSheet.tsx` | **M** | move under U10's `src/ui/dialogs/`; placement becomes the bottom dock (§3.11). |
| Session modal + model picker | `components/chat/ControlCentre.tsx` (750: `ControlCentreSheet`, `ModelPickerSheet`, `FooterStrip`, `FooterTray`, `RunningSection`) | **R+W** | `ControlCentreSheet` → `components/session/SessionModal.tsx` (D-7); `ModelPickerSheet` → `components/model/ModelPicker.tsx`; `FooterStrip`/`FooterTray` **D** (Z-75). |
| Mode / progress | `ModeChip.tsx` (214), `ProgressSheet.tsx` (724), `StatusBar.tsx` | **R+M** | ModeChip stays the switch (Z-74); ProgressSheet content → LH section of the status pane (§3.10); sheet deleted. |
| Side sheets | `BtwSheet.tsx` (173), `DreamSheet.tsx` (410), `SearchSheet.tsx`, `ReviewViewer.tsx`, `MermaidViewer.tsx`, `BrowserSide.tsx`, `NewChatSheet.tsx` (398), `CloseChatSheet.tsx`, `SessionMenus.tsx` (781: header menu, `ResumeSheet`, `TreeSheet`, `RenameSheet`, `PlusMenu`, `WelcomeState`) | mixed | Btw → side-pane tab (Z-31, R+M); DreamSheet **D** (dream ext panels via UI v2); SearchSheet → find bar fallback (Z-79); ReviewViewer → diff tab (Z-40); Mermaid kept; BrowserSide → wave 3 (B8); NewChatSheet **W** as draft (Z-19); CloseChatSheet **D** (D-14); ResumeSheet **D** (Z-19); TreeSheet → tree tab (Z-18); RenameSheet R; WelcomeState **W** (Z-87). |
| Shell | `components/shell/{DesktopShell,CommandPalette,builtins,DesktopNative,ShortcutsDialog}.tsx`, `shell.css`, `lib/{layout,shell,desktop}.ts`, `main.tsx` | **W** | new shell (§3.7); palette → command center (Z-77); `DesktopNative` 20 s polling **D** (Z-84); `lib/desktop.ts shortcuts()` → shortcut registry (Z-94). |
| herdr/tmux/streaming | `screens/Sessions.tsx` (745), `components/SessionPicker.tsx` (988), `components/icons/HerdrIcon.tsx`, `lib/recents.ts`, `lib/recentsContext.tsx`, `lib/liveSessions.ts`, `screens/DesktopView.tsx` (1265), `lib/screen.ts` (805), `lib/desktopUx.ts` (404), `components/ArrowPad.tsx`, `components/FeatureOff.tsx` (desktop kind), `lib/needsYouPoll.ts` (polls `chat_list`) | **D** | D-14, D-15. `FeatureOff` keeps only non-desktop kinds if still referenced by Settings sharing. |
| Terminal | `screens/Terminal.tsx` (1392, herdr layer), `lib/termSession.ts`, `lib/term*.ts`, `components/TermToolbar.tsx`, `components/Composer.tsx` (terminal composer, 192) | **W (wave 3)** | wave 2 deletes the herdr parts and keeps xterm helpers (`termFit`, `termFonts`, `termTheme`, `termScroll`, `termToolbar`, `TermToolbar`) for WP-A29 (§7). |
| Preview | `screens/Preview.tsx` (689), `preview.css` | **D** (desktop) / **M** (phone) | Z-104: ports list → side-pane `+` launcher; phone keeps the forward viewer as a layer. |
| Files / editor | `screens/Files.tsx` (1211 breadcrumb list), `screens/Editor.tsx` (577 CodeMirror 6, vim, conflict sheet), `lib/{fs,filesNav,filesTransfer,filesTarget,editorTarget,editorTheme}.ts` | **W / R** | tree rewritten (Z-101); editor reused + LSP (D-10). |
| Git | `components/git/GitView.tsx` (1071), `git.css`, `lib/git.ts` | **R+M** | re-homed into the status pane (Z-96/97). |
| Board | `screens/Board.tsx`, `components/board/*` (~6 k), `lib/board/*` | **R**; `lib/board/transport.ts` **W** | CLI via host (G-18). |
| Cron | `screens/Cron.tsx`, `components/CronSettings.tsx`, `lib/cron.ts` | **R+W** | DTO re-point (G-17). |
| Settings | `screens/Settings.tsx` (760), `settings/*` (app settings store, migrations), `components/{HostsSettings,Pairing,PairPhone,SharingSettings,RelaySettings,PushAccess,HostSupervisorCard,ComponentsCard,UpdateCard,DesktopUpdate}.tsx` | **R+W** | one Settings screen (D-11): layout W, sections R. |
| Notifications | `lib/{needsYou,push,pushRegister,notifyInit,notifyPoll,notifyCrypto,notifyDeepLink,runningNotification}.ts`, `components/{NeedsYouBell,FloatingMascot,FloatingBell,IncomingPush}.tsx`, `components/board/BoardToasts.tsx` | **R** | needs-you fed by hub events; one app toast stack from BoardToasts. |
| Brand | `components/Unicrab.tsx`, `lib/{unicrab,mascot*}.ts`, `lore.ts`, `mascotWindow.tsx` | **R** | D-22. |
| Capture | `vite.config.{chat-capture,chat-capture-composer,chat-capture-control,chat-capture-needs}.ts` (scripts lived in the archive `docs/m5,m6`, driving `window.__UNIPI_CHAT__`), `vite.config.{board-capture,files-capture,mock}.ts` | **W** | consolidated into `vite.config.capture.ts` + `src/dev/mock/` (WP-A1); board/files configs removed when A19/A21 port their scenarios. |
| Tauri | `src-tauri/src/chat.rs` (H4), `term.rs` (H6), `board.rs` (**this spec**, A21), new `lsp.rs` (**this spec**, A20), `lifecycle.rs`/`desktop.rs` (H9) | — | listed for ownership only. |

Existing strengths kept on purpose: chip editor + undo, attachment upload paths, image editor,
file-ref verification (`lib/chat/fileRefs.ts`, `FileRefText.tsx`), markdown pipeline
(`lib/markdown/*`, `MdBlock.tsx` sticky code header), media previewer, needs-you Unicrab, push E2E,
phone gestures (`lib/gestures.ts`, swipe-to-quote), `lib/back.ts` (Android Back closes top layer).

## 3. Architecture

### 3.1 Layers

```text
 Tauri commands / Channels (src-tauri: chat.rs H4, term.rs H6, board.rs A21, lsp.rs A20, …)
        │                                  ▲ mock host (src/dev/mock) in captures/tests
        ▼                                  │
 lib/host/      HostClient seam: request(op) · openSession · openEvents · openTerm · openLsp
 lib/session/   session stream client → rAF batch → reducer (entries, streaming, tools, queue,
                run state, topics) → turns index;  hub store (cards, notices, meta)
 src/ui/        U9 registry + NodeView (ui-protocol) — fed by the same session stream
 stores         panes · composer drafts · scroll memory · height cache · toasts · unread · shortcuts
 components/    shell · sidebar · timeline · composer · dock · status · files · session · model …
 screens/       Board · Cron · Settings · SetupWizard (sidebar-rail pages)
```

Rule: components never call `invoke` directly; everything goes through `lib/host` so the mock
host can stand in (captures, component tests, web build).

### 3.2 HostClient seam (`lib/host/`)

```ts
interface HostClient {
  hostId: string;                                   // 'local' = This computer, else paired id
  request<T>(op: string, args?: unknown): Promise<T>;          // one-shot ops (fs_*, git, cron_*, …)
  openSession(req: {kind:'open'|'new'|'fork'; session?: string; sinceSeq?: number; cwd?: string;
                    fromEntry?: string}, sink: SessionSink): SessionStream;
  openEvents(sink: (e: HubEvent) => void): Closable;           // events_open (one per host)
  openTerm(req, sink): TermStream;                              // H6 (wave 3 UI)
  openLsp(req: {root: string; language: string}, sink): LspStream; // A20
}
interface SessionStream { cmd<T>(cmd: RpcCommand): Promise<T>; close(): void; }
```

- `lib/host/tauri.ts` implements it over the src-tauri commands/Channels; `lib/host/mock.ts`
  re-exports `src/dev/mock/host.ts` when `window.__UNIPI_HOST__` is set (captures) or a test
  injects one (`setHostClientFactory`). One `HostClient` per host, created by `hosts/context.tsx`.
- Types come from `@unipi/protocol` `session.ts` / `term.ts` / `settings.ts` (H4/H6/H7). Only the
  **envelope** is zod-parsed (`SessionSnapshot`, `Events{items}`, `Ok/Err`); `ClientEvent` payloads
  are narrowed by `type` in the reducer without a full zod pass (hot path, ui-protocol D-U14 rule).
- `cmd()` correlates by `id` (host §7.3); timeouts 30 s except `prompt`/`steer`/`follow_up` (ack
  only) and `_oino/render` (2 s, footers/cards render `null` on timeout).

### 3.3 Stores

| Store | File | Shape / key | Lifetime |
|---|---|---|---|
| hosts | `hosts/context.tsx` (R) | paired hosts + This computer, conn state | app |
| hub | `lib/session/hub.ts` | per host: `cards: Record<sessionId, SessionCard>`, `notices`, `archived?` (lazy), `meta` (pin/unread/titleLock) | app; fed by `sessions_list` + `events_open` |
| session | `lib/session/store.ts` | per open session: `{seq, entries, byId, leafId, streaming, liveTools, run, queue, topics, commands, conn, outbox, banner}` | **LRU 8** live subscriptions; evicted sessions drop state, reopen with snapshot |
| turns | `lib/session/turns.ts` | derived `Turn[]` with stable `turnId`; incremental (only the tail turn recomputes on stream events) | with session |
| ui registry | `src/ui/registry.ts` (U9) | per session + per host scope | with session / host |
| panes | `lib/panes.ts` | per scope `(hostId, projectKey, sessionId?)`: side-pane tabs `{id, kind, params, title}`, active id, recently closed (10), user order; bottom panel tabs; sizes | persisted (app store) per project; session-scoped tabs dropped with the session |
| composer | `lib/composer/drafts.ts` | per `sessionKey` (`hostId/sessionId` or `draft:<hostId>:<cwd>`): text (persisted), chips, pending images + files (memory, D-6), queue-edit recall | text persisted (debounced 500 ms, removed when empty); rest in memory |
| prompt history | `lib/composer/history.ts` | per workspace (`hostId:cwd`), 30, no consecutive dups | persisted |
| scroll memory | `lib/timeline/scrollMemory.ts` | key `(paneId, hostId, sessionId)` → `{bottomLocked} | {scrollTop}` | memory, LRU 200 |
| height cache | `lib/timeline/heightCache.ts` | per timeline instance: `turnId → px` | LRU 4000, cleared on session switch (ZCode `timelineRowHeightCache.ts`) |
| expansion | `lib/timeline/expansion.ts` | `(sessionId, key)` → open (fold, tool, thinking) | memory, LRU 5000 |
| toasts | `lib/toasts.ts` | one stack, max 3 visible | memory |
| unread / needs-you | `lib/needsYou.ts` (R) + hub meta | from hub events | persisted via host meta (unread) |
| shortcuts | `lib/shortcuts/registry.ts` | id → default chord, user chord, scope, source (`app` / `ext:<id>`) | user chords persisted in app settings |
| app settings | `settings/*` (R) | UI prefs, closeAction, autostart, font sizes | app store |

### 3.4 Data flow from the host session stream

1. **Open**: route `/s/:host/:session` → `sessions.ensure(hostId, sessionId)` →
   `openSession({kind:'open', session, sinceSeq})`. First frame `SessionSnapshot` → reducer
   `applySnapshot` (`reconcile` entries by id), `src/ui/registry.applySnapshot(snapshot.ui)`,
   topics seeded. `reset:false` + `replay` → events applied in order.
2. **Live**: `Events{items}` frames → queued; one `requestAnimationFrame` flush applies all queued
   items inside one Solid `batch()` (reducer `applyEvent` per item, `produce`). Delta events
   (`message_update` text/thinking) append to `streaming.parts` only; `message_end` settles the
   entry; `tool_execution_*` update `liveTools`; `queue_update`, `agent_start/end`, run-state,
   topic and agents events update their slices; `extension_ui_request` / `_oino/ui_patch` go to
   the U9 registry; host-only kinds `crashed` / `taken_over` / `reloaded` set `banner`.
3. **Derive**: `turns` recomputes only the **tail turn** for streaming/tool events (O(tail)),
   the whole index only on snapshot/reset/compaction/tree navigation.
4. **Render**: the timeline reads `turns()` by index; each `TurnRow` reads its own turn object
   (stable identity for settled turns ⇒ Solid does no work for them).
5. **Commands**: composer/menus call `session.cmd({type:'prompt', …})` etc. Outgoing prompts get
   an outbox entry (ported `Outboxed`/`Pending`) shown as a pending bubble until the host's
   `message_start` for it arrives; on disconnect they show "Waiting for connection… ✕" (Z-50).
6. **Resume**: on stream error/iroh flap the session store keeps state and reconnects with
   `sinceSeq = seq` (backoff 0.5/1/2/5 s, `lib/reconnect.ts` R); `reset:true` rebuilds from
   `entries`, keeping scroll memory and expansions (keys are entry ids).
7. **Lazy detail**: tool outputs truncated in the snapshot are fetched on expand with
   `get_entries{ids}`; extension cards/tool renderers via `_oino/render` (cache by
   `(target,key,entryId,rev,width)`).

### 3.5 Turn model (`lib/session/turns.ts`)

- A **turn** = one user message entry + everything after it until the next user message of the
  same branch. `turnId` = that user entry id (equals ui-protocol `turnEntryId`). Entries before
  the first user message form a `preamble` turn (`turnId = 'pre:' + firstEntryId`).
- A turn has **segments**: segment 0 starts at the user message; every accepted steer opens a
  new segment inside the same turn (Z-54) with its own fold and navigator entry.
- Per segment: `work` items (thinking, tool calls, intermediate prose, step/subagent rows, cards)
  and `answer` (the last assistant prose). The fold "Worked for 1m 3s · read 4 files, edited 2"
  (Z-21, unipi summary wording from `processSummary`) covers `work`; it is forced open while the
  segment runs, and on interrupted / failed / error (Z-56: label "Stopped", no duration).
- Tail data: `endedAt`, `durationMs`, `costUsd`, `model`, `stopped`, `error`, `source`
  (`user` | `cron` | `steer` | `harness`), `command` (for `/compact` echo hiding, Z-35).
- Tool grouping inside `work` (Z-38): consecutive non-read-only shell calls → one "Execute" group;
  consecutive reads/searches → "Explored"; consecutive writes/edits → "Changed".

### 3.6 Timeline performance design (D-8)

Port of ZCode `docs/ui/chat-virtual-scroll.md` + `v4/timelineRowHeightCache.ts` to Solid:

- `@tanstack/solid-virtual` `createVirtualizer` over **settled turns**; item key = `turnId`;
  `estimateSize = heightCache.estimate(turnId, 72)`; `measureElement` writes back to the virtualizer
  and `heightCache.set` (LRU 4000). overscan 4 turns.
- **Live tail** (Z-22): the running turn renders expanded *outside* the virtual list (below it, in
  the same scroll container); on `agent_end` it migrates into the list with its measured height
  pre-seeded in the cache (no jump).
- **Memo boundaries**: `TurnRow`, `UserBubble`, `SegmentFold`, `ToolRow`, `DiffView`, `MdStatic`
  are components whose props are stable objects from the store; callbacks come from a per-timeline
  context (no inline closures in `<For>`). Completed markdown renders once (static HTML string
  cached per entry id + width class); only the streaming entry uses the incremental block renderer
  (Z-34, no fade-in).
- **Lazy details**: fold bodies, tool bodies, thinking text and diffs construct DOM only when open;
  open state lives in `expansion` (survives unmount).
- **Bottom lock** (Z-23): one boolean. Detach when the user scrolls up by > 48 px (wheel/touch/key
  intent beats layout); re-lock when within 48 px of the bottom or on jump. While locked, content
  growth (stream, expand, ResizeObserver) writes the new bottom `scrollTop` in the same frame;
  width changes use ZCode's synchronous measure-then-pin batch. `scrollbar-gutter: stable`.
  Jump button only when detached **and** overflowing, gradient mask behind it, unread dot when new
  content arrived while detached (Z-24).
- **Scroll memory**: on leave store `{bottomLocked}` or `{scrollTop}`; on return restore in layout,
  re-correct once after the first measure frame unless the user scrolled. Positional jumps (find,
  navigator, fork origin) leave the timeline detached.
- **Older history**: the snapshot carries the current branch; > 400 turns render through the
  virtualizer without paging. Prepends (tree navigation) keep the anchor turn's offset.
- **Budgets** (vitest bench + capture-page measurement, Chromium desktop and the Android phone):
  2 000-turn session opens ≤ 120 ms to first paint; scroll at 60 fps desktop / ≥ 50 fps phone, no
  long task > 50 ms; streaming a 100 k-token reply at host cadence (≤ 1 frame/16 ms) keeps
  input latency < 50 ms; DOM nodes ≤ 3 000 with 2 000 turns; switching between two open
  sessions ≤ 60 ms.

### 3.7 Routing and shell geometry (desktop)

Routes (`HashRouter`, `main.tsx` rewritten in A3):

| Route | View |
|---|---|
| `/` | last open session of the last host, else the draft for This computer |
| `/s/:host/:session` | conversation |
| `/new/:host?cwd=` | draft composer (Z-19) |
| `/board`, `/cron` | rail pages (reserved `unipi.kanboard/board`, `unipi.cron/jobs`; hidden when their gate is false, U12) |
| `/x/:entryId` | extension sidebar page (`fullscreen` slot, U10) |
| `/settings/:section?` | the one Settings screen |
| `/setup` | SetupWizard (R) |

Desktop frame (`components/shell/AppShell.tsx`):

```text
+------------------------------+------------------------------------+--------------------+
|sidebar 264 (220-420, Cmd+B)  |conversation frame                  |side pane (tabs)    |
+------------------------------+------------------------------------+--------------------+
|New chat · search             |header 56: title · project/branch   |tab strip · +       |
|v This computer               |  mode chip · ⋯ · terminal · pane   |Files               |
|  v project  +                |header band (setHeader)             |Git + status        |
|    session rows (5, more)    |timeline: virtual turns             |Diff · Tree         |
|> paired PC                   |  + live tail        [capsule]      |Btw · Subagent      |
|                              |bottom dock: widgets · dialogs ·    |Preview · ext       |
|                              |  queue · composer                  |(browser, terminal  |
|rail: Board · Cron ·          |status strip (setStatus)            | = wave 3)          |
|  ext pages · Settings        |bottom panel (terminal/bottomPane)  |                    |
+------------------------------+------------------------------------+--------------------+
```

- Flush warm surfaces with 1 px rules (unipi look, Z-4); resize handles have a 4 px hit area and a
  2 px line on hover/drag. Side pane default 480 px (min 320), bottom panel default 280 px.
- **Auto-collapse** on window resize (Z-3): side pane collapses when the chat column would be
  < 480 px; sidebar collapses when < 360 px; restored when space returns (unless the user
  collapsed it).
- Rail (Z-1): Board, Cron, extension pages (`fullscreen` entries, order §6.4 ui-protocol),
  Settings at the bottom. Files/Git/Terminal/Browser are **not** rail items (side pane).
- Header (Z-2): no model picker on title click; ⋯ = chat-scoped actions (Rename, Fork, Export,
  Copy session id, Copy path, Copy log path, Reveal folder, Session info); terminal toggle
  (Cmd/Ctrl+J), side-pane toggle (Cmd/Ctrl+Alt+B), mode chip.
- Full view (Z-6, Cmd/Ctrl+Shift+F): the side pane covers the conversation frame; "Chat" becomes
  the first tab; a floating composer (same component instance moved via portal) stays over other
  tabs; Esc/second press exits.
- Split panes (Z-7) are **deferred**: the frame has exactly one conversation.

### 3.8 Side pane and bottom panel containers (Z-5)

- `components/panes/SidePane.tsx` + `lib/panes.ts`: tabs scoped to `(hostId, projectKey)` with
  session-scoped kinds tagged by `sessionId` (hidden when another session is shown, restored when
  it returns). Tab kinds (one registry `lib/panes/kinds.ts`, `{kind, icon, title(params),
  component, singleton, scope}`):

  | Kind | Component | Singleton | Scope | WP |
  |---|---|---|---|---|
  | `files` (reserved `host/files`) | Files tree + editor | yes | project | A19/A20 |
  | `git` (reserved `host/git` + LH section) | status pane | yes | project | A15/A16 |
  | `diff` | full diff viewer (`ReviewViewer` port) | per file | session | A9b |
  | `preview` | Previewer (image/PDF/media/markdown/SVG) | per path | project | A19 |
  | `tree` | session tree (TreeSheet port) | yes | session | A9b |
  | `btw` | side chat (BtwSheet port) | multi | session | A9b |
  | `subagent` | child transcript (`_oino/agent_messages`) | per agent | session | A9b |
  | `browser` (reserved) | browser pane | multi | project | wave 3 (B8) |
  | `terminal` (reserved `host/terminal` in side pane) | xterm tab | multi | project | wave 3 (A29) |
  | `ext` | U10 `sidePane` entry (`ext:<entryId>`) | per entry | entry scope | U10 |

- Strip: icon + title + badge, drag to reorder (order persisted per project), middle-click/✕
  closes (`$close` for ext tabs), `+` launcher (Files, Git, Tree, Side chat, Local ports →
  browser/forward viewer per Z-104, terminal and browser once wave 3 lands, ext panels the user
  can open), overflow menu: tab overview with search, Close / Close others / Close all,
  Recently closed (10) + Reopen (Cmd/Ctrl+Shift+T).
- Status-pane push rule (RULINGS Z-95): the side pane **pushes** the chat only when the app content
  width ≥ 1280 px; below that it opens as an **overlay** over the conversation frame (shadow,
  click-outside closes) and the capsule stays visible.
- `components/panes/BottomPanel.tsx`: same tab model for `bottomPane` slot entries and terminal
  tabs (wave 3). Closing the last tab hides the panel; Cmd/Ctrl+J toggles; height persisted.
- Every pane tab is wrapped in an `ErrorBoundary` (Z-89).

### 3.9 Phone vs desktop

One codebase (D-23); `html[data-shell=desktop]` (existing) switches the shell. Phone keeps the
unipi shell (Z-115):

| Desktop | Phone |
|---|---|
| sidebar hosts → projects → sessions | **Chat** tab = the same list component, full screen; host groups collapsible |
| rail Board / Cron / ext pages / Settings | bottom tab bar Chat · Board · Settings; Cron + ext pages under "More" in the Chat header menu |
| side-pane tab | full-screen **layer** (`lib/back.ts`), opened from the chat ⋯ "Panes" group and the capsule |
| bottom panel | full-screen layer (terminal toolbar capsule, wave 3) |
| dialogs in bottom dock | same dock, collapsible to a one-line "Question waiting" bar (Z-46) |
| hover turn-tail actions | always visible, compact; tap-to-select + swipe-to-quote kept |
| find bar | top bar under the header |
| side chat / tree / subagent tabs | bottom sheets (corvu), large snap |
| modal (overlay slot, Session modal, model picker) | bottom sheets |

Geometry thresholds: desktop shell when `data-shell=desktop` (Tauri desktop or web ≥ 900 px);
navigator rail (Z-25) only ≥ 1100 px conversation width.

### 3.10 Where status lives (RULINGS Z-95, Z-43, G-4, G-20)

| Information | Home | Not shown in |
|---|---|---|
| Git changes, branch, actions | status pane (`git` tab) → Git section | (Files branch chip opens the same tab) |
| Long-horizon state (goal/ralph/swarm/graph progress, steps, pause/stop, history) | status pane → LH section (reserved `unipi.long-horizon/visualize`; ProgressSheet content) | no sheet, no header popover body (ModeChip only switches mode and shows a live count) |
| Todos / plans (workflow `plan_submit`, todo tools) | status pane → Plan & todos section (Z-43); plan **approval** stays a dialog | no inline todo cards |
| Running work (bg tasks, subagents, sidekick, work-tray tabs) | status pane → compact **Running summary** (counts + up to 3 rows) that opens Session modal → Running | the full list exists only in the Session modal (D-7) |
| Context, cost, Info pages | Session modal Context / Info tabs (ring in composer opens Context, Z-73) | no header strip/tray (Z-75) |
| Extension `setStatus` / `setFooter` segments | **status strip**: one 24 px row at the bottom of the conversation frame (under the composer), overflow popover; phone: strip under the header (≤ 4 segments) | not in the capsule |
| Collapsed-pane capsule | top-right of the timeline: branch + `+N −M`, LH mode + progress, running count; click opens/focuses the status pane | replaces unipi's top-right capsule |

The `work-tray.tab` registrations (G-20) of dream/subagents/bg render as sections of Session
modal → Running (their tree via U9 `NodeView`); the `core` work-tray `editorReplace` panel is not
opened by the app (the app intercepts `Shift+↓` to open Session modal → Running instead).

### 3.11 Bottom dock and dialogs (Z-46; amends ui-protocol §3.2)

The composer region becomes a **bottom dock** (`components/dock/BottomDock.tsx`) inside the
timeline scroll viewport, `position: sticky; bottom: 0`, gradient fade above (Z-66). Top to bottom:
jump button anchor → widgets (`widget` slot) → **dialog panel** (pending pi/Oino dialogs, "1 of N"
pager, U10 renders the dialog body) → queue panel (Z-53) → composer tray (`editorReplace`, which
replaces only the composer) → composer → `widgetBelow`. Dialogs **dock above** the composer; the
composer stays usable (typing, steering, Stop). ui-protocol §3.2 "dialogs take over the
composer slot" is superseded by this ruling; U10 implements dialog bodies against
`BottomDock`'s `DialogPanel` slot.

### 3.12 LSP transport — decision: host-side language servers over the host protocol (D-10)

Like Zed (one server per worktree per language, owned by the project side, not the editor view):

- **Host**: new `apps/unipi/crates/unipi-host/src/lsp.rs` + `apps/unipi/crates/unipi-proto/src/lsp.rs`.
  - `lsp_servers{path}` → `{root, servers:[{language, name, command, available, install_hint}]}`;
    `root` = nearest ancestor of `path` with a marker (`Cargo.toml`, `package.json`, `tsconfig.json`,
    `pyproject.toml`, `go.mod`, `compile_commands.json`, `.git`) inside the fs scope.
  - `lsp_open{root, language}` = persistent bi-stream (same shape as `term_open`): `Ok{data:
    {server, attached: bool, initialize_result?}}`, then **raw LSP stdio bytes** in binary frames
    (kind 1 server→app, kind 2 app→server, ≤ 256 KiB chunks, Content-Length framing untouched),
    `Control::Exit{code}` when the server dies.
  - One server process per `(root, language)`; **one attached client** at a time: a new attach
    takes over and the old stream ends with `Err{code:"E_LSP_TAKEN"}` (single-writer spirit). The
    host answers a re-attaching client's `initialize` from the cached result and swallows its
    `initialized`; it sends `didClose` for the previous client's open documents on detach.
  - Idle shutdown 10 min after the last stream closes (`lsp.idle_minutes`); killed on host quit.
  - **Commands come only from host settings** (`config.json` key `lsp.servers.<language>.command`,
    defaults: rust-analyzer, `typescript-language-server --stdio` (ts/tsx/js/jsx),
    `pyright-langserver --stdio`, gopls, clangd, `yaml-language-server --stdio`,
    `vscode-json-language-server --stdio`, `taplo lsp stdio`, marksman). The app only names a
    language; a paired phone can never choose a binary. Root must be inside the fs scope; paired
    devices use the existing fs consent. No auto-download in v2 (install hint shown instead).
- **App**: `@codemirror/lsp-client` (MIT) with a `Transport` over `LspStream`
  (`lib/lsp/{framing,transport,pool,features}.ts`); one connection per `(hostId, root, language)`
  shared by all editor tabs. Features: diagnostics (lint gutter + Files-tree badges), hover,
  completion, signature help, go to definition / type definition (opens or focuses the editor tab),
  find references (results list in the Files tab), rename (workspace edit; open docs edited in place,
  closed files written with `fs_write{base_hash}`, conflicts listed), format document/selection,
  code actions. Sync: `didOpen` on open, incremental `didChange`, `didSave` after `fs_write`,
  `didClose` on tab close. Phone: diagnostics, hover on long-press, go to definition; completion
  from the editor key bar.
- Why not app-side servers: the phone has none, the files live on the host, and the host already
  owns PTY/fs streams with the same framing; Tauri-side spawning would only work on This computer.

### 3.13 `unipi.file-summary` extension (D-1)

- Crate `extensions/unipi/file-summary` (Rust → wasm32-wasip2, `unipi-kit`), manifest id
  `unipi.file-summary`, reserved id `unipi.file-summary/turn-files` (fixes the ui-protocol §8 row).
- **Snapshot before write/edit**: on `tool-call` for the built-in `write` and `edit` tools (before
  execution), the first time a path is touched in the current turn: read it (wasi fs, `project` +
  manifest fs paths); absent ⇒ record `existed:false`; ≤ 5 MB ⇒ zstd-compress, sha256, store
  `ext-session/blobs/<sha>.zst` (content-addressed, dedup); > 5 MB ⇒ record `skipped:"too_large"`
  (listed, no undo). Bash and extension tools are not captured (like ZCode). The in-progress turn
  index is written to kv (`turn:<turnEntryId>`) at each snapshot (crash-safe); at `agent_end` the
  extension appends a session `custom` entry `unipi.file-summary/turn` `{turnEntryId, files:[{path,
  existed, beforeSha?, afterSha?, add, del, skipped?}]}` (line counts via `similar`), so state is
  rebuildable from custom entries + kv (DIRECTION §3.1.6). Blobs live in the session's
  `ext-session` dir and are deleted with the session.
- The turn id is the id of the user message entry that started the run (`turnEntryId`, equal to
  the app's `turnId`).
- **Services** (called by the app with `_oino/service_call{target:"unipi.file-summary", name,
  payload}`): `turn{turnEntryId}` → summary; `diff{turnEntryId, path}` → unified diff before →
  current (≤ 1 MiB); `undo{turnEntryId, paths?, force?}` → `{restored, deleted, failed,
  conflicts}` (conflict = file changed since the turn ended, needs `force`); `undoFrom{turnEntryId,
  force?}` → undoes this and all later turns newest-first (used by Z-28 "Edit · also undo file
  changes"). Each undo appends a `unipi.file-summary/undo` custom entry and calls
  `invalidateTurnFooter`.
- **Footer**: `register-renderer{target: turn-footer, key: "files"}` returns a node tree (TUI and
  non-native frontends: "3 files +12 −04" + file list). The app implements the reserved component
  natively (WP-A26) inside the turn row.
- Core `session/checkpoints.rs` (`_oino/checkpoints` / `_oino/rewind`) stays for the TUI; the app
  does **not** use it (one surface). ui-protocol §8's data-path cell for this row becomes "the
  extension's services".

### 3.14 Host deltas this spec needs (not in host.md)

Each lands inside the app WP named, after the host WP it extends; files are the host/proto/Tauri
files listed in that WP. All additive (proto minor bump within 2.x).

| Delta | Ops / events | WP | After |
|---|---|---|---|
| Session meta | `session_meta_set{session, pinned?, unread?, title_locked?}`; `session_focus{session|null}` per connection; `SessionCard` gains `pinned`, `unread`, `title_locked`, `source` (`cron`), `changes {add, del}`, `last_activity`; stored `~/.oino/host/session-meta.json`. Host sets `unread=true` when a session ends a run / needs the user while **no** connection has it focused (Z-86). | A4 | H4 |
| Archive list | `sessions_list{archived:true}`, `session_restore{session}`, `session_delete{session}` (from trash only), `trash_empty` | A5 | H3/H4 |
| Hub-level commands | `host_cmd{cmd}`: settings/auth/extension subset of `_oino/*` (`get_settings`, `set_setting`, `get_settings_schema`, `get_extension_settings`, `set_extension_setting`, `unset_extension_setting`, `extensions`, `set_extension_enabled`, `auth_providers`, `login`, `logout`, `set_api_key`, `get_shortcuts`) against core services without a session runtime | A23 (skip if H4 already provides it) | H4 |
| Git watch | `git_watch{root, on}`; `HubEvent::GitChanged{root}` on `events_open` (metadata 1 s debounce, worktree 60 s, ZCode `git-auto-refresh.md`) | A15 | H7 |
| Board | `board_cli{args, project?}` (runs `unipi-kanboard … --json` as actor user, subcommand allow-list, ext-abi §5.5 contract); `board_watch{on}` → `HubEvent::BoardChanged{project}` | A21 | H3, X3 |
| Cron source | `SessionCard.source = "cron"` + `cron_job` id for cron-run sessions; `cron_*` DTO changes from WP-H8 consumed | A22 | H8 |
| LSP | `lsp_servers`, `lsp_open`, host settings key `lsp` | A20 | H2, H7 |

### 3.15 Capture harness (WP-A1)

- `vite.config.capture.ts`: like `vite.config.chat-capture.ts`, port 1450; a plugin swaps
  `lib/host/factory.ts` to return `MockHost` when the URL has `scenario=`.
- `src/dev/mock/host.ts` — `MockHost implements HostClient`: in-memory hosts, cards, sessions, fs
  tree, git status, board/cron data; every op answered; commands recorded on
  `window.__UNIPI_MOCK_LOG__` for assertions.
- `src/dev/mock/gen.ts` — deterministic generators: sessions with N turns, tool mixes, diffs,
  thinking, images, steers, compaction, errors, subagents; `src/dev/mock/fixtures/*.json` — a real
  recorded session converted to oino entries (archive `docs/m5/fixtures/rec-math.json`).
- `src/dev/mock/scenarios/<name>.ts` — `{hosts, cards, sessions, script}`; `script` replays
  `SessionEventDto`s on a virtual clock (`speed=0` settles instantly, `live=1` streams).
- Scenario names are listed per WP; screenshots go to `docs/dev/v2/design/shots/app/`.
- Component tests use the same `MockHost` (`createMockHost(scenario)`), so captures and tests
  share fixtures.

### 3.16 Amendments to sibling specs (made by this spec)

1. ui-protocol §3.2: dialogs dock above the composer (§3.11), not replacing it.
2. ui-protocol §6.2 `status` row: final home = status strip (§3.10); `sidePane`/`bottomPane` phone
   = layers from the chat ⋯ "Panes" group (§3.9).
3. ui-protocol §8: `<file-summary-ext>/turn-files` = `unipi.file-summary/turn-files`, data via its
   services (§3.13); `oino.browser/browser` reads `unipi.browser/browser` (RULINGS ext-abi §11).
4. ui-protocol / browser.md paths `apps/unipi/apps/mobile/src/…` read `apps/unipi/src/…`.
5. host.md: the deltas of §3.14.
6. browser.md WP-B8 builds on A3 (side pane), A11 (composer chip API `composerChips.add`), A12
   (screenshot → attachment through the image editor), and owns Z-67/Z-108..Z-110.

## 4. Rulings applied to "ask" items

Owner: "take Rec; where Rec is ask, take the option the Rec text recommends". Where the Rec
text names no preferred option, this spec keeps today's behaviour and adds nothing (rule 2).

| Item | Applied as |
|---|---|
| Z-7 split panes | deferred; not built |
| Z-9 / Z-10 views | by project only, running-first + needs-you-first; **no** flat "Recently updated" view, no user groups; relative time on each row |
| Z-12 marquee | no marquee; fade mask on overflow, full title in tooltip |
| Z-18 tree | side-pane tab on desktop, sheet on phone |
| Z-20 export/share/delete | Export (JSONL via `_oino/export_jsonl`, Markdown rendered by the app) + Delete (confirm) in the row menu; no web share links |
| Z-24 unread on jump button | yes, small dot |
| Z-28 reset with files | yes: Edit offers "Also undo file changes from this turn on" (file-summary `undoFrom`) |
| Z-32 media cards | no inline cards; file refs as today |
| Z-33 stream display settings | skipped |
| Z-34 fade-in | static-once-complete rule only, no fade-in |
| Z-43 todos/plans | status pane section (RULINGS) |
| Z-49 retry | Retry in the turn-tail bar of the **last** turn (tree jump to its user message + resend) |
| Z-51 lock elsewhere | notice "Taken over from the terminal" in the winner; read-only banner + "Take back" in the loser |
| Z-61 long paste | ≥ 5120 chars → "Pasted text · N lines" chip, expanded inline on send (RULINGS) |
| Z-62 `#` / `$` | not added (skills stay in `/`, sessions in the palette) |
| Z-65 dictation | deferred (RULINGS) |
| Z-68 suggested prompts | skipped; empty state shows a Unicrab lore line (Z-87) |
| Z-74 mode control | ModeChip stays in the **header** as the switch (+ Ctrl+Shift+M cycles); progress lives in the status pane |
| Z-75 session strip/tray | dropped; one-line cost in the turn end |
| Z-80 extension commands in palette | yes: Actions scope lists the session's `/` commands (runs insert `/cmd ` into the composer) |
| Z-92 rolling labels | yes (model, thinking, mode labels; reduced motion = instant) |
| Z-94 Ctrl+B conflict | shell wins: Cmd/Ctrl+B = sidebar. Extension chords that collide with app chords are rebound by the registry to an app-chosen alternate shown in Ctrl+/ and Settings; subagents "all to background" gets **Alt+B** and a button in the Running summary |
| Z-95 status panel | RULINGS: pane = Git + LH + todos/plans + Running summary; Session modal = only full Running view; push only ≥ 1280 px |
| Z-99 git graph | skipped |
| Z-101 Files home | side-pane tab only (no Files rail screen) |
| Z-103 Office/PPTX | skipped |
| Z-104 Preview overlap | ports list = "Local ports" in the side-pane `+` launcher (opens the browser tab when wave 3 lands, the forward viewer before that and on the phone); Preview screen deleted |
| Z-108 responsive viewport | yes (owned by WP-B8) |
| Z-111 Add task | `+` menu only (chat ⋯ entry removed); task refs in text stay tappable |
| Z-114 appearance | UI font size 12–20 px (`--ui-font-size`) + code font size/wrap/line numbers; **no** light theme |
| G-14 updaters | one Update card in Settings › About fed by the app updater + host update row + the `updater` extension's check result; the extension's overlay is never opened by the app (`/readme` opens as a preview tab) |

## 5. Work packages

Each WP is sized for one worker session, ends green (§1 gates), commits only its files with
`[app-a<n>]`, and writes its screenshots to `docs/dev/v2/design/shots/app/`. "Scenarios" are
capture-harness scenarios (§3.15) the WP adds and screenshots at desktop 1280×820 and phone
412×915 unless stated. Host WPs are `H<n>` (host.md), UI-protocol WPs `U<n>`, ext-abi `X<n>`.

### WP-A1 — Prune v1 surfaces, HostClient seam, capture harness
- **Scope**: delete the D-14/D-15 code (§2 "herdr/tmux/streaming" row, `CloseChatSheet`,
  `ResumeSheet`, `DreamSheet`, `lib/chat/{progress,lhMode,dream,transport,list,newChat,open,
  closeTarget,bridgeError,boardPortal}.ts` callers stubbed behind `lib/session` placeholders, herdr
  parts of `screens/Terminal.tsx`, `screens/Preview.tsx` desktop path) and their tests; add
  `lib/host/{client,tauri,mock,factory}.ts` (§3.2) and route existing one-shot ops (fs, git, blob,
  push, forward, sharing, cron, pairing) through `request()`; add the capture harness (§3.15) and
  delete `vite.config.chat-capture*.ts`; re-point `NoHostState`/`OfflineState`/`Requires` to
  HostClient connection state (Z-90).
- **Files**: deletions above; `src/lib/host/*`, `src/dev/mock/{host,gen}.ts`,
  `src/dev/mock/scenarios/{empty,no-host,offline}.ts`, `src/dev/mock/fixtures/rec-math.json`,
  `vite.config.capture.ts`, `src/components/{EmptyState,Requires,ConnectionState}.tsx`,
  `src/main.tsx` (routes of deleted screens removed), `src/lib/{fs,git,blob,push,forward,sharing,
  cron}.ts` (transport only).
- **Deps**: H1 (done). Can run before H4 (session types stubbed from host.md §7.3).
- **Acceptance**: `rg -n "herdr|tmux|chat_list|chat_open|mux_|desktop_open|screen_open" src` returns
  only comments/migration notes; typecheck + vitest green (deleted suites removed, not skipped);
  `lib/host/mock.test.ts` covers request routing + recorded log; capture `empty`, `no-host`,
  `offline` scenarios render (screenshots, both shells).
- **Profile**: opus-ui. **Closes**: Z-90; D-14, D-15.

### WP-A2 — Session data layer on the host session protocol
- **Scope**: `lib/session/` stream client, reducer, turns index, outbox, hub store (§3.3–3.5):
  snapshot/replay/reset, rAF-batched event apply, `cmd()` correlation, reconnect with `sinceSeq`,
  LRU 8 live sessions, hub cards from `sessions_list` + `SessionChanged`, notices from
  `HostNotice`, banners for `crashed`/`taken_over`/`reloaded`; outbox bubbles + "Lost the
  connection · Retry" banner (Z-50); `buildTurns` with segments, fold summary, tool-group
  classification, tail data; `lib/session/selectors.ts` (title, cwd, running, needs-you, cost).
- **Files**: `src/lib/session/{stream,store,reducer,turns,hub,outbox,selectors,types}.ts` + tests;
  `src/lib/host/tauri.ts` (session/event channel wiring to H4's `chat.rs` commands);
  `src/dev/mock/scenarios/{stream-basic,stream-steer,stream-reconnect}.ts`.
- **Deps**: A1, H4 (`packages/protocol/src/session.ts` + `src-tauri/src/chat.rs`).
- **Acceptance**: reducer tests from H4's JSON fixtures (`packages/protocol` session fixtures):
  snapshot → replay → live deltas → `message_end` equals a fresh snapshot of the same session
  (deep-equal); reset path; seq gap ⇒ reopen with `sinceSeq`; outbox lifecycle (pending → acked →
  lost); `turns.test.ts` for preamble, segments on steer, fold forced open on running/stopped/error,
  tool groups, compaction, `/compact` echo flag; incremental tail recompute (spy: settled turns keep
  identity); `turns.bench.ts`: 2 000-turn build ≤ 25 ms, tail update ≤ 0.5 ms. No UI screenshots
  (data only) except `stream-reconnect` banner via the old ChatView adapter if still mounted.
- **Profile**: opus-ui-high. **Closes**: Z-50; D-9 (data).

### WP-A3 — App shell, routing, side pane + bottom panel containers
- **Scope**: §3.7–3.8 desktop shell: `AppShell` frame, sidebar frame (content placeholder until
  A4), rail (Board/Cron/ext pages/Settings; Z-1), header (Z-2), collapse + widths + resize
  auto-collapse (Z-3), handles (Z-4), `SidePane` tabs container with `+` launcher, overview,
  close variants, recently closed, reorder, push-vs-overlay at 1280 px (Z-5), full view (Z-6),
  `BottomPanel` container, status strip row, tab-kind registry, `ErrorBoundary` per tab, routes of
  §3.7; "Local ports" launcher section + Preview screen removal (Z-104); split panes explicitly not
  built (Z-7). Deletes `components/shell/DesktopShell.tsx` NAV/Ctrl+1..6 screens model and
  `lib/shell.ts` list|detail panes.
- **Files**: `src/components/shell/{AppShell,Sidebar,Rail,ConversationHeader,ResizeHandle,
  StatusStrip}.tsx`, `shell.css`; `src/components/panes/{SidePane,TabStrip,TabOverview,
  PaneLauncher,BottomPanel}.tsx`, `panes.css`; `src/lib/{panes,layout}.ts`, `src/lib/panes/kinds.ts`;
  `src/main.tsx`; tests; scenarios `shell-desktop`, `shell-narrow`, `shell-sidepane-tabs`,
  `shell-fullview`.
- **Deps**: A1, A2.
- **Acceptance**: `panes.test.ts` (open/close/reorder/recently closed/scope switch/singletons/
  persistence), `layout.test.ts` (auto-collapse thresholds 480/360, restore rules, 1280 push rule);
  `AppShell.test.tsx` (routes, Cmd/Ctrl+B/J/Alt+B/Shift+F, header ⋯ items); screenshots at
  1440×900, 1280×820, 1000×800 (overlay mode) and phone (unchanged tab bar); visual check against
  ZCode `docs/ui/side-pane-tabs.md`, `workspace-layout.md` in the unipi look.
- **Profile**: opus-ui-high. **Closes**: Z-1, Z-2, Z-3, Z-4, Z-5, Z-6, Z-7, Z-104.

### WP-A4 — Sessions sidebar (hosts → projects → sessions)
- **Scope**: sidebar content and phone Chat list (one component): host groups (This computer
  first, paired PCs collapsible, Z-8; PcChip kept only on Board/Cron), project groups from card
  `cwd` (D-9), ordering pinned → running → needs-you → recent (Z-9), relative time (Z-10), 5 per
  project + "Show more" (+5) (Z-11), row content: state glyph + word + mode badge + title with fade
  mask + relative time + `+N −M` + unread dot + cron clock badge slot (Z-12), Pinned group + Pin/
  Unpin (Z-14); host delta **session meta** (§3.14) incl. `session_focus` reporting.
- **Files**: `src/components/sidebar/{SessionList,HostGroup,ProjectGroup,SessionRow}.tsx`,
  `sidebar.css`; `src/lib/session/hub.ts` (meta); `screens/Chat.tsx` (phone list → SessionList);
  `apps/unipi/crates/unipi-proto/src/session.rs` (meta DTOs), `apps/unipi/crates/unipi-host/src/sessions/meta.rs` (+ hub
  wiring, test `tests/session_meta.rs`), `apps/unipi/packages/protocol/src/session.ts` (meta zod),
  `src-tauri/src/chat.rs` (op passthrough); scenarios `sidebar-many` (3 hosts, 12 projects, 300
  sessions), `sidebar-states`.
- **Deps**: A2, A3, H4.
- **Acceptance**: ordering/paging/grouping unit tests; meta round-trip test (Rust + TS fixture);
  unread set only when unfocused (host test); sidebar with 300 sessions renders ≤ 16 ms (bench,
  rows virtualized when a group > 50); screenshots desktop + phone; tooltips on icon-only controls.
- **Profile**: opus-ui. **Closes**: Z-8, Z-9, Z-10, Z-11, Z-12, Z-14.

### WP-A5 — Session actions, archive, new-chat draft
- **Scope**: row context menu (right-click / long-press): Pin, Rename, Archive, Mark as unread,
  Fork, Copy id / path / log path, Reveal folder, Export, Delete (Z-16); header ⋯ keeps
  chat-scoped actions only; Archive with inline confirm state, "Archived" list per host with
  Restore and Delete all archived (Z-13; host delta archive list §3.14); Rename dialog from menu +
  `/name`, manual name locks auto-rename (`title_locked` meta; `extensions/unipi/utility/src/rename/
  mod.rs` skips locked sessions) (Z-15); Export JSONL/Markdown + Delete with confirm (Z-20); one
  "New chat" (Cmd/Ctrl+N) + per-project `+` opening the draft composer with host + folder pickers
  (`fs_roots`, recent cwds), first prompt creates the session via `session_new` (Z-19); deletes
  `NewChatSheet` herdr/tmux steps and the "New session"/"New chat here" duplicates.
- **Files**: `src/components/sidebar/{SessionMenu,ArchivedList,RenameDialog}.tsx`,
  `src/components/draft/{DraftView,HostPicker,FolderPicker}.tsx`, `src/lib/session/actions.ts`,
  `src/lib/export.ts`; `apps/unipi/crates/unipi-host/src/sessions/{hub.rs (restore/delete/trash_empty)}`,
  `apps/unipi/crates/unipi-proto/src/session.rs`, `packages/protocol/src/session.ts`, `src-tauri/src/chat.rs`;
  `extensions/unipi/utility/src/rename/mod.rs`; scenarios `session-menu`, `archived`, `draft`.
- **Deps**: A4, H3, H4.
- **Acceptance**: component tests for each menu action (MockHost log asserts the op), archive →
  restore round trip, delete-all confirm, draft → first prompt creates and routes to
  `/s/:host/:id`; host test `archive_restore`; utility harness test: locked title not renamed;
  screenshots of menu, archived list, draft (desktop + phone).
- **Profile**: opus-ui. **Closes**: Z-13, Z-15, Z-16, Z-19, Z-20.

### WP-A6 — Timeline core: turn-unit virtualization (D-8)
- **Scope**: §3.6 in full: `Timeline` (virtual settled turns + live tail), `TurnRow`, `SegmentFold`
  "Worked for … / Working for … / Stopped" with unipi summary wording and the live clock (Z-21,
  Z-22, Z-56), steer segments (Z-54), bottom lock + jump button + unread dot (Z-23, Z-24), scroll
  memory, height cache, expansion store, static markdown once complete (Z-34), compaction divider
  with hidden `/compact` echo (Z-35), harness/command rows + "Sent by cron" label (Z-36), loading
  skeleton + image shimmer (Z-88), bottom dock placement inside the viewport with gradient (Z-66,
  the dock's contents come from A9/A11/A12); Z-33 display toggles not built. Old `ChatView.tsx`
  row virtualizer, `RunningRow`, `ProcessGroup`, `TurnEnd` deleted.
- **Files**: `src/components/timeline/{Timeline,TurnRow,LiveTail,SegmentFold,UserBubble,Prose,
  Compaction,HarnessRow,JumpButton,Skeleton}.tsx`, `timeline.css`;
  `src/lib/timeline/{heightCache,scrollMemory,expansion,bottomLock,mdCache}.ts`;
  `src/components/chat/ChatView.tsx` → `src/components/conversation/Conversation.tsx`; tests +
  `timeline.bench.ts`; scenarios `chat-long` (2 000 turns), `chat-streaming` (live), `chat-steer`,
  `chat-stopped`, `chat-compaction`, `chat-cron`.
- **Deps**: A2, A3.
- **Acceptance**: `heightCache.test.ts` (LRU, refresh order, estimate), `bottomLock.test.ts`
  (48 px detach, user intent beats layout, re-lock), `scrollMemory.test.ts` (LRU 200, restore +
  second correction yields to user scroll), `Timeline.test.tsx` (live tail migrates without offset
  jump; fold forced open while running; jump button only when detached and overflowing); §3.6
  budgets measured in the capture page (`performance.measure` marks dumped to
  `window.__UNIPI_PERF__`) and recorded in `docs/dev/v2/qa/app-perf.md`; screenshots of every
  scenario incl. mid-stream and detached-with-unread states.
- **Profile**: opus-ui-high. **Closes**: Z-21, Z-22, Z-23, Z-24, Z-33, Z-34, Z-35, Z-36, Z-54,
  Z-56, Z-66, Z-88; D-8.

### WP-A7 — Turn content and turn-tail actions
- **Scope**: user bubble fold 120 px + fade + centred button, chips outside the fold (Z-29); turn
  tail bar bottom-left, hover-revealed (always on phone): Copy (whole turn, glyph→check 1.2 s,
  Z-93), Quote, Fork (any message also via row menu, Z-17), Retry on the last turn (Z-49), More;
  time label `HH:MM` / `Yesterday HH:MM` / `M/D` / `Y/M/D` + "Done in · $cost" on the same line
  (Z-26, Z-27); Edit on the user bubble = tree jump into the composer, with "Also undo file
  changes" when file-summary is enabled (Z-28); selection → floating "Quote" → quote chip in the
  composer, swipe-to-quote kept on phone (Z-30); file refs kept, no media cards (Z-32); query
  navigator rail ≥ 1100 px with tooltip excerpts and exact jumps (Z-25).
- **Files**: `src/components/timeline/{TurnActions,UserBubble,SelectionQuote,TurnNavigator,
  TimeLabel}.tsx`, `src/lib/timeline/{timeLabel,actions}.ts`; scenarios `turn-actions`,
  `turn-navigator`, `bubble-fold`.
- **Deps**: A6 (A27 for the undo-files option; hidden until present).
- **Acceptance**: `timeLabel.test.ts` (all four formats + locale), action tests via MockHost
  (fork → `session_fork`, retry → `_oino/navigate_tree` + `prompt`, edit restores text+images into
  the composer), navigator jump lands the turn at the top and detaches; screenshots hover/phone.
- **Profile**: opus-ui. **Closes**: Z-17, Z-25, Z-26, Z-27, Z-28, Z-29, Z-30, Z-32, Z-49, Z-93.

### WP-A8 — Tool rows, thinking, cards and extension renderers
- **Scope**: keep unipi tool row anatomy with hover-only chevron (Z-37); Execute / Explored /
  Changed groups inside the fold (Z-38); bodies: run, edit diff, write (markdown preview for `.md`),
  read/search/web/memory heads, todo and mcp renderers, In/Out fallback (Z-39); thinking row
  "Thinking…" shimmer → "Thought for 12s" + one-line preview, hidden when empty (Z-41); image
  chips + previewer kept (Z-44); code blocks kept (Z-45); lazy full output via `get_entries`;
  **extension renderers** for `customMessage` / `customEntry` / `tool` through `_oino/render` +
  U9 `NodeView`, replacing the hard-coded unipi 3.x content types in `Cards.tsx` (G-21); native
  cards kept: model change, compaction, harness.
- **Files**: `src/components/timeline/tools/{ToolRow,ToolGroup,ToolBody,ThinkingRow,bodies/*}.tsx`,
  `src/components/timeline/cards/{RendererCard,ModelChangeCard}.tsx`, `src/lib/session/renderCache.ts`;
  delete `components/chat/Cards.tsx`, `Transcript.tsx`; scenarios `tools-mix`, `tools-groups`,
  `renderer-cards`.
- **Deps**: A6, U9.
- **Acceptance**: grouping tests; render cache keyed `(target,key,entryId,rev,width)` with 2 s
  timeout ⇒ nothing rendered; every unipi card type from U7b fixtures renders through NodeView
  (golden DOM snapshots); collapsed tool rows mount no body DOM (test); screenshots.
- **Profile**: opus-ui. **Closes**: Z-37, Z-38, Z-39, Z-41, Z-44, Z-45, G-21.

### WP-A9 — Bottom dock, dialog placement, errors, crash containment
- **Scope**: `BottomDock` (§3.11) with ordered regions and the `DialogPanel` frame (pager "1 of N",
  collapse to "Question waiting" bar on phone; composer stays usable) (Z-46); approval layout in
  unipi style — summary, detail area (max 40 vh), Allow once / Always / Deny / Deny with note, origin
  badge (Z-47; U10 plugs the `approval` kind into it); error rows + "Details" disclosure with the
  raw error and Copy (Z-48); takeover notice "Taken over from the terminal" and the loser's
  read-only banner with "Take back" (re-`session_open`) (Z-51); Solid `ErrorBoundary` at root
  (Retry / Reload app) and per conversation, pane tab and slot entry (Z-89). Moves
  `DialogCard.tsx` to `src/ui/dialogs/` unchanged for U10.
- **Files**: `src/components/dock/{BottomDock,DialogPanel,ApprovalPanel}.tsx`, `dock.css`,
  `src/components/timeline/ErrorRow.tsx`, `src/components/conversation/{TakeoverBanner,
  ReadOnlyBanner}.tsx`, `src/components/ErrorBoundaries.tsx`; scenarios `dialog-dock`,
  `dialog-approval`, `error-row`, `taken-over`, `crash-boundary`.
- **Deps**: A6, A3.
- **Acceptance**: typing + Esc/Stop work while a dialog waits (test); pager order = arrival order;
  phone collapse; taken-over event ⇒ composer disabled + banner, Take back reopens; a throwing pane
  tab shows its boundary while the chat keeps streaming (test); screenshots.
- **Profile**: opus-ui. **Closes**: Z-46, Z-47, Z-48, Z-51, Z-89.

### WP-A10 — Session side tabs: diff, tree, side chat, subagents
- **Scope**: `diff` tab = full diff viewer (ReviewViewer port, unified/split on desktop, open in
  editor at line) opened by "View diff" from tool rows (Z-40); `tree` tab = TreeSheet port with
  filters, Go to this point, Start a new branch here, restore into composer (Z-18); `btw` tab =
  multi-turn side chat with btw semantics (read-only, never touches the main session) via
  `_oino/service_call` `btw.ask` / `btw.list`, "Ask in main chat"; the app intercepts `/btw` and
  the `+` → Side question entry so the extension's `editorReplace` panel is not opened (Z-31);
  `subagent` tab = child transcript (`_oino/agents`, `_oino/agent_messages`, Stop via
  `_oino/agent_abort`) opened from subagent rows and cards; deletes the completion-card "Open
  session" paths (Z-42). Phone: sheets.
- **Files**: `src/components/panes/tabs/{DiffTab,TreeTab,BtwTab,SubagentTab}.tsx`,
  `src/lib/panes/kinds.ts` (registrations); delete `components/chat/{ReviewViewer,BtwSheet}.tsx`,
  `SessionMenus.tsx` TreeSheet; scenarios `tab-diff`, `tab-tree`, `tab-btw`, `tab-subagent`.
- **Deps**: A3, A6, A8.
- **Acceptance**: each tab opens from its entry point, persists per session, closes with the
  session; btw never sends to the main session (MockHost log); subagent tab streams child events;
  screenshots desktop + phone sheets.
- **Profile**: opus-ui. **Closes**: Z-18, Z-31, Z-40, Z-42.

### WP-A11 — Queue, steering, stop
- **Scope**: global setting follow-up mode (queue | steer); Enter = default, Cmd/Ctrl+Enter = the
  opposite; chevron menu keeps "Send now (stops run)" (Z-52); queue panel in the dock: drag reorder
  on desktop, arrows on phone, Edit = recall into composer refused when a draft exists, Remove,
  Send now, held queue after interrupt with Resume and auto-drain (`_oino/queue_edit`,
  `clear_queue`) (Z-53); Esc stops from any focus in the conversation unless a dialog/popup
  consumed it; Send ↔ Stop (Z-55). Deletes `lib/chat/sendChoice.ts` and the S/Q inline choice.
- **Files**: `src/components/dock/QueuePanel.tsx`, `src/lib/composer/{sendMode,escapeStop}.ts`,
  `src/components/composer/SendButton.tsx`; scenarios `queue`, `queue-held`.
- **Deps**: A9, A2.
- **Acceptance**: unit tests for mode/modifier matrix, recall guard, held-queue resume, Esc
  consumption rules; MockHost log for `steer`/`follow_up`/`abort`/`_oino/queue_edit`; screenshots.
- **Profile**: opus-ui. **Closes**: Z-52, Z-53, Z-55.

### WP-A12 — Composer core
- **Scope**: split `Composer.tsx` into `components/composer/*`; placeholder by state (empty session
  capability hint, follow-up, running "Keep typing to queue…", phone short form) (Z-57); ↑/↓ prompt
  history per workspace, 30, no consecutive dups (Z-58); persisted text drafts per session (removed
  when emptied) + in-memory chips/attachments (Z-59); paste ≥ 5120 chars → "Pasted text · N lines"
  chip, expanded inline on send (Z-61); `/` (commands, prompt templates, skills with source labels)
  and `@` files kept, no `#`/`$` (Z-62); `+` menu: Photo, Camera, File, Side question, Board task…
  (no New session/Resume) (Z-63); dictation not built (Z-65); no suggested prompts (Z-68); public
  chip API `composerChips.add(sessionKey, chip)` for B8 element/screenshot chips.
- **Files**: `src/components/composer/{Composer,ChipEditor,SlashMenu,MentionMenu,PlusMenu,
  PastedTextChip}.tsx`, `src/lib/composer/{drafts,history,paste,chips}.ts`; delete
  `components/chat/Composer.tsx`, `lib/chat/{paste,chips}.ts` after moving; scenarios
  `composer-states`, `composer-paste`, `composer-slash`.
- **Deps**: A9.
- **Acceptance**: draft persistence across reload (adapter mock), history walk rules, paste
  threshold 5119/5120 and inline expansion in the sent `prompt` text (MockHost log), undo/redo kept;
  screenshots of every placeholder state and menus.
- **Profile**: opus-ui. **Closes**: Z-57, Z-58, Z-59, Z-61, Z-62, Z-63, Z-65, Z-68.

### WP-A13 — Attachments, images, composer toolbar, permission/plan control
- **Scope**: attachment states queued → uploading % → ready / failed + retry (auto once), send gate,
  max 8 (Z-60); **ellipse/circle tool** in `ImageEditor.tsx` (drag bbox, Shift = circle, stroke
  colour/width as the other shapes, undo) (D-5); pending images move from the `images` signal to a
  per-chat store beside `pendingFilesFor` so they survive chat switches (D-6); toolbar order and
  seven-step compression on narrow widths (Z-64) with a **permission/plan mode control**: chip
  "Ask / Auto / Full" + Plan toggle bound to the workflow extension's commands (`/permission`,
  `/plan`) and showing its `WORKFLOW_STATUS` topic (G-6).
- **Files**: `src/components/composer/{Toolbar,AttachmentChip,PermissionControl}.tsx`,
  `src/components/chat/ImageEditor.tsx` → `src/components/composer/ImageEditor.tsx`,
  `src/lib/{imageEdit,chat/attachments}.ts` → `src/lib/composer/{imageEdit,attachments}.ts`;
  scenarios `composer-attachments`, `image-editor-ellipse`, `composer-narrow`.
- **Deps**: A12.
- **Acceptance**: ellipse shape tests (hit test, serialization into the flattened PNG), D-6
  regression test (attach → switch chat → back ⇒ images still there; separate per chat), upload
  state machine tests, compression order at 6 widths (test), permission chip reflects topic +
  sends the command; screenshots incl. the editor with an ellipse.
- **Profile**: opus-ui. **Closes**: Z-60, Z-64, G-6; D-5, D-6.

### WP-A14 — Model, thinking, context, working mode
- **Scope**: model picker sheet kept but wider, shared search over Single and Fusion columns,
  sentence-case "Now …", compact right-aligned buttons, Vision/context badges, Ctrl+M (Z-69, Z-76);
  entry points only composer chip + Ctrl+M + `/model` (title and tray entries deleted) (Z-70);
  separate brain chip with level text, click = list, Ctrl+T cycles (Z-71); toast "Switched from X to
  Y" + model-change card kept (Z-72); context ring visible at 0 %, click/hover opens Session modal →
  Context (Z-73); ModeChip in the header as the switch with live count, Ctrl+Shift+M cycles, Goal /
  Ralph / Graph starters from it (Z-74); `FooterStrip`/`FooterTray` deleted, cost on the turn end
  (Z-75); rolling label animation for model/thinking/mode (Z-92).
- **Files**: `src/components/model/{ModelPicker,ModelChip,ThinkingChip,ContextRing}.tsx`,
  `src/components/conversation/ModeChip.tsx`, `src/components/RollingLabel.tsx`; `ControlCentre.tsx`
  split (picker out, strip/tray deleted); scenarios `model-picker`, `model-fusion`, `thinking-chip`,
  `mode-chip`.
- **Deps**: A13, A2.
- **Acceptance**: picker search shared across columns, `set_model` / `set_thinking_level` /
  `cycle_thinking_level` in MockHost log, shortcuts, rolling label honours reduced motion;
  screenshots.
- **Profile**: opus-ui. **Closes**: Z-69, Z-70, Z-71, Z-72, Z-73, Z-74, Z-75, Z-76, Z-92.

### WP-A15 — Git pane (status pane, Git section)
- **Scope**: re-home `GitView` into the `git` side-pane tab (reserved `host/git`): source select
  Unstaged / Staged / Branch (vs base) / **Last turn** (`GitCmd::TurnFiles` with the last turn's
  touched files from the turns index) + unipi grouping (Conflicts / Staged / Changes / Untracked)
  and History tab, virtualized file rows, lazy per-file diff, find in changes, reveal in tree, open
  in editor (Z-96); all unipi write actions kept inside the pane (Z-97); host **git watch** delta,
  refresh on `GitChanged`, no polling (Z-98); no graph (Z-99); entry points: capsule, side pane,
  Cmd/Ctrl+Alt+B, Files branch chip — no second git UI (Z-100).
- **Files**: `src/components/status/{StatusPane,GitSection}.tsx`, `src/components/git/*` (moved),
  `src/lib/git.ts`; `apps/unipi/crates/unipi-host/src/git_watch.rs` (+ hub event wiring, test
  `tests/git_watch.rs`), `apps/unipi/crates/unipi-proto/src/{git.rs,session.rs}` (watch op + event),
  `packages/protocol/src/{git,session}.ts`; scenarios `git-pane`, `git-last-turn`, `git-history`.
- **Deps**: A3, H7.
- **Acceptance**: last-turn file set derived from tool rows (test), watcher debounce test (Rust),
  MockHost `GitChanged` triggers one refresh; 2 000 changed files scroll at 60 fps; screenshots.
- **Profile**: opus-ui. **Closes**: Z-96, Z-97, Z-98, Z-99, Z-100.

### WP-A16 — Status sections: LH, plan & todos, Running summary, capsule
- **Scope**: status pane sections per §3.10: LH section = native component for reserved
  `unipi.long-horizon/visualize` (ProgressSheet content: mode, goal, elapsed, iterations, graph/
  steps, pause/resume/stop, history) fed by LH topics; Plan & todos section (workflow plan state +
  todo tool state; plan approval stays a dialog) (Z-43); Running summary (counts + 3 rows, "Open in
  Session" → Session modal Running, Alt+B all to background) (Z-95); work-tray tabs (G-20) route to
  Session modal Running sections; collapsed **capsule** top-right (branch ±, LH progress, running
  count); push ≥ 1280 px else overlay. Deletes `ProgressSheet.tsx`, the ModeChip popover body.
- **Files**: `src/components/status/{LhSection,PlanSection,RunningSummary,Capsule}.tsx`,
  `status.css`, `src/lib/status/{lh,plan,running}.ts`; scenarios `status-lh-graph`,
  `status-todos`, `status-running`, `capsule`.
- **Deps**: A15, A17, A2.
- **Acceptance**: section visibility follows owner enablement (LH off ⇒ section gone); summary
  counts equal Session modal counts (shared selector test); capsule visible when pane closed or in
  overlay mode; screenshots 1440 (push) and 1100 (overlay) + phone layer.
- **Profile**: opus-ui. **Closes**: Z-43, Z-95, G-20.

### WP-A17 — Session modal (Context / Running / Info)
- **Scope**: `ControlCentreSheet` → `SessionModal` as the native component of reserved
  `unipi.info-screen/dashboard` (D-7): Context tab (`_oino/get_context_usage`, breakdown, cache hit,
  compact actions); Running tab = the only full Running view (bg tasks with log / Stop / Re-run /
  Send to background via `_oino/running_bash` + bg services, subagents via `_oino/agents`, sidekick,
  work-tray tab trees via U9 `NodeView`); Info tab with sub-pages **Session, Usage, Tools, Skills,
  Modules** (G-13; `get_session_stats`, `get_commands`, `_oino/extensions`) and a **Limits**
  section for watchdog caps max turns / cost / wall / tool errors (G-16; new watchdog services
  `caps.get` / `caps.set` per session, persisted as a custom entry).
- **Files**: `src/components/session/{SessionModal,ContextTab,RunningTab,InfoTab,LimitsSection}.tsx`;
  `extensions/unipi/watchdog/src/{lib.rs,caps.rs}` (services) + harness test; scenarios
  `modal-context`, `modal-running`, `modal-info`, `modal-limits`.
- **Deps**: A2, A3, U9 (work-tray trees), X-ABI services (done).
- **Acceptance**: each tab from MockHost data; Running actions send the right commands; watchdog
  harness test: caps set via service stop the run at the limit; screenshots desktop modal + phone
  sheet.
- **Profile**: opus-ui. **Closes**: G-13, G-16; D-7.

### WP-A18 — Command center, find, shortcuts
- **Scope**: Cmd/Ctrl+K (and Cmd/Ctrl+Shift+P) command center with scopes All / Actions / Sessions
  / Files, prefixes `>` `#` `@`, empty state Recent sessions (3) + grouped commands, ≤ 3 per group +
  show more (Z-77); per-workspace search history chips (20, dedup) (Z-78); Cmd/Ctrl+F find bar over
  the conversation highlighting loaded text in the virtual timeline, ↑/↓/Enter/Shift+Enter wrap,
  "Search whole session" falls back to host search (SearchSheet logic) — one surface (Z-79); session
  `/` commands in Actions (Z-80); Cmd/Ctrl+P → Files scope (`fs_search`) (Z-81); shortcut registry
  with ZCode shell map + unipi screen/board/files/editor keys + Ctrl+/ list + rebinding in Settings
  (Z-94); extension shortcuts from `_oino/get_shortcuts` bound via `_oino/invoke_shortcut`, conflicts
  rebound per §4 (Alt+M permission, Alt+P plan, Ctrl+Alt+C bg clear, Alt+Enter fusion default, Alt+B
  subagents to background; Shift+↓ opens Session modal → Running) (G-7).
- **Files**: `src/components/command/{CommandCenter,FindBar}.tsx`, `src/lib/shortcuts/{registry,
  defaults,conflicts}.ts`, `src/lib/find/{index,highlight}.ts`; delete `components/shell/
  CommandPalette.tsx`, `components/chat/SearchSheet.tsx`, `lib/desktop.ts` shortcuts; scenarios
  `cmdk-empty`, `cmdk-files`, `find-bar`, `shortcuts-list`.
- **Deps**: A3, A4, A6.
- **Acceptance**: ranking + scope tests, find across virtualized turns (matches in unmounted turns
  scroll into view then highlight), conflict resolver tests (no extension chord shadows an app
  chord; table rendered in Ctrl+/), rebinding persisted; screenshots.
- **Profile**: opus-ui. **Closes**: Z-77, Z-78, Z-79, Z-80, Z-81, Z-94, G-7.

### WP-A19 — Notifications, unread, toasts
- **Scope**: persisted unread dot + Mark as unread (meta delta from A4) (Z-82); needs-you Unicrab +
  drawer kept, fed by hub `SessionChanged` instead of `chat_list` polling (Z-83); event-driven OS
  notifications from hub events with unipi seed + max-3 rules, suppressed for the focused session,
  no replay of cold snapshots; `DesktopNative` polling deleted (Z-84); one app-wide toast stack in
  the BoardToasts style (action, exit fade, hover pause), used by pi `notify` effects and app
  confirmations (Z-85); completion seen in foreground is not unread, minimizing right after marks it
  (Z-86); **notify re-spec, app side** (G-12): phone push settings use the host `notify_config` /
  `notify_register` E2E path (`unipi1:` payloads), `unipi://s/<host>/<session>` deep links route to
  the session, cron/needs-you pushes open the right screen; the notify extension's own channel
  config (Gotify/Telegram/ntfy, event matrix) appears via its settings schema (U11) with a "Send
  test" action.
- **Files**: `src/lib/{needsYou,notifyDeepLink,push,toasts}.ts`, `src/lib/notifications/route.ts`,
  `src/components/{NeedsYouBell,Toasts}.tsx`; delete `components/shell/DesktopNative.tsx` polling,
  `lib/needsYouPoll.ts` (A1 already); scenarios `needs-you`, `toasts`, `unread`.
- **Deps**: A4, H4 (hub events), H7 (push/notify).
- **Acceptance**: routing tests (focused session suppressed, seed on first look, max 3, no replay),
  deep-link parser tests, toast timing tests (info 4 s, warning 6 s, error sticky); manual phone
  push check recorded in the WP report; screenshots.
- **Profile**: opus-ui. **Closes**: Z-82, Z-83, Z-84, Z-85, Z-86, G-12.

### WP-A20 — Files tab: tree, preview, editor hosting
- **Scope**: `files` side-pane tab (reserved `host/files`): lazy `fs_list` tree, virtualized,
  compact folders, dotfiles shown, fuzzy name search + content search (`fs_search`), git-changed
  filter + badges, gitignore dim, watchers on expanded dirs (poll-free via `fs` events if H7 offers
  them, else refresh on focus), row menu Rename / Delete / Copy path / Mention in chat, upload by
  drop (Z-101); `preview` tab = Previewer (images with checkerboard, PDF, media, markdown/SVG
  preview ↔ source; no Office) on desktop, layer on phone (Z-103); the existing CodeMirror editor
  (`screens/Editor.tsx`) hosted as the tree's editor area (tree right, editor left, resizable).
  Deletes `screens/Files.tsx` breadcrumb list and the Files rail item.
- **Files**: `src/components/files/{FilesTab,FileTree,TreeRow,FileSearch,EditorArea}.tsx`,
  `files.css`, `src/lib/files/{tree,watch}.ts`, `src/components/media/Previewer.tsx` (tab mode);
  delete `vite.config.files-capture.ts` (scenarios ported); scenarios `files-tree`,
  `files-search`, `files-preview`.
- **Deps**: A3, H7.
- **Acceptance**: tree model tests (lazy expand, compact folders, 50 k entries flattened virtual
  list at 60 fps), search tests, Mention inserts an `@path` chip into the composer, upload via
  `blob_put` + `fs_import`; screenshots.
- **Profile**: opus-ui. **Closes**: Z-101, Z-103.

### WP-A21 — Editor + LSP (host language servers)
- **Scope**: §3.12 in full: host `lsp.rs` + proto + Tauri `lsp.rs` + `HostClient.openLsp`;
  `@codemirror/lsp-client` transport, connection pool, features (diagnostics, hover, completion,
  signature help, definition, references, rename with workspace edits, format, code actions);
  editor tabs with dirty state, save (`fs_write{base_hash}`, conflict sheet kept), vim option kept,
  phone key bar kept; "Open in external editor" split button on the editor and the header (Z-102);
  Settings › Editor › Language servers list (available / install hint).
- **Files**: `apps/unipi/crates/unipi-proto/src/lsp.rs`, `apps/unipi/crates/unipi-host/src/lsp.rs` (+ dispatch,
  settings key, tests `tests/lsp.rs` with a fake stdio server), `apps/unipi/src-tauri/src/lsp.rs`,
  `packages/protocol/src/lsp.ts`, `src/lib/lsp/{framing,transport,pool,features}.ts`,
  `src/components/files/{Editor,EditorTabs,LspStatus}.tsx` (from `screens/Editor.tsx`),
  `package.json` (`@codemirror/lsp-client`); scenarios `editor-lsp` (mock LSP answering hover,
  completion, diagnostics), `editor-conflict`.
- **Deps**: A20, H2, H7.
- **Acceptance**: Rust: framing passthrough, take-over (`E_LSP_TAKEN`), cached initialize on
  re-attach, idle shutdown, commands only from settings (request naming a binary is rejected);
  TS: Content-Length parser (split/joined chunks), pool reuse per `(host, root, language)`,
  rename applies edits to open + closed files with conflicts listed; manual check with real
  rust-analyzer on this repo recorded (hover + go to definition + diagnostics); screenshots.
- **Profile**: opus-ui-high. **Closes**: Z-102; D-10.

### WP-A22 — Board on the `unipi-kanboard` CLI
- **Scope**: keep the Board UI (lanes, list, dashboard, filters, DnD, keys, sheets) (Z-111); rewrite
  `lib/board/transport.ts` + `src-tauri/src/board.rs` to the host **board** delta (`board_cli`,
  `board_watch` → `BoardChanged`), JSON contract per ext-abi §5.5 (stderr JSON on failure) (G-18);
  PcChip kept on the Board; "Add task…" only in the composer `+` menu (chat ⋯ entry removed).
- **Files**: `src/lib/board/{transport,api,live}.ts`, `src-tauri/src/board.rs`,
  `apps/unipi/crates/unipi-host/src/board_cli.rs` (+ dispatch, watcher via `unipi-kanboard`'s project watch,
  test `tests/board_cli.rs` with the real CLI built from `apps/unipi/crates/unipi-kanboard`),
  `apps/unipi/crates/unipi-proto/src/board.rs` (v2, small), `packages/protocol/src/board.ts` (v2);
  delete `vite.config.board-capture.ts` (ported); scenarios `board-lanes`, `board-dashboard`.
- **Deps**: A3, H3, X3 (done).
- **Acceptance**: existing board tests green against the CLI-shaped mock; host test runs `list`,
  `add`, `move`, error path; change notifications refresh the view; screenshots match the archive
  `docs/m9` board shots in layout.
- **Profile**: opus-ui. **Closes**: Z-111, G-18.

### WP-A23 — Cron on job files + host firing
- **Scope**: re-point `lib/cron.ts` / `screens/Cron.tsx` to the H8 `cron_*` DTOs over job files
  (list → detail → form with live schedule preview, Run now, Pause all, run history) and show
  `/cron` command output through the reserved page instead of a text dump (G-17); "Sent by cron"
  bubble label data (`source`) and a clock badge on cron sessions in the sidebar (Z-112).
- **Files**: `src/lib/cron.ts`, `src/screens/Cron.tsx`, `src/components/CronSettings.tsx`,
  `src/components/sidebar/SessionRow.tsx` (badge), `apps/unipi/crates/unipi-host/src/sessions/card.rs`
  (`source`); scenarios `cron-list`, `cron-form`, `cron-badge`.
- **Deps**: A3, A4, H8.
- **Acceptance**: DTO zod tests vs H8 fixtures, form preview tests, Run now → run appears with
  `source:"cron"` card; screenshots.
- **Profile**: opus-ui. **Closes**: Z-112, G-17.

### WP-A24 — Settings screen layout, appearance, extensions, lifetime prefs
- **Scope**: one Settings screen (D-11): two columns (264 px nav, 64 px icon rail below `lg`),
  groups Basics / Agent capabilities / Hosts & devices / Data / About, **global search** across all
  sections and extension namespaces, standard Back on secondary pages (Z-113); Appearance: UI font
  size 12–20 px (`--ui-font-size`), code font size, wrap, line numbers, chat/terminal sizes kept, no
  light theme (Z-114); **Extensions** section: list from `_oino/extensions` with enable toggles
  (`_oino/set_extension_enabled`), crash/disabled notices with the reason, safe-mode banner naming
  suspects (from `HostNotice` / `_oino/hello safeMode`) (G-19); About: one Update card (G-14);
  Lifetime: close action (ask / minimize to tray / quit), start at login (D-20; H9 commands); host
  settings via `host_settings_get/set`; pi agent settings + extension sections via `host_cmd`
  (§3.14) — the schema-driven form itself is U11.
- **Files**: `src/screens/Settings.tsx`, `src/screens/settings/{SettingsNav,SettingsSearch,
  AppearanceSection,ExtensionsSection,AboutSection,LifetimeSection}.tsx`, `src/settings/*`,
  `src/components/{UpdateCard,DesktopUpdate}.tsx` merge; `apps/unipi/crates/unipi-host` `host_cmd` dispatch if
  missing; scenarios `settings-nav`, `settings-search`, `settings-extensions`, `settings-safe-mode`.
- **Deps**: A3, H4, H7, H9.
- **Acceptance**: search finds app, host, pi and extension keys (fixture schemas); font-size var
  applied app-wide (test); extension toggle round trip; safe-mode banner text names suspects;
  screenshots desktop + phone.
- **Profile**: opus-ui. **Closes**: Z-113, Z-114, G-14, G-19; D-11 (layout), D-20 (UI).

### WP-A25 — Harness reachability: MCP, memory, skills, agents, pi built-ins
- **Scope**: every remaining unipi feature reachable (DIRECTION §4.2.1). Settings › Agent
  capabilities gets **MCP**, **Memory**, **Skills**, **Subagents** pages: each shows the
  extension's settings form (U11) plus "Manage…" actions that run the extension's command in the
  active chat and so open its UI v2 panel there (U10 renders it); the same actions are command
  center entries.
  - G-8 MCP: status + server list + catalog + JSON edit + reload = mcp extension panels
    (`/mcp-status`, catalog/add/settings/sync/reload runners).
  - G-9 Memory: new **browse panel** in the memory extension (list, search, forget, consolidate,
    global vs project) built with the `oino-ui` builder; opened by `/memory`.
  - G-10 Skills: exposure mode, pins, vault (schema form) + the skills-editor overlay (`/skills`).
  - G-11 Subagents: `/agents` profile manager panel; "All running to background" is the Running
    summary button + Alt+B (A16/A18), verified here.
  - G-15 pi built-ins: the composer routes them to app surfaces instead of the "terminal only"
    toast (`TERMINAL_ONLY` deleted): `/export` → Export (A5); `/import` → draft "Import session…"
    (`_oino/import_jsonl`); `/clone` → row menu Duplicate (`clone`); `/share` → **dropped** (no
    share service; message explains Export); `/scoped-models` → model picker "Cycle set" checkboxes;
    `/login` `/logout` → new Settings › **Providers** page (`_oino/auth_providers`, `_oino/login`
    with the OAuth URL opened in the system browser, `_oino/set_api_key`, `_oino/logout`);
    `/settings` → Settings; `/changelog` → About › What's new; `/hotkeys` → Ctrl+/ list; `/trust`
    → Settings › Projects › Trusted folders (core trust list; if core exposes none, dropped with the
    same notice); `/bug` → About › Report a bug (issue URL); `/quit` → app Quit.
- **Files**: `src/screens/settings/{McpPage,MemoryPage,SkillsPage,SubagentsPage,ProvidersPage,
  ProjectsPage}.tsx`, `src/lib/composer/builtins.ts`, `src/lib/session/commands.ts`;
  `extensions/unipi/memory/src/panel.rs` (+ `lib.rs` wiring, harness test); scenarios
  `cap-mcp`, `cap-memory`, `cap-skills`, `cap-agents`, `providers`.
- **Deps**: U10, U11, A24, A18, A5, A14.
- **Acceptance**: a reachability test walks the audit table (zcode-audit "Unipi feature
  reachability") as data: every row has a resolvable app surface id (route, tab kind, command
  center action, settings page) — failing on any gap; built-in routing test per command; memory
  panel harness test (list/search/forget); screenshots of each page and opened panel.
- **Profile**: opus-ui. **Closes**: G-8, G-9, G-10, G-11, G-15.

### WP-A26 — `unipi.file-summary` extension (Rust)
- **Scope**: §3.13 in full: snapshots before `write`/`edit` (≤ 5 MB, zstd, content-addressed,
  dedup, once per path per turn), kv in-progress index, `unipi.file-summary/turn` and `/undo` custom
  entries, services `turn` / `diff` / `undo` / `undoFrom`, turn-footer renderer node tree, reserved
  id `unipi.file-summary/turn-files` in the manifest, blobs in `ext-session` (deleted with the
  session); added to the distribution extension list.
- **Files**: `extensions/unipi/file-summary/{Cargo.toml,extension.json,README.md,src/{lib,snapshot,
  store,services,footer,diff}.rs,harness/tests/file_summary.rs}`, `extensions/Cargo.toml`
  (member), the dist package list of WP-D1 (one line).
- **Deps**: U3 (turn-footer target + reserved ids), U6 (SDK `register_turn_footer`), X6 (WIT 2.0.0
  cut).
- **Acceptance**: harness tests: one snapshot per path per turn; new file ⇒ `existed:false`;
  6 MB file ⇒ `skipped`; bash write not captured; custom entry at `agent_end` with correct
  `+X −Y`; kill mid-turn → resume rebuilds from kv + entries; `undo` restores/deletes, conflict
  without `force`, success with it; `undoFrom` newest-first; dedup (same content twice = one
  blob); TUI golden of the footer node at 40/100 cols; `cargo xtask wasm` builds it; clippy/fmt.
- **Profile**: glm-lead (Rust extension; no UI). **Closes**: D-1 (snapshots).

### WP-A27 — Turn file summary footer (app)
- **Scope**: native component for reserved `unipi.file-summary/turn-files` inside the turn row (ZCode
  `ConversationFileSummaryPanel.tsx`, `docs/ui/task-file-change-summary.md`): collapsed "N files
  +X −Y", expand to rows (path, +/−, status), click = inline lazy diff (`diff` service) or "View
  diff" → diff tab (A10), Undo per file and Undo all (confirm), conflict prompt "Changed since this
  turn — undo anyway?", "Undone" state, skipped files marked; requests only for visible turns ± 1
  screen and the running turn (≤ every 500 ms); height change updates only that turn's height-cache
  entry. Enables A7's "Also undo file changes" on Edit.
- **Files**: `src/components/timeline/footers/TurnFilesFooter.tsx`, `src/lib/fileSummary.ts`,
  registration in `src/ui/slots/ReservedHost.tsx` (U12 hook); scenarios `turn-files`,
  `turn-files-undo`, `turn-files-conflict`.
- **Deps**: A26, A6, A10, U10.
- **Acceptance**: request throttling + visibility tests, undo flows via MockHost service log,
  footer hidden when the extension is disabled (entry gone), no re-measure of other turns (spy);
  screenshots collapsed/expanded/conflict.
- **Profile**: opus-ui. **Closes**: D-1 (UI).

### WP-A28 — Empty states, motion, tooltips and accessibility
- **Scope**: new-chat empty state: Unicrab + time-of-day greeting + short capability hint + lore
  line, no recents (Z-87); exit animations ≤ 200 ms for collapsibles, toasts, popovers, sheets, tabs
  (reduced motion = none) (Z-91); shared `Tooltip` (300 ms, per-trigger timer, works on disabled
  controls) on every icon-only control with plain-English text, lucide only, no emoji; keep 44 px
  targets, 2 px focus ring, colour + icon + word (Z-116).
- **Files**: `src/components/{Tooltip,EmptyChat}.tsx`, `src/style.css` / `ui.css` (exit keyframes),
  sweep of icon buttons across `src/components/**`; test `a11y.test.tsx`; scenarios `empty-chat`,
  `motion-exits` (video frames via 3 timed screenshots).
- **Deps**: A6, A12, A3 (runs after most UI WPs to sweep them).
- **Acceptance**: `a11y.test.tsx` renders the main scenarios and asserts zero buttons without an
  accessible name and zero icon-only buttons without a tooltip; reduced-motion test; screenshots.
- **Profile**: opus-ui. **Closes**: Z-87, Z-91, Z-116.

### WP-A29 — Phone shell pass
- **Scope**: §3.9 everywhere: Chat tab = SessionList; bottom tab bar Chat · Board · Settings + More
  (Cron, extension pages); side-pane tabs and bottom panel as layers from the chat ⋯ "Panes" group
  and the capsule; sheets for modal/tree/side chat/subagent; dock collapse; actions always visible;
  placeholder ≤ 2 lines; Android Back closes the top layer; safe areas (Z-115).
- **Files**: `src/components/shell/{PhoneShell,TabBar,PanesMenu}.tsx`, `src/lib/back.ts`, phone CSS in
  touched component folders; scenarios: every scenario of A3–A27 at 412×915 and 360×780.
- **Deps**: A3, A6, A9, A10, A16, A17, A20 (after them).
- **Acceptance**: phone screenshots of all scenarios reviewed; a run on the plugged-in Android phone
  (debug APK or the capture page on device) with Back-stack and keyboard (IME) checks recorded;
  perf rows of §3.6 on the phone recorded in `docs/dev/v2/qa/app-perf.md`.
- **Profile**: opus-ui. **Closes**: Z-115; D-23.

### WP-A30 — Integration, performance gates, QA
- **Scope**: end-to-end on a real host: `unipi-host serve --data-dir <tmp>` (H10 pattern, faux
  provider) + the app in `tauri dev` (This computer) and the web capture page against the same
  host through a dev bridge; scripted flows: new chat → stream 100 k tokens → tool rows → file
  summary undo → queue/steer → dialog → archive/restore → takeover from `oino` TUI → Board/Cron/
  Settings; §3.6 budgets on desktop and phone; closure tables re-verified against the code.
- **Files**: `apps/unipi/e2e/*.mjs` (agent-browser driven), `docs/dev/v2/qa/{app-qa,app-perf}.md`.
- **Deps**: all A WPs, U9–U12, H10.
- **Acceptance**: opus-ui-qa report with zero P0/P1; perf table filled; every Z/G row of §9 has a
  screenshot or test reference.
- **Profile**: opus-ui-qa (review) + opus-ui-high (fixes).

## 6. Order and dependency graph

| WP | Depends on | Profile |
|---|---|---|
| A1 prune + seam + harness | H1 | opus-ui |
| A2 session data layer | A1, H4 | opus-ui-high |
| A3 shell + containers | A1, A2 | opus-ui-high |
| A4 sessions sidebar (+ meta delta) | A2, A3, H4 | opus-ui |
| A5 session actions + draft (+ archive delta) | A4, H3, H4 | opus-ui |
| A6 timeline core | A2, A3 | opus-ui-high |
| A7 turn content + actions | A6 | opus-ui |
| A8 tool rows + renderers | A6, U9 | opus-ui |
| A9 bottom dock + errors | A3, A6 | opus-ui |
| A10 session side tabs | A3, A6, A8 | opus-ui |
| A11 queue / steer | A2, A9 | opus-ui |
| A12 composer core | A9 | opus-ui |
| A13 attachments + toolbar | A12 | opus-ui |
| A14 model / thinking / mode | A2, A13 | opus-ui |
| A15 git pane (+ watch delta) | A3, H7 | opus-ui |
| A16 status sections + capsule | A2, A15, A17 | opus-ui |
| A17 Session modal | A2, A3, U9 | opus-ui |
| A18 command center + shortcuts | A3, A4, A6 | opus-ui |
| A19 notifications + toasts | A4, H4, H7 | opus-ui |
| A20 files tab | A3, H7 | opus-ui |
| A21 editor + LSP (+ lsp delta) | A20, H2, H7 | opus-ui-high |
| A22 board (+ board delta) | A3, H3, X3 | opus-ui |
| A23 cron | A3, A4, H8 | opus-ui |
| A24 settings layout + extensions | A3, H4, H7, H9 | opus-ui |
| A25 harness reachability | A5, A14, A18, A24, U10, U11 | opus-ui |
| A26 file-summary extension | U3, U6, X6 | glm-lead |
| A27 turn files footer | A6, A10, A26, U10 | opus-ui |
| A28 empty / motion / a11y | A3, A6, A12 | opus-ui |
| A29 phone shell pass | A3, A6, A9, A10, A16, A17, A20 | opus-ui |
| A30 integration + QA | all, U9–U12, H10 | opus-ui-qa + opus-ui-high |

UI-protocol app WPs slot in as: **U9** after A1 + A2 (registry fed by the session stream, harness
from A1); **U10** after U9 + A3 + A9 (side-pane / bottom-panel containers, status strip, dialog
panel); **U11** after U8 + U9 + A24 (form engine inside the new Settings layout); **U12** after U10
and the native components it routes to (A15, A16, A17, A20, A22, A23, A27).

Critical path: H4 → A2 → A3 → A6 → A9 → (A12 → A13 → A14) ∥ (U10 → U12) → A25 → A29 → A30.
Parallel lanes once A3 is in: {A4 → A5, A18, A19}, {A15 → A16 (after A17)}, {A20 → A21},
{A22}, {A23}, {A24 → U11}, {A26 (Rust, independent after U3/U6)}. Disjoint file sets per lane
(§5 Files); shared touch points (`main.tsx`, `lib/panes/kinds.ts`, `ui/slots/ReservedHost.tsx`)
are append-only registrations.

## 7. Wave-3 list (reference)

Not implemented in wave 2; containers they plug into are delivered by A3 (side pane, bottom panel,
launcher) and A12/A13 (composer chips, image attachments).

| WP | Scope | Spec | Closes |
|---|---|---|---|
| **WP-A31 terminal pane** | xterm.js tabs on host PTYs (`term_open/attach/list/kill`, H6): bottom panel + side-pane tabs, tab names = cwd leaf with lowest free number, closing the only tab hides the panel and keeps the PTY, PTY exit closes its tab, focus the visible xterm, resize throttled while dragging (Z-105); http(s) links open in the side browser tab (Z-106); phone: unipi toolbar capsule, gestures and terminal composer re-pointed to host PTYs (Z-107); reuses `lib/term{Fit,Fonts,Theme,Scroll,Toolbar}.ts`, `TermToolbar.tsx`; deletes the rest of `screens/Terminal.tsx`. Profile opus-ui-high. Deps A3, A29, H6. | this spec | Z-105, Z-106, Z-107; D-3 |
| **WP-B8 browser pane UX** | tab strip + URL bar in the side pane, element picker → composer chip (`composerChips.add`), user screenshot → composer (image editor), profile + stealth badge, take-over strip, load-error page, open externally, responsive viewport, phone sheet; `BrowserSide.tsx` / `BrowserView.tsx` re-homed as the `browser` tab kind; Local ports launcher entries open here | [browser.md](browser.md) §WP-B8 (paths per §3.16) | Z-67, Z-108, Z-109, Z-110; D-4, D-21 (pane) |
| **WP-D1..D9 installers** | NSIS / dmg / deb / apk, updater channel (feeds the A24 Update card), CI matrix | [dist.md](dist.md) | — |
| **WP-U13** | renderer parity QA + authoring docs | [ui-protocol.md](ui-protocol.md) | — |

## 8. Risks

| # | Risk | Impact | Mitigation |
|---|---|---|---|
| R1 | H4 session protocol or its TS fixtures land late or change | A2 and everything after it stall | A1 stubs types from host.md §7.3; A2 builds against H4's fixture files only; reducer mirrors core `ClientEvent` names, so drift shows up as fixture test failures, not runtime bugs |
| R2 | Turn-unit virtualization in Solid misbehaves with very tall turns (one turn with 500 tool calls) | jank, scroll jumps | fold keeps work collapsed (lazy DOM); tall settled turns render their work list through an inner virtual list when > 200 items; height cache keyed by turn; bench scenario `chat-giant-turn` in A6 |
| R3 | Bottom-lock regressions during width transitions (side pane open/close, sidebar collapse) | chat jumps away from the bottom | ZCode synchronous measure-then-pin batch; A6 test drives width changes while streaming; A30 QA script repeats it on device |
| R4 | LSP servers are heavy (rust-analyzer RAM/CPU) and run on the host for remote phones | host load, battery on laptops | one server per (root, language), idle shutdown 10 min, opt-out setting `lsp.enabled`, servers start only when an editor tab opens |
| R5 | LSP take-over between devices surprises users | lost completions on the other device | clear "Language features moved to <device> · Take back" notice; editing still works without LSP |
| R6 | Duplicate snapshot storage: core `checkpoints.rs` and `unipi.file-summary` both copy files before write/edit | disk use per session doubles for touched files | 5 MB cap + zstd in the extension; core keeps 100 checkpoints max; follow-up (outside this spec): a core setting to disable checkpoints when the extension is enabled, decided at wave-2 QA with measurements |
| R7 | Settings without a session (`host_cmd`) is not in host.md | Settings screen cannot read pi/extension settings | delta owned by A24 (§3.14); if H4 already exposes an equivalent, A24 uses it |
| R8 | Extension panels opened from Settings need an active chat ("Manage…" in A25) | MCP/memory/skills management unavailable before a chat exists | disabled with "Open a chat to manage" + a one-click "New chat here"; settings forms (U11) work without a chat |
| R9 | Many WPs touch `main.tsx`, `lib/panes/kinds.ts`, `ReservedHost.tsx` in parallel | merge conflicts | append-only registration files; each WP adds its own module and one line; A3 creates the registries first |
| R10 | Mock host diverges from the real host | captures pass, real app fails | MockHost built on the same `@unipi/protocol` zod schemas and H4/H7 fixtures; A30 runs the flows on the real host |
| R11 | Phone performance (Pixel 7a class) misses the §3.6 budgets over iroh relay | sluggish phone chat | host delta coalescing (H4), LRU 8 sessions, static markdown, lazy detail; A29 measures on the device and files fixes before A30 |
| R12 | Removing the per-screen PC chip confuses multi-host users | hard to find a PC's sessions | host groups in the sidebar with status dot and collapse state remembered; Board/Cron keep the chip |

## 9. Closure tables

### 9.1 Z items → exactly one WP

| Z | Item | WP | | Z | Item | WP |
|---|---|---|---|---|---|---|
| Z-1 | primary navigation | A3 | | Z-59 | draft persistence | A12 |
| Z-2 | conversation header | A3 | | Z-60 | attachment states | A13 |
| Z-3 | sidebar collapse/width | A3 | | Z-61 | long paste chip | A12 |
| Z-4 | frames & handles | A3 | | Z-62 | mention triggers | A12 |
| Z-5 | side pane of tabs | A3 | | Z-63 | `+` menu | A12 |
| Z-6 | side pane full view | A3 | | Z-64 | composer toolbar | A13 |
| Z-7 | split panes (deferred) | A3 | | Z-65 | dictation (deferred) | A12 |
| Z-8 | host switcher | A4 | | Z-66 | composer spacing | A6 |
| Z-9 | grouping & views | A4 | | Z-67 | web element/screenshot chips | B8 (wave 3) |
| Z-10 | time groups | A4 | | Z-68 | suggested prompts (skip) | A12 |
| Z-11 | paging | A4 | | Z-69 | model picker form | A14 |
| Z-12 | row content | A4 | | Z-70 | model picker entry points | A14 |
| Z-13 | archive | A5 | | Z-71 | thinking control | A14 |
| Z-14 | pin | A4 | | Z-72 | model switch feedback | A14 |
| Z-15 | rename | A5 | | Z-73 | context indicator | A14 |
| Z-16 | session context menu | A5 | | Z-74 | working mode | A14 |
| Z-17 | fork | A7 | | Z-75 | session strip/tray | A14 |
| Z-18 | tree / branches | A10 | | Z-76 | fusion pairing UI | A14 |
| Z-19 | new session flow | A5 | | Z-77 | command center | A18 |
| Z-20 | export / delete | A5 | | Z-78 | search history | A18 |
| Z-21 | turn fold | A6 | | Z-79 | find in chat | A18 |
| Z-22 | live tail | A6 | | Z-80 | extension commands in palette | A18 |
| Z-23 | follow threshold & jump | A6 | | Z-81 | quick open file | A18 |
| Z-24 | unread dot on jump | A6 | | Z-82 | unread state | A19 |
| Z-25 | query navigator rail | A7 | | Z-83 | needs-you surface | A19 |
| Z-26 | timestamps | A7 | | Z-84 | OS notifications | A19 |
| Z-27 | turn-tail actions | A7 | | Z-85 | toasts | A19 |
| Z-28 | edit sent message | A7 | | Z-86 | background completion rule | A19 |
| Z-29 | user bubble fold | A7 | | Z-87 | new chat empty state | A28 |
| Z-30 | selection actions | A7 | | Z-88 | loading | A6 |
| Z-31 | side chat (btw) | A10 | | Z-89 | crash containment | A9 |
| Z-32 | preview cards (keep refs) | A7 | | Z-90 | offline / no host | A1 |
| Z-33 | stream display settings (skip) | A6 | | Z-91 | exit animations | A28 |
| Z-34 | streaming render rule | A6 | | Z-92 | rolling labels | A14 |
| Z-35 | compaction marker | A6 | | Z-93 | micro-feedback | A7 |
| Z-36 | harness/command rows | A6 | | Z-94 | shortcuts + rebinding | A18 |
| Z-37 | tool row anatomy | A8 | | Z-95 | status panel (RULINGS) | A16 |
| Z-38 | tool grouping | A8 | | Z-96 | git review sources | A15 |
| Z-39 | expanded tool views | A8 | | Z-97 | git write actions | A15 |
| Z-40 | inline vs full diff | A10 | | Z-98 | git auto-refresh | A15 |
| Z-41 | thinking display | A8 | | Z-99 | git graph (skip) | A15 |
| Z-42 | subagent rows → tabs | A10 | | Z-100 | git entry points | A15 |
| Z-43 | todos / plans | A16 | | Z-101 | file tree | A20 |
| Z-44 | images in tool output | A8 | | Z-102 | editor (+ LSP) | A21 |
| Z-45 | code blocks | A8 | | Z-103 | preview pane | A20 |
| Z-46 | dialogs dock | A9 | | Z-104 | Preview overlap | A3 |
| Z-47 | permission prompt | A9 | | Z-105 | terminal tabs | A31 (wave 3) |
| Z-48 | error rows | A9 | | Z-106 | terminal links | A31 (wave 3) |
| Z-49 | retry | A7 | | Z-107 | phone terminal input | A31 (wave 3) |
| Z-50 | connection errors | A2 | | Z-108 | browser toolbar extras | B8 (wave 3) |
| Z-51 | lock taken elsewhere | A9 | | Z-109 | agent/user handoff | B8 (wave 3) |
| Z-52 | default while running | A11 | | Z-110 | browser on phone | B8 (wave 3) |
| Z-53 | queue panel | A11 | | Z-111 | board | A22 |
| Z-54 | steered segments | A6 | | Z-112 | cron | A23 |
| Z-55 | stop | A11 | | Z-113 | settings layout | A24 |
| Z-56 | interrupted turn | A6 | | Z-114 | appearance | A24 |
| Z-57 | placeholder by state | A12 | | Z-115 | phone shell | A29 |
| Z-58 | prompt history | A12 | | Z-116 | tooltips / a11y | A28 |

### 9.2 G gaps → the WP that closes them

| G | Gap | Closed by | How |
|---|---|---|---|
| G-1 | extension panels / `custom` dialogs render nowhere | **U10** (on A3 containers + A9 dock) | slot hosts for overlay/sidePane/bottomPane/editorReplace/fullscreen; `custom` → modal |
| G-2 | dialog kinds multiSelect/form/approval/clipboardReadImage/editExternally | **U10** (approval layout from A9) | `src/ui/dialogs/**` in the bottom dock |
| G-3 | `setWidget` not rendered | **U10** | `WidgetStack` in the bottom dock (A9 region) |
| G-4 | `setStatus` not rendered + where it lives | **U10** (home ruled in §3.10, strip built in A3) | status strip under the composer; phone strip under the header |
| G-5 | no extension settings (21 namespaces) | **U11** (inside A24 layout) | schema-driven sections + search |
| G-6 | permission / plan mode control | **A13** | composer PermissionControl + Plan toggle |
| G-7 | extension shortcuts unbound | **A18** | shortcut registry + conflict rebinding |
| G-8 | MCP UI | **A25** | Settings › MCP + mcp panels |
| G-9 | memory browse/search/forget | **A25** | memory browse panel + Settings › Memory |
| G-10 | skills management | **A25** | Settings › Skills + skills editor |
| G-11 | subagent profiles + all-to-background | **A25** | `/agents` panel, Running summary button + Alt+B |
| G-12 | notify / E2E push / deep links | **A19** (host side H7) | push settings on host notify path, `unipi://` routes, notify ext config via U11 |
| G-13 | info-screen Usage/Tools/Skills/Modules | **A17** | Session modal Info sub-pages |
| G-14 | two updaters | **A24** | one Update card |
| G-15 | pi built-ins without app surface | **A25** | routing table (§5 A25), `/share` dropped explicitly |
| G-16 | watchdog caps | **A17** | Session modal Info › Limits + watchdog services |
| G-17 | cron re-point | **A23** | H8 DTOs over job files |
| G-18 | board via CLI | **A22** | `board_cli` + `board_watch` |
| G-19 | extension crash notice + toggle | **A24** | Settings › Extensions + safe-mode banner |
| G-20 | work-tray tabs | **A16** | Running summary → Session modal Running sections |
| G-21 | extension tool/message renderers | **A8** | `_oino/render` + NodeView cards |

### 9.3 D items → where they are realised

| D | WP | | D | WP |
|---|---|---|---|---|
| D-1 file summary | A26 (ext) + A27 (UI) | | D-13 host features | kept: A19 (push), A24 (pairing/relay/sharing/supervisor/updater rows), A3 (ports launcher) |
| D-2 git pane + LH | A15 + A16 | | D-14 herdr/tmux removed | A1 |
| D-3 terminal | A3 (container) + A31 (wave 3) | | D-15 streaming deferred | A1 |
| D-4 browser UX | B8 (wave 3) | | D-16 `/answer` removed | U7a (extension side); app has no surface |
| D-5 ellipse | A13 | | D-17 UI v2 slots | U9–U12 on A3/A9 containers |
| D-6 image persistence | A13 | | D-18 reserved screens | U12 routing A15/A16/A17/A20/A22/A23/A27 |
| D-7 Session modal | A17 | | D-19 crash notice / toggle | A24 |
| D-8 chat performance | A6 (+ A2 turns) | | D-20 tray / close / login | H9 + A24 (UI) |
| D-9 sessions by project | A2 + A4 | | D-21 browser engines | browser.md (B8 pane) |
| D-10 files + LSP | A20 + A21 | | D-22 brand | kept throughout (A28 sweep) |
| D-11 one Settings | A24 + U11 | | D-23 one codebase | A29 |
| D-12 screens kept | A22, A23, A24, SetupWizard (R) | | D-24 host lifetime + This computer | A4 (host groups) + H2/H9 |

## 10. Owner questions

None blocking. Interpretations taken where the audit's Rec named no preferred option are listed in
§4 (notably: no `#`/`$` composer triggers, no flat "Recently updated" view, no inline media cards,
no stream fade-in, Alt+B for subagents-to-background). Each is cheap to reverse if the owner
disagrees at wave-2 QA.
