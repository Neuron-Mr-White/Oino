# Research: GPUI desktop + mobile frontend for Oino v1

Status: research note (work id `audit-oino`, 2026-10-26). No code changes.
Charter goal: a GPUI desktop + mobile distro, speed first, UX learned from ZCode, that
bundles oino-core + provider extensions + all unipi extensions; embeds core in-process
(fast path) and can also talk RPC/ACP to a remote oino.

Sources (cloned/fetched 2026-10-26; GitHub API for stars/dates):

| Project | URL | HEAD read | License | Stars / last push |
|---|---|---|---|---|
| gpui-mobile | <https://github.com/itsbalamurali/gpui-mobile> | `5fd54f4` (2026-10-23) | GPL-3.0+ **or** AGPL-3.0+ **or** Apache-2.0 (Cargo `license`) | 438 / 2026-10-23 |
| gpui-kit (formerly gpui-component) | <https://github.com/longbridge/gpui-kit> (`longbridge/gpui-component` redirects) | `7b14e03` (2026-10-24) | Apache-2.0 code, CC BY 4.0 docs | 18.6k / 2026-10-26 |
| longbridge gpui-mobile fork | <https://github.com/longbridge/gpui-mobile> | — | as upstream | 47 / 2026-10-20 |
| ZCode | <https://github.com/zai-org/ZCode> | `3c5b2276` (2026-10-20, v3.16.2) | Apache-2.0 | 8.2k / 2026-10-20 |
| Zed `gpui_tokio` | <https://github.com/zed-industries/zed/blob/main/crates/gpui_tokio/src/gpui_tokio.rs> | main | Apache-2.0 (gpui crates) | — |
| Local experiment | `/home/oi/Projects/Personal/unimportant/gpui-learn` | — | — | — |

## 1. GPUI crate landscape (versions matter more than anything else)

| Crate (crates.io) | Latest | Date | Publisher | Notes |
|---|---|---|---|---|
| `gpui` | 0.2.2 | 2025-10-22 | zed-industries | Official, **a year stale**; what `gpui-learn` uses |
| `gpui-pre` | 0.3.10 | 2026-10-26 | huacnlee (Longbridge) | Weekly snapshots of Zed `crates/gpui` (0.3.10 = zed@08cbb88); siblings `gpui-pre-platform`, `-macros`, `-wgpu`, `-web`, `-sum-tree`, `-reqwest-client`, `gpui-pre-reqwest` |
| `gpui-fast` / `-platform` / `-web` | 0.1.10 | 2026-10-19 | huacnlee | Optional faster render/layout fork, selected via gpui-kit feature `gpui-fast` |
| `gpui-kit` | 0.8.0 | 2026-10-19 | Longbridge | One-dep facade: re-exports GPUI + `gpui-base` + `gpui-component` + assets |
| `gpui-component` | 0.8.0 | 2026-10-19 | Longbridge | Styled components |
| `gpui-pre-mobile` | 0.1.0 | 2026-09-15 | Longbridge fork | Mobile platform packaged against gpui-pre |
| `gpui-mobile` | 0.1.0 | 2026-03-03 | itsbalamurali | Old publish; repo moved on via git |

gpui-kit main pins `gpui = { package = "gpui-pre", version = "=0.3.9" }` and
`gpui_platform = gpui-pre-platform =0.3.9 (font-kit, x11, wayland, runtime_shaders)`;
its comment warns any snapshot may break the API and must be bumped together with
`Cargo.lock` (`script/check-gpui-pin.ts`). **gpui-mobile main pins `gpui-pre =0.3.6`**
and `gpui-pre-wgpu =0.3.6` with wgpu 29; its example uses gpui-kit at git rev
`13c716b6`. gpui-kit's own mobile guide (`website/docs/mobile.md`) pins
`gpui-pre-mobile` rev `0b882efd` + `gpui-pre =0.3.4` and states explicitly that mixing
GPUI versions yields incompatible types. ⇒ Oino must choose **one** gpui-pre version for
desktop and mobile and keep a fork/patch of the mobile platform until upstreams align.

## 2. gpui-mobile

- What: implementation of GPUI's `Platform` trait for iOS and Android "following the
  same architecture as Zed's desktop platform crates" (README).
- Rendering: wgpu — **Metal on iOS**, **Vulkan (preferred) / GL ES 3.0 on Android**.
  Text: CoreText (iOS), cosmic-text + swash (Android). Touch input with momentum
  scrolling, keyboard, safe-area insets, dark mode, emoji.
- Platforms: iOS 13+ device+sim ✅; Android arm64 API 26+ ✅; armv7/x86_64 untested.
- Build requirements: Rust (README says 1.75+, realistically the gpui-pre MSRV);
  iOS: macOS + Xcode 15+ + XcodeGen; Android: SDK + NDK r25+ + `cargo-ndk`
  (`cargo ndk -t arm64-v8a -P 31 …` then Gradle `assembleDebug`). `example/build.sh ios
  --device|--simulator`, `android --device|--emulator`. Library crate-type
  `staticlib + cdylib + rlib`.
- Android IME: hosts must extend `dev.gpui.mobile.GpuiInputActivity` (not
  `NativeActivity`) for multistage IME composition; `GpuiPathPicker.java` for file
  prompts.
- Extras: ~30 Flutter-plugin-like "packages" behind default features (clipboard,
  notifications, file_selector, share, webview, local_auth, deeplink, …) and "glass"
  (Apple) + "material" component sets. Oino needs only a handful
  (`clipboard`, `notifications`, `file_selector`, `share`, `path_provider`,
  `shared_preferences`, `deeplink`) — build with `default-features = false`.
- Maturity: 438★, 18 open issues (Android GL/Vulkan backend selection #66/#67, NDK
  `xattr` breakage #64, font rendering #37, emulator run #34, "upstream to gpui-ce?"
  #38). TODO lists accessibility, routing, assets as missing. Treat as **experimental**.
- gpui-kit's position (mobile.md): mobile is *not* a primary target; validated in
  limited scope — "an AI chat area built from TextView, Button, Menu, Popover,
  Scrollbar, Input, Textarea and text selection passed functional and performance
  testing on both platforms". Recommended pattern on iOS: **embed one GPUI view inside a
  native UIKit host** (native nav bar + native bottom input, GPUI in between) via
  `gpui_ios_set_embedded()` and UIKit child-controller containment.

## 3. gpui-kit / gpui-component

- Layers: `gpui-base` (unstyled behaviour/state/a11y), `gpui-component` (styled
  system, 75+ components), `gpui-shell` (optional JS extension host — not needed by
  Oino), `gpui-kit` (single dependency, `gpui_kit::application()`, `open_window`,
  `init(cx)`). Platforms: macOS, Windows, Linux (CI matrix ubuntu/macos/windows;
  `x86_64-windows-msvc`), plus `wasm32-unknown-unknown`. Edition 2024.
- Components relevant to an agent client (`crates/component/src/`): `dock` (Dock,
  TabPanel, serializable layouts), `resizable`, `sidebar`, `tab`, `list` /
  `virtual_list` / `message_scroller`, `message` + `bubble` (chat messages),
  `text` (TextView / Markdown / HTML), `input` (Input/Textarea/code editor with
  tree-sitter or syntect highlighting, LSP hooks), `diff` ("readonly, virtualized patch
  display inspired by Diffs from Pierre", with conflict resolution), `table`, `tree`,
  `command` (command palette), `dialog`, `sheet`, `popover`, `notification`,
  `questionnaire` (structured question forms — fits ask-user / elicitation),
  `attachment`, `status_bar`, `title_bar`, `spinner`/`shimmer`/`skeleton`, `setting`,
  `theme`, `kbd`, `select`, `switch`, `menu`/`native_menu`, `speech`.
- Showcase: Longbridge Pro desktop app (production since day one).
- Linux deps (`script/install-linux.sh`, Ubuntu 24.04): `gcc g++ clang
  libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev
  libzstd-dev libasound2-dev vulkan-validationlayers libvulkan1` (+
  `libwebkit2gtk-4.1-dev` only for `gpui-webview`). macOS: Xcode (Metal). Windows:
  MSVC toolchain (DirectX renderer in gpui platform) — `script/install-window.ps1`.
- Release profile tip from its `Cargo.toml`: build `gpui-pre` at `opt-level = 3` even
  in dev for usable frame rates.

## 4. ZCode (UX reference only)

What it is: Z.ai's "AI programming workbench" — Electron desktop app + web UI + TUI CLI
agent, TypeScript monorepo (`apps/zcode-cli`, `packages/{ui,web,desktop,server,
rpc,protocol,android,…}`), v3.16.2. Not GPUI; we borrow UX, not code.

UX facts (from `DESIGN.md`, `docs/ui/*.md`, `docs/v4-refactor/06-ui.md`):

- **Shell**: three regions — left `WorkspaceSidebar` (default/min width 264 px,
  collapsible with a persistent re-open button, macOS traffic-light safe area); right
  top workspace `Header` (task title primary, project secondary; icon-only toggles for
  terminal and side pane; editor quick-open; more-menu with copy path/ids); right main
  area = conversation + right **side pane** (tabs: Git/diff review, Browser, Code
  Viewer, Terminal, Plan, subagent sessions, plugin panels) + bottom **Terminal** row
  with draggable split (`workspace-layout.md`, `workspace-header-structure.md`).
- **Sidebar**: fixed nav order New Task → Search/Command Center → Automations →
  Plugins → task-view switcher; workspaces as groups with tasks/sessions underneath,
  pinned section, unread/attention dots, timeline grouping, per-workspace file tree that
  slides in (`sidebar-primary-navigation-order.md`, `sidebar-task-*`).
- **Conversation**: virtualised timeline of server-computed rows (row-level updates,
  incremental markdown for streaming rows), left "query navigator" rail to jump between
  user prompts, find-in-conversation, sticky bottom dock with composer, queue panel,
  error/quota banners, permission + question dialogs (`06-ui.md`,
  `conversation-turn-navigator.md`, `chat-view-composition.md`).
- **Panes**: one data layer per session (ref-counted subscriptions, 30 s keep-warm),
  layout layer (split tree, ≤4 panes), focus layer (single focused pane; global
  shortcuts route there; "closing a pane ≠ stopping a session"). Performance bar:
  4 panes streaming simultaneously without dropped frames. (Split-pane *entry points*
  were removed 2026-10-16; the model stays.)
- **Tool calls**: compact one-line rows (status + title + summary), expand on demand;
  edit/patch tools render an inline lightweight unified diff with a "View diff" button
  to open the full diff viewer in the side pane; streaming tool input preview shows
  write/edit paths and line counts while the model is still emitting arguments
  (`tool-display-rendering.md`, `tool-inline-diff.md`, `streaming-tool-input-preview.md`).
- **Subagents**: single-line Agent/Task summary row in the parent; click opens a
  read-only child-session tab in the side pane (`agent-tool-call-renderer.md`,
  `subagent-session-side-tabs.md`).
- **Diff / review**: Git pane with per-file change cards and context menu (reveal in
  file manager, copy path, reveal in tree); per-turn file change summary with
  file-level undo/redo that never truncates chat history (`git-review-file-context-menu.md`,
  `task-file-change-summary.md`, `rewind-button-any-message.md`).
- **Approvals**: permission dialog options are `allow_once`, `allow_project`
  (persisted rule), `deny` (+ optional free-text feedback that goes into the denied
  tool result); "guarded" mode recognises dangerous commands (recursive rm, git clean
  -f, robocopy /MIR, …) and forces confirmation (`permission-project-approval.md`,
  `dangerous-command-approval-v1-plan.md`).
- **Composer**: single model+thinking control (model name + discrete thinking slider),
  prompt history, drafts per task, attachments/long-paste as attachments, `@` mentions,
  slash commands with skill suggestions; queue items editable/reorderable/"send now"
  (`composer-model-thinking.md`, `chat-message-queue.md`).
- **Command Center**: `Cmd/Ctrl+K` modal with `>` commands, `#` conversations, `@`
  files; history per workspace (`quickpick-command-palette.md`).
- **Status panel**: collapsible Git / Goal / Progress / Run sections with a mini capsule
  state (`status-panel-header-layout.md`).
- **Design character**: "calm, dense, operational"; strict UI type scale tokens; light/
  dark themes; keyboard-first; desktop/web parity; mobile `/remote` uses drawers instead
  of side tabs.

## 5. Local experiment `gpui-learn`

`Cargo.toml`: edition 2024, `gpui = { version = "0.2", features = ["font-kit",
"wayland", "x11"] }` → `Cargo.lock` resolves **gpui 0.2.2**. `src/main.rs` (101 lines)
is the GPUI text-alignment/decoration example (`div()` flex column, `StyledText` with
highlights, `Application::new().run`, `cx.open_window`). A built
`target/debug/gpui-learn` binary exists, so the Linux (Wayland/X11) toolchain is in
place on this machine. No README/notes, no gpui-kit or mobile. Takeaway: the machine
can build GPUI; the experiment must move from `gpui 0.2` to `gpui-pre 0.3.x` to use
gpui-kit and mobile.

## 6. Architecture recommendation

### 6.1 Process model

```text
apps/oino-desktop (GPUI main thread)              remote / headless
 ├─ oino-ui (views, ZCode-style shell)             oino --mode rpc  (JSONL, pi-compatible)
 ├─ oino-client  ◀── trait SessionBackend ──▶      oino --mode acp  (ACP v1)
 │    ├─ InProcessBackend ── oino-core (tokio rt, 2–4 workers, own threads)
 │    ├─ RpcBackend       ── stdio / TCP / SSH to `oino --mode rpc`
 │    └─ AcpBackend       ── any ACP agent (incl. oino --mode acp)  [later]
 └─ bundled: provider .wasm + unipi .wasm + resources
```

- `oino-client` defines one async **command/event API** — the same Rust enums `oino-rpc`
  serialises (pi RPC commands, `json.md` events, extension-UI requests). The in-process
  backend passes the enums through channels with zero serialization; RPC backend
  serialises them; so the UI cannot tell which is active and RPC parity is tested for
  free.
- **Tokio ↔ GPUI bridge**: oino-core is tokio-based (wasmtime async, reqwest, process).
  Run it on its own multi-thread runtime created at startup; GPUI owns the main thread
  and its executors. Use the `gpui_tokio` pattern (Zed: global runtime handle;
  `Tokio::spawn(cx, fut)` returns a GPUI `Task`, cancelled on drop). Event delivery:
  core pushes `AgentEvent`s into an `async_channel`/`futures::channel::mpsc` bounded
  channel per subscribed session; a GPUI `cx.spawn` loop drains it, **coalesces deltas
  per frame** (append text deltas, keep last state), and calls `cx.notify()` on the
  session entity once per batch. Commands go the other way via `Tokio::spawn`. Never
  block the GPUI thread on core locks.
- Extension UI: core emits `ExtensionUiRequest {id, method: select|confirm|input|
  editor|notify|setStatus|setWidget|setTitle|set_editor_text, …}` (pi
  `rpc-extension-ui.md`); the UI renders natively (gpui-kit `dialog`, `questionnaire`,
  `notification`, `status_bar`) and replies with `ExtensionUiResponse`. Same payload in
  TUI, GPUI, RPC, and `_oino/extension_ui` over ACP.

### 6.2 Crate / app layout

```text
crates/oino-client      # SessionBackend trait + InProcess/Rpc backends (no UI deps)
crates/oino-ui          # GPUI views shared by desktop and mobile (gpui-kit); feature `mobile`
apps/oino-desktop       # bin: window shell, menus, tray, updater, bundling of .wasm assets
apps/oino-mobile        # staticlib/cdylib: iOS (UIKit host, XcodeGen) + Android (GpuiInputActivity, Gradle)
apps/oino-mobile/ios    # Swift host project
apps/oino-mobile/android# Gradle project
```

- Desktop: `gpui-kit = { version = "=0.8.x", features = ["component", "assets",
  "syntect"] }` (tree-sitter later for diff highlighting), pinned `gpui-pre`.
- Mobile: start as a **remote client first** (RpcBackend over SSH/TCP to a desktop or
  server oino) — phones rarely want wasmtime + bash locally; in-process core on Android
  may come later (to verify: wasmtime Cranelift on `aarch64-linux-android`; iOS forbids
  JIT → would need wasmtime's Pulley interpreter, so *iOS = remote only* for v1).
  Native host owns navigation + composer per gpui-kit guidance; GPUI renders the
  conversation view.
- Pin strategy: one `gpui-pre` version workspace-wide; vendor/fork
  `longbridge/gpui-mobile` (already packaged against gpui-pre) at a rev matching the
  gpui-kit pin; CI checks `cargo tree -d -i gpui-pre` is a single version. Use
  `[profile.dev.package.gpui-pre] opt-level = 3`.
- Licensing: choose gpui-mobile's Apache-2.0 option (tri-licensed) to stay compatible
  with Oino's MIT/Apache; gpui-kit Apache-2.0; ZCode Apache-2.0 (UX reference only). Do
  **not** copy code from Zed's GPL app crates (`agent_ui`, `acp_thread`) — study only.

### 6.3 What to borrow from ZCode (v1 scope)

1. Shell: left session sidebar (workspaces → sessions, unread/running dots, pinned),
   header (session title, cwd, model), conversation centre, right tabbed side pane
   (Diff review, File viewer, Terminal output, Subagent sessions, Extension panels),
   optional bottom terminal. Map to gpui-kit `Sidebar`, `Dock`/`TabPanel`, `Resizable`.
2. Conversation: virtualised row list with stable row ids (message id / tool-call id)
   and per-row entities so streaming patches one row; incremental markdown for the tail
   row; query-navigator rail later.
3. Tool rows: one-line collapsed summary; inline unified diff for edit/write with
   "open in side pane" → gpui-kit `diff`; bash output collapsed with tail preview;
   streaming tool-argument preview from `toolcall_delta` events.
4. Approvals: dialog with `allow once / allow always (project) / deny + feedback`,
   reused by the ACP `request_permission` option kinds.
5. Composer: combined model + thinking control, slash-command and `@file` completion,
   queue panel with steer vs follow-up, drafts per session.
6. Command palette (`Cmd/Ctrl+K`), keyboard-first; dense, calm theme with light/dark.
7. Multi-pane: data layer per session with ref-counted subscriptions and "close pane ≠
   stop session" — design the client API for it even if v1 ships one pane.
8. Performance acceptance: N sessions streaming concurrently at 120 fps target; measure
   with `gpui-fps`.

### 6.4 Build hosts

- Linux (this machine): Wayland + X11, Vulkan; packages above.
- macOS (`desmondgiam`): Xcode + Metal; also the only host for iOS builds (XcodeGen,
  `aarch64-apple-ios`, `-sim`).
- Windows (`coffee`): MSVC + DirectX; check gpui-kit `install-window.ps1`.
- Android: NDK r25+, `cargo-ndk`, arm64 device over USB; avoid `api-level-31` ndk
  feature (README warns it breaks older devices).

## 7. Risks

- GPUI API churn (weekly `gpui-pre` snapshots; official `gpui` crate stale) — isolate
  GPUI in `oino-ui`/apps; core and client crates must not depend on GPUI.
- Mobile platform version skew (0.3.4 / 0.3.6 vs kit 0.3.9/0.3.10) — expect to maintain
  a fork.
- iOS cannot JIT wasmtime → remote-only or Pulley; Android local core is heavy.
- gpui-kit is 0.x with frequent breaking releases (0.7 → 0.8 recently) — pin exact.
