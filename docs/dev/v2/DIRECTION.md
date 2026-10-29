# Oino v2 direction — UniPi on Oino

Status: **decided by the owner** (discussion session, 2026-10-29/31). This file is the input
contract for the v2 graph. It supersedes [v1 CHARTER](../v1/CHARTER.md) items 2–3 (GPUI as
the product, no TUI), [gpui-app.md](../v1/gpui-app.md), and the transport and service
parts of [round6-direction.md](../v1/round6-direction.md). Everything in v1 not contradicted
here still holds (pi 1.4.0 parity, pi session format v3, WASM providers, ACP).

Rule for all work: **finish complete specs, not a half run.** Every phase ends green
(build, tests, lints, QA) before the next one depends on it.

---

## 1. What Oino is and what UniPi is

| | Oino | UniPi (on Oino) |
|---|---|---|
| Role | A **blank coding-agent core** with pi 1.4.0 parity (agent loop, sessions, tools, resources, compaction, settings, extension runtime) | The **official extension set + host + app** built on Oino |
| Ships | `oino` binary: interactive TUI, or a CLI when run non-interactively (print / JSON / RPC / ACP modes) | unipi WASM extensions (+ their CLIs and skills), `unipi-host`, the UniPi app |
| Extensions | None compiled in. Discovers installed extensions like pi | Bundled by the UniPi distribution, not by `oino` |

- **Monorepo like pi's**: one repo, tidy package split (core, agent, ai, session, tools, TUI,
  RPC, ACP, …), with unipi living under `extensions/unipi/*`, its host and its app. A user can
  build on Oino core without any unipi part.
- `oino-bundle` (extensions compiled into the `oino` binary) moves to the **UniPi
  distribution**: the installers ship the extension packages, and `oino` loads them from the
  extensions dir.
- The TUI (`oino-tui`) is **kept** as a client package, like pi's TUI. It is not frozen for
  deletion any more.

## 2. Architecture

```
  phone app ──iroh──┐
  other PC app ─iroh┤
                    ▼
  UniPi app (Tauri) ──local socket──► unipi-host ──links──► oino-core (+ wasm host)
   "This computer"                     sessions, cron, browser, fs, git, pty,
                                       pairing, push, forward, sharing
  oino TUI / CLI ───────────────────► oino-core (same session files, same lock)
```

- **unipi-host** is a process that links oino-core in-process and runs **all sessions** in
  one process (ZCode and infinity both do one host for many sessions; cheaper than process
  per session). It is a *client* of core, like the TUI. It is never linked into the UI
  process.
- **The app** (Tauri 2 + SolidJS) is a pure client of a host.
  - On a PC: "This computer" (local socket, works offline) plus paired hosts over iroh.
  - On the phone: paired hosts only (pure remote client).
  - A PC is host and client at the same time.
- **Host lifetime** (owner Q4 = no service install):
  - The app adopts a running host or spawns it as its child.
  - Tray icon with Show / Hide / Quit. On window close the app **asks once: close or minimize
    to tray** (remembered, changeable in Settings).
  - A **start at login** setting uses per-user login items (Windows Startup/Run key, macOS
    login item, Linux XDG autostart) and starts minimized to the tray. No admin, polkit or
    service permissions.
  - Cron and remote access work while the tray is alive. Quit stops them.
- **Transport**: iroh (QUIC, stable endpoint id, n0 relays and discovery, hole punching,
  relay fallback, optional self-hosted relay). Pairing with QR and link, short-lived code with
  lockout, a paired-devices list with revoke. Reference: unipi-app `crates/host`
  (`endpoint.rs`, `pair_link.rs`, `pairing.rs`, `local.rs`) and `apps/mobile/src-tauri`
  (`net.rs`, `local_host.rs`, `host_store.rs`, `host_supervisor.rs`).
- **App ↔ host protocol**: keep unipi-app's `proto` / `packages/protocol`, made as
  performant as possible. Only the chat/session part is rewritten for oino sessions. Herdr,
  tmux, pi app-bridge and desktop streaming are removed from it.
- `oino --mode rpc` and ACP stay for third-party clients and editors.
- `oino cron serve` is **merged into unipi-host** and removed from `oino`.

## 3. Core work (the freeze is lifted for exactly this scope)

All of it stays inside pi session format v3 (extension state uses `custom` entries), and
the pi parity tests stay green. Measure before and after.

### 3.1 Durability (model: deepseek-harness)
1. The session log is the only truth. Model history, UI and wake-up are derived from it.
2. **Flush points** (fsync, fail-closed): before each model request, before each tool with
   side effects, at each step boundary. Stream chunks are never persisted; one settled
   message per model attempt.
3. **Tool call recorded before it runs.** On resume, open calls are closed as "interrupted,
   unknown outcome" and never auto-retried.
4. **Interrupted-turn repair on resume** (in the resume layer). Only a torn tail is
   discarded, never truncation.
5. **Single-writer lock per session.** Opening a session that runs elsewhere (TUI, another
   app, another host) **closes the other runner and reloads the session here** to continue
   (Devin-like).
6. **Extension state rule**: every extension must be rebuildable from its session `custom`
   entries plus its kv store. State held only in memory is disposable. Audit and fix the
   unipi ports (long-horizon, background-tasks, subagents, workflow first).

### 3.2 Efficiency (model: hydro-project/infinity)
1. One shared wasmtime `Engine` (and compiled component cache) for all sessions.
2. Lazy session loading: a session is a record until a message arrives.
3. **Hibernating subagents**: an idle child drops its task and WASM instances and is woken
   by a cold resume from log plus kv (depends on 3.1).
4. Lighter children: a subagent instantiates only the extensions its spec needs.
5. Benchmarks: RSS and CPU for 1 / 10 / 50 concurrent subagents, before and after.

### 3.3 Crash protection
1. A trapping or panicking extension is disabled for that session; the app shows a notice.
2. Per-session panic isolation in the host: a crashed session is marked crashed and the
   other sessions keep running.
3. Safe mode: after 2 failed host starts, start with extensions off and a banner naming
   the suspect.
4. `oino --exclude-extension <id>` (repeatable), plus pi's `--no-extensions`. A
   per-extension toggle in the app.

### 3.4 Extension API (WIT) changes
1. **Remove `kanboard.wit`.** Kanboard becomes unipi-style: extension (guard, monitor,
   reminders, bus hooks) + skill + the `unipi-kanboard` CLI shipped inside the extension
   package and called via `process.exec`. **No kanboard daemon.** The app's Board uses the
   CLI.
2. **Replace `cron.wit` and `browser.wit`** with one generic host-service call
   (`host-service.call(name, op, json)`, permission per service name). Oino carries no
   unipi-specific ABI.
   - Cron: jobs are files written by the cron extension; unipi-host fires them.
   - Browser: a unipi-host service. Under a plain `oino` TUI the browser tools report
     "unavailable".
3. Extension UI protocol v2 (§5).

## 4. The app

### 4.1 Source and identity
- Commit a checkpoint on a new branch, then **delete** `apps/oino-ui`, `apps/oino-desktop`,
  `apps/oino-mobile`, `apps/vendor` and `apps/casing`. No GPUI from then on.
- Copy the **newest unipi-app (1.0.0)** from
  `/home/oi/Projects/Personal/archived/unipi/unipi-app` (`apps/mobile`, `crates/host`,
  `crates/proto`, `packages/protocol`, MIT) and build from there. Do not modify the archived
  repo or touch the owner's live `~/.unipi` or its running unipi-host.
- The product is still **UniPi** (package name `unipi-oino` is acceptable). Keep the
  Unicrab brand, palette, mascot, floating mascot and lore.
- Tauri identifier `dev.unipi.oino`. Data in `~/.oino` (host data in `~/.oino/host`). Own
  iroh key, socket and tray name, so it **coexists with the old unipi**.
- Migration from `~/.unipi` (memory, kanboard, cron, profiles) is a **skill** the agent runs
  on request. There is no automatic import.

### 4.2 UI rules
1. **Every unipi harness feature is reachable from the UI.**
2. **No redundant UI**: no two surfaces for the same thing.
3. **ZCode UX differences are listed and ruled on by the owner** before implementing them
   (§7 phase A). Where ZCode's chat performance is better, take theirs:
   - TanStack virtual with a whole turn as the virtual unit.
   - LRU row-height cache keyed by turn id.
   - Memoized rows and lazy detail render.
   - A single bottom-lock follow state.
   - Per-session scroll memory.
4. Desktop and phone share one codebase. Remove unipi-only screens tied to herdr/tmux. The
   desktop streaming view is deferred.

### 4.3 Screens and features
| Area | Decision |
|---|---|
| Chat, Sessions | Oino sessions grouped by project, many running at once. ZCode chat optimizations |
| Terminal | Like ZCode: xterm.js, PTY on the host (works from the phone), multiple tabs, bottom panel + side-pane tabs. tmux/herdr are the user's business. Agent bash output stays in chat tool rows |
| Browser | §6 |
| Files + editor | Tree + CodeMirror editor + LSP like Zed (rust-analyzer and others) |
| Git pane | Follows ZCode (review-oriented, last-turn scope, lazy per-file diffs). **Long-horizon status lives in this pane**, plus a small capsule that stays visible when the pane is collapsed (replaces the top-right capsule) |
| Per-turn file summary | ZCode's collapsed "N files +X/−Y" line at the end of each turn, expand to files, undo per file or per set. **An extension**: snapshots files before write/edit (content-addressed, compressed, ≤5 MB per file, deleted with the session). Bash changes are not captured (like ZCode) |
| Image annotation | Add an **ellipse/circle** tool. Fix: images above the composer vanish when switching chats (move pending images into a per-chat store like `pendingFilesStore`) |
| Info / Session modal | Context / Running / Info tabs (info-screen extension), as today |
| Board, Cron, Settings, SetupWizard, Preview, Editor | Kept |
| Host features | All kept: push notifications (ntfy/UnifiedPush, E2E encrypted), port forwarding, sharing/blob transfer, QR pairing, relay settings, host supervisor, app updater |
| Settings | JSON for both pi agent `settings.json` and app/host settings, in **one** Settings screen |
| TUI-only extensions | Removed, or ported when useful in the GUI (e.g. `/answer` removed; info-screen kept) |

## 5. Extension UI (ratatui style, pi conventions)

- pi's `ctx.ui` methods stay as they are (parity): `select`, `confirm`, `input`, `editor`,
  `notify`, `setStatus`, `setWidget`, `setTitle`, `setHeader`, `setFooter`, `custom`.
- Panel and widget trees use **ratatui vocabulary**:
  - Widgets: `Block` (title, borders), `Paragraph`, `List`, `Table`, `Tabs`, `Gauge`,
    `LineGauge`, `Sparkline`, `BarChart`, `Chart`, `Scrollbar`.
  - GUI extras: `Markdown`, `Code`, `Diff`, `Image`, `Log`, `Tree`, `KeyHints`, `Input`,
    `Toggle`, `Select`, `Button`, `Spinner`, `Empty`.
  - Layout: direction + `Constraint::{Length, Percentage, Ratio, Min, Max, Fill}`.
  - This is UI protocol **v2**: old OUS v1 node names are dropped, and the TUI renderer and
    unipi extensions are updated. A guest-side Rust builder crate mirrors ratatui's API.
- The same tree renders in the TUI (ratatui) and the app (SolidJS components, themed),
  locally and over iroh. No extension JS, no iframes.
- **Slots** (cordis-style: an entry registry pushed as data, ordered, disposed when the
  extension unloads):

  | Placement | App slot |
  |---|---|
  | `overlay` | modal |
  | `sidePane` | right-pane tab |
  | `bottomPane` | bottom-panel tab |
  | `editorReplace` | composer tray |
  | `fullscreen` | sidebar page |
  | `setWidget` | above the composer |
  | `setStatus` | status bar |
  | `turnFooter` (new) | end of turn (file summary) |
  | extension settings schema | Settings section (schema-driven form) |
- **Rich first-party screens** (Board, Cron, Files, Git, Browser, Terminal, Info) are
  built-in components opened via **reserved panel ids**. They are shown only when the owning
  extension is enabled and talk to the host through services or the CLI. Third-party
  extensions get widgets only.

## 6. Browser (anti-bot)

Research: memory `oino_browser_stealth_research_2026`.
- **Base: `oino-browser`** (pipe transport, isolated worlds, AX snapshots, session-owned
  tabs, humanize), run as a unipi-host service.
- Port from unipi-app `crates/host/src/browser`:
  - The CloakBrowser downloader/verifier (`cloak.rs`, Ed25519-pinned, first-use download,
    never redistributed; users install it themselves).
  - Cookie import from the user's real Chrome (`cookies.rs`).
  - The agent/user handoff gate (`gate.rs`).
- Engines: **CloakBrowser, headful** by default (real window, hidden or on Xvfb). **The
  user's real Chrome + dedicated profile** for Google sign-in and sensitive sites. Camoufox
  and Fingerprint-Chromium are on the watchlist only. Never render agent pages in the Tauri
  webview; no window reparenting.
- CDP hygiene:
  - Never `Runtime.enable` or `Console.enable` (oino calls `Runtime.enable` today in
    `profile.rs` — remove it).
  - Evaluate in isolated worlds with an explicit `contextId`.
  - No main-world `addScriptToEvaluateOnNewDocument`; avoid `Runtime.addBinding`.
  - No `--enable-automation`; add `--disable-blink-features=AutomationControlled`.
  - Never `--disable-gpu` or SwiftShader when headful.
  - Persistent aged profiles, one fingerprint seed each.
  - Humanized agent input. Optionally detach CDP during loads on sensitive sites.
- CI canaries: leaky.page, bot.sannysoft.com, CreepJS, browserscan, deviceandbrowserinfo.
  Must score no worse than a real-Chrome baseline.
- Pane and phone view use `Page.startScreencast` (not page-detectable).
- ZCode UX on top: tab strip + URL bar in the side pane, element picker → composer chip,
  user screenshot → composer.

## 7. Long-horizon reliability (graph failures)

- A node failing for a transient reason (429, network, provider 5xx, crashed worker)
  **auto-retries from its own session**, up to 3 times with backoff 30 s / 2 min / 10 min.
  Retry-After waits don't count.
- After that, the node is failed and its downstream nodes stay blocked. The operator agent
  is nudged to replace, skip or re-plan, and the user is notified.
- Depends on core durability (§3.1).

## 8. Distribution

- One installer per platform, nothing else to install: **NSIS `.exe`** (Windows), **`.dmg`**
  (macOS), **`.deb`** (Linux), **`.apk`** (Android). iOS stays build-compatible from the same
  Tauri codebase, not shipped yet.
- The installer bundles the app, `unipi-host`, `oino` (put on PATH), the unipi extension
  packages and their CLIs (`unipi-kanboard`, …).
- In scope for testing: Windows, macOS, Linux and an Android phone (builds run on coffee
  (Windows) and desmondgiam (macOS); the phone is plugged in).

## 9. Process

- **Phase A: ZCode UX audit** (`opus-ui-high`): list every ZCode vs unipi-app UX difference
  (chat, composer, sidebar, model picker, tool rows, git, terminal, browser, palette,
  motion, empty/error states). **Stop for owner rulings** before UI work.
- Then, in dependency order:
  1. Checkpoint + delete.
  2. Core work (§3).
  3. Repo/package split and bundle move (§1).
  4. unipi-host.
  5. App copy + chat/session rewrite.
  6. Screens.
  7. Browser + terminal.
  8. Extension UI v2 + slots.
  9. Long-horizon recovery.
  10. Installers.
  11. Full QA on every platform.
- UI implementation and UI decisions: `opus-ui` (medium) or `opus-ui-high` (hard). UI QA:
  `opus-ui-qa`. Model `omniroute/cc/claude-opus-5-5`.
- Commit prefixes per work id. Never touch the owner's live `~/.unipi`, `~/.pi` or running
  unipi-host.
