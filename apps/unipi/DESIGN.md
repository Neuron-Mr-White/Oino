# UniPi App — Design

Status: v0.3 (2026-10-06). M0 and M1 are built (pairing, iroh, herdr session list). M2 is next.

**v0.3 decisions (2026-10-06, user):**
- **M2 = Terminal with herdr and tmux, both first-class.** SSH moves to M3, as the second way in.
- The bottom nav shows a **Chat** tab now, disabled, marked as coming (§2).
- Chat will map **one pi per herdr workspace (or tmux window)** to one conversation. So the host must model herdr and tmux exactly, down to which pane runs pi and which pi session file it writes (§5.6).
- The session picker follows Moshi's new sheet: **[Tmux | Herdr]** plus **Skip**, which opens a plain shell (§3.1.1).

**v0.2 decisions:**
- UI moves from Dioxus to **SolidJS** (§5.1).
- The MVP is **Terminal (herdr + tmux, Moshi-grade) + Board + Settings**.
- **Chat and the other non-TUI views are deferred** (§7).
- Transport is **"whichever path works"**:
  - iroh covers LAN, the user's own tailnet, a direct internet path and a relay.
  - **SSH** is the second way in.
  - Both are first-class (§5.3).

UniPi App is the **first-party mobile companion for pi + UniPi**. It shows your agents on your own PC and lets you control them. It is Android first, then desktop. iOS comes later.

| Pillar | What it is | Phase | Kanboard |
|---|---|---|---|
| **Terminal** | The exact herdr or tmux session from the PC, streamed byte for byte. Moshi-grade input UX. | **MVP** | UNI-109 (PoC done), UNI-112, UNI-116 |
| **Board** | The UniPi kanboard on the phone. | **MVP** | UNI-114 |
| **Settings** | Built up front: typed, validated, migratable. | **MVP** | — |
| Plumbing | iroh + SSH transports, pairing, auto-reconnect | **MVP** | UNI-110, UNI-111 (reconnect part) |
| **Chat** | The real pi/unipi conversation, drawn as native UI. It is not scraped from the terminal. | Deferred | UNI-113 |
| Inbox + push notifications | Needs-you feed and Approve/Reply from a notification | Deferred | UNI-115 |

Competitors (Moshi, QuickTUI) are terminal clients. They have to *guess* what an agent is doing from its screen output. UniPi owns the agent, so the app knows the real state: messages, tool calls, questions, permissions, cost and context. Every design choice should make use of that.

---

## 1. Brand and visual language

### 1.1 Source: Unicrab

The palette comes from the mascot pixel art (`../docs/assets/unicrab-pixel.png`, and the values in `../packages/core/src/hints/crab-data.ts`) and from the logo wordmark (`../docs/assets/unipi-logo.png`, `renderWordmark()` in `../packages/core/src/hints/index.ts`).

| Token | Hex | From | Use |
|---|---|---|---|
| `shell` | `#F03018` | crab body | **Primary**: main buttons, active tab, focus ring, brand |
| `shell-hi` | `#F01818` | crab highlight red | Pressed/hover state of primary |
| `claw` | `#F07818` | joints | Secondary accent, "working" activity |
| `amber` | `#F0A818` | claw bands | Badges, counters |
| `gold` | `#F0C030` | claw tips | Highlights, selection marks, star/favorite |
| `cream` | `#F0F0A8` | eyes | Cursor, caret, focus glow, mascot eyes |
| `face` | `#181818` | face plate | Text on primary buttons (dark on red, like the face on the shell) |
| `ink` | `#000000` | outline | Pixel outlines, OLED black |
| `deep` | `#C00018` | shadow red | Destructive only, always with an icon and label |

Wordmark colours (U N I P I) are the **secondary/semantic set**:

| Token | Hex | Letter | Semantic use |
|---|---|---|---|
| `wm-white` | `#FFFFFF` | U | — |
| `wm-cyan` | `#28BEF0` | N | Info, links, terminal view, transport badges |
| `wm-green` | `#5AD25A` | I | Success, idle/done, online |
| `wm-yellow` | `#FAC828` | P | Warning |
| `wm-red` | `#F03C32` | I | Error |

### 1.2 Surfaces (dark, warm)

Moshi uses pure black, cool charcoal and lime. We keep the OLED-dark idea, tint it warm so the crab colours sit naturally, and swap Moshi's green ambient glow for an **ember glow**.

| Token | Hex | Use |
|---|---|---|
| `bg` | `#0A0908` | App background |
| `surface-1` | `#141211` | Cards, list groups |
| `surface-2` | `#1C1917` | Sheets, inputs, toolbar keys |
| `surface-3` | `#26211F` | Pressed rows, selected segment |
| `line` | `#2E2825` | Dividers, card borders |
| `text` | `#F5EFE6` | Primary text (warm white) |
| `text-muted` | `#A39A90` | Secondary text, icons |
| `text-dim` | `#6E655D` | Hints, timestamps |
| `glow` | `shell` at 8–10 % | Radial glow behind the Home header (Moshi does the same with green) |

Light theme: not in MVP (recorded only).

### 1.3 Agent state colours

These are used everywhere: session cards, sidebar rows, chat header, notifications. Each state always pairs **colour + icon + word**, so it never relies on colour alone.

| State | Colour | Icon | Source |
|---|---|---|---|
| Needs you (question / permission) | `shell`, pulsing | crab eyes `!` | unipi ask_user / permission, herdr `blocked` |
| Working | `claw` | spinner | pi running, herdr `working` |
| Done (unseen) | `wm-green` + dot | check | turn end, herdr `done` |
| Idle | `text-muted` | circle | herdr `idle` |
| Error / exited | `wm-red` | x | process exit ≠ 0 |
| Offline (host unreachable) | `text-dim` | cloud-off | transport |

### 1.4 Type, shape, icons

- **UI font:** Inter, bundled. Titles 22–28 sp regular weight. Uppercase muted section labels at 12 sp with letter-spacing (Moshi pattern; it reads well).
- **Mono:** JetBrains Mono, bundled (OFL). Used for terminal, endpoints, commands, keycaps and code in chat. Terminal and chat have **separate size settings**.
- **Pixel accent:** pixel art is only for mascot moments (splash, empty states, the crab state glyph). The rest of the UI is clean, so it doesn't look like a toy.
- **Radius:** cards 16, sheets 24 (top), inputs 12; buttons, chips and the toolbar capsule are pill-shaped.
- **Icons:** Lucide (outline, 1.5 px). Terminal keys use glyphs (`⇥ ⎋ ⌃ ↑`) with an optional text label setting.
- **Touch:** minimum 44 dp targets; rows 56–64 dp.
- **Motion:** 150–200 ms ease-out; a sheet drags with a spring. The Unicrab (§1.5.1) is the only looping animation.

### 1.5 Mascot in the app

- Splash: the pixel crab with the wordmark. The Android 12+ system splash (dark bg + crab) is held until the page parses, then the in-app splash hops the crab and fades in the wordmark (~1 s, until the first host list loads); the launcher icon is an adaptive crab on `#0A0908` with a monochrome layer.
- Empty states use the crab (or a Lucide icon in compact list states) and one short line that says what to do next. A lore line only where there is genuinely nothing to do (e.g. Inbox empty → "Unicrab is watching the tide. Nothing needs you."); never the same lore line under every state (UNI-218 F11).
- The crab shows the overall state wherever the bell used to be (§1.5.1).
- App icon: the pixel crab. Alternate icons are recorded only.

### 1.5.1 The animated Unicrab (UNI-200)

The "needs you" bell is the animated Unicrab everywhere: the floating button on the phone, the crab in the desktop page header, the Android overlay over other apps and the optional desktop mascot window. The badge count stays.

**How the rig works (no hand-drawn frames).**
1. `apps/mobile/scripts/sprite-from-png.py` reads `src/assets/unicrab-pixel.png` (1254² upscaled pixel art), finds its real cell boundaries from colour-edge density (the art's cells aren't all the same width), takes each cell's median colour, clusters the colours into a small palette and labels every cell: outline, visor (the near-black face), eyes (pale yellow inside the visor), then the coloured islands the outline separates: the one touching the visor is the **body**, islands above it are **claw-left/right**, below it **legs-left/right**. Outline cells join their nearest part, so a moving part carries its own outline. It also derives the eye masks: open, left, right (shifted one cell), blink/closed (one row), happy (`^ ^`). Output: `src/assets/unicrab/unicrab.sprite.json` (24×20 cells, 11 colours, ~4 KB). Re-run it after changing the PNG; never edit the JSON by hand.
2. `src/assets/unicrab/unicrab.anim.json` is the one animation table: for each state, `frameMs`, whole-cell `[dx, dy]` per frame for the tracks `all`, `torso` (body + visor + eyes), each claw, each leg group and `zzz`, the eye mask per frame, `blink` and `opacity`.
3. Renderers move parts, never redraw them. Web: `<Unicrab state size>` (`components/Unicrab.tsx`) is one SVG of crisp rects (`shape-rendering: crispEdges`, same-colour runs merged), parts in `<g data-track>`; `lib/unicrab.ts keyframesCss()` turns the table into CSS `@keyframes` with `steps(1, end)` and whole-cell translations; a JS timer adds random blinks. Android: `UnicrabView.kt` reads the same two JSON files (the APK's asset dir points at `src/assets/unicrab/`) and draws the same cells on a Canvas with whole device pixels per cell.
4. `prefers-reduced-motion` (web) / animator scale 0 (Android) hold frame 0 and never blink.

| State | When (`mascotState`, in priority order) | Motion |
|---|---|---|
| needs-you | an unread waiting item (ask_user, permission, input…) | hops 3 cells, both claws wave, legs tuck |
| done | an unread finished item | happy bounce, eyes `^ ^`, claws up |
| working | an agent is working on any PC | claws clack alternately, eyes scan left/right, legs shuffle |
| offline | every paired PC failed its last chat_list | dim (45 %), eyes closed, still |
| sleep | nothing changed for 5 min | slow sink, eyes closed, Z's drift |
| idle | otherwise | 1-cell breathe, random blink |

**Phone: the floating crab** (`components/FloatingMascot.tsx`, AssistiveTouch-style). 56 px button, drag it anywhere; on release it snaps to the nearer side (position kept as side + fraction of the free height, so rotation keeps it). Half-transparent after 3 s untouched unless something needs you. Tap → the needs-you drawer; long-press (or right-click) → quick actions: Needs you, Chats, Mark all read, Hide the crab. A new waiting/finished item pops a chat bubble beside it (on the screen side, 6 s fade in → hold → fade out, one at a time, oldest first; only unread items, so read ones never come back) with the summarized reply: the same `body` the pi notify extension sends to ntfy (recap summary when recap is on; with "minimal" app detail it falls back to "<session> finished / needs you"). Tap the bubble → that chat. Settings → Unicrab → Floating crab turns it off.

**Android: over other apps** (opt-in, Settings → Unicrab → Over other apps). Asks for "Display over other apps" (SYSTEM_ALERT_WINDOW, the system page; the switch turns on when you come back with it granted). `MascotOverlayService.kt` is a `specialUse` foreground service (a silent ongoing "Unicrab is floating" notification with Turn off) that owns a `TYPE_APPLICATION_OVERLAY` window: the crab + the fade bubble, draggable with side snap. While UniPi is in front it hides (the in-app crab is there). Fed by the page (`window.UniPiOverlay.update`, the same state/count/bubble) while the app runs, and by its own poll of the user's ntfy topic (same `unipi1:` decrypt, key from the notify prefs) every 30 s while the app is in the background. Tap the crab → UniPi; tap a bubble → `unipi://chat?host=&pid=`.

**Desktop: mascot window** (opt-in, Settings → Unicrab → Mascot window, or the tray's "Mascot window" check item). A fully transparent, frameless, shadowless, always-on-top window (label `mascot`, `mascot.html`) at the bottom-right of the primary monitor: only the animated crab, no tile behind it (UNI-255), plus the fade bubble when there's a message. Crab size Small/Medium/Large = 72/96/128 px (Settings → Unicrab → Crab size; default 96); the window is sized to the crab plus bubble room (`mascot_size`), and a size change resizes it keeping its bottom-right corner. Transparency: `transparent(true)` + a transparent webview background colour; on Linux tao gives a transparent window an RGBA visual (needs a compositor: GNOME Wayland/X11 always has one). The main window feeds it over the `mascot-feed` event (state, count, bubble, size). Click the crab → UniPi in front + the drawer; click a bubble → that chat; drag moves it.

Native files live in the tracked `src-tauri/gen/android/` (like `MainActivity.kt`, `UniPiFiles`): `UnicrabView.kt`, `MascotOverlayService.kt` (+ `OverlayBridge`), the manifest service/permissions and the `sourceSets` asset dir in `app/build.gradle.kts`. A `cargo tauri android init` regeneration must keep them (check `git diff` after one).

### 1.6 Shared UI tokens and components (UNI-218 QA2)

Everything that must look the same on every screen lives in `apps/mobile/src/ui.css` (imported last, so it wins) and a few components. New screens use these; don't re-create them per screen.

| Token | Value | Use |
|---|---|---|
| `--page-gutter` | 16 phone / 20 desktop | left/right inset of every page (header, lists, chip rows, board lanes). Use `.page-body` or `padding-inline: var(--page-gutter)`. |
| `--page-head-bar` | 44 | PageHeader's top row (PC chip / back + tools). |
| `--chip-h` | 38 | toolbar pills (`.tool-chip`, `.files-pill`), segmented control. The PC chip is 34 with a 44 hit area. |
| `--btn-h` | 44 | `.btn` (primary / secondary / ghost / danger), all pill-shaped. |
| `--radius-float` | 16 | desktop dialogs (`.sheet`, `.picker-sheet`), palette, menus. |
| `--empty-top` | 64 | EmptyState top offset inside its box (not vh). |

| Component / class | What |
|---|---|
| `<PageHeader title lead tools meta titleSlot>` (`components/PageHeader.tsx`, `.page-head`) | The one page header for Sessions, Chat, Board, Files, Preview, Settings: lead row (PC chip or back) + tools, title 30/36 600, meta 13 muted. Desktop: lead row always 44 px so the title is at x = sidebar + 20, y = 58 on every screen, and the needs-you bell renders at the end of `tools`. Phone: no `lead` → tools on the title row, 52 px kept free for the floating crab (its default spot, top right). |
| Detail header (`.detail-head`, `--detail-head-h` 56) | The one bar on top of every detail pane / full-screen viewer: chat, editor, web preview, browser, desktop, media/reader, review, mermaid, and the board's `.task-head`. On the desktop: 56 px, padding 0 12 0 8, back button 40, title 16/600 + 12 px dim sub-line, actions right, 1 px bottom rule. Add the class to the `<header>`; keep the screen's own class for its content. Phone headers keep their safe-area paddings. The terminal's 38 px tab strip (`.term-tabs`) is a tab bar, not a detail header. Chat puts its session strip (model · ctx · cost) inside this header on the desktop instead of a band below it. |
| `<PcChip>` (`.pc-chip`) | "Which PC" chip: dot + name + chevron, 34 px. Chat, Board, Files, Preview. |
| `.section-label` (= `.board-section-label`, `.palette-group`, `.shortcuts-group-title`, `.chat-group-head`) | 12 px / 500 / .1em / uppercase / text-dim, margin 18 0 10. Write the string in sentence case; CSS uppercases it. `.section-row` puts a count/link (`.section-row-aside`) on the right. |
| `.btn .btn-primary / .btn-secondary / .btn-ghost / .btn-danger`, `.dialog-actions` | Buttons. Primary is shell red (`--color-shell`), never `--color-wm-red` (that's the error colour). Older `.pair-primary`, `.files-sheet-btn.primary`, `.preview-btn.primary`, `.banner-button.primary` are mapped onto the same look. |
| `<EmptyState title body icon action compact tone>` (`components/EmptyState.tsx`) | Every empty / first-run / error state: crab or icon tile, 20/26 title, one line, actions. `compact` for list columns and detail placeholders (`PanePlaceholder` uses it). `<NoHostState what onAdd>` is the one "No PC paired yet · Add a PC" state on every host screen. |
| `<BoardSheet>` | Every dialog/sheet (title 20/26 600, ✕, body, sticky footer). With no `autofocus` field the dialog itself takes focus, so nothing shows a focus ring on open. |
| Focus | `:focus-visible` 2 px shell outline, offset 2; inputs get a 1.5 px inset ring that follows their radius. |
| Hover (desktop) | any `button`/`[role=button]` whose class contains `row` gets `surface-2` on hover. |
| Scrollbars (desktop) | thin, `rgb(110 101 93 / 45%)` thumb on a transparent track, in Chromium and WebKitGTK. |
| Wording | One noun: **PC** ("Add a PC", "No PC paired yet", "Choose a PC"). Desktop strings say click/right-click/download, not tap/long-press/"to phone" (`desktopShell()`). |
| List widths | list | detail screens default to `DEFAULT_LIST_WIDTH` 400 (`lib/layout.ts`); the board keeps main − 560. |

---

## 2. Information architecture

A **session** is the core object: one herdr pane (later a tmux pane) on a host, usually running pi. A session that runs pi can be viewed two ways, **Chat** or **Terminal**, and you flip between them with a segmented control in the session header. This is our main idea: Moshi has "show as chat" as an experimental scrape, while ours is the native default.

```
MVP bottom nav:  Sessions · Chat (disabled, "Soon") · Board · Settings   (Inbox joins after the MVP)

Sessions ─┬─ Ongoing: live background connections (instant resume)
          ├─ Recent: cards for targets you opened, not connected now (thumbnail, agent state, route chip)
          │    └─ Terminal view: exact stream + toolbar + composer
          │          (the [Chat | Terminal] toggle arrives with Chat)
          ├─ Hosts: one row per paired host → Session picker sheet
          │    ├─ [Tmux | Herdr] segmented: tmux sessions, or herdr workspaces
          │    │    └─ drill in: herdr workspace → tab → pane, tmux session → window → pane
          │    └─ Skip ▷|: plain login shell, no multiplexer
          └─ + New: herdr workspace or tmux session (in a directory)
Hosts ──── host list (under the Sessions header): paired unipi-hosts and SSH profiles
Board ──── kanboard lanes, task detail, add, move, note
Settings ─ Hosts & pairing · Terminal · Input · Board · About
```

Moshi has no bottom navigation; it uses header icons (inbox, settings) and a FAB. We have four tabs, so a bottom nav earns its space. Inside a session view the terminal layer covers the bottom nav, so the terminal gets the full height.

**Chat tab (placeholder):** a speech-bubble icon in the second slot, at reduced opacity with a small "Soon" tag. It is not a link: tapping does nothing, it is skipped by keyboard focus, and it carries `aria-disabled`. It becomes live with §3.4.

---

## 3. Screens

Every screen lists the Moshi reference shots that show the pattern. Paths are relative to `/home/oi/Downloads/moshi/`; see `INDEX.md` there.

### 3.1 Sessions (home)
Ref: `01-home-hosts-sessions/02-home-one-session-thumbnail.jpg`, `03-home-two-sessions-mosh-ssh-herdr.jpg`, `03-session-picker-tmux-herdr/02-picker-peek-over-home-herdr-workspaces.jpg`

- Header: the shared app header (crab, title, radial glow).
- **Needs you** strip at the top when any herdr pane is blocked. Tapping it opens that pane.
- **Ongoing** (UNI-128 follow-up): sessions with a live background connection right now (the terminal was left with the yellow **−**, the knob drag-down, Android Back, or by switching targets in the picker). Same two-per-row cards as Recent, most recently used first, with a status dot before the caption (green live, amber connecting/reconnecting, grey ended). Tapping one resumes it instantly: the xterm screen, its scrollback and the attach stream never went away. Long-press menu: **Open**, **Rename label**, **Close connection**. Close connection detaches only — it closes our attach stream so the host frees its PTY client — and moves the card to Recent; the herdr workspace / tmux session keeps running on the PC, so it needs no confirm. At most 4 live connections; opening a fifth closes (detaches) the least recently used background one. A plain Skip shell has no PC-side session, so closing its connection ends that shell.
- **Recent** cards, two per row, for targets you opened before and are not connected to now (stored per host, newest first, at most 6):
  - a mini-terminal thumbnail from a `preview` snapshot, refreshed every 2 s while Home is visible
  - the herdr workspace (or tmux session) name as the caption, the host name under it with the multiplexer icon
  - the route chip in the corner (Moshi puts its Mosh/SSH badge there)
  - our additions: the agent state chip, and later the model and context meter
  - tap reconnects to the same target (it then shows under Ongoing). Long-press opens a floating menu (**Reconnect**, **Rename label**, **Remove from Recents**) anchored to the card; it never shifts the layout.
  - Nothing on Home (and nothing in the picker's row menu, which is rename-only) kills a real herdr workspace or tmux session. The host's `workspace_close` / `tmux_session_kill` actions stay in the protocol but the app doesn't expose them.
- **Hosts** section: one row per paired host with its online dot and route chip. Tapping a row opens the session picker sheet for that host (§3.1.1).
- The FAB `+` opens the add-host sheet (§3.2).

### 3.1.1 Session picker sheet
Ref: `03-session-picker-tmux-herdr/01–05`

A Corvu bottom sheet that slides up from the bottom to **half** the screen; dragging up snaps it to **full** (only then does it add status-bar clearance above the handle), dragging down closes it.

- **Top row:** segmented **[⟩_ Tmux | 🐑 Herdr]** switch (a sliding thumb) on the left, **Skip ▷|** on the right.
  - The segment remembers the last choice per host. The first time, it picks herdr if the host has herdr.
  - If a host lacks one multiplexer, that half shows dimmed with a one-line hint ("tmux isn't installed on oi-pc"). If it lacks both, the segment is replaced by the hint and Skip stays.
  - **Skip** opens a plain login shell on the host (`$SHELL -l` in `$HOME`) with no multiplexer. Over SSH (M3) it is a direct SSH shell.
- **Herdr list:** one row per **workspace**, across every running herdr session.
  - Title: the workspace label. Subtitle: the herdr session name, then tab and pane counts ("default · 3 tabs").
  - Right side: the agent state chip (idle, working, blocked, done) and a π badge when a pane in it runs pi.
  - **Active** pill (green, as in Moshi) on the workspace herdr has focused.
  - Tap = attach to that workspace (§5.6 explains what this does to the PC view). The chevron drills into tabs, then panes; tapping a tab or pane attaches with that one focused.
  - Stopped herdr sessions appear as one row each ("Start") at the bottom.
- **Tmux list:** one row per **session**.
  - Title: the session name. Subtitle: "N windows · <relative activity>".
  - **In use** (amber dot) when another client is attached.
  - Tap = attach. The chevron drills into windows, then panes.
- **Footer row:** "+ New herdr workspace" or "+ New tmux session", with a directory picker fed by recent directories on the host.
- Pull down to refresh. Errors use the shared error banner inside the sheet.

### 3.2 Pair a host
Ref: `02-add-connection-pairing/01-add-method-sheet-qr-vs-ssh.jpg`, `02-easy-pair-qr-macos.jpg`, `03-easy-pair-qr-linux.jpg`, `05-lan-discovery-sheet.jpg`

The add-host sheet offers two routes, the same shape as Moshi's sheet:

1. **Pair with QR (recommended, full features).**
   - On the PC, `unipi-host pair` shows a QR code in the terminal. It holds a minimal iroh ticket (endpoint id + home relay URL only; direct/LAN paths are found by hole punching and n0 address lookup) and a one-time code (valid 10 minutes, single use). The QR uses EC level L and is drawn light-modules-as-blocks, pinned white-on-black on a TTY, so it is right-way-round on dark terminals (version 8, 57×29 cells for a typical link).
   - The phone scans it, proves the code over the encrypted iroh connection, and the host adds the phone's endpoint ID to its allowlist.
   - The scan screen is our own UI over a windowed camera preview (transparent WebView): dimmed scrim, square viewfinder with corner brackets, hint, Close; Android Back cancels. Paste-link is the fallback.
   - The screen has a camera view, a copyable install command for Linux and macOS, and a "What happens" expander (Moshi pattern). (UNI-110)
2. **Connect with SSH (no pairing, or no unipi-host).**
   - Form fields: name, host (a LAN IP, a tailnet IP or MagicDNS name, or a public host), port, user, and auth (an in-app ed25519 key or a password).
   - The host key is trusted on first use and pinned after that.
   - If `unipi-host` exists on the server, the app runs it over SSH and gets every feature. If it doesn't, the app falls back to **basic SSH**: terminal attach and the session list only. (§5.3)

- **LAN discovery** ("2 hosts found"): paired hosts on the same Wi-Fi appear without scanning (iroh mDNS address lookup; Android needs a multicast lock).
- There is no mosh, ET, YubiKey or jump-host support (recorded in §4.2).

### 3.3 Session view — Terminal
Ref: `04-terminal/01-raw-shell-minimal-chrome.jpg`, `02-unipi-tui-minimal-chrome.jpg`, `03-chrome-expanded-toolbar.jpg`, `04-keyboard-open-toolbar.jpg`

- **Top chrome (UNI-128, Moshi pattern, ref `~/Downloads/moshi/Screenshot_2026-10-07-10-31-24-505…`):** one slim (~38 px) bar for every target, right under the status bar: a drag knob centred on top, a round yellow **−** (background → Home), a round green **list** (opens the session picker in place; picking swaps the target — the previous one stays connected under Ongoing — and replaces the `/term` history entry, so Back still goes Home), the title `host: session` in muted mono, and the route badge (LAN / Tailnet / Direct / Relay) on the right. Tapping the knob (or a short swipe up on the bar) collapses it to a knob strip with two status dots, remembered globally; tap the knob to expand again.
- **Knob drag-down = background (Moshi):** a downward drag that starts on the top chrome moves the whole terminal screen with the finger, revealing Home underneath (the `/term` route renders Home below the terminal layer). Released past 22 % of the screen height, or flung down faster than 0.5 px/ms (at least 24 px of travel), it slides away and the session goes to the background, exactly like the yellow **−**; otherwise it snaps back. Background never detaches: the app keeps one hidden terminal screen (xterm + attach stream) per live session in an app-level store (`lib/liveSessions`), so reopening is instant with the scrollback intact. herdr's own top bar (workspace, tab, agents, switch) sits right below it and is reachable by touch (see Gestures: tap = mouse click). Android Back closes the open overlay (history panel, composer, picker) first, otherwise leaves the terminal to Home.
- Picker sheet: the herdr follow-focus note ("Opening a workspace also switches it on your PC") is dismissed for good by **Got it**.
- No left/right padding: the grid runs edge to edge; only the sub-cell remainder is split evenly on both sides, painted in the terminal background.
- The terminal is xterm.js 6 (WebGL renderer, fit and unicode11 add-ons) with the Unicrab ANSI theme. Font stack: JetBrains Mono Variable, then the bundled **Symbols Nerd Font Mono** (MIT, symbol ranges only) so powerline/devicon/MDI glyphs render; xterm opens only after both faces load (bounded wait) and re-measures if a face lands later.
- **Phone-size policy:**
  - The default is **"Fit phone"**, as Moshi does. The phone's client size reflows the session while attached. When the phone leaves, the PC client's size takes over again.
  - The option is **"Keep PC size"**: the phone doesn't affect the size and gets pinch-zoom plus pan. It works on **tmux only** (`attach -f ignore-size`).
  - The M0 spike (`docs/m0/herdr-sizing.md`) found that herdr 0.9 uses the size of the most recently attached or resized client, and has no option to ignore a client's size. For herdr the option shows as unavailable, with a short explanation, and the PC view returns to its own size when the phone detaches.
- **Multiplexer parity (M2, both first-class):**
  - herdr: the workspace/tab/pane tree and agent states come from the herdr socket API (§5.6).
  - tmux 3.2+: the session/window/pane tree comes from `list-*` with `-F` formats (§5.6).
  - Both: create and rename from the picker (closing a real workspace/session is left to the PC); swipe to switch tab/window; two-finger swipe to switch pane.
  - **Skip** sessions are a plain shell: no tree, no switching, the same toolbar and composer.
- **Reconnect (MVP part of UNI-111):** the app re-attaches without a prompt after a network change or when it resumes. herdr and tmux redraw the whole screen on attach, so nothing is lost. A thin banner shows "Reconnecting via Relay…" and the like. Screen-state diffs come later.
- **Toolbar capsule**, docked above the keyboard (Moshi pattern, Moshi proportions: 34 px keys, 6 px gaps, slim capsule, invisible hit-slop). Default order (Moshi icon mode in brackets): Ctrl `^`, Shift `⇧`, Esc `↺`, Tab `⇥`, Opt `⌥`, **arrow drag pad** `✥`, shortcuts, history; composer and keyboard pinned right. **Use icons for modifier keys** (Settings → Input, default off) switches Ctrl/Shift/Esc/Tab/Opt between mono text labels and icons.
  - Keys never take focus from the IME textarea, so tapping them keeps the soft keyboard up.
  - Modifiers (Ctrl/Shift/Opt): tap = one-shot (applies to the next key, toolbar or soft keyboard, then turns off), tap again = off, double-tap or long-press = lock (bar marker), tap while locked = off.
  - Arrow drag pad: not a button; it acts like a held arrow key (UNI-128). Dragging past a 12 px dead zone sends one arrow in the dominant direction with a haptic tick (native `performHapticFeedback`); holding off-centre auto-repeats after 400 ms at 100 ms easing to 70 ms per arrow; the direction follows the finger's offset from the press point (a turn sends one arrow and restarts the delay); back to the centre or lifting stops. A tap alone sends nothing; armed modifiers apply.
  - Capsule inner padding is 8 px on both sides; the key row's scrollport starts at that padding so Ctrl keeps its inset after a scroll-snap.
  - Keyboard key: lit while the soft keyboard is up; tapping it then closes the keyboard.

  `⌃` is always present. The rest can be reordered (UNI-116).
- Gestures:
  - double-tap = paste
  - one-finger vertical drag = scroll (with momentum): wheel reports (SGR/X10) when the app tracks the mouse (herdr, tmux `mouse on`), arrow keys on the alt screen without mouse, local scrollback otherwise
  - tap: when the app tracks the mouse, a left click (press + release, SGR/SGR-pixels/X10) at that cell, so herdr's header (switch, tabs, close ×) and panes work by touch; the keyboard stays as it was (toolbar keyboard key opens it). Without mouse tracking a tap focuses the keyboard
  - two-finger swipe = next/previous herdr pane
  - swipe from the edge = sidebar
  - pinch = font size
- Selection mode: long-press, then drag. It is needed for copying out of TUIs.

### 3.4 Chat (UNI-113) — in progress (decided 2026-10-07)
Ref: deepseek-harness web chat (MIT, React; studied, re-implemented in Solid — no code lifted), plus the Moshi refs above.

**Live in both places.** A chat is a pi already running in a **herdr** pane (tmux panes stay Terminal-only). pi has no way to attach to a running TUI from outside (RPC mode is stdin/stdout of a pi it starts; the session file is written only when a message ends), so a small unipi module inside pi, **app-bridge** (`packages/app-bridge` in the unipi repo), streams pi's live events out and takes messages in:
- Every pi with the bridge listens on `~/.unipi/bridge/<pid>.sock` (0600) and writes `<pid>.json` `{pid, sessionFile, sessionId, cwd, herdrPaneId}`. herdr already reports each pane's pi session file (`agent_session`), so `unipi-host` matches pane → socket by session file (pane id as a cross-check).
- Wire: newline JSON shaped like pi's RPC event stream. pi → phone: a snapshot (current branch entries, model, thinking level, context use, commands, idle/running, a partial in-flight message), then live events (message start/update deltas/end, tool start/update/end, turn/agent start/end, model and thinking changes, compaction, ui dialogs). Phone → pi: prompt, abort, set model/thinking, compact, dialog answer.
- A message typed on the phone goes in with `pi.sendUserMessage`, so the TUI shows it as a normal user message; a message typed in the TUI reaches the phone through the same event stream.
- **Sending while pi works interrupts it (steer)**, like Enter in the TUI. Long-press Send = "after it finishes" (follow-up). Until delivered, the message sits in a queue strip above the composer.
- **Dialogs:** every `ctx.ui` select / confirm / input / editor, and unipi's `ask_user`, is answerable from the phone. The bridge wraps pi's shared dialog object: the TUI dialog and the phone card race, first answer wins, the other closes. (A wrap of pi internals: re-check on each pi bump.)
- Built-in TUI commands (`/model`, `/compact`, …) can't be sent as text; the phone maps `/model`, thinking and `/compact` to bridge calls. Others say "terminal only".

**Where it opens:** the Chat tab has a **PC switcher like the Board's**, then a picker of that PC's herdr sessions → workspaces → the pi panes in them (name, cwd, agent state). The Terminal view of a pi pane gets a Chat | Terminal switch.

**Start / close (UNI-260):** the Chat list's **+** (desktop palette "New chat") opens one sheet: PC → Where (an existing herdr workspace gets a new tab; "New workspace"; tmux window/session on PCs with tmux) → Folder (chips from running pis' cwds and the Files roots, or Browse…) + name → optional first message. The host starts the PC's launcher (Settings → PCs → "pi launcher", default `pi`) in the new tab and answers when the pi's bridge is up; the chat opens and the first message waits in its outbox until live. Chat ⋯ → **Close chat…**: "Stop pi and close tab" (graceful SIGTERM, then the herdr tab / tmux window) or "Just hide from list" (this device, per PC; "n hidden · Show" brings it back). pis in tmux are listed too (matched by process ancestry). Wire: PROTOCOL §10.

**Look (deepseek-harness patterns, our warm theme):**
- User messages: right bubble (radius 20). Assistant: no bubble, full-width markdown; code blocks with a sticky language + Copy header.
- **Folded by default:** a turn's thinking and tool calls fold into one line — live: "Reading files · src/x.ts" with a shimmer; done: "Read files, edited files · 1m 3s". Tap to open: one-line tool rows (icon · title · arg summary, error first line in red, `+N −M` for edits) that open to the diff / shell output / IN-OUT.
- Thinking: one line showing the latest finished paragraph; tap to read.
- unipi events: one native card for every harness message (`details.unipiHarness` {source, title, severity}); own renderers for command echo, compaction, memory cards, subagent/sidekick steps, kanboard notices, progress.
- A typed `/command …` (its `unipi-command-echo`) is the user's own bubble: the command is a chip at the start and the text uses the normal font at full bubble width. When it opened a harness turn (kanboard-do, graph/goal, memory-process…), that turn hangs under the bubble as a small "Kanboard · task budget" chip. The chip expands the injected text inline, minus the user's own words it repeats (UNI-251). Internal commands (`/unipi-app-session`) never show.
- **Working mode (UNI-269):** a tinted chip in the chat header (icon + name + live count: `Graph 3/7`, `Ralph 5/12`, `Goal 45%`; pulse while it runs; dashed "Goal paused" for a parked owner; hidden in plain mode). Tap → a small anchored popover (never a sheet): what the mode does, the run's live state + bar, Show progress, and Switch mode (Goal / Ralph / Swarm / Graph / Plain; switching away from running work says "Pauses the graph", Plain "Stops the graph" and needs a second tap). ⋯ → Working mode opens it in plain mode. Chat list rows carry a small mode badge. Tints: goal gold, ralph green, swarm violet, graph cyan. Wire: PROTOCOL §7.1.
- Running clock row while pi works; "Stopped" / error rows; jump-to-bottom button; following re-engages on your own send.
- Composer: `/` lists the session's real commands (extension commands, prompt templates, skills) with descriptions, filtered as you type; Send ↔ Stop; context ring under it; model + thinking sheet. Questions and dialogs take over the composer slot.

### 3.5 Composer (shared by Chat and Terminal)
Ref: `05-chat-composer-attach/01-composer-chat-via.jpg`, `02-composer-alt-actions.jpg`, `03-add-attachment-sheet.jpg`, `08-settings-input/11-composer-current.jpg`

- **MVP:**
  - A multi-line field ("Send to pane…"). You review the text, then send. Send pastes it into the pane as a bracketed paste, then presses Enter.
  - History: resend from recent prompts. The history is stored per host. The history panel has a Back and a Close button in its header (also when empty: "No sends yet."), and Android Back closes it.
- **After MVP:**
  - `+` opens the attachment sheet: camera, photo, file. The file is **uploaded to the host** (a temporary directory in the project), and its path is inserted into the prompt. This is how screenshots reach the agent.
  - 🎤 Dictation uses Android's on-device SpeechRecognizer. The default is to review before sending; auto-send is optional.
  - Slash-command palette: typing `/` lists the **real pi/unipi commands** fetched from the host. Moshi ships static per-agent catalogs; ours are live.

### 3.6 Sidebar / switch sheet
Ref: `03-session-picker-tmux-herdr/03-picker-half-herdr-workspaces.jpg`, `07-settings-general/08-sidebar.jpg`

- Inside a terminal, the same tree as the picker (§3.1.1) for the current host: herdr session → workspace → tab → pane, or tmux session → window → pane. An option flattens panes under workspaces.
- Each row shows a state chip, the agent title (or herdr tab name), and optionally the model and context used.
- Attention-first ordering. Actions: new tab running pi or a shell, rename, close.

### 3.7 Inbox — DEFERRED (the MVP shows a "Needs you" strip on Sessions, driven by herdr agent state)
Ref: `06-inbox-files/01-agent-events-empty.jpg`

- A feed of: **Needs you** (questions and permissions, answerable inline), **Done** (turn finished, with a summary line) and **Errors**.
- Swipe to dismiss; tap opens the session at that message.
- The empty state shows the crab with a line of lore.

### 3.8 Notifications — DEFERRED
Ref: `10-settings-hooks-notify/04-notifications-behavior.jpg`, `05-notifications-devices-webhook.jpg`

- Sources are unipi `notify` events (input_needed, permission, done), forwarded by the host.
- Android notification actions: **Approve / Deny**, or **Reply** with inline text, without opening the app.
- Tapping opens Inbox or the session (configurable).
- Delivery while the app is backgrounded is an open question (see §6).

### 3.8.1 How notifications work today (UNI-161)

The deferred feed above is still not built. What *is* built is pi's own
notification delivery to the phone, on the user's own ntfy server:

**Producing.** pi's `notify` package (installed in the pi repo, not here)
fires on its events (input_needed, permission, done) and publishes to the
server/topic the user set in its own Settings UI, written to
`~/.unipi/config/notify/config.json`'s `ntfy` block
(`{server_url, topic, token, app_detail}`). Before a phone has ever paired,
messages are plaintext; once a key exists both sides encrypt.

**The key.** `notify_config` (host, `crates/host/src/notify.rs`) generates a
per-pairing 256-bit key at `<host dir>/notify-key` (0600) on first use and
hands it to the app; pi's notify package reads the *same* file to encrypt.
The key is the only shared secret: the ntfy server only ever sees
ciphertext — the message body becomes
`unipi1: + base64(iv[12] || AES-256-GCM(JSON) || tag[16])`, the title is
pinned to the neutral string "UniPi", and no plaintext tags are sent
(docs/m6/PROTOCOL.md §"Encrypted payload format").

**Delivery, three paths:**

- **UnifiedPush (background, needs a distributor app):** the app registers
  with whatever distributor is installed (usually the ntfy app) and stores
  the endpoint URL in native SharedPreferences. `syncPushEndpoint()` sends
  it to every paired host (`notify_register`), which appends to
  `~/.unipi/config/notify/app-endpoints.json`; pi's notify package fans out
  to that list as well as the topic. The app need not be running:
  `PushServiceImpl.kt` decrypts with the key in prefs and posts a native
  notification; Approve/Deny/Reply buttons are queued and replayed when the
  app next opens (`takePendingAction`), and a tap opens
  `unipi://chat?host=&pid=`.
- **"Keep listening" (app open / resumed, no distributor needed):** the
  in-app poll reads the topic's recent history
  (`GET {server}/{topic}/json?poll=1&since=600s`), decrypts `unipi1:`
  messages and feeds them into the Needs-you drawer.
- **ntfy's own client:** a message the app did not decrypt (no key, or a
  plain one) still reaches the phone as an ordinary OS notification from
  the browser/service-worker path — it just can't be routed into the
  drawer.

**What each setting does** (Settings → Phone notifications):

| Key | Meaning |
|-----|---------|
| `enabled` | The whole feature. Off = no registration, no poll. |
| `serverUrl` / `topic` / `token` | Your ntfy server, its topic, and an access token if the server needs one. |
| `keepListening` | Poll the topic on resume and every 20 s while the app is visible (the fallback when no distributor is installed). |
| `key` / `keySourceHostId` | The per-pairing E2E key (base64, 32 bytes) and the host it came from. |

`notify_config` ("Get from this PC") fills server/topic/token/key from the
paired host in one step and remembers the host in `keySourceHostId`;
`notify_test` ("Test") makes the host encrypt-and-publish one message so the
whole chain can be proven without a live pi session.

**What needs the app open.** Only "Keep listening". The UnifiedPush
registration and the native key (both in SharedPreferences), plus the
notification actions, survive the app being closed, killed — or updated: an
APK update keeps both. What did *not* survive was the JS settings, which
were re-defaulted on every app start (UNI-286, so `enabled` silently came
back false), and the E2E key, which was pushed to native only once at mount,
before the settings had loaded, so background decryption never received it.

**Desktop app:** no UnifiedPush and no ntfy path — the desktop window
polls every paired PC's herdr state itself (every 20 s,
`components/shell/DesktopNative.tsx`) and raises native OS notifications for
new "needs you" / finished items while its window isn't focused (first look
seeds only, max 3 at a time). That switch lives in its own localStorage key
(`unipi.desktop.notify`), not in the notify settings.

### 3.8.2 Git (UNI-217)

Basic git for the repo that holds a folder on the PC. The wire is `{op:"git"}`
(docs/m5/PROTOCOL.md §8). The UI is `components/git/GitView.tsx`, a layer
mounted once in main.tsx and opened with `lib/git.ts` `openGit()`.

- **Ways in:** in Files, when the folder is in a repo, a branch chip sits in the
  toolbar (branch, ↑ahead ↓behind, and an amber dot if the folder has changes);
  ⋯ → Git… does the same. In a chat, ⋯ → **Git** opens the session's cwd.
  Outside a repo you get the EmptyState "Not a git repository" with Back.
- **Header** (the 56 px `.detail-head`): back, repo name, and a sub-line
  (`main · origin/main ↑2 ↓1`, "not pushed yet", "rebase in progress"). Then
  Pull (ff-only) and Push, each with a count badge and a spinner while running,
  and ⋯: Switch branch…, Fetch, Pull with rebase, Stash changes, Pop stash (n), Refresh.
- **Changes tab:** groups Conflicts / Staged / Changes / Untracked
  (`.section-label` with count, group +/−, and Stage all / Unstage all). Each row
  has a status badge (the Files git-badge colours), name and folder, +N −M (green
  and red, like the chat's changes drawer; "bin" for binary files), and a +/−
  stage toggle. Right-click or the row menu gives Stage, Open in editor, Copy path,
  and Discard changes… / Delete file… (asks first in a BoardSheet).
- **Commit composer** at the bottom: an auto-growing message box, an Amend
  pill (fills in HEAD's message), and "Commit N files" (`.btn-primary`), which
  is disabled until something is staged and there is a message. Ctrl/⌘+Enter
  commits. A toast confirms "Committed abc1234 · subject".
- **Diff:** tapping a file opens the shared diff view (`renderDiffHtml`, the
  same as the chat's review viewer). It shows the staged diff vs HEAD, the
  worktree diff, or an untracked file as all additions. The header has
  Discard / Open in editor / Stage·Unstage. Binary files and mode-only changes
  get compact EmptyStates.
- **History tab:** 50 commits per page with Load more. Each row has the
  subject, hash (amber), author, relative date, and ref pills (head green,
  remote cyan, tag yellow). Opening a commit shows its body, files with +/−,
  and the full diff.
- **Branch sheet:** a "New branch name" field with Create, then Local (current
  one checked, ahead/behind shown) and Remote. Picking a remote branch makes
  a tracking branch. A filter appears when there are more than 8 branches.
- **Errors:** an inline ErrorBanner shows git's text and the host's hint
  (e.g. "Auth needed: …"), and can be dismissed. Pull, push and fetch show a
  "Pulling…" pill while they run.
- **Phone vs desktop:** on the phone the Git view and a file or commit open
  full screen (Back closes them). On the desktop it sits right of the sidebar
  as a list (420 px) | detail split with a "Pick a file" placeholder: ↑/↓
  select, Space stages, Ctrl/⌘+Enter commits, Esc closes. It also adds palette
  commands (Git: Commit, Pull, Push, Fetch, Switch branch, Stage all, Refresh).

### 3.9 Board
Ref: the kanboard web UI (`../crates/kanboard/web`, SolidJS) — Moshi has no board.

**Scope (2026-10-07, user):** feature parity with the kanboard web UI (the Dashboard came later as the **Overview**, UNI-117), in this app's look (Unicrab palette, `.sheet` bottom sheets, Settings-style rows) and re-thought for one-handed phone use. Where the desktop interaction fits a phone it is kept as-is; where it does not, the phone pattern below replaces it. Code (types, helpers, markdown, layout logic) is copied from the web UI into `apps/mobile/src/lib/board/`, not imported across packages.

- **Data path:**
  - The UI only talks to `boardTransport()` (`lib/board/transport.ts`): under Tauri the Rust core → iroh → `unipi-host` → the local daemon (`board_availability`, `board_request`, `board_events`); in `vite dev` the browser fetches `/kb/*`, which vite proxies to the daemon (`UNIPI_KB_DEV_URL`), as one synthetic PC "dev (vite proxy)".
  - `unipi-host` reads the daemon's port and token from `~/.unipi/kanboard/daemon.json` and starts `unipi-kanboard serve` if none is running. The phone never talks to the daemon's port directly; writes are user-actor.
  - Over basic SSH (no unipi-host) the board is unavailable and says how to install unipi-host.
  - `lib/board/api.ts` mirrors every web endpoint with a `hostId` first; `ApiError` carries `status` (0 = transport failed), `needsComment`, `needsAgent`. `lib/board/store.ts` holds one (PC, project) at a time: `refresh()` (pull-to-refresh), `reload()` (what an SSE revision calls), `applyTask()` / `optimistic()` for writes; stale responses after a switch are dropped.
- **Parity list (web → phone):** PC + project switcher incl. archived projects and the PC-wide "Agents running" list (web sidebar); lanes Backlog · Todo · In Progress · Blocked · In Review · Done · Cancelled · Archive with counts; per-lane "about" text (web ⓘ), Done all / Summarize quick actions and the lane ⋯ menu (Move all to Done, Archive all, Summarize & archive, Archive without summary, Hide column); cards with id, up to two labels (+N), agent chip with elapsed time, stale tag, title (derived titles italic), blocked reason, optional excerpt, priority tag, dependency tag (ready / waiting / locked, dep ids jump to that card), same-lane dependency chains drawn as connected blocks; Filter (priorities, Running now, Waiting on dependencies, labels; live count); Display (columns, Group linked tasks, Show description excerpt, Compact cards); Board / List views; Review queue / Blocked scopes; text filter; search (the ⌘K palette: tasks, actions, projects); New task (title optional but title or description required, Backlog | Todo, priority, labels picker with create, "runs after" picker without cycles, `@` task mentions, attachments uploaded after creation, Create more, discard-draft guard, Recreate prefill); the unreadable-task-files banner with the `validate --fix` hint; toasts with one action (Undo / Open).
- **Dropped on the phone:** keyboard shortcuts (J/K, C, /, B/L, ?, ⌘K) and the shortcuts dialog, the light theme, sidebar collapse, hover-only affordances, HTML5 drag & drop. The desktop shell has them again (§5.8), except the light theme.
- **Overview** (the kanboard Dashboard, UNI-117; `components/board/Dashboard.tsx`, logic in `lib/board/dashboard.ts`). Opened from Board ⋯ → "Overview of all projects" or the palette; it replaces the lanes with every project on the PC. Contents: greeting and summary line, KPIs (waiting on you, amber after a day; agents; ready; done in 7 days with a spark), a needs-you inbox grouped by project (oldest wait first, 3 per group and 10 overall until "+N more" / "Show all", collapsible groups, All/Review/Blocked filter, Approve with Undo, Rework, and Answer opening the task), agents running, up next, 14-day bars with tiles (median review wait, blocked, files needing repair), clustered activity with a step trail, and project cards (sort by attention/recent/name, review/blocked/ready badges, stack bar, ⋯ and right-click Archive). Rows open the task in its project. Live: one revision stream per project (≤ 12), reloading after a 400 ms debounce. One column on the phone, two at ≥ 1100 px.
- **Phone UX decisions (vs the web UI):**
  - *Header owns context, not chrome.* No app header on this tab: a small PC chip (status dot) sits above the project name as the large title, which is itself the project picker. Search and ⋯ (Refresh, Review queue, Blocked, Summarize board, Board settings, Archive/Unarchive project, live-updates state) are the only icons. Why: the web's sidebar + breadcrumb + toolbar would eat a third of the screen; the project is what the user reads first.
  - *Lanes are pages, not columns.* Six to eight 280-px columns cannot share a 412-px screen. Each lane is a full-width page in a horizontal scroll-snap pager (`scroll-snap-stop: always`, so a fling moves exactly one lane), and every page keeps its own vertical scroll. A scrollable lane strip above (status glyph, name, count; the active pill tinted in the lane colour and centred) makes every lane one tap away; far jumps are instant rather than animating through every lane. A light haptic tick marks each lane change.
  - *Backlog follows Settings.* `board.showBacklog` (default off) drives the Backlog column, so the first page is Todo — what an agent will pick next. Display's Backlog chip and the lane ⋯ "Hide column" write the same setting. Archive is hidden by default like the web; Cancelled is shown.
  - *Primary action at the thumb.* New task is a FAB above the bottom nav, and it is lane-aware: on the Todo page it creates into Todo, elsewhere into Backlog (the web's default). Toasts sit beside the FAB, never over it. Backlog/Todo pages also end with an "Add task" row.
  - *Bottom sheets instead of popovers.* Filter, Display, PC picker, project picker, search and New task are `.sheet` bottom sheets: reachable, swipe-down to dismiss, capped to the visual viewport so they ride above the soft keyboard, and Android Back closes the top one (`pushBack`). Small action lists (⋯ header menu, lane menu, and in UNI-131 the card long-press menu) use the Sessions-style anchored popover instead, which keeps the board visible.
  - *One tap per choice.* Status and priority are segmented rows in the New task sheet instead of popovers; filter options are 42-px toggle chips; display options are Settings-style switch rows. The Filter sheet's button shows the live result ("Show 5 tasks").
  - *Pickers drill in, they don't stack.* Labels and "runs after" open as pages inside the New task sheet (← and Back return to the form), so there is never a sheet on a sheet. On touch screens their search field is not auto-focused (picking is mostly tapping; tap the field to type or create), and rows never steal focus, so an open keyboard stays up while toggling.
  - *Search and filter merged.* The web has both a toolbar filter box and a ⌘K palette; here one search sheet finds tasks by id, title, body or label (tap = jump to the card and open it), lists the palette's actions and projects, and offers "Filter the board by …" to keep the text as a removable chip in the toolbar.
  - *Drafts are protected.* A dirty New task form disables swipe-to-dismiss and turns Back / ✕ / outside-tap into an inline "Discard this draft?" strip (web UNI-61).
  - *Reorder and move without drag & drop.* Long-pressing a card (the Sessions gesture) opens the quick-action menu at the card; moves only list lanes in `allowedMoves` (UNI-131).
  - *States instead of blank screens.* Skeleton cards and count placeholders while loading; per-lane empty states with the web's copy, and "No tasks here match the filter · Clear filters" when a filter empties a lane; a full-height crab state with Retry (and Switch PC) for an offline PC, a PC without unipi-host (the host's hint), a daemon error or no projects; a banner over still-visible data when a refresh fails ("Showing the last loaded board").
  - *Pull to refresh* on every lane page and the list (it only engages on a mostly-vertical pull at the top, so it never fights the lane swipe). *Live updates* (UNI-132, mirrors the web's `watchBoard`): while the tab shows a project and the app is visible it subscribes to the daemon's `/events?project=` revision stream through the board transport; every revision (the daemon sends the current one on connect, then one per change — CLI and agent writes included) triggers a 250 ms-debounced task reload plus the projects/running refresh, so bursts coalesce into one fetch. An open task page reads its task from the store, so it follows along, while title-in-focus, description editor and note drafts are never overwritten. Hiding the app, switching project/PC or leaving the tab unsubscribes; coming back resubscribes and the connect revision is the one catch-up reload. A dropped stream (daemon idled out and restarted, PC gone) reconnects with the terminal backoff (500 ms → 8 s, then every 15 s while visible) re-checking the PC first, and reloads once back; only a PC without the board or an invalid request goes to *off*, where pull to refresh still works. The LED is the PC chip's dot: steady green = board reachable, a soft halo = live, breathing = (re)connecting; the ⋯ menu spells it out ("Live updates: on / reconnecting… / off · pull to refresh"). Why: no polling and no stale board after the PC's daemon restarts, without a second indicator in a header that is already tight.
  - *Touch and performance:* 44-px targets (hit-slop where the visual is smaller, e.g. dep ids, lane pills), cards press-scale like Sessions cards, long lists use `content-visibility: auto` on cards and rows, and the main tail spacer is replaced by a FAB-height bottom padding inside each page.
  - *Remembered context:* the last PC and the last project per PC are remembered; otherwise `board.defaultProject` (slug or name), else the most recently active project.
- **Task detail and actions (UNI-131).** Ref: `docs/m4/design/31-…55-*.png` (captured by `docs/m4/capture-board.py`). All writes live in `lib/board/taskActions.ts` (every one upserts the server answer with `store.applyTask`, then reloads the project); the screen only calls the hooks in `lib/board/actions.ts`.
  - *Detail is a page, not a sheet.* Tapping a card / search result / dependency pushes a full-screen task page over the board (it covers the bottom nav like the terminal layer): Back/← returns to the board, or to the previous task after a dependency jump (the pages stack). A sheet would stack sheets and fight swipe-to-dismiss while scrolling a long activity.
  - *Order of the page = what you need first:* header (←, status glyph + id — tap copies — and ⋯ task actions), the title edited in place, status / priority / label chips, then the **In Review / Blocked report as a coloured card** with one-tap next steps (Mark done · Rework…, Answer & move to Todo) and "Read the full report", the agent run (agent, session, pid, elapsed; a stale claim says why and offers Release to Todo / Backlog with a required note), description, Runs after (ready / waiting / locked, tap opens, ✕ unlinks) + Needed by, attachments, activity, details (created/updated, copyable file path).
  - *Status and priority chips open Sessions-style anchored menus;* the status menu lists only `rules.allowedMoves` (never the current lane), marks moves that need a note, and the comment sheet ("from → to", the daemon's hint, note with @ mentions) opens first when `commentRequired` — or after a 409 `needsComment` refusal, retrying with the note. Rework from In Review and answering a Blocked task offer an optional note in the same sheet.
  - *Final lanes:* a move picked from a menu into Done / Cancelled / Archived asks in a confirm sheet (a mis-tap in a long menu is easy on a phone; "Keep it" vs "Cancel task" so the two buttons never both say Cancel). Explicit buttons ("Mark done") and the comment sheet are their own confirmation. Every move carrying an `undoToken` shows a toast with **Undo** that acts on the PC/project/task where it happened, even after switching boards. A task landing in Todo with dependencies still in Backlog offers (never forces) to schedule them (web schedule.ts).
  - *Description editor takes the whole page* (the web's inline textarea leaves three lines above a phone keyboard): Write | Preview, a field that fills the space above the keyboard and a markdown key row docked on it (bold, italic, code, list, checklist, link, @, attach). Cancel with changes asks first.
  - *Note composer pinned above the keyboard* (auto-growing field, Send, attach). `@` mentions are a horizontal chip strip above the field, not a caret popover (the keyboard owns the bottom half); exact / prefix id matches rank first, then title matches; a tap inserts the id and keeps the keyboard up. An unsent note guards Back.
  - *Attachments:* camera / photo library / files from one attach menu (three hidden inputs; `capture="environment"` for the camera), uploaded immediately via `api.upload` (25 MB limit checked before sending), markdown inserted at the cursor. Images render inline as thumbnails → black full-screen viewer (pinch 1–5×, double-tap, pan, swipe between images, swipe down to close, Share/Save); files open the share sheet or download.
  - *Activity:* newest first, sanitised markdown, status changes labelled like the web (Finished / Blocked / Rework …), long entries clamped with Show more, reports open in a full-page reader with Copy; 15 at a time ("Show older").
  - *Card long-press* (the Sessions gesture) opens an anchored quick menu at the card: Open, allowed moves, a one-tap priority segment, four reorder buttons (top / up / down / bottom — no drag & drop; a dependency chain moves as one block and keeps its order, so "up" means "before the previous block"; disabled at the edges), Duplicate, Recreate…, Archive, Cancel task. A running card lists no moves (the agent moves it on); a stale one offers Release.
  - *Edits send exactly what changed* (title on blur/Enter, priority, labels via the labels sheet with create, body); links go through a picker sheet that never lists a task that would close a loop.
  - *Project / lane actions:* the ⋯ header menu has Summarize board, Board settings, Archive / Unarchive project (archive shows a toast with Undo); lane menus have Done all (In Review, confirm), Archive all (confirm, danger) and Summarize & archive (Done). Summaries are one bottom sheet for both scopes (instruction from settings or a prompt, Generate → result with Edit, Copy, With note — resummarize from the previous summary plus a note —, Regenerate → Save & archive N tasks). Generating shows the PC, elapsed time and a progress bar; the sheet can be hidden while the agent works and a toast with **Open** brings the result back ("Stop waiting" drops it); `needsAgent` shows a configure banner.
  - *Board settings* mirror the web SettingsDialog as a tabbed sheet in the Settings look — Summaries (model picker page: searchable, grouped by provider, "pi default" first, Refresh; instruction with Customize / Reset; the agent command read-only), Sessions (sessions at once, tasks per turn, chain gate), Archive (auto-archive, cold storage). They are the PC's daemon settings (all projects); Save sends one flat patch; unsaved changes ask before closing.
  - *Toasts* sit beside the FAB with at most one action (Undo / Open).
- Deferred: "start this task as a pi session" (comes with Chat).

### 3.10 Settings (MVP set)
Ref: `07-settings-general/01–03`, `05-fonts-size.jpg`, `09-advanced.jpg`, `08-settings-input/01-toolbar-preview.jpg`, `03-configure-toolbar-current.jpg`, `07-new-shortcut-builder.jpg`, `14-shortcuts-panel-tabs.jpg`

Settings is built first, as infrastructure. Every feature reads its settings through one typed store, so later work adds keys, not plumbing.

- **Store:**
  - One zod schema (`SettingsV1`) with a default for every key, persisted through `tauri-plugin-store`.
  - A version number and migration functions, so a newer app never breaks on old data.
  - Invalid values fall back to their defaults per key, with a one-time toast. The app never crashes on bad settings.
- **Hosts & pairing:**
  - paired hosts and SSH profiles: rename, re-pair, forget
  - SSH keys: generate an ed25519 key, copy the public key, delete
  - pinned host keys
  - relay: n0 public relays (default) or a custom relay URL
  - path preference: auto (default) or prefer SSH
- **Terminal:**
  - font size and line height
  - cursor style and blink
  - scrollback
  - phone-size policy (Fit phone / Keep PC size)
  - keep screen on
  - reopen last session
  - tmux: apply friendly options on attach (mouse, titles)
  - herdr: follow phone focus (on: tapping a workspace focuses it, which also moves the PC view; off: attach without changing focus)
- **Input:**
  - toolbar editor with a live preview (add, remove, reorder; `⌃` is fixed)
  - custom shortcut builder (modifier + key, or text, with a hint label)
  - shortcuts panel tabs (Favorites, herdr, tmux)
  - gesture bindings (double tap, swipe, two-finger swipe, pinch)
  - autocorrect off in the terminal, on in the composer
  - haptics on/off
- **Board:** default project; show backlog.
- **Sharing with phones / This computer (desktop app, UNI-248):** switches for web apps, browser, desktop and push, pending requests (Allow 1 h / 8 h / until off, Deny), and "Keep running after logout" (systemd linger via pkexec). **Files from your PCs** (any app): each paired PC's push state, plus Ask. See docs/m7/SHARING.md.
  - UNI-263: the unipi-host row says "Version X is available" with **Update** (runs `unipi-host update`: download, sha256, atomic replace, restart its service) and the switch **Update the host by itself** (`unipi-host update --mode auto|prompt`; default prompt = user decision 6). `serve` checks the feed every 6 h and records `update.json`; `sharing status --json` carries `update{mode,latest,available,checked_at,error}`.
- **About:**
  - updates (UNI-263, see §5.9): Android — download + system installer; iPhone — "UniPi X is available, open unipi.nrn.one/ios in Safari"; desktop app — "Check for updates", and a prompt sheet (Later / Update and restart) when a newer version is out
  - version and host versions
  - licences
  - crab lore
  - export diagnostics (redacted logs).

---

## 4. Feature triage from the Moshi study

The question for each feature: does it converge with *UniPi-first remote control of your own PC's agents*, and does it raise productivity?

### 4.1 Adopt

| Feature (Moshi) | Our version | Phase |
|---|---|---|
| Session cards with live thumbnails | Plus herdr agent state (MVP); model, context and cost come with Chat | MVP |
| Herdr session picker | Full herdr tree sidebar, attention-first | MVP |
| Collapsible terminal chrome | Same | MVP |
| Toolbar capsule above keyboard, Ctrl always shown, Ctrl lock | Same | MVP |
| Press-and-drag arrows / D-pad | Drag-arrows key | MVP |
| Double-tap paste, pinch font, swipe = pane/tab | Same | MVP |
| Composer ("Chat via Moshi") | Composer shared by Chat and Terminal | MVP |
| Attach photo/file → upload to host | Upload to host, path inserted in prompt | v1 |
| Agent events inbox (approval, question, done) | Inbox with inline answer/approve from unipi events | v1 |
| Notifications with tap-to-open | Plus Approve/Deny/Reply actions | v1 |
| QR "Easy Pair" + install command | iroh ticket QR, `unipi-host` install | MVP |
| SSH connection form, key generation, LAN discovery | SSH profiles (full via `unipi-host stdio`, plus basic); ed25519 generation; host-key pinning | MVP |
| tmux sessions | Same parity as herdr | MVP |
| Shortcut builder + shortcuts panel tabs | Plus live unipi slash-command catalog | v1 |
| Dictation, history resend | On-device Android STT; word replacements later | v1 |
| LAN host discovery | iroh mDNS address lookup | MVP |
| Chat vs terminal font sizes, autocorrect split | Same | MVP |
| Keep screen on, reopen last terminal | Same | MVP |
| Diff view of workspace changes (hook-gated) | Native git diff viewer from host | v1 |
| Session web preview (port detection) | Port forward over iroh + in-app browser | v2 |
| Sidebar settings: show model/context, used vs remaining | Same, from real pi data | v1 |

### 4.2 Recorded only (not planned)

- **Transports:** Mosh, Eternal Terminal, jump host, agent forwarding, UDP port range, mosh-server path, wake probe. iroh and SSH cover these needs; mosh's *ideas* go into UNI-111.
- **Credentials:** YubiKey PIV over USB/NFC, importing keys from a file, biometric unlock for keys. (SSH key generation, password auth and host-key pinning are in the MVP.)
- **Theme gallery** (Dracula, Nord, Catppuccin, Solarized and so on), follow-system, light themes. We ship one Unicrab dark theme.
- **Fonts:** download on demand (Iosevka and others), font import, ligature/OpenType options, CJK fallback fonts. Revisit if users ask.
- **Alternate app icons** (Moshi's cat variants). Fun, later.
- **Zen mode / Zen ring** (floating radial key menu): one input surface is enough for MVP.
- **Volume-key bindings, D-pad corner bindings, triple-tap custom binding:** too fiddly for now.
- **Hardware-keyboard shortcut sheet** (⌘K, ⌘1–9…): relevant for the desktop/tablet phase (UNI-117).
- **Speech engines:** Whisper, Parakeet, cloud with quota, BYOK. We use Android's on-device recogniser first.
- **Files tab uploading to a vendor API, URL copied to clipboard:** we upload to the host instead.
- **File sync, credential sync, sync folder.**
- **Show on Home:** simulators, web servers icons.
- **Live Activities / Dynamic Island / Apple Watch:** iOS only, deferred.
- **Demo data, device list, webhook + push token, token rotation:** our host owns delivery; revisit with notifications.
- **Shell settings:** detect recent dirs from Claude/Codex history, open in tmux, export `MOSHI_CLIENT=1`. Ours could mean exporting `UNIPI_APP=1` and listing recent pi project dirs; maybe in v1.
- **Pro licence, What's New, Recommend, Discover:** business chrome.
- **Other app (`00-other-app-agent-team/`):** a multi-agent team chat with role agents, a human/agent roster, filters (All/Agents/Private/Humans), agent cards with tool chips, and "Leave — agents continue without you". Possible inspiration for a future multi-agent view of herdr panes as a "team"; not planned.

---

## 5. Architecture

### 5.1 Stack (versions checked 2026-10-05)

| Layer | Choice | Why |
|---|---|---|
| UI | **SolidJS 1.9** + TypeScript, **Vite 8**, **Tailwind 4.3** (`@tailwindcss/vite`), **Kobalte** (accessible primitives), **Corvu** (drawer and bottom sheets), Lucide icons | Official Tauri template. xterm.js and zod are native here. Fine-grained reactivity with no virtual DOM. Same stack as `../crates/kanboard/web`, so code can be reused. |
| Terminal | **xterm.js 6** + `addon-fit`, `addon-webgl`, `addon-unicode11`, `addon-web-links` | Proven in the PoC; WebGL keeps fast output smooth |
| Validation | **zod 4** | Every boundary is parsed; errors are humane (§5.5) |
| App core | **Tauri 2.12** (Rust), plugins: barcode-scanner, store, clipboard-manager, haptics, os, log, deep-link | Transport, keys and storage live in Rust |
| Command bindings | **tauri-specta** → generated TypeScript for Tauri commands and events | No hand-typed command names |
| P2P transport | **iroh 1.3** + `iroh-tickets` + iroh mDNS address lookup | Endpoints are dialed by public key; direct, hole-punched or relay |
| SSH transport | **russh 0.64** + `ssh-key` | Pure Rust, works on Android |
| Host | `unipi-host` (Rust): tokio, iroh, portable-pty, herdr socket client, tmux CLI, reqwest (kanboard) | One small daemon |
| Tests | vitest 5 + @solidjs/testing-library (UI); cargo test (Rust); a real emulator for end-to-end | |

Dioxus was dropped. In a Tauri webview it adds a WebAssembly ↔ JavaScript bridge on every terminal byte and every Tauri call, has no Tauri template, and can't use zod.

### 5.2 Components

```
unipi-app/
├─ apps/mobile/          SolidJS UI (src/) + Tauri core (src-tauri/)
├─ crates/proto/         Rust wire types + framing (serde), shared by host and app core
├─ crates/host/          unipi-host daemon + CLI (pair, serve, stdio)
├─ packages/protocol/    zod schemas = the TypeScript side of the wire contract
└─ protocol/fixtures/    JSON cases (valid + invalid) that BOTH sides must agree on

PC   unipi-host
     ├─ iroh endpoint, ALPN "unipi/app/1", allowlist of paired phone endpoint IDs
     ├─ stdio mode: the same protocol on one stream (used over SSH)
     ├─ herdr: socket API (tree, agent state, events) + PTY `herdr --session <s>`
     ├─ tmux: `list-*` -F formats + PTY `tmux attach -t <s>`
     └─ kanboard: forward HTTP/SSE to the local unipi-kanboard daemon
Phone Tauri core: connection manager (iroh, SSH), streams → Tauri Channels
     └─ SolidJS UI: xterm.js, screens; zod parses everything from the core
```

**Files scope (UNI-220).** Files, the editor, uploads and chat file links open paths under `$HOME`, a live session's cwd, or the host's extra safe roots: `/tmp`, `/var/tmp`, `/mnt`, `/media`, `/run/media/$USER`, `/srv`, `/opt` (those that exist), plus `unipi-host fs allow <path>` and minus `unipi-host fs deny <path>` (`config.json` `fs` block, no restart). System folders and other users' homes are never allowed; symlinks are judged by their target. `fs_roots` lists them and the Files screen shows them as the Places pills. Details: `docs/m7/FILES-SCOPE.md`.

### 5.3 "Whichever path works"

The user picks a host, not a network. The connection manager finds a route.

1. **iroh (paired hosts).** One mechanism covers every route:
   - **LAN:** the host binds on all interfaces and advertises them.
   - **The user's tailnet:** if the phone runs Tailscale, the host's `100.x` address is just another direct address. No Tailscale code or SDK is needed.
   - **Internet:** hole-punched direct path, or the **relay** as a last resort.
   - The path chip names the route in use: LAN (RFC1918), Tailnet (100.64.0.0/10), Direct (other IP) or Relay. iroh switches to a better route mid-session on its own.
2. **SSH (profiles).**
   - With `unipi-host` on the server, every logical stream is one SSH exec channel running `unipi-host stdio`, which speaks the same protocol. **Every feature works.**
   - Without it, **basic SSH**: the session list comes from exec (`herdr session list --json`, `tmux list-sessions -F …`), and terminal attach is a PTY channel running `herdr --session X` or `tmux attach -t X`.
3. **Auto order:**
   - A host can have both an iroh pairing and an SSH profile.
   - Auto tries iroh first. If no stream opens within 4 s, it races SSH, and the first one to open wins.
   - The winner is remembered for each host and network.

Security:
- iroh connections are authenticated by endpoint key, and the host rejects any endpoint ID that isn't on its allowlist before it reads a byte.
- SSH pins the host key on first use, and a mismatch is a hard stop.
- The phone's iroh secret and SSH private keys live in app-private storage. Wrapping them with an Android Keystore key comes later.

### 5.4 Wire protocol (`unipi/app/1`)

- Every logical operation is one **bidirectional stream**: one QUIC bi-stream on iroh, one exec channel on SSH, or (this PC's own desktop app, §5.10) one connection to the host's local socket `app.sock` — same frames, same Hello.
- **Frames:** `u8 kind · u32 length · payload`. The kinds are:
  - `0` JSON control (UTF-8, validated by serde on the Rust side and zod on the UI side)
  - `1` terminal output bytes
  - `2` terminal input bytes
- **The first frame** on a stream is a JSON `Hello{proto, op, …}`, where `op` is one of `list`, `tree`, `attach`, `shell`, `preview`, `pair` (built by M2), then `events`, `mux.action`, `board.http`, `board.sse` (later).
- **Replies:** the host answers `Ok{…}` or an `Err` envelope (§5.5), then streams.
- **Tree:** `{op:"tree"}` returns both multiplexers in one reply (§5.6).
- **Attach:** `{op:"attach", target, cols, rows, size_policy}`, where `target` is a herdr or tmux target (§5.6). After that, kind 1/2 frames carry bytes, and JSON frames carry `resize`, `exit` and `title`.
- **Shell (Skip):** `{op:"shell", cols, rows}`. Same streaming as attach.
- **Preview:** `{op:"preview", target, lines}` returns the last `lines` rows of the target's active pane as text with ANSI colours, for Home thumbnails.
- **Local-only admin ops (proto 1.14, §5.10):** `host_info`, `pair_begin{ttl_secs?}`, `pair_status{code, cancel?}`, `devices_list`, `device_revoke{id}` — only over `app.sock`; over iroh `E_UNAUTHORIZED` (even for paired phones). Contract: `protocol/fixtures/admin.json`.
- **Versioning:** `proto` is a semver string. The host refuses incompatible major versions with `E_PROTO_VERSION`, and the UI tells the user which side to update.

### 5.5 Errors (zod 4 at every boundary)

- **One envelope everywhere** (host → core → UI):
  `{ code: "E_…", message: string, hint?: string, path?: (string|number)[], retryable: boolean, detail?: unknown }`
- **Rust:** `thiserror` enums per layer (transport, mux, board, settings, pairing) map onto stable `code`s. There is no `unwrap` outside tests.
- **TypeScript:**
  - `packages/protocol` defines the zod schemas for every message, plus `AppError`.
  - The UI never touches raw payloads. Every Tauri command result and every channel message goes through `schema.safeParse`.
  - Validation failures become `E_SCHEMA` with zod 4's issue path, prettified for the log (`z.prettifyError`) and summarized for the user.
- **Contract tests:** the fixtures in `protocol/fixtures/` are checked by both the Rust tests (serde accepts or rejects) and vitest (zod accepts or rejects). If the two disagree, CI fails.
- **UX mapping:**
  - Retryable errors show an inline banner with Retry and a backoff countdown.
  - Errors that need a fix show a sheet: what happened, why, and one action ("Re-pair", "Trust new key?" → hard stop, "Install unipi-host").
  - Unknown errors show a short message plus "Copy details".
  - Raw stack traces are never shown.
- **Kanboard errors:** problems from the kanboard API (for example an illegal move) are passed through as `E_BOARD_*` with the daemon's own message.

Proven in the PoC:
- Stripping `HERDR_*` env vars before spawning a herdr client is required.
- `herdr --session <name>` attaches to the session, or creates it if missing.
- A raw byte channel (Tauri `InvokeResponseBody::Raw`) into xterm.js works.
- The herdr socket (`~/.config/herdr/herdr.sock`, protocol 22) answers newline-delimited JSON `{id, method, params}`. It offers `session.snapshot`, `pane.*`, `events.subscribe` and `pane.agent_status_changed`. The schema comes from `herdr api schema --json`.

### 5.6 Multiplexer model (herdr + tmux)

The host gives the phone one model for both multiplexers. The model is detailed enough that Chat can later bind **one pi to one conversation** without guessing from the screen.

**Tree reply** (`tree` op): `{ herdr: HerdrState, tmux: TmuxState }`, each either `{available:false, reason}` or the tree below.

| herdr (socket API, protocol 22) | tmux (`list-sessions/-windows/-panes -F`) |
|---|---|
| session `{name, running, default}` from `herdr session list --json` | session `{id "$3", name, attached (client count), windows, created, activity}` |
| workspace `{id "w4E", label, number, focused, agent_status, tab_count, pane_count}` | window `{id "@5", index, name, active, panes}` |
| tab `{id "w4E:t16", label, number, focused, agent_status}` | pane `{id "%7", index, active, current_command, cwd, pid, title}` |
| pane `{id "w4E:p18", focused, title, terminal_title, cwd, agent?}` | pane `agent?` (below) |

Running herdr sessions are read with one `session.snapshot` call on each session's socket (`socket_path` from `session list`). Stopped sessions carry no workspaces.

**Pane agent** (same shape for both): `agent?: { name: "pi" | string, status: "idle"|"working"|"blocked"|"done"|"unknown", session?: { kind, value } }`.
- herdr reports this itself: `agent`, `agent_status` and `agent_session` (for pi: `kind:"path"`, the pi session file under `~/.pi/agent/sessions/`).
- tmux knows nothing about agents. The host marks a pane `name:"pi"` when its foreground process is `pi` (`pane_current_command`), with `status:"unknown"`. A later unipi change will set the tmux pane options `@unipi_status` and `@unipi_session` from inside pi, and the host reads them when present.
- **Chat binding (later):** a conversation is keyed by host + pi session file. That key survives pane moves, renames and reattach, which pane IDs do not.

**Attach targets** (discriminated on `mux`):
- `{mux:"herdr", session, workspace_id?, tab_id?, pane_id?}`: the host calls `workspace.focus`, then `tab.focus` and `pane.focus` as given, on that session's socket, then starts `herdr --session <name>` in a PTY with every `HERDR_*` variable removed.
- `{mux:"tmux", session, window_id?, pane_id?}`: the host creates a **grouped session** (`new-session -d -t <session> -s unipi-<device>-<n>`, `destroy-unattached on`), selects the window and pane, then runs `attach-session` in a PTY (`-f ignore-size` when the size policy is Keep PC size). A grouped session has its own current window, so the phone browsing windows does not move the PC client.

**What attaching does to the PC view:**
- herdr keeps one focus for the whole session (the snapshot has a single `focused_workspace_id`), so focusing a workspace from the phone also moves an attached PC client. M2 opens with a spike in a throwaway herdr session to confirm this. If it is confirmed, the picker shows "Also switches the PC view" under the Active pill the first time, and the setting **Follow phone focus** (default on) lets the user attach without changing focus.
- Sizing follows §3.3: herdr uses the latest client's size; tmux can ignore the phone's size.

**Safety for development:** `unipi-host serve --protect <session>` (repeatable) refuses attach and focus calls for that herdr or tmux session. Tests use a throwaway herdr session and a private tmux socket (`UNIPI_TMUX_SOCKET`, passed as `tmux -L`), never the user's own.

### 5.7 Platforms (UNI-117)

One frontend, three ways to run it. `apps/mobile/src/lib/platform.ts` says which one we are in and what works there; nothing else may sniff `__TAURI_INTERNALS__`.

| Mode | What | Detected by |
|---|---|---|
| `tauri-mobile` | The Android (iOS) app | Tauri + os plugin `android`/`ios` |
| `tauri-desktop` | The Linux / Windows / macOS app | Tauri + any other OS |
| `web` | A plain browser: `vite dev`, or this app's web build served by the kanboard daemon at `/` (it replaced `crates/kanboard/web`, see "Web build" below) | no Tauri |

**Capability matrix** (`can(cap)`):

| Capability | Phone app | Desktop app | Web | Fallback when missing |
|---|---|---|---|---|
| `hosts` (paired PCs, iroh) | ✓ | ✓ | — | inline notice in Settings → Hosts |
| `terminal` | ✓ | ✓ | — | Sessions shows the page notice |
| `chat` | ✓ | ✓ | — | Chat shows the page notice |
| `files` | ✓ | ✓ | — | Files shows the page notice |
| `blob` (attachments over iroh) | ✓ | ✓ | — | needs `hosts` anyway |
| `board` | ✓ (iroh → daemon) | ✓ (iroh → daemon) | ✓ (same-origin daemon JSON API) | — |
| `portForward` (Preview) | ✓ | ✓ | — | Preview's own "needs the app" card |
| `desktopStream` (Preview → Desktop, UNI-216) | ✓ | ✓ | — | `<Requires cap="desktopStream">` notice; a WebView without WebCodecs H.264 gets its own "can't play the stream" card |
| `notifications` (ntfy / UnifiedPush) | ✓ | — (later: native) | — | Settings section hidden |
| `haptics` | ✓ | — | — | Settings row hidden, calls are no-ops |
| `deepLinks` (`unipi://`) | ✓ | — (later) | — | paste the link |
| `updater` (APK; iPhone: notice only) | ✓ (Android: in-app APK update; iOS ad hoc: "open unipi.nrn.one/ios" card from the feed's `app.ios`) | — | — | Update card hidden |
| `desktopUpdater` (tauri-plugin-updater, UNI-263) | — | ✓ (AppImage, macOS .app, Windows NSIS update in place after a prompt; .deb/tar.gz get "Download") | — | card hidden |
| `camera` (QR pairing) | ✓ | — | — | "QR scanning works in UniPi on your phone. Paste the pairing link instead." |
| `fileDragDrop` | — | ✓ | ✓ | — |
| `clipboard` | ✓ (plugin) | ✓ (plugin) | ✓ (`navigator.clipboard`) | — |
| `backButton` | ✓ | — | — | Esc does Back everywhere |
| `windowTitle` | — | ✓ | ✓ | — |
| `nativeNotifications` (tauri-plugin-notification) | — (has `notifications`) | ✓ | — | Settings → Desktop hidden; the phone keeps ntfy/UnifiedPush |
| `tray` (system tray: show/hide, status line) | — | ✓ | — | — |
| `windowState` (size/position remembered, tauri-plugin-window-state) | — | ✓ | — | — |
| `localSharing` (this PC's sharing switches via its `unipi-host` CLI, UNI-248) | — | ✓ | — | phones and other PCs get "Ask the PC to turn this on", and can turn off what they have (UNI-253, docs/m7/SHARING.md) |
| `localHost` ("This computer": this PC's own host over `app.sock`, offline; admin ops, UNI-231/233) | — | ✓ (Linux, macOS; Windows: named pipe later) | — | the host is reached over iroh like any other PC; Settings → This computer hides the phones list and Pair a phone falls back to `unipi-host pair --json` |
| `nativeMenus`, `multiWindow` | — | — (later) | — | — |

**The standard notice.** `<Requires cap=… fallback=…>` (components/Requires.tsx) renders its children or `<UnavailableNotice>`: one look (tinted icon tile, title, one line) with three wordings from `unavailable(cap)` — "Not available in the web version" (works in the app), "Desktop only", "Phone only" (and "Not available yet"). `variant="page"` for a whole screen, `"inline"` inside a list or settings group; `fallback={null}` hides a control that has no use here (a haptics switch).

**Rule for new features.** Every feature that touches the native core, a plugin or a JS bridge declares the capabilities it needs (add a new one to `platform.ts` + this table if none fits) and what each mode shows instead. Gate the UI with `<Requires>` / `can()`; never let a missing capability surface as a raw error. Phone behaviour must not change when a feature adds desktop or web support.

**Web mode transport.** Only the Board works: `lib/board/transport.ts` `devTransport` talks to the kanboard daemon's JSON API as one synthetic PC (`dev`, named "This computer" when the daemon serves the page). Base `/kb` under `vite dev` (proxied to `UNIPI_KB_DEV_URL`), `''` (same origin) in a production web build, `window.__UNIPI_KB_BASE__` to override. A token-gated daemon (non-loopback bind or `--require-auth`) is opened once with `?t=<token>`; the daemon sets the HttpOnly `kb_token` cookie and every same-origin fetch / EventSource sends it. A 401 shows the "open the link with ?t=…" hint. For chat / terminal / files a future web transport needs the daemon (or `unipi-host`) to expose them over HTTP/WebSocket on the PC, plus auth stronger than the board's token, because those are shell access.

**Web build — the kanboard daemon's UI (UNI-117, decided 2026-10-09).** The kanboard daemon (`unipi/crates/kanboard`) embeds this app's web build instead of its own Solid UI. That leaves one frontend for phone, desktop and browser; later the user can pick between starting the Tauri app and opening the web page. Full design, parity table and leftovers: `docs/m7/KANBOARD-MIGRATION.md`.
- `npm run build:web` (`vite build --mode web` → `apps/mobile/dist-web/`, base `/`, plus a `unipi-web.json` version stamp). The `webStubs()` plugin swaps the app-only screens and layers in `main.tsx` (Sessions, Terminal, Chat, Files, Editor, chat layer, previewer) for an empty stub. They would only show the notice anyway, so this keeps xterm, CodeMirror, pdf.js and the chat stack out of the binary (6 MB vs 9.7 MB).
- Entry: the daemon's links `/p/<slug>` and `?project=&view=list` → `lib/webEntry.ts` remembers the project for the web PC and switches to `#/board`. `?t=` is the daemon's cookie redirect. A 401 later in the session shows the token hint (the transport turns the daemon's HTML gate page into `TOKEN_HINT`).
- Screens a browser can't run show the page notice with **Get the UniPi app** (https://unipi.nrn.one) and **Open the board**. Settings shows only what works there: the Hosts notice, Board, About. Chat-only actions (Go to a chat, Add to chat) are hidden.
- Supply chain into the daemon: `unipi/crates/kanboard/scripts/build-ui.mjs` takes the first that works of env dir → this checkout (`npm run build:web`) → the published tarball pinned in `ui.lock.json` (`scripts/publish-web.sh` uploads `releases/web/unipi-web-<ver>-<sha12>.tar.gz`, immutable, sha256-pinned) → the deprecated `crates/kanboard/web` with a cargo warning. `/api/health` reports `ui: {source, version}`.

### 5.8 Desktop shell (UNI-117)

At ≥ 900px outside the phone app (Tauri desktop, or web at desktop width) the app swaps its phone chrome for a desktop shell; below that, or on Android, nothing changes (every rule is under `html[data-shell=desktop]`).

- **Sidebar** replaces the bottom tab bar: brand, the Ctrl/⌘+K search button, Sessions · Chat · Board · Files · Preview · Settings (screens a mode can't use stay, marked "app", and show the notice).
- **List | detail.** A screen's list is its route; its detail is the overlay the phone shows full screen (chat layer, task page, editor, terminal). A screen declares the split with `registerPane({ screen, open, placeholder? })` (lib/shell.ts); the overlay carries `shell-detail` (or is one of the existing overlay classes) and is placed right of the list. A splitter resizes the list (drag, arrow keys, double-click resets; remembered per screen in `unipi.shell.listWidth`).
- **Keyboard.** Ctrl/⌘+K command palette (screens register commands with `registerCommands`, `when` hides those that can't run), Ctrl/⌘+1…6 screens, Esc = Back (closes the top sheet, menu, pane).
- **Mouse.** Right-click opens the same long-press menus, at the cursor (`menuAnchor(event, el)`); hover states on cards, rows, menus and icon buttons.
- **Sheets become dialogs.** Bottom sheets (`.sheet`, corvu drawers) render as centred dialogs, 560px wide, fade + scale instead of slide.
- **Board** shows lanes side by side as columns (the pager stops snapping); the task page opens in the detail pane. Kanboard-UI parity on the desktop shell (UNI-117 migration): **drag & drop** cards between columns and reorder within one (`lib/board/dnd.ts`). Refused lanes fade, an insert line snaps to chain starts, a move that needs a note opens the comment sheet and then places the card, an undoable move keeps its natural position, running and final cards refuse with a toast, and the strip and columns autoscroll near the edges. **Keyboard** (`lib/board/keys.ts`): C new, / search, J/K/↑↓ select (cyan ring), Enter open, B/L views, R/X review/blocked, ? shortcuts sheet, Esc clears the text filter. **OS file drop** and **Ctrl/⌘+Enter** on the note composer, description editor, New task and comment sheet (`lib/board/fileDrop.ts`, `can('fileDragDrop')`). The phone keeps long-press menus.
- **Desktop window** (tauri.conf.json): 1280×820, min 480×560, centred, dark background. Phone-only plugin permissions (barcode scanner, haptics) moved to `capabilities/mobile.json` (`platforms: android, iOS`).

**Desktop-first screens (UNI-117 part 2).** Pure key decisions live in `lib/desktop.ts` (unit-tested); the full list is `shortcuts()` there, shown by Ctrl/⌘+/ and the palette's "Keyboard shortcuts".
- **Chat:** list | conversation; the transcript column is 940px with prose capped at 76ch. Composer: Enter sends, Shift+Enter new line; while pi works Enter sends with this chat's last Steer/Queue choice (Steer the first time, as in pi's TUI) and Ctrl/⌘+Enter opens the Steer / Queue choice (S / Q, ←/→ + Enter, Esc). Esc in the composer while pi works stops it. Ctrl/⌘+L focuses the composer, Alt+↑/↓ switches to the previous/next chat of the list. Files dropped on the chat attach (images inline, others uploaded as chips); images pasted from the clipboard attach (`fileDragDrop`). Messages show Copy / Quote / Start from here / Fork / Retry / More on hover instead of tap-to-select; right-click a message opens its action menu at the cursor (links, file refs, task refs keep their own menus). The model picker / control centre are the standard centred dialogs. The open chat is marked in the list.
- **Chat + browser (UNI-296):** the PC's browser opens as a resizable side pane right of the chat (globe icon in the chat header, ⋯ → Browser, Ctrl/⌘+Shift+B; width and open state remembered; the chat list folds away when list + chat + browser don't fit). On the phone it is a full-height sheet over the chat. The pane's header names the browser profile (a picker when there are several), shows Stealth / Not stealthy (with the Chrome fallback's reason on a line below), then the URL row. While an agent drives, an ember strip "Agent is using the browser · Take over"; taken over, a cyan strip "You have control · the agent waits · Give back" (docs/m7/BROWSER.md).
- **Sessions / Terminal:** session list | terminal. xterm takes the physical keyboard and mouse directly (its own key encoding, mouse reports, wheel, Shift+drag selection); the phone toolbar, arrow pad, IME textarea and touch gestures are off. Ctrl+Shift+C / Ctrl+Shift+V copy/paste (⌘C/⌘V on a Mac), Ctrl+= / Ctrl+- / Ctrl+0 font size (saved to Settings), Ctrl+Shift+K the palette (plain Ctrl+K is the shell's), Esc goes to the shell. Live connections show as tabs over the terminal pane (✕ detaches; the session keeps running on the PC).
- **Files:** list | editor (the list stays mounted beside the editor; picking another file while the editor has edits asks Save / Discard first). ↑/↓/Home/End select, Enter opens, Backspace goes up, F2 renames, Delete asks then deletes, right-click = the row menu. Files dropped from the OS upload into the folder shown. Editor: Ctrl/⌘+S saves, Ctrl/⌘+F finds; the phone key row is hidden.
- **Settings:** two columns — the section list | the picked section (↑/↓ move in the list). A "Desktop" section (desktop app only) switches native notifications.
- **Native (desktop app, `src-tauri/src/desktop.rs`):** tray icon with a status line ("2 agents need you · 1 working"), Show/Hide and Quit; OS notifications for new "needs you" / finished items while the window isn't focused (first look only seeds, max 3 at once, own 20 s poll); the window title follows the screen and the open chat / terminal / file; window size and position are remembered. `desktop_status`, `desktop_notify`, `desktop_set_title` exist on Android too and answer `E_UNSUPPORTED`; the plugins and the tray are registered under `#[cfg(desktop)]`, permissions in `capabilities/desktop.json` (`platforms: linux, macOS, windows`).

### 5.9 Releases, downloads and updates (UNI-263)

One script, `scripts/release.sh X.Y.Z`, builds every artifact into `releases/X.Y.Z/` (Linux parts here; Apple parts on `ssh mac` in parallel via `scripts/mac-build.sh` and `scripts/ios-build.sh --only adhoc`; Windows cross-built with cargo-xwin + makensis from `scripts/setup-release-tools.sh`), signs the desktop updater artifacts, uploads versioned files, then flips the pointers. Server layout and recovery: `.agents/skills/unipi-app/references/deploy.md`.

**Permalinks** (always the newest): `https://unipi.nrn.one/download/<kind>` with kind = `android`, `ios`, `macos`, `windows`, `linux` (AppImage), `linux-deb`, `linux-tar`, `host-linux-x64`, `host-linux-arm64`, `host-macos` (Intel; runs on Apple silicon too), `host-macos-x64`, `host-macos-arm64`, `host-windows-x64`. Also `/unipi.apk`, `/ios` (itms-services install page), `/install.sh`, `/install.ps1`. `/` lists them all with version, size and sha256.

**Feeds.**
- `/latest.json` — the contract old app builds already parse (`app{version,build,url,sha256,size,notes,date}`, `host{version,files}`), fields only added: `app.ios{version,url,page,size,sha256}`, `host.files` for `darwin-x64`, `darwin-arm64`, `windows-x64`, `desktop{…}` (same as below), `downloads{kind:{url,file,version,size,sha256,label}}`, `install_windows`.
- `/desktop/latest.json` — Tauri v2 updater shape: `{version, notes, pub_date, platforms{"linux-x86_64[-appimage]", "darwin-x86_64[-app]", "darwin-aarch64[-app]", "windows-x86_64[-nsis]": {url, signature}}}`; macOS uses one universal `.app.tar.gz` for both arches.

**Who updates how.**
| What | Mechanism | Decision |
|---|---|---|
| Android app | `lib/update.ts` + `UpdateBridge.kt`: APK sha256 → system installer | banner + Settings → About; the user taps Update |
| iPhone app (ad hoc) | none possible; `app.ios` → "open unipi.nrn.one/ios" card | the user installs from Safari |
| Desktop app (AppImage, .app, NSIS) | `tauri-plugin-updater` (app_update.rs), minisign signature checked against `plugins.updater.pubkey`; check 8 s after start + every 4 h + "Check for updates" | always a prompt (Later / Update and restart), never silent |
| Desktop app (.deb, tar.gz) | can't replace itself without root | prompt/card shows "Download X" (the download page) |
| unipi-host | `unipi-host update` (feed → platform file → sha256 → `--version` check → atomic replace → restart systemd unit / LaunchAgent `dev.unipi.host` / task "UniPi Host"); desktop app's This computer **Update** | owner setting `update.mode` prompt (default) / auto (`serve` updates itself) |

**Pair a phone from the desktop app (UNI-266).** Settings → This computer → *Pair a phone*, the Add a PC dialog's "Pair a phone with this PC" link and the palette's *Pair a phone* open one dialog (`components/PairPhone.tsx`, `lib/pairPhone.ts`; desktop app only, `can('localSharing')`). The core (`src-tauri/pair_phone.rs`, `pair_phone_start` with a Channel, `pair_phone_cancel`, `local_host_start`) runs this PC's own `unipi-host pair --json --ttl 10m` — found like the sharing CLI: `$UNIPI_HOST_BIN`, `~/.local/bin`, `%LOCALAPPDATA%\UniPi\unipi-host.exe`, `PATH` (+ `/opt/homebrew/bin`, `/usr/local/bin` on macOS). `pair --json` prints `{"state":"waiting","link","code","expires_at","host"}`, then `{"state":"paired","device"}` or `{"state":"expired","reason":"timeout|replaced|locked|cancelled"}`; errors are one `{"state":"error","message","hint"}` line, exit 1 (serve not running = no answer on its control socket/pipe, not just a leftover endpoint.json). Closing stdin (a pipe) cancels and removes the code. The QR is an SVG from `uqr` (EC M, 4-module white quiet zone). States: starting / waiting (QR, 10:00 countdown, code, Copy link) / paired / expired (New code) / error (not installed → install command; not running → Start unipi-host + the service command; too old → update). Windows host data dir: `%USERPROFILE%\.unipi\app-host` when HOME isn't set (the logon task's old `%LOCALAPPDATA%\UniPi\.unipi\app-host` is kept if it already holds the identity). install.ps1 puts `%LOCALAPPDATA%\UniPi` on the user PATH (REG_EXPAND_SZ, idempotent, broadcasts WM_SETTINGCHANGE).

**Keys.** Android: `~/Projects/Neuron/credentials/android/` (never changes). Desktop updater: `~/Projects/Neuron/credentials/unipi-updater/unipi-updater.key` + `password` (minisign, chmod 600; the public key is in tauri.conf.json — losing the private key means every desktop install must be reinstalled by hand). Apple: ad hoc signing as in `scripts/ios-build.sh`; macOS desktop is ad-hoc codesigned (no Developer ID yet: right-click → Open the first time).

---

### 5.10 This computer: the local host (UNI-228…234)

The desktop app reaches **this PC's own** `unipi-host` without iroh, so it works with no internet; plan and decisions: `docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md`.

- **One host per data dir** (`~/.unipi/app-host`, or `$UNIPI_HOST_DIR`). `serve` holds an exclusive lock on `host.lock` for its life, which records `{pid, version, mode, started_at}`. `mode` is `service` (systemd/LaunchAgent/logon task; the installers pass `--mode service`, systemd is detected anyway), `app` (spawned by the desktop app, with `--exit-with-parent <pid>`) or `manual`. A second serve exits 75 naming the owner. `unipi-host status [--json]` reports it (exit 3 = not running).
- **Local socket** `<dir>/app.sock` (0600 in a 0700 dir). Each connection is one op, exactly like a QUIC bi-stream: same frames, same Hello, same `Handler::handle`, with `Peer::Local`. Only the same user may connect (`SO_PEERCRED`/`getpeereid` uid check). That user can already read `secret.key` and the pi sessions, so this gives away nothing new.
- **`Peer::Local` is the owner.** No pairing needed, and pairing over the socket is refused. It counts as device `local`, named "This computer", and every per-device allow list (forwarding, browser, desktop, push, sharing) allows it.
- **Admin ops**, local only: `host_info` (version, proto, mode, pid, tools found, phone count), `pair_begin` / `pair_status` (the QR flow; `cancel` closes it at once), `devices_list` / `device_revoke`. The CLI `pair` shares the same pairing code path.
- **App core:** `OpSend`/`OpRecv` (`op_stream.rs`: Iroh or Local halves, AsyncRead/AsyncWrite plus `finish()`). `NetState::open_op` is the single entry point. When the ticket's id is the local host (`local_host.rs`: socket present plus `endpoint.json`), it connects to `app.sock`; if that fails, it falls back to iroh. `path_for` reports `PathKind::Local`.
- **"This computer" in the UI:** a synthetic host (not persisted, `transport: "local"`, path chip **Local**) at the top of the list. An existing pairing with the same id merges into it, keeping its name and its iroh ticket as the fallback. It can't be forgotten.
- **Settings → This computer** (UNI-233) shows "Run by the background service / this app / a terminal", the phones list (last seen, Unpair with a confirm), and **Pair a phone** through `pair_begin`.
- **Features on This computer:** web apps open `http://127.0.0.1:<port>` directly (no tunnel); the Desktop stream tab is hidden (it would be this same screen); the remote browser stays.
- **Older hosts** (< 0.4.8, no socket): everything goes over iroh as before; the admin-only UI hides itself.
- **Not yet** (UNI-235…238): the app adopting / starting / spawning the host (supervisor), the bundled sidecar, close-to-tray and Always on.

## 6. Open questions

1. ~~herdr multi-client sizing~~ **Answered in M0:** the latest client wins, and there's no ignore-size option. "Keep PC size" is tmux-only (§3.3). We could ask herdr upstream for an ignore-size client flag.
2. **Background delivery on Android** (deferred with notifications): a foreground service holding the connection, UnifiedPush/ntfy, or FCM.
3. ~~Chat bridge source~~ **Answered 2026-10-07:** a unipi extension (app-bridge) inside pi; RPC mode can't attach to a running TUI (§3.4).
4. **Host packaging:** for the MVP, `cargo install --path crates/host`. Later, an npm platform package like `kanboard-bin`, so `unipi` installs it.
5. **Thumbnails:** periodic `preview` snapshots (herdr `pane.read`, tmux `capture-pane -p -e`; every 2 s while Home is visible) vs a reduced-rate stream.
6. **herdr focus:** is focus per server or per client? The M2 spike answers it (§5.6).

---

## 7. MVP plan

Scope: Terminal (herdr + tmux, both first-class), Board, Settings, iroh + SSH transports, pairing, auto-reconnect.
Not in the MVP: Chat, Inbox, push notifications, attachments, dictation, slash palette, desktop and iOS builds.

| Milestone | Delivers | Done when |
|---|---|---|
| **M0 Foundation** | Monorepo; Solid + Vite + Tailwind + Tauri Android shell with Unicrab tokens and the nav shell; `packages/protocol` (zod) + `crates/proto` + fixtures contract test; error envelope; Settings store; herdr sizing spike | The APK boots to the 3-tab shell in the emulator; contract tests pass; the spike result is written into §6 |
| **M1 Host + iroh** | `unipi-host serve/pair/stdio`; allowlist; herdr tree/list/attach; app connection manager (iroh) with path chip; QR pairing flow | The phone pairs by QR and lists herdr sessions over LAN; still works with Wi-Fi off (Tailnet or Relay) |
| **M2 Terminal (herdr + tmux)** | Chat tab placeholder; `tree`/`attach`/`shell`/`preview` ops; host herdr + tmux model (§5.6); session picker sheet with Skip; Home (recent cards, hosts); terminal view, toolbar, gestures, composer, reconnect | Daily-drivable on the user's real herdr and tmux setups, over iroh |
| **M3 SSH** | SSH profiles (full via `unipi-host stdio`, plus basic); Skip as a direct SSH shell; key generation; host-key pinning | The M2 flows work over SSH, with and without unipi-host |
| **M4 Board** | Host kanboard bridge; Board screens; SSE live updates | Add, move and note round-trip against the real board |
| **M5 Polish** | Haptics, motion, empty states with the crab, error sheets, accessibility, performance pass | A UX review with the user signs it off |

## 8. Screenshot index

The Moshi reference set lives at `/home/oi/Downloads/moshi/`, grouped in 11 folders with an `INDEX.md` mapping each file to its original name.

| Folder | Count | Contents |
|---|---|---|
| `00-other-app-agent-team` | 7 | Not Moshi: agent-team chat app (zh/en) |
| `01-home-hosts-sessions` | 3 | Home, connections, session thumbnails |
| `02-add-connection-pairing` | 10 | QR pair, manual SSH/Mosh/ET forms, LAN discovery, key chooser |
| `03-session-picker-tmux-herdr` | 5 | Session picker sheet: [Tmux \| Herdr] + Skip, herdr workspaces with Active, tmux session In use |
| `04-terminal` | 4 | Terminal: minimal chrome, expanded toolbar, keyboard |
| `05-chat-composer-attach` | 3 | Composer, alternate actions, attachment sheet |
| `06-inbox-files` | 2 | Agent events inbox, files (both empty) |
| `07-settings-general` | 15 | Settings root, theme, fonts, chat mode, sidebar, advanced, shell, keys, icon |
| `08-settings-input` | 21 | Toolbar, zen ring, composer config, shortcuts, keyboard, gestures |
| `09-settings-speech` | 2 | Speech engines, replacements, history |
| `10-settings-hooks-notify` | 5 | Agent hooks setup, notifications, devices, webhook |
