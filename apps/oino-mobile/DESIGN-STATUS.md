# Oino Mobile — design status (work package `design-mobile`, design.md §6 / §10.2)

Screens are in `apps/oino-ui/src/mobile/` (owned by design-mobile); the platform hooks are
in `apps/oino-mobile/src/platform.rs`. Screenshots: `docs/dev/v1/design/shots/mobile/`
(Android 15, 720×1640 @ 2.0, `scripts/screenshots.sh`, demo backend) and
`ios-sim-*.png` from the iOS simulator (`scripts/ios-remote.sh --simulator --screenshot`).

## Gaps (design.md §9.2)

| # | Status | Where |
|---|---|---|
| MG1 Sessions | Closed: crab glyph (aggregate state) + ⌕ + ⚙ app bar, "Sessions" Display title + Display sheet, Needs-you card (Approve / Answer / Review), uppercase sections with ⊕, 56 rows (dot, title, workspace · count, capsule or age), swipe left → Pin / Delete (confirm), long press → Open / Rename / Fork / Copy id / Pin / Delete, pull to refresh, New session pill | `sessions.rs`, `gesture.rs` |
| MG2 Conversation | Closed: app bar (back, title + "workspace · model", status dot, stop, drawer, ⋯), status capsule row → Status sheet, right drawer 88 % | `conversation.rs`, `drawer.rs` |
| MG3 Interactions | Closed: approvals / questions / plan reviews dock with stacked 44 pt buttons and no number keys; small dialogs, plan reviews and docks taller than half the screen move into a bottom sheet (pending ones not dismissible; Back cancels only small dialogs); local notification when backgrounded | `components/interaction` (touch branches), `sheets.rs` |
| MG4 Sheets | Closed: handle 36×4, 24 top corners, title Lg/500, rows 56, Model (providers as sections, check, Manage models, six-segment thinking control), Actions (chat, panels, extension commands, shortcuts, status chips), Status (goal, progress, run → Agent drawer, git) | `sheets.rs` |
| MG5 Search / board / pages | Closed: M7 search (auto-focus, All / Actions / Sessions / Files, palette sections + fuzzy groups), M9 lane pager, M5 pages for diff / output / overlay panels with a 52 detail header | `search.rs`, `board.rs`, `pages.rs` |
| MG6 Settings | Closed: stacked pages (Appearance, Models & Providers with sign-in / sign-out, Sessions, Permissions, Extensions with enable switches, Extension settings hub with Global/Project scope and field editor sheet, Remote, About) | `settings.rs` |
| MG7 Touch sizes | Closed: `Density::Touch` → tool rows 32, option rows 56, gutters 12, 85 % bubbles, editable text 16, UI text 15, `BtnSize::Touch` 44 | `design/{tokens,theme}.rs`, components |
| MG8 Remote attach | Adapt: hosts list ("This phone" first, paired desktops), "Pair a desktop" page (camera instructions + paste field), `oino://` links open it; pairings persist in `mobile.json`. Connecting needs `oino serve` + the RPC backend (not in the tree) — paired hosts show "not connected" | `hosts.rs`, `prefs.rs` |
| MG9 Notifications | Closed: `osNotify` → toast (foreground) or local notification (background); pending interaction → local notification; iOS badge = sessions that need you; Android launcher dot from notifications | `shell.rs`, `platform.rs` |

## §7 unipi surfaces on mobile

All rows render through the shared components at touch density: hints (dock strip, tap
cycles), header / splash (welcome hero), work tray / btw (composer-area panel), footer
(top segments in the context header, MetaRow top-2 + overflow), process line (dock strip),
info-screen dashboard / hints browser / graph / MCP / skills / updater (M5 pages), approvals /
forms / plan review / dock dialogs (M3), goal / run / fusion status (capsule row + Status
sheet), memory / dream / kanboard cards (timeline cards), subagents / sidekick (AgentRow →
drawer Agent tab), board (M9), app-bridge control (drawer tab), settings hub (M8), toasts at
the top under the app bar (swipe up / tap dismisses), Unicrab aggregate (M1 glyph + badge).

## Shared component changes (touch branches only; desktop unchanged)

`OinoTheme::{touch, tool_row, option_row, gutter}`, `tokens::touch`, `BtnSize::Touch` and
"no kbd hints on touch" in `Btn`; conversation (no header / query rail / floating status on
touch, dock measurement → sheet promotion, `touch_sheet` cards skipped), approval (stacked
44 pt buttons), dock dialog / question (56 rows, no number keys), meta row (top-2), composer
(16 px text), user bubble (85 %), tool / explore / thinking rows (`t.tool_row()`),
`InteractionView::{touch_sheet, touch_dismissible, dismiss, promote}`. Guard test now scans
`src/mobile/` (exception removed).

## Deviations / adaptations

| Item | Change | Reason |
|---|---|---|
| §6 tablets / landscape ≥ 720 dp | Phone layout everywhere | Desktop `Workspace` needs window chrome + split frames; follow-up |
| §6.3 "full-height" sheet | Interaction sheets fit their content (max 90 %) | A full-height sheet left half the screen empty under short cards |
| §6.9 project picker | Board title shows the workspace; no picker | The board follows the session's workspace; switching = open another session |
| §6.10 | Pairing stored, not connected (token not kept) | No RPC backend / keystore yet |
| A1.4 mobile browser | Custom Tabs via the raw intent protocol (no `androidx.browser`) | Offline Gradle; same behaviour in Chrome |
| §10.2 acceptance perf | 120 Hz scroll / input-to-paint not measured | Attached phone is 60/90 Hz Redmi A5; no GPU profiler run (follow-up) |
| Settings hub, providers | Need an open session (frontend commands are per backend) | Same as desktop's backend binding |
| Android activity restart | `android_main` exits the process when the activity ends | gpui-pre 0.3.10 asserts executors on the first native thread; a second activity in the same process panicked |

## Fixes outside the screens

- Vendor `gpui-mobile` patches 6–8 (`apps/vendor/gpui-mobile/VENDOR.md`): `platform_info`
  for gpui-pre 0.3.10, iOS build without `camera` / `video_player`, keyboard only on a tap.
- `components/keys.rs`: `escape` / `i` in the focused timeline double-leased the pane
  (panic on Android back / typing) — now focuses the composer directly.
- Android: a panic hook logs a backtrace to logcat.

## Screenshots (`docs/dev/v1/design/shots/mobile/`)

`m01-*` sessions (dark, light, swipe, row menu), `m02-*` conversation / welcome /
composer with keyboard, `m03-*` approval / question / plan sheet, `m04` drawer, `m05` info
page, `m06-*` model / actions / status sheets, `m07` search, `m08-*` settings, `m09-*` board,
`m10-*` hosts / pairing; `ios-sim-*` iPhone 17 Pro simulator (sessions, conversation,
approval sheet, model sheet light, board).

## Platform status

- **Android**: debug APK builds (`scripts/build.sh android --no-wasm --demo`), installs and
  runs on `863d005830483036345110df72ad6c` (Android 15). `daff66e` (Redmi Note 7, MIUI)
  rejects USB installs until "Install via USB" is confirmed on the phone.
- **iOS**: simulator build on `mac` (Intel, Xcode 26.3, `x86_64-apple-ios`, unsigned) via
  `scripts/ios-remote.sh --simulator`. Device builds need `OINO_IOS_TEAM`; components cannot
  JIT on devices (Pulley request stands).
