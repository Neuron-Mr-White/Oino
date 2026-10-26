# Oino Desktop and Oino Mobile (GPUI)

Status: binding. Owners: `gpui-desktop` (`apps/Cargo.toml`, `apps/oino-ui`,
`apps/oino-desktop`), `gpui-mobile` (`apps/oino-mobile`, `apps/vendor/gpui-mobile`,
`apps/oino-ui/src/mobile/`). Reference: [research-gpui.md](research-gpui.md); ZCode for UX
only (no code copied); Zed GPL crates are study-only.

## 1. Goals

1. **Speed first**: first frame ≤ 150 ms and interactive ≤ 400 ms after launch on the Linux dev
   machine with a warm wasm compile cache; 120 fps target while streaming; 4 sessions streaming
   at once without dropped frames (measured with a frame-time overlay in debug builds); idle CPU
   ≈ 0 (no polling; event driven).
2. Installing Oino Desktop is enough: it bundles `oino-core`, all provider components, all
   unipi components, docs and the `oino-kanboard` helper (D14). The `oino` CLI binary is
   installed alongside (shell integration optional) so terminal users get the same version.
3. Every capability of the TUI exists in the desktop app (session management, tree, fork,
   settings hub, extension panels, dialogs, shortcuts), plus desktop-native affordances.

## 2. Workspace and pins (D3, D17)

```text
apps/Cargo.toml          # workspace: members = ["oino-ui", "oino-desktop", "oino-mobile"], exclude vendor build dirs
apps/oino-ui/            # lib: shared GPUI views + app state (feature "mobile" for touch layouts)
apps/oino-desktop/       # bin: window shell, menus, tray, packaging, updater, embedded serve
apps/oino-mobile/        # lib (staticlib + cdylib + rlib): mobile entry points
  ios/                   # XcodeGen project (UIKit host), Swift glue
  android/               # Gradle project (GpuiInputActivity subclass)
apps/vendor/gpui-mobile/ # vendored longbridge/gpui-mobile platform (Apache-2.0 option), patched to gpui-pre =0.3.9
```

Pins (exact, in `apps/Cargo.toml` `[workspace.dependencies]`): `gpui-kit = "=0.8.0"` (features
`component`, `assets`, `syntect`), `gpui = { package = "gpui-pre", version = "=0.3.9" }`,
`gpui_platform = { package = "gpui-pre-platform", version = "=0.3.9" }`. CI asserts a single
`gpui-pre` version (`cargo tree -d -i gpui-pre` empty). `[profile.dev.package.gpui-pre]
opt-level = 3`. Path deps into the root workspace: `oino-client` (features `in-process`,
`rpc`), `oino-view`, `oino-protocol`, `oino-core`, `oino-wasm-host`, `oino-bundle` (desktop
only). Mobile depends only on `oino-client` (feature `rpc`, no `in-process`), `oino-view`,
`oino-protocol` — no wasmtime, no core.

## 3. Desktop architecture

### 3.1 Threads and runtime bridge

- GPUI owns the main thread. At startup the app creates a tokio multi-thread runtime (4
  workers) held in a GPUI global (`oino_ui::rt::Tokio`, the Zed `gpui_tokio` pattern
  re-implemented: `Tokio::spawn(cx, fut) -> gpui::Task<T>` cancelled on drop).
- `SessionHub` (oino-client) runs on that runtime with `InProcessBackend`s. Optional embedded
  `oino serve` listener (Settings → Remote access) shares the same hub so phones attach to
  live desktop sessions.
- Event delivery: per open session, a GPUI task drains the backend `EventStream`
  (`futures::channel::mpsc`), feeds `oino_view` reducers, and calls `cx.notify()` **once per
  frame** (coalescing text deltas and tool updates; latest state wins). The GPUI thread never
  blocks on core; commands go through `Tokio::spawn`.

### 3.2 State model

| Entity | Content | Source |
|---|---|---|
| `AppState` | workspaces (cwd groups), session list, settings, themes, keybindings, shortcut registry | `_oino/list_sessions`, settings commands |
| `SessionModel` (one per open session; ref-counted; kept warm 30 s after its last view closes) | `oino_view::Transcript` (rows keyed by entry id / tool call id), `UiState` (status, frame, widgets, panels, dialog queue), run state, queue, model/thinking, context usage | backend events |
| `PaneLayout` | split tree (≤ 4 panes), focus | user |

"Closing a pane ≠ stopping a session" (ZCode): sessions keep running in the hub.

### 3.3 Bundling and first run

`oino-desktop` links `oino-bundle` (all components + docs). First run: extract resources, start
background pre-compilation of components into `~/.oino/cache/wasmtime/`, show onboarding
(provider login via `/login` flow: provider list → OAuth/API key dialog). Uses the same
`~/.oino` agent dir as the CLI.

## 4. Mobile architecture (D16)

- **Remote-only**: `RpcBackend` over TCP+TLS to `oino serve` (desktop or server). Pairing by
  scanning the `oino://` QR (modes §5.1) with the camera (gpui-mobile `deeplink` + platform
  camera intent / `AVCaptureSession` in Swift glue) or pasting the URL; tokens stored in the
  platform keystore (Keychain / Android Keystore via vendored packages).
- Screens: session list (hub sessions + recent on host), conversation (GPUI), dialogs and
  panels (bottom sheets), settings (subset: model/thinking, extension settings hub read/write),
  notifications (push of `osNotify` while app foregrounded; background push is out of scope).
- iOS: UIKit host with native navigation bar and native bottom input bar; one embedded GPUI
  view renders the conversation and sheets (`gpui_ios_set_embedded`); composer text is sent to
  Rust through the Swift glue. Android: `GpuiInputActivity` subclass; GPUI renders everything
  including the composer (IME supported by the activity).
- Default features off for gpui-mobile packages; enable only `clipboard`, `notifications`,
  `file_selector`, `share`, `path_provider`, `shared_preferences`, `deeplink`.

## 5. UX specification (ZCode-inspired, Oino-specific)

### 5.1 Shell (desktop)

```text
┌──────────── Sidebar (264px, collapsible) ────────────┬──────────────────── Main ───────────────────────────────┐
│ [+ New session]  [⌘K Search/Command]                │ Header: session name · cwd · model+thinking · run state │
│ ▸ Workspace (cwd)                                    ├──────────────────────────────┬──────────────────────────┤
│    • session (running ● / attention ◆ / idle)        │ Conversation (virtualised)   │ Side pane (tabs):        │
│    • session                                         │  query rail (jump to prompts)│  Diff · Files · Terminal │
│ ▸ Workspace                                          │  rows: user / assistant /    │  · Tree · Subagents ·    │
│ Pinned · Recent                                      │  tool (1-line, expandable) / │  Extension panels        │
│                                                      │  custom (OUS) / summaries    │                          │
│ [Extensions] [Settings]                              ├──────────────────────────────┴──────────────────────────┤
│                                                      │ Widgets (aboveEditor) · Composer · frame segments       │
│                                                      │ Queue panel (steer / follow-up, editable)               │
└──────────────────────────────────────────────────────┴─────────────────────────────────────────────────────────┘
 Status bar: extension statuses (setStatus) · context % · cost · tps
```

gpui-kit mapping: `Sidebar`, `TitleBar`, `Dock`/`TabPanel`, `Resizable`, `virtual_list`/
`message_scroller`, `TextView` (markdown), `diff`, `Input`/`Textarea`, `command`, `dialog`,
`sheet`, `notification`, `questionnaire` (form dialogs), `status_bar`, `setting`, `kbd`.

### 5.2 Conversation rows

- Stable row ids (entry id; tool call id for tool rows); streaming patches only the tail row;
  incremental markdown for the streaming row.
- Tool rows: collapsed one line (status icon, title from render hints, summary); expand on
  click; edit/write show an inline unified diff (first 40 lines) with "Open diff" → side pane;
  bash shows a tail preview (last 5 lines) with "Open output"; streaming tool-argument preview
  from `toolcall_delta` (path and line counts for write/edit).
- Thinking blocks collapsed by default (setting `hideThinkingBlock`).
- Custom messages/entries: OUS via `_oino/render`-equivalent call (cached); provenance cards
  with violet `harness` rail.
- Subagent/sidekick runs: one-line summary row; click opens a read-only child session tab in
  the side pane (events from `_oino/agents` + child session file).
- Compaction and branch summaries: collapsible summary cards.

### 5.3 Composer

Model + thinking combined control (model name + discrete thinking slider); slash commands,
skills, templates and `@file` completion via `get_completions`; attachments (drag-drop,
paste images, `insertEditorToken` tokens); prompt history per session; drafts per session;
`Enter` send / `Shift+Enter` newline; while streaming, `Enter` = steer, `Alt+Enter` =
follow-up (pi TUI semantics); queue panel items editable/removable ("send now" = steer).

### 5.4 Dialogs, panels, approvals

- Dialogs render as modal `dialog`s (select/confirm/input/editor), `questionnaire` (form),
  `approval` dialog with the four ACP option kinds and optional feedback (ZCode approvals),
  `custom` as modal OUS.
- Panels: `overlay` → modal sheet; `sidePane` → side pane tab; `editorReplace` / `bottomPane`
  → bottom dock replacing the composer; `fullscreen` → window overlay.
- Notifications: toast (`notify`), OS notification (`osNotify`) when the window is unfocused.

### 5.5 Navigation and keyboard

`Cmd/Ctrl+K` command palette (`>` commands incl. extension shortcuts and slash commands,
`#` sessions, `@` files); `Cmd/Ctrl+N` new session; `Cmd/Ctrl+B` toggle sidebar;
`Cmd/Ctrl+J` toggle side pane; `Cmd/Ctrl+\` split; `Cmd/Ctrl+.` abort; `Cmd/Ctrl+Shift+T`
tree view; `Cmd/Ctrl+,` settings. Extension shortcuts per ui-protocol §8; user overrides
from `keybindings.json`.

### 5.6 Settings and extensions

Settings window: General (theme, density, fonts), Models & Providers (login/logout, API keys,
default model, scoped models), Sessions, Extensions (list from `_oino/extensions`, enable/
disable, permissions, install from file/URL), Extension settings hub (ui-protocol §6.6,
search + scope toggle), Keybindings, Remote access (serve on/off, pairing QR, tokens).

### 5.7 Design character

Calm, dense, operational; light/dark themes from gpui-kit theme tokens mapped to OUS tones
(ui-protocol §6.5); strict type scale; keyboard-first; no animations beyond spinners and
streaming caret.

## 6. Build matrix

| Target | Host | Toolchain / deps | Command | Artifact |
|---|---|---|---|---|
| Linux x86_64 | local | Rust 1.99, `clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libasound2-dev libvulkan1` | `cargo xtask desktop --release` | AppImage + `.tar.gz` (cargo-packager) |
| Windows x86_64 (MSVC) | `ssh coffee` | VS Build Tools 2022, Rust 1.99 MSVC | `cargo xtask desktop --release` on coffee (repo synced via `git push` to a branch + `git pull` on coffee, or `rsync`) | NSIS installer (cargo-packager) |
| macOS universal (aarch64 + x86_64) | `ssh desmondgiam` | Xcode 16+, Rust 1.99 both targets | `cargo xtask desktop --release --universal` | `.dmg` (ad-hoc signed; notarization when a Developer ID is configured) |
| Android arm64 | local + USB device | Android SDK, NDK r27, `cargo-ndk`, JDK 17 | `cargo xtask mobile android --device` (`cargo ndk -t arm64-v8a -P 26` + Gradle `assembleDebug` + `adb install`) | APK |
| iOS arm64 device | `ssh desmondgiam` + USB device | Xcode, XcodeGen, Apple dev team id (env `OINO_IOS_TEAM`) | `cargo xtask mobile ios --device` | `.ipa`/run on device |

`cargo xtask desktop|mobile …` delegates to `apps/oino-desktop/scripts/build.sh` /
`apps/oino-mobile/scripts/build.sh` (owned here; README §4.2); the desktop script runs `cargo
xtask wasm` first (bundle input), then builds and packages the app.
Release workflow (integration, P4): build all artifacts on the hosts above, upload to a GitHub
release with `SHA256SUMS`; the desktop updater (cargo-packager updater with signature
verification, key in CI secrets) checks that release feed.

## 7. Phasing inside gpui items

gpui-desktop: (1) workspace + pins + shell with fake backend (`oino-client` fake), (2)
conversation + composer on `InProcessBackend`, (3) dialogs/panels/OUS renderer, (4) settings,
extensions, tree/fork/sessions, (5) packaging on all three desktop hosts. gpui-mobile:
(1) vendor + build hello on Android and iOS devices with gpui-pre 0.3.9, (2) RpcBackend +
pairing, (3) conversation + dialogs, (4) packaging. gpui-mobile places mobile-only views under
`apps/oino-ui/src/mobile/` (owned by gpui-mobile) and requests shared-view changes from
gpui-desktop.

## 8. Acceptance

- Desktop on all three OSes: create session, stream a reply with tool calls, approve a
  workflow dialog, open an extension panel (info-screen), fork from tree, settings hub edit,
  4 concurrent streaming sessions at ≥ 100 fps on the Linux dev machine.
- Mobile on a physical Android and iOS device: pair with desktop serve, attach to a live
  session, answer a dialog first (desktop dialog dismisses), send a prompt, see streaming.
