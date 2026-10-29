# Browser (v2): anti-bot browser service + ZCode browser UX

Status: **binding** (work id `spec-browser`, 2026-11-02, branch `v2`). Hierarchy:
[DIRECTION.md](../DIRECTION.md) §6 > this spec. It **amends** [browser-control.md](../../v1/browser-control.md)
(the v1 binding spec, implemented in `crates/oino-browser` + `extensions/oino/browser`):
everything in v1 §1–§14 still holds **except** where this spec contradicts it — engine
policy (§4.1), display default (§4.4), CDP hygiene (new hard rules), the WIT `browser`
interface (replaced by the generic host-service call, DIRECTION §3.4), and the app
surface (GPUI panes are gone; the app is the imported unipi Tauri/SolidJS app).
Research basis: memory `oino_browser_stealth_research_2026`; unipi-app 1.0.0 browser
implementation (imported verbatim under `apps/unipi/`).

Inputs read for this spec:

- oino (branch `v2`): `crates/oino-browser/src/{engine,launch,config,service,profile,page,
  snapshot,frontend,commands,display,profiles,policy,hub,tab}.rs`, `crates/oino-browser-protocol/src/`,
  `wit/oino-extension/browser.wit`, `crates/oino-extension-api/src/browser.rs`,
  `crates/oino/src/browser.rs`, `extensions/oino/browser/`, `docs/dev/v1/browser-control.md`.
- unipi-app 1.0.0 (`apps/unipi/`): `crates/host/src/browser/{mod,cloak,cookies,gate,agent,
  humanize,display,session,profiles}.rs`, `crates/proto/src/browser.rs`,
  `apps/mobile/src/screens/BrowserView.tsx`, `apps/mobile/src/lib/{browser,browserPane}.ts`,
  `apps/mobile/src-tauri/src/browser.rs`.
- ZCode UX reference: `packages/ui/src/browser-use/*` (`BrowserViewportToolbar`,
  `UnifiedBrowserView`, `BrowserUseSidePaneContent`), `packages/ui/src/lib/workspaceSidePane.ts`,
  `docs/ui/web-element-context.md`.

Sibling specs this coordinates with (do not redefine their content):

- **`specs/ext-abi.md`** (`spec-ext-abi`) — the generic `host-service.call(name, op, json)`
  WIT interface and manifest permission model the browser service is exposed through.
- **`specs/host.md`** (`spec-host`) — unipi-host host services registry (how a service
  name resolves to an implementation inside the host process).

---

## 1. Summary of decisions

| # | Decision | One-line justification |
|---|---|---|
| D1 | Engine default is **CloakBrowser, headful** (hidden display or hidden real window). First browser use triggers a **signed first-use download** (Ed25519-pinned port of unipi `cloak.rs`) after a one-time confirm; the binary is never bundled or redistributed. | Owner ruling (DIRECTION §6); stealth is the default posture, not an opt-in. |
| D2 | The user's **real installed Chrome + a dedicated profile** (own `--user-data-dir`, never attach to the user's running Chrome) is the engine for Google sign-in and other sensitive logins. Engine is **pinned per profile**; user picks in the profile picker / Settings, agent picks with `browser_profile` and gets a hint on sensitive origins. | Google blocks sign-in from non-Google-signed builds; a dedicated profile keeps the user's cookies out of reach and keeps one stealth posture per profile. |
| D3 | Camoufox and Fingerprint-Chromium are **watchlist only** (names reserved in settings, no implementation). | Neither has a maintained Windows build / pipe-CDP story today; CloakBrowser covers the need. |
| D4 | Display per OS: **Linux** hidden = private Xvfb display, or Xwayland when only that exists (never the user's compositor); **Windows/macOS** hidden = a **real window kept off-screen** (`--window-position=-32000,-32000`); **headless only as last resort**, always with a visible "less stealthy" warning (badge + status note). | A real composited window is the strongest anti-bot placement available per OS; the fallback ladder is explicit and surfaced. |
| D5 | The CDP hygiene list (§5) is a set of **testable requirements**; each rule has a compliance verdict for the current code with `file:line` and the change. Two violations exist today (`Runtime.enable`/`DOM.enable` in `profile.rs`, main-world helper JS in `page.rs`) and one omission (`--disable-blink-features=AutomationControlled`). | "Anti-bot" must be enforced by tests, not folklore. |
| D6 | Ports from unipi: `cloak.rs` (signed downloader), `cookies.rs` (real-Chrome cookie import), `gate.rs` arbitration semantics (already pure in v1; take-over rules extended), `display.rs` (Xvfb + Xwayland backends), and the **full** `humanize.rs` (MIT) replacing v1's simplified humanizer, plus `session.rs`'s adaptive screencast quality. | Proven code, verified by the old app in production. |
| D7 | The browser is a **unipi-host service** exposed through the generic host-service interface (DIRECTION §3.4): `host-service.call("browser", op, json)`. `wit/oino-extension/browser.wit` is deleted. Under a plain `oino` TUI/CLI (no unipi-host), the browser tools are **not registered** and `/browser` reports "unavailable". | Oino core carries no unipi-specific ABI; the extension surface survives unchanged through one generic call. |
| D8 | The app pane (desktop side pane) and the phone sheet view the browser through **`Page.startScreencast`** with adaptive quality (port of unipi `Adapt`), input relay, and the take-over gate. Never the Tauri webview, never window reparenting. | One engine for agent and user; screencast is not page-detectable; the phone gets the same frames. |
| D9 | ZCode browser UX on the unipi app: side-pane browser tabs, tab strip + URL bar + back/forward/reload, **element picker** (hover highlight → click → composer chip with selector/text/screenshot), **user screenshot → composer attachment**, profile picker, engine/stealth badge. | Owner ruling (DIRECTION §4.3/§6); the imported app already has the chrome, picker and screenshot-to-composer are new. |
| D10 | CI canaries: `cargo xtask browser-canary` + a nightly/dispatch CI job run **headful under Xvfb**, comparing CloakBrowser against a real-Chrome baseline on leaky.page, bot.sannysoft.com, CreepJS, browserscan.net, deviceandbrowserinfo.com/are_you_a_bot. Binary detections fail; numeric scores have per-site budgets; screenshots are archived artifacts. | Continuous verification of D1–D5 against live detectors, without PR-gating flakiness. |

## 2. Architecture (v2)

```text
 phone app ──iroh──┐                         other PC ──iroh──┐
                   ▼                                            ▼
  UniPi app (Tauri/SolidJS, apps/unipi/apps/mobile) ── app proto (crates/proto) ──► unipi-host
    Browser pane / sheet   screencast frames + BrowserCmd + takeover/profiles        │
                                                                     ┌────────────────┤
 oino TUI / CLI ──► oino-core (no browser)                           │  oino-core sessions
                                                                    │  host-service registry
 extension: extensions/unipi/browser (WASM)                          │   ┌──────────────────┐
   19 browser_* tools, skill, prompt, turn-tail shot, /browser ──────┼──►│ service "browser"│
   via WIT host-service.call("browser", op, json)                    │   │ backed by       │
                                                                     │   │ oino-browser    │──► CloakBrowser / Chrome
                                                                     │   └──────────────────┘    (CDP pipe, hidden display)
```

- `crates/oino-browser` + `crates/oino-browser-protocol` stay root-workspace crates (exact
  location may move in the repo/package split; acceptance is "no `oino-*` core crate
  depends on `oino-browser`"). The service is **linked by unipi-host only**
  (`crates/unipi-host` per `specs/host.md`). `crates/oino/src/browser.rs` (in-process
  `BrowserService` for the `oino` binary) is deleted in WP-B7; the plain TUI gets
  "unavailable" (D7). The extension id renames `oino.browser` → **`unipi.browser`**
  (per `specs/ext-abi.md`); app-proto paths in this spec are the imported-source paths
  (`apps/unipi/…`), adjusted by `specs/host.md`'s crate moves.
- Agent plane: the extension calls `host-service.call("browser", <op>, <BrowserRequest
  JSON>)`; ops are exactly the op-tagged `BrowserCommand`/`BrowserRequest` shapes of
  `oino-browser-protocol` (unchanged wire format — the v1 JSON is the v2 op table). The
  `approval` field rides along as today. Status: op `"status"`. The permission is
  `host-service` scoped to service name `"browser"` (see `specs/ext-abi.md`).
- Viewer plane (pane/phone): the app talks **app proto** (`apps/unipi/crates/proto/src/browser.rs`:
  `browser_open` stream, `BrowserCmd`, `BrowserEvent`, `browser_takeover`, `browser_profiles`,
  `browser_status`, `browser_window`), and unipi-host backs those ops with `oino-browser`'s
  `BrowserHost` (execute/status/subscribe/attach_view/frontend). The v1 `_oino/browser_*` RPC
  set is dropped with the GPUI apps; the unipi proto is the only frontend protocol.
- Take-over, handoff, dialogs, downloads, site permissions: unchanged semantics from v1 §7/§9/§10.

## 3. Engine policy

### 3.1 Resolution order

| Priority | Source |
|---|---|
| 1 | Profile pin: `~/.oino/browser/profiles/<name>/oino-profile.json` → `"engine": "cloak" \| "chrome"` |
| 2 | Setting `browser.engine`: `auto` (default) \| `cloak` \| `chrome` \| `path` (+ `browser.executablePath`) |
| 3 | `auto` = CloakBrowser if a verified build is installed → else **first-use install flow** (§3.2) → else system Chrome-family (`engine.rs` candidate order, unchanged) with the "Not stealthy" badge |

`EngineChoice::Auto` no longer silently falls back to Chrome (v1 behaviour,
`engine.rs:86-88`): the first `ensure()` without an installed CloakBrowser returns
`backend_unavailable { reason: "cloak_not_installed" }` carrying an **install action**
(host task id once confirmed). The app shows a confirm dialog; the TUI shows a `/browser
install-cloak` hint. Declining falls back to system Chrome for that session with
`stealth_note "Not stealthy: a regular Chrome-family browser"` (existing wording) and the
install offer stays available in Settings → Browser.

### 3.2 CloakBrowser first-use signed download (port of unipi `cloak.rs`)

Port `apps/unipi/crates/host/src/browser/cloak.rs` verbatim into
`crates/oino-browser/src/cloak.rs` (public surface kept: `SIGNING_PUBKEYS`,
`PLATFORM_VERSIONS`, `ensure()`, `locate()`, `wanted_version()`), with these deltas:

- Cache root `~/.cache/oino/cloakbrowser/<version>/` (today) + wrapper reuse
  (`~/.cloakbrowser/`, `CLOAKBROWSER_CACHE_DIR`) — as v1 §4.2, unchanged.
- `ensure()` becomes a **host task** (progress percent in the app status area; resumable
  is not required, restart re-verifies). Triggered only by: the first-use confirm above,
  Settings → Browser → "Install CloakBrowser (stealth, ~210 MB)", or
  `unipi-host browser install-cloak` / `/browser install-cloak`.
- Signature rules are non-negotiable and unchanged: SHA256SUMS + `.sig` from
  `cloakbrowser.dev` (GitHub Releases mirror fallback, same origin for both files),
  Ed25519 against pinned keys, manifest version must equal the requested pin,
  archive hash must match; extract to staging dir + rename. Licence: the binary is
  **never bundled, never redistributed** (installers download on first use; the
  installer spec must note this).
- `browser.cloak.version` override stays; pins move with upstream wrapper releases
  (risk R2).

### 3.3 Real Chrome for Google sign-in and sensitive sites

- **Never attach to the user's running browser.** A dedicated `--user-data-dir` per
  profile always, even for the user's real Chrome executable (a separate user-data-dir is
  a separate process — unipi `mod.rs` doc, verified).
- The **engine is a property of the profile**: `oino-profile.json { "engine": "chrome" }`.
  The default `default` profile follows the global setting. The first time a user asks
  for Google sign-in (Settings row "Use my Chrome for Google sign-in…" or the profile
  picker's "New Chrome profile"), a profile is created pinned to `chrome`; its name shows
  the engine badge ("Chrome") in the picker, and `ProfileInfo.engine` (new field,
  `oino-browser-protocol/src/status.rs`) surfaces it everywhere.
- **Who picks**:
  - User: profile picker in the pane header (per-profile engine badge + engine on
    create), Settings → Browser → Profiles, `/browser profile <name>`.
  - Agent: `browser_profile {action:"use"}` / `status` (existing tool) — engine shows per
    profile; the agent never installs CloakBrowser or imports cookies.
- **Sensitive-origin hint** (host-side heuristic, `policy.rs`): new setting
  `browser.sensitiveOrigins` (globs, default `["accounts.google.com"]`). When the agent
  navigates a `cloak` profile tab to a matching origin, the `browser_navigate` result text
  appends `Google sign-in may fail on CloakBrowser; a Chrome profile works better
  (browser_profile)` and `details.browser.hint = "real_chrome_profile"`. No forced switch —
  the agent and user decide. Cookie import (§6.3) is the other path for logins.
- Launch flags for the real-Chrome profile: the common stealth set (§5 rule H5) applies to
  **both** engines, including `--disable-blink-features=AutomationControlled`; never
  `--enable-automation`.

### 3.4 Watchlist (no implementation)

`browser.engine` reserves `camoufox` and `fpchromium` (parse error today: "watchlist
engine, not implemented — see docs/dev/v2/specs/browser.md §3.4"). Promotion criteria:
maintained Windows + macOS + Linux builds, CDP pipe support, a licence permitting
user-download automation, and canary parity with CloakBrowser. Fingerprint seeds and
profile plumbing are engine-agnostic and must not grow CloakBrowser-specific coupling
beyond `launch.rs::flags`.

## 4. Display policy

`browser.display` values stay `auto | hidden | headless | window`; the **default resolution
changes** and hidden gains a per-OS mechanism.

| OS | `hidden` (default for `auto`, both engines) | `window` | `headless` |
|---|---|---|---|
| Linux | Private **Xvfb** display (`display.rs`: from `:99` up, `-screen 0 1920x1080x24 -nolisten tcp -terminate`, or `-displayfd`), `DISPLAY=:N`, `WAYLAND_DISPLAY` removed, `--ozone-platform=x11`. If `Xvfb` is absent but **`Xwayland`** exists (Wayland-only host), run Xwayland on a private display (`-noreset`; port unipi `display.rs` `Backend::{Xvfb, Xwayland}` detection). **Never** the user's own display/compositor. | Real window on the user's desktop | Last resort (§4.2) |
| Windows | Headful, **real window created off-screen**: `--window-position=-32000,-32000` + the anti-backgrounding flags (already present) keep compositing active. Canary-asserted `document.visibilityState === "visible"`. | Real window | Last resort |
| macOS | Same off-screen window mechanism as Windows. | Real window | Last resort |

Notes:

1. **Resolution change** (v2): `auto` ⇒ `hidden` for every engine, not only
   "Xvfb + cloak" (violates today at `service.rs:226-231` `placement_choice`, and
   `service.rs:329` maps `Auto` to `Placement::Headless`). On Windows/macOS `hidden` is
   the off-screen window (v1 mapped `hidden`→headless there: the `Xvfb`-only fallback at
   `service.rs:309-317` plus `Headless|Auto` at `service.rs:329`).
2. **Headless is a last resort**, reached only when: the user explicitly sets
   `browser.display = headless`, **or** hidden failed (no Xvfb **and** no Xwayland on
   Linux; off-screen window refused/unavailable on Windows/macOS) and the user declined a
   visible window. Whenever a CloakBrowser profile actually runs headless, every surface
   shows the warning: status `stealth_note "Less stealthy: no real window — install Xvfb
   (Linux) or allow a window"` (`service.rs:505-510` extended), the pane badge
   (existing `stealthBadge`, `browserPane.ts:79`), Settings, and the TUI `/browser` panel.
   The CLI `--browser=ephemeral` (CI/scripts) keeps headless with the warning logged.
3. `Show window` / `Show browser window` switches the profile to `window` with the v1
   relaunch-reopen semantics (logical tab ids survive; reopen ≤ 32 http/https/about:blank
   tabs). Off-screen → on-screen moves are a relaunch (simplest correct path; no
   `SetWindowPos` bridge is in scope).
4. Common flags are unchanged except §5 H5 (add `--disable-blink-features=
   AutomationControlled`). The anti-backgrounding set (`--disable-backgrounding-occluded-
   windows`, `--disable-renderer-backgrounding`, `--disable-background-timer-throttling`)
   stays — it is what keeps frames and rAF alive on a hidden/off-screen window.

## 5. CDP hygiene — testable requirements

Every rule below is enforced by a unit test (flag sets, setup-call lists) and/or the CI
canaries (§10). "Now" states the current code's verdict with `file:line`; "Change" is the
work item. The rules replace v1 §4.4's informal notes.

| # | Requirement | Now (v2 branch) | Change |
|---|---|---|---|
| H1 | **Never `Runtime.enable` or `Console.enable`.** Both enable the JS debugger agent in the renderer and are flagged by leak tests (leaky.page debugger/CDP rows). | **Violated**: tab setup calls `Runtime.enable` (`crates/oino-browser/src/profile.rs:300`), and OOPIF child sessions call it again (`profile.rs:586`). Console/errors rings consume `Runtime.consoleAPICalled` / `Runtime.exceptionThrown` (`profile.rs:495-527`). No `Console.enable` anywhere. | Remove both. Console + error logs switch to the **`Log` domain**: `Log.enable` + `Log.entryAdded` (source `console-api` / `exception`), mapped into the same rings (`tab.rs`). Ring tests updated; `browser_logs` semantics unchanged. |
| H2 | **Evaluate only in an isolated world with an explicit `executionContextId`.** Main-world JS from the controller is detectable (bindings, `Error` stack shape) and perturbs page state. | **Complied for `browser_evaluate`/probe/cursor/copy**: `page::evaluate` creates a world (`page.rs:887-949`) and resolves args into it. **Violated for helper JS**: `raf()` runs `Runtime.evaluate` with no context (`page.rs:452-462`); `object_of` (`page.rs:366-385`) resolves refs into the main world and `call_on` (`page.rs:392-417`) runs `ENABLED`/`CONTAINS`/`DESCRIBE`/`FOCUS_CHECK`/`CLEAR_JS`/`SELECT_JS`/file-input checks there (`commands.rs` call sites). | Resolve refs with `DOM.resolveNode { executionContextId }` into the tab's cached isolated world (one world per frame per tab, created lazily, reused — also cheaper than v1's world-per-call); run every helper through it; `raf` barrier evaluates in the world. Grep-test: no `Runtime.evaluate`/`callFunctionOn` call without `executionContextId` in the crate. |
| H3 | **No main-world `Page.addScriptToEvaluateOnNewDocument`.** | Complied (absent). | Keep absent. The element picker (§9.3) is the only page injection allowed: transient, isolated-world, user-initiated, torn down after picking; never during agent automation. |
| H4 | **Avoid `Runtime.addBinding`.** | Complied (absent). | Keep absent; introducing it anywhere needs an owner decision (it is page-visible by design). |
| H5 | **Flags: never `--enable-automation`; always `--disable-blink-features=AutomationControlled`.** | No `--enable-automation` anywhere (asserted, `launch.rs:331`). **Omission**: no `--disable-blink-features=AutomationControlled` (`launch.rs:67-108`). | Add it to the common flag set for **both** engines. Extend the `flag_sets` unit test (`launch.rs:317-341`): asserts both. |
| H6 | **Never `--disable-gpu` or SwiftShader flags while headful.** GPU-disabled renderers are a classic artifact (WEBGL_renderer strings). | Complied (neither flag is ever emitted; hidden placement uses no GPU flags). | Add a guard test: `flags()` never emits `--disable-gpu`, `--use-angle=swiftshader`, `--use-gl=swiftshader` in any placement. (On Xvfb Chromium falls back on its own — that is what a VM user looks like; we do not add flags.) |
| H7 | **Pipe transport.** No TCP DevTools port for other local processes to attach to. | Complied on Unix (`--remote-debugging-pipe`, fds 3/4, `launch.rs:273-300`; default `TransportChoice::Auto`→pipe, `service.rs:331-335`). Windows uses loopback port 0 + `DevToolsActivePort` (`launch.rs:81-84`) — documented residual risk. | Unchanged. Windows pipe remains out of scope (handle-inheritance code we don't want to own). `browser.externalCdp` stays off by default. |
| H8 | **Persistent aged profiles, one fingerprint seed each.** Fresh-profile smell and seed churn are red flags; identity must be stable. | Mostly complied: profiles persist under `~/.oino/browser/profiles/<name>`; `oino-profile.json {created, lastUsed, cookieDomains}` (`profiles.rs:35-45`); one `.cloakbrowser-seed` per profile, generated once (`launch.rs:116-129`); no history deletion. | Add a seed-integrity rule: never regenerate an existing seed; profile **age** is preserved (never recreate the dir; the "Clear browser data" action keeps meta + seed — already specified v1 §12). Document: profiles get their "age" by surviving; we do not fabricate history. |
| H9 | **Humanized agent input with pointer micro-movement.** Raw teleporting pointers and instant `insertText` are behavioural tells. | Partly: v1 humanizer is a simplified Bézier (12/24 steps, fixed 8/14 ms, `page.rs:664-700`), per-char keys 35/90 ms (`page.rs:853-885`), notched wheel (`commands.rs notches_for`). `auto` preset = on only for cloak (`commands.rs:105-110`). | Port unipi `humanize.rs` (MIT, CloakBrowser wrapper logic) as `crates/oino-browser/src/humanize.rs`: overshoot Bézier paths, element-box click targeting (inputs centre-biased), inter-key delays with mistype+correction, eased wheel plans; presets `default`/`careful` map to existing `browser.humanize` values. Only rewrites agent input, never user input (`frontend.rs input_event` stays raw). Pure-plan unit tests port with it. |
| H10 | **No idle CDP chatter; optional quiet mode on sensitive loads.** Open DevTools connections themselves aren't page-visible, but constant command traffic during loads is observable via timing; screencast only while a viewer is attached. | Partly: screencast starts/stops with viewers (`frontend.rs:33-146`); no periodic polls exist; tab setup runs once per tab. Nothing origin-aware. | New setting `browser.cdpQuiet` (default `true`): while a tab's origin matches `browser.sensitiveOrigins` and no viewer is attached and no agent command is in flight, the service issues **no** CDP commands on that tab's session (no probes, no screencast restarts); the first viewer/command resumes normally. Cannot truly "detach" (pipe stays), so the contract is command-silence, tested by counting calls in a fixture run. |
| H11 | **Never render agent pages in the Tauri webview; no window reparenting.** | Complied by design (screencast canvas; `Page.startScreencast` is not page-detectable — research memo). | Keep as an invariant test in the app: the Browser pane renders frames only via the proto stream (no `webview`/`iframe` usage in BrowserView code paths). |

Domain-enable budget after H1/H2: `Page.enable` (frame/dialog/screencast events),
`Network.enable {maxPostDataSize: 0}` (logs), `Log.enable` (console/errors),
`Target.setDiscoverTargets` + `Target.setAutoAttach` (popups/OOPIF). **`DOM.enable` is
dropped** (no DOM events are consumed; `DOM.*` commands work on demand — v1 called it at
`profile.rs:302` and `profile.rs:586` for nothing).

## 6. Ports from unipi (inventory)

| Source (apps/unipi/crates/host/src/browser/) | Destination | Notes |
|---|---|---|
| `cloak.rs` (702 lines) | `crates/oino-browser/src/cloak.rs` | §3.2. Verbatim + cache-root rename + host-task progress. |
| `cookies.rs` (696) | `crates/oino-browser/src/cookies.rs` | §6.3. Chrome/Chromium/Brave v10/v11 (Linux Secret Service / `--password-store=basic`), macOS Keychain; Windows refused (App-Bound) with a clear error. One domain at a time, user action only, values never logged; `Storage.setCookies` into the target profile; record in `oino-profile.json cookieDomains`. |
| `gate.rs` + `agent.rs` (take-over arbitration) | already re-implemented pure in v1 (`policy.rs:151` + `tab.rs`/`frontend.rs`) | Keep the pure model. Port the **wording and rules delta**: input within `DRIVING_WINDOW` (3 s) counts as driving; take-over expires after `TAKEOVER_IDLE` (10 min) — both already in v1 (`frontend.rs:396-425`, `service.rs:425-440`). The unipi CDP **proxy** (`gate.rs::serve`) is not ported — v1's in-process gate refuses before dispatch; the optional external endpoint stays out of scope (below). |
| `display.rs` (385) | `crates/oino-browser/src/display.rs` | §4: add `Backend::{Xvfb, Xwayland}` detection + `-displayfd` support; keep the shared-display, `-terminate`, `:99..199` scan. |
| `humanize.rs` (836) | `crates/oino-browser/src/humanize.rs` | §5 H9. MIT notice kept (CloakHQ). |
| `session.rs` `Adapt` | `crates/oino-browser/src/frontend.rs` | §8.1: adaptive JPEG quality 25–80, `everyNthFrame` 1–4, -20 quality on oversize frames; feed from viewer ack RTT. |
| `profiles.rs` eviction (`pick_eviction`) | already in v1 (`service.rs:284-307` idle victim pick) | Unchanged. |
| `mod.rs` placement policy (`placement_for`, `HEADLESS_NOTE`, `can_show_window`) | `crates/oino-browser/src/service.rs` | §4 rewrites `placement_choice` on the unipi model extended with the off-screen window. |
| `mod.rs` `CANDIDATES`, snap rule | already ported (`engine.rs:180-218`, `profiles.rs:20-36`) | Unchanged. |

Not ported: the CDP proxy/agent endpoint (`agent_endpoint.rs`, `gate.rs::serve` —
superseded by the in-process service), `session.rs`'s phone-stream framing (replaced by
proto + `frontend.rs`), unipi's config re-read loop (v2 reads oino settings).

### 6.3 Cookie import (user action only)

Surface: Settings → Browser → "Import logins from Chrome for a site…", the CLI
`unipi-host browser cookies --from chrome --domain <d> --into <profile>`, and — on the
host-local app only — a pane context action. The agent may *suggest* it (skill rule,
unchanged v1 §11.4) and can see `cookieDomains` via `browser_profile status`, nothing more.
Decryption runs in the host process; only the decoded cookie list crosses to the service.

## 7. The `browser` host service

Contract summary (envelope details belong to `specs/ext-abi.md`; registration to
`specs/host.md` — this section fixes the browser's part):

- Service name: `"browser"`. Manifest permission: `host-service: browser` (the existing
  `browser` permission flag migrates to this).
- Ops (op-tagged JSON, exactly the v1 wire shapes in `oino-browser-protocol/src/command.rs`):
  the 19 tool commands (`navigate`, `snapshot`, `click`, `hover`, `type`, `press_key`,
  `scroll`, `select_option`, `drag`, `file_upload`, `handle_dialog`, `take_screenshot`,
  `evaluate`, `wait_for`, `tabs`, `logs`, `downloads`, `handoff`, `profile`) plus
  host-facing `status`, `turn_shot`, `turn_ended`, `session_closed`, `close_agent_tabs`.
  A call takes `BrowserRequest { …command, approval }`; replies are `BrowserResult`
  (isError + `details.error`), so host-errors stay permission/transport only.
- Scope: the host resolves `(session_id, workspace, profile?, own?)` from the caller the
  way core does today (`BrowserScope`, `oino-browser-protocol/src/scope.rs`) — nested
  in-process agents resolve to the root session; `browserScope:"own"` agents get an
  isolated context (v1 §6).
- Frontend/viewer ops are **not** on this interface: the app pane/phone speak app proto
  directly to unipi-host (§2), which calls `BrowserHost::{attach_view, frontend,
  subscribe}` in-process.
- Unavailable by design: under plain `oino` (TUI/CLI/RPC/ACP) no host service `"browser"`
  exists; the extension registers **no tools**, `/browser` prints
  `browser unavailable — this oino has no unipi-host`, and the prompt section is absent.
  Same behaviour when the service exists but reports `enabled:false` / no engine
  (existing register/unregister logic, `extensions/oino/browser/src/lib.rs:42-86`, moves).

## 8. Screencast plane (app pane + phone)

### 8.1 Frames

- Source: `Page.startScreencast` per tab (not page-detectable), started on the first
  attached viewer, stopped when the last detaches (`frontend.rs:33-146` — keep).
- **Adaptive quality** (port of unipi `session.rs` `Adapt`): quality clamped 25–80,
  `everyNthFrame` 1–4; smoothed ack-RTT > 700 ms ⇒ −15 quality / +1 skip, > 400 ms ⇒ −10;
  < 250 ms ⇒ +5 / −1 skip; a frame over the wire budget ⇒ −20. Implemented in
  `frontend.rs` (today a fixed `quality 70, everyNthFrame 1` at `frontend.rs:74-87`): the
  viewer `ViewAck` (exists, `frontend.rs:310-320`) feeds the estimator; parameter changes
  re-issue `Page.startScreencast`. In-process panes ack immediately so
  they pin quality 70+; the phone benefits most.
- Flow control: ≤ 2 un-acked frames per viewer (exists, `profile.rs:825-851 push_frame`).
  A new viewer gets a still (`Page.captureScreenshot` jpeg) so it is never blank
  (`frontend.rs:89-115` — keep).
- Phone defaults: `maxFps 10`, frames only while the sheet is visible (proto behaviour,
  unchanged).

### 8.2 Input relay and take-over

- Viewer input → `BrowserInput` → trusted CDP `Input.*` on the tab's session
  (`frontend.rs:input_event`): mouse/wheel/key/text/IME/clipboard-paste/history/goto/
  resize — unchanged. User input is never humanized (H9).
- Take-over: any user input while the agent is driving (or within 3 s of its last
  command) flips `controlledBy = user`; agent driving commands then fail with
  `taken_over` (gate, `policy.rs:151`); Give back / handoff Done/10-min idle returns
  control (`frontend.rs:142-198`, `service.rs:425-440`). Handoff cards, dialogs,
  downloads dock at the canvas bottom (app side). All unchanged from v1; verified
  against the imported app's `agentStrip` (`browserPane.ts:26`).

## 9. App UX (ZCode patterns on the unipi app)

The app is the imported Tauri/SolidJS app (`apps/unipi/apps/mobile`, SolidJS, works on
desktop and phone). ZCode is the **UX reference only** — its Electron guest/webview
mechanics are explicitly not taken; everything renders from proto screencast frames.

### 9.1 Side-pane browser tabs

- ZCode model (`workspaceSidePane.ts:11-29`): side-pane tabs typed `browser`, each with
  frozen `ownerTaskId` (conversation) + `workspaceKey`, title/favicon from page metadata,
  agent-opened popups marked. Map to oino: side-pane tabs keyed by `TabInfo.id`, owned by
  the session; opening/selecting a tab the agent reveals opens/selects the pane tab
  (existing `reveal` event handling, app side).
- The pane keeps the imported persistence (`browserPane.ts`): open/width/per-profile
  state per pane, `PANE_MIN`/`PANE_DEFAULT` clamps (desktop only).

### 9.2 Browser chrome (tab strip + URL bar + nav)

Already present in `BrowserView.tsx` (tab strip with agent `bot` badges, URL bar with
back/forward/reload/stop, address submit, page state from `page` events, profile chip,
stealth badge). Keep, with:

- **Engine/stealth badge** (`stealthBadge`, `browserPane.ts:79`): shows engine name +
  stealth state + `stealth_note` warning when headless (§4.2). `BrowserStatusData`
  already carries `engine/stealth/stealth_reason/stealth_note/display`.
- **Profile picker** with per-profile **engine badge** (needs `BrowserProfileInfo.engine`
  added to proto + `ProfileInfo.engine` in `oino-browser-protocol`).
- Viewport controls (ZCode `BrowserViewportToolbar`): keep minimal — the existing Fit
  toggle plus explicit width/height inputs in the ⋯ menu (bounds 320–2560 × 240–2560),
  driving `BrowserInput::Resize`. Zoom stays pane-level (CSS scale of the canvas), not
  page zoom.

### 9.3 Element picker → composer chip (new)

ZCode reference: `docs/ui/web-element-context.md` (flow, state boundaries, captured
fields). Our mechanics (no Electron):

- Entry: pointer icon in the browser pane toolbar (desktop pane first; the phone sheet
  can adopt it later — same host op).
- Mechanism: the pane calls a new frontend op **`picker_start`** (host-local, on the
  BrowserHost frontend surface, not the agent tool surface). The host injects, in the
  tab's **isolated world** (H3): mousemove/scroll listeners + a transient highlight
  overlay (absolutely-positioned box + label) built from `elementFromPoint`. The page's
  main world sees nothing but the transient DOM nodes (acceptable: user-initiated,
  seconds-long, removed on exit; never on agent-driven tabs mid-automation).
- On click: the host collects the ZCode field set — page URL/title, tag, role, accessible
  name, CSS selector + XPath, visible + nearby text (truncated), safe attribute
  whitelist, bounding rect, computed-style summary, sanitized HTML excerpt — plus an
  **element screenshot** (existing `element_box` + `captureScreenshot` clip path,
  `page.rs:1022-1050`). Overlay torn down; op returns `{element, screenshotPath}`.
- Composer chip: the app shows a `WebElementContextAttachmentChip`-equivalent (role/name/
  selector + thumbnail); submitting flattens to a Markdown text block appended to the
  prompt (fields above, screenshot attached as an image attachment), exactly ZCode's
  "prompt text context, chip is the visual shell" boundary. Chips are per-chat
  in-memory (cleared on chat switch), consistent with the pending-images fix
  (DIRECTION §4.3). Esc cancels picking; no cookie/storage/value reads; password fields
  masked.

### 9.4 User screenshot → composer attachment (new)

- Toolbar camera button on the browser pane: captures the current view. Implementation:
  prefer the latest screencast frame from the host (`frontend` op **`view_capture`**:
  `Page.captureScreenshot` jpeg q85 when the tab is live — same non-detectable path);
  returns a path under `~/.oino/browser/shots/<session>/user-*.jpg`.
- The app adds it to the composer as a normal image attachment (same store as annotated
  images; works with the ellipse tool). Never enters model context unless sent.

### 9.5 Phone

Browser sheet (existing `BrowserView` sheet variant): tab strip, URL bar, take-over
strip, handoff/needs-you items, touch mapping (tap/drag/long-press/pinch/IME) — all
imported and kept. Phone-only deltas: default `maxFps 10` + adaptive quality (§8.1);
element picker deferred (report only).

### 9.6 TUI (plain `oino`)

`/browser` reports unavailable (§7). No TUI browser viewer is added in v2 (v1's
`/browser` panel goes away with the in-process service).

## 10. CI canaries

Job: `cargo xtask browser-canary` (new `xtask/src/browser_canary.rs`), wired as a
`.github/workflows/ci.yml` job `browser-canary` — `schedule` (nightly) +
`workflow_dispatch`, never per-PR (flaky-network sites). Runs on `ubuntu-latest`:
`apt-get install -y xvfb` (+ `google-chrome-stable` present on runners). **Runs headful
under Xvfb** for both engines:

1. Baseline: real system Chrome, headful, private Xvfb display, fresh persistent profile
   dir reused across runs (aged: kept in the CI cache, `actions/cache` on
   `~/.cache/oino-canary/chrome-profile`).
2. Candidate: CloakBrowser (installed by the canary itself through `cloak::ensure` — this
   also exercises the downloader), headful, private Xvfb display, its own aged profile
   cache.
3. Visits, via the oino-browser service itself (real CDP path, real humanizer off for
   determinism): `https://leaky.page`, `https://bot.sannysoft.com`,
   `https://abrahamjuliot.github.io/creepjs/`, `https://www.browserscan.net/`,
   `https://deviceandbrowserinfo.com/are_you_a_bot`. Reading results uses the AX snapshot
   pipeline (`browser_snapshot`-equivalent on the service) — no new CDP surface.
4. Report: `target/browser-canary/<run>/report.json` + per-site screenshots + the two
   profiles' `BrowserStatus`; uploaded as artifacts.

**Pass criteria** (fail the job otherwise):

| Site | Hard fail | Budget vs baseline |
|---|---|---|
| leaky.page | any CDP/debugger/headless/automation row detected | trust score ≥ baseline − 3 |
| bot.sannysoft.com | any of `WebDriver`, `Chrome`, `Permissions`, `Languages`, `WebGL Vendor` red when baseline is green | — |
| CreepJS | page verdict contains "bot"/"headless" | `lies` count ≤ baseline; score report-only |
| browserscan.net | — | bot score ≥ baseline − 5 |
| deviceandbrowserinfo.com/are_you_a_bot | verdict ≠ baseline verdict ("not a bot") | — |

Plus a generic self-check asserted on both engines: `navigator.webdriver === false`,
`document.visibilityState === "visible"` under hidden placement (validates §4 off-screen/
Xvfb mechanics), and the H5–H7 flag sets. First run records the baseline; subsequent runs
diff it. Failures open an issue (template in the job) — canaries are a tripwire, not a
release gate; the owner decides on regressions (R4).

Local: `cargo xtask browser-canary --local` (uses an installed Chrome + CloakBrowser,
`#[ignore]`d in normal test runs).

## 11. Settings deltas (`browser.*`, oino settings)

| Key | v2 | Change |
|---|---|---|
| `browser.display` | default `auto` ⇒ **hidden per OS** (§4) | resolution change; `hidden` gains per-OS mechanism |
| `browser.engine` | `auto` no longer silently falls back (§3.1) | behaviour change |
| `browser.sensitiveOrigins` | globs, default `["accounts.google.com"]` | new (§3.3, H10) |
| `browser.cdpQuiet` | bool, default `true` | new (H10) |
| profile `engine` pin | `oino-profile.json {"engine": "cloak"\|"chrome"}` | new (§3.3) |
| everything else (`profile`, `maxRunning`, `maxTabs`, `idleMinutes`, `humanize`, `evaluate`, `approvals`, origins, `turnScreenshot`, `ambientContext`, `externalCdp`, `transport`, `viewer.maxFps`, `cloak.version`) | unchanged from v1 §12 | — |

CLI: `unipi-host browser install-cloak | cookies … | status | profiles … | stop | window`
mirror the v1 `oino browser` set, now host-side.

## 12. Work packages

Order: WP-B1 → (WP-B2 ∥ WP-B3 ∥ WP-B4) → WP-B5 → WP-B6 → WP-B7 → WP-B8; WP-B9 after
B2+B3+B4 (parallel with B5–B8). Each WP lands `[<wp-id>] …` commits and keeps
`cargo fmt/clippy/test` green for touched crates; `cargo xtask wasm` + extension harness
green where extensions move.

### WP-B1 `browser-v2-contracts` — protocol + settings deltas

| File | Change |
|---|---|
| `crates/oino-browser-protocol/src/status.rs` | `ProfileInfo.engine` (option, `"cloak"\|"chrome"`), `BrowserStatus.hint` field on navigate results (`BrowserToolDetails.browser.hint`) |
| `crates/oino-browser-protocol/src/command.rs` | no shape changes (document the op table as the host-service op table, §7) |
| `crates/oino-browser/src/config.rs` | `sensitive_origins`, `cdp_quiet`; `DisplayChoice::Auto` docs |
| `crates/oino-settings` / settings schema | new keys + defaults |
| tests | serde round-trips, clamp/defaults |

Dep: none. Acceptance: schema + round-trip tests; `docs`/`deviations` note pointing at
this spec.

### WP-B2 `browser-cdp-hygiene` — rules H1–H7

| File | Change |
|---|---|
| `crates/oino-browser/src/profile.rs` | remove `Runtime.enable`/`DOM.enable` from attach (299-313) and OOPIF setup (586); add `Log.enable`; map `Log.entryAdded` → console/errors rings |
| `crates/oino-browser/src/tab.rs` | ring ingest from Log entries (source/kind mapping) |
| `crates/oino-browser/src/page.rs` | cached per-frame isolated world; `object_of`/`call_on`/`raf` run in it (explicit `executionContextId` everywhere) |
| `crates/oino-browser/src/launch.rs` | `--disable-blink-features=AutomationControlled`; guard test for H5/H6 flag sets |
| `crates/oino-browser/tests/real_browser.rs` | assertions: console/errors still collected (fixture page logs); no Runtime/DOM enables (fixture asserting via CDP echo of sent commands is enough: unit-test the setup list) |

Dep: B1. Acceptance: new unit tests (setup-call list, flag sets, "no bare Runtime calls"
grep test); `OINO_TEST_BROWSER` real tests green incl. logs; hygiene matrix §5 updated to
"complied" with new line refs.

### WP-B3 `browser-cloak-install` — signed downloader + engine policy

| File | Change |
|---|---|
| `crates/oino-browser/src/cloak.rs` (new) | verbatim port of unipi `cloak.rs` + §3.2 deltas (cache root, host-task progress callback) |
| `crates/oino-browser/src/engine.rs` | resolution per §3.1 (profile pin → setting → auto-with-install; today `Auto` silently falls back at `engine.rs:86-88`); `CloakNotInstalled` carries the install action |
| `crates/oino-browser/src/profiles.rs` | engine pin in meta; `ProfileInfo.engine` |
| `crates/oino-browser/src/service.rs` | first-use flow: `cloak_not_installed` result + install task; sensitive-origin hint on navigate (`policy.rs` globs) |
| `crates/oino-browser/Cargo.toml` | `ed25519-dalek`, `sha2`, `reqwest`, `flate2`/`tar`/`zip` |

Dep: B1. Acceptance: ported unit tests (signature/manifest/version pinning — unipi's
tests port too); mocked-download test; watchlist names parse with the documented error.

### WP-B4 `browser-display-policy` — per-OS hidden placement

| File | Change |
|---|---|
| `crates/oino-browser/src/display.rs` | port unipi `Backend::{Xvfb, Xwayland}` detection + `-displayfd` |
| `crates/oino-browser/src/launch.rs` | off-screen window flags for Win/macOS hidden; placement-specific flag test |
| `crates/oino-browser/src/service.rs` | `placement_choice` rewrite (§4: auto ⇒ hidden everywhere; headless last resort + notes); `Show window` relaunch path unchanged |

Dep: B1. Acceptance: placement matrix unit tests; real-browser test on Linux asserting
`visibilityState == visible` under hidden; status notes exercised.

### WP-B5 `browser-humanize` — full humanizer

| File | Change |
|---|---|
| `crates/oino-browser/src/humanize.rs` (new) | unipi `humanize.rs` port (MIT notice), pure plans + presets |
| `crates/oino-browser/src/page.rs`, `src/commands.rs` | `move_to`/`click_at`/`press`/`insert_text`/wheel driven by plans; `auto` = on for cloak (unchanged), now also default-on for chrome profiles (owner: humanize default everywhere; `off` available) |
| tests | ported plan tests; timing bounds smoke |

Dep: B2 (page.rs isolated world). Acceptance: plan tests green; real-browser click/type
under humanizer unchanged behaviour.

### WP-B6 `browser-cookies-sensitive` — cookie import + cdpQuiet

| File | Change |
|---|---|
| `crates/oino-browser/src/cookies.rs` (new) | unipi `cookies.rs` port (decryption stays host-side; `Storage.setCookies` into target profile; `cookieDomains` bookkeeping) |
| `crates/oino-browser/src/service.rs` / `frontend.rs` | `cookies_import` frontend op (host-local only); `cdpQuiet` enforcement (H10) |
| host CLI / Settings surface | `unipi-host browser cookies …`; Settings row |

Dep: B1, B2. Acceptance: ported crypto unit tests (v10/v11 fixtures from unipi); a
fixture run proving zero CDP calls on a sensitive-origin tab while quiet; import into a
live profile integration test (`OINO_TEST_BROWSER`).

### WP-B7 `browser-host-service` — unipi-host service + TUI unavailability

| File | Change |
|---|---|
| `wit/oino-extension/{browser.wit, worlds.wit, registry.wit}` | delete `browser.wit`; the world imports `host-service` (per `specs/ext-abi.md`) |
| `crates/oino-extension-api/src/browser.rs` | `BrowserHost` trait + WIT glue move out of core into the unipi host side (host-internal trait); permission becomes `host-service: browser` |
| `crates/oino-wasm-host`, `crates/oino-core` | remove browser wiring; no core dep on `oino-browser` |
| `crates/oino/src/{lib.rs, browser.rs, main.rs, cli.rs}` | delete in-process service; `--browser` flags removed or map to a clear error |
| `extensions/oino/browser` → `extensions/unipi/browser` (id `unipi.browser`) | move + re-point to `host-service.call("browser", …)`; unavailable behaviour (§7); skill/turn-shot unchanged |
| unipi-host (per `specs/host.md`) | register service `"browser"` backed by `BrowserService`; expose proto ops (`browser_open` etc.) on `BrowserHost` |
| tests | extension harness with a fake host-service; plain-`oino` test asserting no tools + `/browser` message |

Dep: B1–B4 landed; `spec-ext-abi`/`spec-host` implemented enough to call through.
Acceptance: `cargo xtask wasm` builds `extensions/unipi/browser`; harness green; plain
`oino` TUI shows "browser unavailable"; the host runs the 19 tools end-to-end (faux
provider smoke or harness).

### WP-B8 `browser-app-ux` — pane + phone UX

| File | Change |
|---|---|
| `crates/oino-browser/src/frontend.rs` | adaptive quality (§8.1); `picker_start/picker_stop/view_capture` frontend ops |
| `apps/unipi/crates/proto/src/browser.rs` | `BrowserProfileInfo.engine`; picker/capture passthrough on the host-local surface |
| `apps/unipi/apps/mobile/src/screens/BrowserView.tsx` | picker toolbar button + highlight UX; screenshot button; engine badge in profile picker; headless warning badge |
| `apps/unipi/apps/mobile/src/lib/{browserPane.ts, composer*}` | element-chip store (per-chat, in-memory) + Markdown flatten on submit; screenshot→attachment |
| `apps/unipi/apps/mobile/src-tauri/src/browser.rs` | command wiring for the new ops (host-local only) |
| unipi-host | proto op ↔ `BrowserHost` mapping for picker/capture |

Dep: B7 (+ B6 for cookies UI if Settings rows ship together). Acceptance: manual/QA
script — picker chip appears in composer and round-trips into a sent message; screenshot
attachment annotatable; take-over/handoff still work; phone sheet regression; pane
screenshots in `docs/dev/v2/design/shots/browser-*.png`.

### WP-B9 `browser-canary-ci` — canaries

| File | Change |
|---|---|
| `xtask/src/{main.rs, browser_canary.rs}` | `browser-canary` subcommand (§10): baseline + candidate runs, report, criteria |
| `.github/workflows/ci.yml` | `browser-canary` job (schedule + dispatch), artifact upload, issue template |
| `docs/dev/v2/specs/browser.md` | canary runbook (how to read failures, bump pins) |

Dep: B2, B3, B4. Acceptance: one green nightly run against live sites with artifacts;
deterministic unit tests for the verdict parsers using saved fixture snapshots.

## 13. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | **CloakBrowser licence**: binary must never be bundled/redistributed; offline installs can't get it. | Downloader is the only path (signed); offline degradation to real-Chrome profile with badge; installer spec must not vendor the binary. |
| R2 | **Version lag**: pinned free build (146.x family) ages vs live Chrome; entropy from engine version itself. | Pins bump with upstream wrapper releases; canaries catch staleness (R4); `browser.cloak.version` escape hatch. |
| R3 | **Behavioural scoring**: fingerprint patches + humanizer don't defeat interaction biometrics (pointer dynamics, rhythm). | Humanizer default-on (H9), headful default, persistent profiles; documented residual risk; `careful` preset; CreepJS/browserscan budgets as tripwires. |
| R4 | **Canary flakiness** (live sites change, network). | Nightly/dispatch only, never PR-gating; per-site budgets; artifacts + issue template; baseline re-record procedure in the runbook. |
| R5 | **Off-screen window quirks** (Windows/macOS occlusion marking windows hidden; clamping). | `visibilityState == visible` asserted in canaries and a real-browser test; fallback ladder documented (§4.2); if an OS clamps, the release note upgrades that OS to `window`-by-default and the owner is asked. |
| R6 | **Xvfb absent on user machines** (Linux). | Status note with install hint (existing wording), Chrome fallback ladder, canary CI proves the note fires. |
| R7 | **Google sign-in heuristics change** (blocking real Chrome with CDP too). | Real-Chrome dedicated profile is the best-effort path (no automation flags, no CDP detectability beyond connection); handoff (`browser_handoff`) is the escape hatch; cookie import covers non-Google logins. |
| R8 | **Phone bandwidth** for high-fps screencast. | Adaptive quality + `maxFps 10` + visible-only streaming (existing). |
| R9 | **Main-world JS removal (H2) regresses actionability** (isolated-world objects must resolve identically). | Real-browser test suite covers obscured/enabled/hit-test paths; isolated world is shared-DOM so semantics are identical. |

## 14. Owner questions

1. **Off-screen window fallback (R5):** if canaries show `visibilityState: hidden` or OS
   clamping on Windows/macOS, should hidden-by-default degrade to (a) a real visible
   window by default, or (b) headless with the warning? Spec default: (b) until ruled.
2. **First-use download friction:** confirm-dialog on first browser use (chosen) vs
   silent auto-download. Spec default: confirm; one flag flips it.
3. **Canary cadence:** nightly (chosen) vs weekly; and whether a canary job should also
   run on PRs touching `crates/oino-browser/**` as non-blocking advisory.

## 15. Out of scope (v2)

External CDP endpoint for third-party tools (`browser.externalCdp` stays a setting with
no implementation); WebM recording; PDF/print; network interception tools; raw CDP for
the model; Windows cookie import (App-Bound); window reparenting/embedding the engine in
the app; Camoufox/Fingerprint-Chromium; fabricating profile age/history (H8); per-tab
proxies; remote/cloud browser providers.
