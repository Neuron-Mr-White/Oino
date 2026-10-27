# Oino v1.0.0-alpha.2 — status

Date: 2026-10-27 · branch `v1-alpha` · work id `integration-2` (follow-up round: core-agents,
core-settings, frontend-cmds, providers-more, unipi-utility, design-spec / design-desktop(-fix) /
design-mobile, release-builds).
Parity targets: `@earendil-works/pi-coding-agent` **1.4.0**, unipi **3.0.0**, ZCode design
([design.md](design.md)).
Legend: ✅ done · 🟡 partial · ❌ missing · ⛔ diverges by design. Settled deviations are folded
into the appendix of each spec; open items per work item stay in [deviations/](deviations/).

## 1. Gates (this commit)

All run locally with `TMPDIR=$PWD/target/tmp`; components rebuilt with rustc 1.99.0.

| Workspace | fmt | clippy `-D warnings` (pedantic) | tests | other |
|---|---|---|---|---|
| root (`crates/*`, `xtask`) | ✅ | ✅ | ✅ 699 passed (`cargo test --workspace`, incl. `crates/oino/tests/bundled.rs`, wasm-host e2e with the real unipi agents / settings components) | ✅ rustdoc `-D warnings`; ✅ `cargo xtask wit`; ✅ `scripts/docs-check.sh`; ✅ `scripts/install-smoke.sh`; all crates except `oino-wasm-host` / `xtask` on `[lints] workspace = true` |
| `extensions/` | ✅ | ✅ (`--target wasm32-wasip2`) | ✅ 664 native (4 ignored); ✅ 7 wasm-host harness suites (`extensions/unipi/*/harness/run.sh`: ask-user 5, core 3, kanboard 3, notify 5, subagents 5, utility 9, workflow 6) | ✅ `cargo xtask wasm` (37 components: 8 providers, 24 unipi, 5 fixtures) + `cargo xtask checksums` |
| `apps/` (GPUI) | ✅ | ✅ | ✅ 83 | — |

`cargo xtask layering` and `cargo deny` (quality §1, §6) do not exist yet.

## 2. End-to-end smoke (release `oino` 1.0.0-alpha.2, temp agent dir, bundle of 33 packages)

| Area | Result | How |
|---|---|---|
| print | ✅ `Hello from faux.` (3.9 s cold, compile cache empty) | `-e extensions/dist/oino.fixture.provider-faux --model faux/faux-1 -p hi` |
| JSON | ✅ header + full event stream; unipi tools (`run_subagent`, `memory_*`, `web_search`, `image_*`, `plan_submit`, …) in the request | `--mode json` |
| RPC | ✅ 17/17 checks | `_oino/hello`, `prompt`, `get_commands` (51 feature commands at alpha.2; renamed since, [commands.md](commands.md)), `get_messages` |
| panel button action | ✅ | `/info` → `openPanel unipi.info-screen/dashboard`; `_oino/ui_action{actionId: refresh}` → `updatePanel`; `close` → `closePanel` |
| settings write | ✅ | `_oino/get_settings_schema` (25 namespaces), `_oino/set_extension_setting footer.enabled=false` lands in `~/.oino/settings.json` as `extensionSettings.footer.enabled`, read back, `unset` |
| btw | ✅ | `/btw …` answers in the panel; session message count unchanged |
| long-horizon continuation | ✅ | `/goal-start {objective}` + prompt → `unipi:lh-continue` nudge delivered by the turn arbiter |
| kanboard nudge | ✅ | `oino-kanboard` on `PATH`: `/kanboard onboard`, `-add`, task to `todo`, `/kanboard-autowork start` → two `unipi:kanboard-next` nudges, then "autowork stalled — no progress after 2 offers" (faux never calls tools) |
| ACP | ✅ | `initialize` (agentInfo 1.0.0-alpha.2), `session/new`, `session/prompt` → `agent_message_chunk` "Hello from faux.", `end_turn`, `usage_update`, `available_commands_update` |
| TUI | ✅ | pty: typed prompt, reply, unipi chrome, `/quit` exit 0 — alternate screen and `--tui-mode regular` |
| real provider | ✅ | Oino Router key from `~/.oino/auth.json` (`router` → provider `oino`): `oino/deepseek/deepseek-flash` print with a `bash` call |
| subagents (real) | ✅ | `run_subagent` foreground → child answered `391`; `child:Subagent …` usage entry |
| fusion (real) | ✅ | preset `lead = sidekick = oino/deepseek/deepseek-flash`; `sidekick` tool → persistent child answered `PINEAPPLE`; `child:Sidekick` usage; auto-rename child (`child:session name`) ran too |
| install | ✅ | `scripts/install-smoke.sh` (source install into a temp prefix + faux prompt) |

Subagents, btw, fusion, memory side-save, dream and long-horizon also have e2e tests with the
native faux provider (`crates/oino-wasm-host/tests/unipi_agents.rs`), settings with the real
components (`unipi_settings.rs`); both ran in the gates.

## 3. Parity matrix — pi 1.4.0

| Area | Status | Notes / next step |
|---|---|---|
| Agent loop (`pi-agent-core`) | ✅ | golden vs pi; TypeBox `format` not validated |
| `AgentSession` (prompt, steer, follow-up, abort, queue modes, retry, overflow recovery) | ✅ | `prompt` returns at run start (runtime appendix #5) |
| Session v3, `SessionManager`, tree, labels, context edits, legacy migration | ✅ | goldens `session-context`, `legacy-migration` |
| Compaction, branch summaries, split turns, file tracking | ✅ | golden `compaction` |
| Built-in tools `read write edit bash grep find ls` (+`powershell`) | ✅ | Windows paths untested; image bytes differ from Photon |
| Bash detach / background adoption (`control.bash-detach`, `running-bash`, `tasks.adopt-bash`) | ✅ | core-gui-gaps: session registry → `HostServices::take_bash` → host `TaskService::adopt` ([frontend-api.md](frontend-api.md) §6) |
| Resources: AGENTS.md / SYSTEM.md / APPEND_SYSTEM.md, skills, prompt templates, trust | ✅ | |
| Themes | 🟡 | custom theme files discovered, only built-in themes rendered |
| System prompt (sections, docs section, D13) | ✅ | docs root = bundled `oino.docs` |
| Settings manager + extension settings registry | ✅ | schema, layers, scoped writes, secret fields, change events; `oino:settings:<ns>` bus topic ❌ |
| Models: registry, `models.json`, `auth.json`, scopes, virtual models | ✅ | dynamic catalogs refreshed at startup |
| Providers (pi-ai wires) | ✅ | 8 components: chat completions (+Copilot, Cloudflare), Responses (+ChatGPT sign-in), Anthropic, Codex, Azure, Google (Gemini, Vertex), Bedrock, Oino Router; 165 pi oracle fixtures |
| Provider transports / auth edge cases | 🟡 | WebSockets ❌, Anthropic federation ❌, Bedrock SSO/IMDS ❌ (`extensions/providers/README.md` M1–M10) |
| OAuth `/login` `/logout` | 🟡 | flows for Anthropic, Copilot, Codex, ChatGPT implemented and fixture-tested; never run against live accounts |
| CLI flags and help | ✅ | |
| Print / JSON modes | ✅ | goldens `print`, `json-mode` |
| RPC mode + extension UI sub-protocol | ✅ | golden `rpc`; RPC / TUI / print exit on `ctx.shutdown()` once idle (`shutdown_requested`); ACP leaves it to the client |
| `_oino/*` frontend commands | ✅ | completions, ui_action, editor_event, shortcuts, settings, service_call, auth, topics, bus_get, agents, queue_edit, checkpoints / rewind, running_bash, extensions — in-process (runtime-agnostic) and RPC (`FrontendApi`, [frontend-api.md](frontend-api.md)) |
| Interactive TUI | 🟡 | full chrome, dialogs, panels with buttons, settings hub, completions, editor intents, `/login`, regular mode ✅; inline images ❌; core settings still via `LocalHost` |
| Package manager + `oino update` | ✅ | dir / archive / https / `git+https`; self-update from `release-manifest.json`; `oino auth`, `ext`, `sessions`, `mcp` ❌ |
| Extension API (pi `ExtensionAPI`) via WIT | 🟡 | events, tools, commands, shortcuts, flags, renderers, providers, virtual models, UI, settings r/w, `models.complete/stream/credential-for`, nested agents, turn arbiter, `control.*`, bash detach, `disabledExtensions`, dialog-aware deadlines ✅; provider hooks routing, `models.classify/generate-image`, `set-ui-policy` ❌ |
| ACP (Zed etc.) | ✅ | protocol v1, permissions, elicitation, client fs/terminal, `--model`; refusals, `mcpServers` forwarding ❌; Zed not tried by hand |
| `oino serve`, `--remote`, multi-client | ❌ | specified (modes §5); blocks mobile/desktop remote attach |
| `/share`, `/bug`, pi telemetry, pi-mcp, codemode | ⛔ | out of scope (charter; MCP is `unipi.mcp`) |

## 4. Parity matrix — unipi 3.0.0

All 23 packages plus `unipi.app-bridge` are ported and bundled (load order
`unipi_kit::suite::LOAD_ORDER`), and cron is ported as an Oino-native feature (owner
decision 2026-10-28, superseding D22): crate `oino-cron`, extension `unipi.cron`, the
`oino cron` CLI (serve/install/…), `/cron`, and the desktop Automations page
(unipi-port §3.24). Not ported by decision (D22): browser skills/state, app push
encryption, Herdr, the unipi desktop app.
Since `unipi-native`: feature commands have plain names (no `unipi:` prefix, no aliases;
`/settings` and `/changelog` merged into the core built-ins, the Fusion picker is `/fusion`,
[commands.md](commands.md)) and there is no legacy unipi support (no `~/.unipi` reads,
imports, migrate prompts, legacy argument shapes, renderers or settings keys).

| Package | Status | Missing / next step |
|---|---|---|
| core (hints, header, work tray) | ✅ | late-night hints need a `host-info` UTC offset (R3) |
| utility | 🟡 | settings, auto-rename (verified live), `/answer`, attachments, image tools, `/move`, doctor ✅; render styles for built-in tools need core `tools.override` / `render` for built-ins (U1–U2); trust.json move (U5) |
| workflow (permissions, plan mode) | ✅ | mode and rules in the settings files |
| long-horizon | ✅ | goal continuation through the arbiter verified (e2e + smoke); graph runs not e2e-tested |
| memory | 🟡 | markdown memories, tools, recall, side-save ✅; MemPalace untested against a real install |
| dream | ✅ | nested-agent dream runs with resume (e2e) |
| skill-registry | ✅ | |
| info-screen | ✅ | dashboard panel + actions verified in the smoke |
| subagents | ✅ | foreground/background/read verified (e2e + live); UI policy fixed at spawn (R2) |
| background-tasks | 🟡 | tasks, tray, notifications, adopting a running bash call (`tasks.adopt-bash`) ✅; notification `deliverAs: followUp` (R1) |
| btw | ✅ | verified in the smoke |
| web-api | ✅ | no TLS fingerprinting (`impersonated: false`) |
| ask-user | ✅ | forms unavailable over ACP |
| mcp | 🟡 | stdio + HTTP ✅; pi `mcp.json` not readable (R1); ACP `mcpServers` not forwarded (R3) |
| notify | ✅ | wizards and secrets in settings |
| kanboard | ✅ | CLI/daemon, panel, desktop native board, monitor nudges through the arbiter (smoke); TUI renders the board panel generically; `/kanboard open` opens the side pane at any window width ([gui-qa-fix] QA-5) |
| command-enchantment | ✅ | TUI and desktop merge `complete-input` |
| compactor | ✅ | `llm` method lacks pi's previous-summary parameter (R1) |
| footer | ✅ | |
| updater | ✅ | never installs (by design); `/changelog` falls back to GitHub release notes when no `CHANGELOG.md` exists ([gui-qa-fix] QA-4) |
| cron (Oino-native) | ✅ | scheduler crate + daemon + user service install, `/cron` + `cron_*` tools + skill + settings namespace, desktop Automations page (`docs/dev/v1/design/shots/desktop/66-automations.png`); verified by `scripts/round3/cron-e2e.sh` with the faux provider (every-minute job: 3 runs, sessions written; `at:` job; pause/resume; missed-run catch-up after the daemon stopped; install/uninstall in a scratch HOME) plus one real `oino/deepseek/deepseek-flash` run ([docs/cron.md](../../../docs/cron.md)) |
| input-shortcuts | ✅ | TUI and desktop send editor intents; no composer-history push (Q2) |
| fusion | ✅ | sidekick handoff verified (e2e + live); startup model not persisted to core settings (R4) |
| watchdog | 🟡 | stuck detection + session caps + kill / background via `bash-detach` ✅; process stats Linux-only |
| app-bridge | ✅ | control-centre panel; `_oino/service_call` exists, no native GPUI consumer yet |

## 5. Parity matrix — ZCode design (design.md §8, desktop + mobile)

Source of truth: `apps/oino-desktop/DESIGN-STATUS.md` (54-row checklist, perf numbers),
`apps/oino-mobile/DESIGN-STATUS.md`, [design/review-desktop.md](design/review-desktop.md)
(review FAIL → Resolution: all P0 fixed except perf, P1 fixed except keyboard leftovers and shots).
Desktop: 44 ✅ · 8 🟡 · 1 ⛔ (#50 ZCode product features) · 1 n/a (#51 mobile).

| Group (design.md §8 rows) | Desktop | Mobile | Gaps |
|---|---|---|---|
| Theme, type, spacing, icons (1–5) | ✅ | ✅ | unipi warm OLED dark + derived light (A1) |
| Motion (6) | 🟡 | 🟡 | exit animations, run-label shimmer |
| Frames, sidebar, header, welcome (7–13) | ✅ | ✅ (M1/M2 app bars, sections, swipe rows) | tablet layout on mobile ❌ |
| Timeline, tool rows, diffs, bash, thinking (14–22, 26) | ✅ | ✅ (touch rows) | "Thought for Ns" timing |
| Subagent rows (23) | 🟡 | 🟡 | core `_oino/agents` feed ✅ (core-gui-gaps); GUI wiring open |
| Turn undo / rewind (24–25) | 🟡 | 🟡 | core file checkpoints + `_oino/rewind` ✅ (core-gui-gaps); GUI buttons open |
| Composer, model menu, context ring, mentions, history (27–30, 32, 34) | ✅ | ✅ | model-menu submenus / pinned group |
| Attachments (31) | 🟡 | 🟡 | pasted images not converted |
| Queue (33) | 🟡 | 🟡 | core `_oino/queue_edit` (remove / send-now / move / edit) ✅; GUI wiring open |
| Permission, guard, elicitation, plan, waiting badge (35–39) | ✅ | ✅ (docked + sheet promotion) | — |
| Status panel (40) | 🟡 | 🟡 | core `_oino/topics` feed (+ `oino:git`) ✅; StatusPanel wiring and keys open |
| Side pane, browser, PTY terminal (41–43) | ✅ | ✅ browser (Custom Tabs / SFSafariViewController); terminal ❌ on mobile | — |
| Command center, modifiers, settings + hub, toasts, split panes, board (44–49) | ✅ | ✅ (search, stacked settings + hub, lane pager) | palette history chips |
| Performance (52) | 🟡 | 🟡 | desktop 4-pane run p99 9.3–14.9 ms vs 10 ms; mobile 120 Hz not measured |
| i18n, window chrome (53–54) | ✅ | ✅ (safe areas, system bars) | Windows / macOS rendering not checked on the build hosts |
| Remote attach (mobile M10, D16) | — | 🟡 | hosts list + pairing stored; needs `oino serve` |
| unipi surfaces (design.md §7) | ✅ | ✅ | all render through OUS / docked cards / native board; `_oino/*` wired |


## 5b. Browser control ([browser-control.md](browser-control.md), work id `browser-impl`)

| WP | Status | Notes |
|---|---|---|
| WP1 contracts | ✅ | `crates/oino-browser-protocol` (commands, results, status, view events, input, frontend requests); `oino-extension-api::browser::BrowserHost`, `HostServices::browser_execute/status`, manifest permission `browser`; WIT `browser` (`execute`, `status`). `ClientEvent::Browser` / `_oino/browser_*` RPC ❌ (frontends use `BrowserHost` in-process) |
| WP2 `oino-browser` | ✅ | in-house CDP client (pipe on Unix, loopback ws on Windows/`transport: port`), engine detection (CloakBrowser cache → Chrome/Chromium/Edge/Brave → Playwright cache), Xvfb hidden display (`-terminate`), `--no-sandbox` only where AppArmor blocks userns for home-dir builds, process-group kill, AX snapshot (shadow DOM, same-origin frames, OOPIF) with stable refs, actionability + `obscured`, trusted input, dialogs, popups, downloads, logs, screenshot ladder, isolated-world evaluate, waits, reuse-tab navigate, per-session isolation, take-over gate, handoff, residency LRU, idle reaper, default-deny site permissions, screencast views + viewer input. Real-Chromium test `crates/oino-browser/tests/real_browser.rs` (`OINO_TEST_BROWSER=...`) ✅ with Playwright Chromium 1248. CloakBrowser install/download, cookie import, external CDP endpoint, `browserScope: own` contexts, relaunch on display change ❌ |
| WP3 host wiring | ✅ | wasm-host import + `LocalServices::set_browser`, nested agents resolve to the root session; `oino` CLI installs one service per process (`OINO_BROWSER=off\|ephemeral\|headless\|window`; `--browser` flag and `oino browser …` ❌) |
| WP4 `oino.browser` | ✅ | `extensions/oino/browser`: 19 tools (token budget test), approval dialog (once / site / deny), prompt section, turn-tail screenshot entry + renderer, `/browser`, `skill:browser` (replaces unipi's agent-browser skill). Ambient context ❌ (DEVIATIONS.md) |
| WP5 desktop | 🟡 | screencast Browser tab with agent/user tabs, take-over strip, dialog + handoff cards, mouse/wheel/key/clipboard, cursor probe; reveal opens the side tab; fallback web view. IME, file drag-drop, site-permissions popover, Fit-to-pane, Settings → Browser section ❌ (settings via `settings.json` `browser.*`) |
| WP6 mobile / WP7 TUI / WP8 integration | ❌ | |

Verified end to end: `oino -p` with `oino/deepseek/deepseek-flash` opened a local fixture page, clicked twice, read `clicked 2`; Oino Desktop (Xvfb + lavapipe) with Kimi K2.6: the agent's tab appeared in the side pane, frames showed `clicked 1`, user clicks/typing in the screencast reached the page; no browser or Xvfb process left after the app was killed.

## 6. Known issues

1. **Bash detach** is wired (core-gui-gaps); needs the WASM host's task service
   (`LocalServices`), so hosts without it report `unsupported`.
2. **Shutdown**: RPC, TUI and print / JSON exit once idle; ACP does not (client lifecycle).
3. **Nested agents**: `UiPolicy` is fixed at spawn (no `set-ui-policy`), child depth is not on
   `call-context`, `oino_agent` events reach only the spawning extension.
4. **Remote**: `oino serve` / `--remote` do not exist, so mobile remote attach and a remote TUI
   are unavailable (mobile embeds the core instead).
5. **OAuth** sign-in is verified only with the faux provider and recorded fixtures.
6. **First start** after an update compiles ~24 MB of WASM (≈ 4 s on 32 cores, longer on
   laptops); later starts use `~/.oino/cache/wasmtime`.
7. `models.classify` / `generate-image` unsupported; TUI inline images are placeholders.
8. `/tmp` quota on the dev machine: run builds/tests with `TMPDIR=$PWD/target/tmp`.
9. **iOS devices** cannot JIT: components need wasmtime Pulley (not wired); iOS is simulator-only.
10. **macOS EMFILE**: `oino` does not raise its open-file limit; under the 256-descriptor soft
    limit of SSH / `nohup` shells every bundled component fails to init (`os error 24`). Works
    with `ulimit -n 4096` (§7).
11. **Desktop perf**: four streaming panes do not reliably meet the 10 ms p99 budget.
12. **Release artifacts** in §7 are 1.0.0-alpha.1 (`52d3120`); alpha.2 has not been rebuilt on the
    build hosts. Nothing is signed, tagged or published.
13. **Desktop GUI QA**: the round-3 defects QA-1…QA-7 and GA-45 are fixed and live-verified
    ([gui-qa.md](gui-qa.md) Resolution; commits `d94a75f`…`1e1242c`); the 30-min faux soak passes
    (2.80 MB/min, [stability.md](stability.md)). GA-12 (unipi splash UTC-hour greeting) stays
    open until `host-info` grows the R3 local-offset field.
14. **`--session-id`** only opens an existing session; pi creates one that does not exist
    (modes.md deviation #13, core `SessionSource` has no explicit id). Cron works around it by
    passing `--session <agent>/sessions/--<cwd>--/<ts>_<run id>.jsonl` for every run, so the run
    id is the file-name id, not the header id.

### 6.1 gui-audit area E (core-gui-gaps)

| Id | Status | Core / backend side ([frontend-api.md](frontend-api.md)) | GUI side still open |
|---|---|---|---|
| GA-40 | ✅ | `InProcessBackend` runs commands on its tokio runtime when awaited elsewhere; session background work uses the stored `Handle` | C1: drop the `Tokio::spawn` workaround if wanted |
| GA-41 | 🟡 | host finds bare programs next to the running executable | K3: packaging must ship `oino-kanboard` beside `oino-desktop` (apps/) |
| GA-42 | ✅ | handler deadlines pause while a dialog is open; dead handlers' dialogs are withdrawn | C2: drop cards on `extension_ui_dismiss` |
| GA-43 | ✅ | core fires retained `setShortcuts` | S3 / GA-16: bind keys, palette, settings page |
| GA-44 | ✅ | `_oino/agents`, `agents_changed`, `agent_event`, `_oino/agent_messages` | P3 / GA-15: Agent tab timeline |
| GA-45 | ✅ | `ExtensionMode::Desktop` / `Mobile`; `host-info.frontends` from mode + `_oino/hello` | none — `embedded.rs` passes `Desktop` and registers `oino-desktop` ([gui-qa-fix] `1e1242c`, verified via `/doctor`) |
| GA-46 | ❌ | — | K6: idle cost (GPUI refresh loop, thread pools) |

## 7. Release builds (1.0.0-alpha.1 artifacts, 2026-10-27, `v1-alpha` @ 52d3120, not tagged or published)

The builds are run with `cargo xtask remote-build <host|local> [--jobs n] [--timeout-mins n] [-- <build-host.sh args>]`
(`xtask/src/remote.rs`). The tool works in three steps:

1. **Sync.** It sends `HEAD` to the host as a `git bundle` (incremental when the host already
   has an ancestor commit) and checks it out detached in `~/oino-release`. It also sends a
   tarball of the local `extensions/dist`, because the build hosts have no wasm toolchain.
2. **Build.** It runs `scripts/release/build-host.sh` on the host, detached (`nohup` on Unix,
   WMI `Win32_Process.Create` → Git Bash on Windows), so a dropped SSH session cannot kill it.
   It polls once a minute, bounded by `--timeout-mins`, and kills the job when that is reached.
3. **Fetch.** It copies the artifacts into `target/release-artifacts/<target>/` and the build
   log into `target/release-artifacts/logs/<host>.log`.

`build-host.sh` checks `cargo xtask checksums`, then builds the `oino` / `oino-kanboard`
release binaries and runs `apps/oino-desktop/scripts/build.sh --release --no-wasm`. With
`--android` it also builds the mobile APK. It then runs smoke tests and writes `smoke.txt`,
`SHA256SUMS` and `build-info.txt` into each target directory.

The commands that produced these artifacts:

- `cargo xtask remote-build local -- --android --serial <device>`
- `cargo xtask remote-build coffee -- --formats nsis`
- `cargo xtask remote-build mac -- --universal --formats app,dmg`

| Target | Artifact | Size |
|---|---|---|
| x86_64-unknown-linux-gnu (local) | `oino` | 75.0 MiB |
| | `oino-kanboard` | 5.6 MiB |
| | `oino-desktop` | 127.1 MiB |
| | `oino-desktop-1.0.0-alpha.1-linux-x86_64.tar.gz` | 72.4 MiB |
| | `oino-desktop_1.0.0-alpha.1_amd64.deb` | 44.5 MiB |
| | `oino-desktop_1.0.0-alpha.1_x86_64.AppImage` | 112.1 MiB |
| x86_64-pc-windows-msvc (coffee) | `oino.exe` | 59.1 MiB |
| | `oino-kanboard.exe` | 4.2 MiB |
| | `oino-desktop.exe` | 82.5 MiB |
| | `oino-desktop_1.0.0-alpha.1_x64-setup.exe` (NSIS, per-user) | 26.6 MiB |
| universal-apple-darwin (mac, Intel host, aarch64 cross-compiled + `lipo`) | `oino` | 129.7 MiB |
| | `oino-kanboard` | 9.4 MiB |
| | `oino-desktop` | 176.1 MiB |
| | `Oino.app` (ad-hoc signed, not notarized) | 177 MiB |
| | `Oino_1.0.0-alpha.1_universal.dmg` | 77.1 MiB |
| x86_64-apple-darwin (mac) | `oino` | 66.9 MiB |
| | `oino-kanboard` | 4.8 MiB |
| aarch64-linux-android (local) | `oino-mobile-1.0.0-alpha.1-release.apk` (signed with the debug key) | 39.4 MiB |

The smoke test results are in `target/release-artifacts/<target>/smoke.txt`.

| Target | `--version` (oino, kanboard, desktop) | faux print run¹ | Desktop `--demo` | Packages |
|---|---|---|---|---|
| Linux | ✅ ✅ ✅ | ✅ | ✅ Xvfb + lavapipe screenshot (welcome screen renders) | ✅ AppImage `--version`; ✅ deb metadata |
| Windows | ✅ ✅ ✅ | ✅ | 🟡 alive after 20 s in the console session²; screenshot shows the lock screen | ✅ NSIS silent install → installed `--version` → silent uninstall |
| macOS | ✅ ✅ ✅ (universal and x86_64; `lipo`: x86_64 arm64) | ✅ with `ulimit -n 4096` (❌ at 256, known issue 10) | 🟡 alive after 20 s; no screenshot (no Screen Recording grant over SSH; display asleep) | ✅ `.app` binary `--version`; ✅ `hdiutil verify`; ✅ `codesign -dv` ad-hoc universal |
| Android | — | — | ✅ installed and launched on a 25028RN03A (Android 15); `android-launch.png` shows Sessions | ✅ `unzip -t` |

¹ The print run uses `-e extensions/dist/oino.fixture.provider-faux --model faux/faux-1 -p hi`
with a temporary agent dir and the full bundle (33 packages unpacked), and expects
`Hello from faux.`.

² `scripts/release/desktop-shot.ps1` starts the app through a one-time scheduled task in the
interactive session.

Problems found:

1. **macOS EMFILE** (known issue 10). This is a product bug, not a build problem.
2. **create-dmg.** cargo-packager's `create-dmg` controls Finder through AppleScript, which
   times out over SSH (-1712). `build-host.sh` falls back to `hdiutil create -format UDZO`;
   the resulting `.dmg` has no window layout or background.
3. **Universal naming.** cargo-packager names the `.dmg` `_x64` even for universal builds, so
   the fallback names it `_universal`.
4. **Toolchains.** No build host has a wasm toolchain, so `extensions/dist` is shipped from
   the local machine. The mac's default toolchain is 1.97.1, but rust-toolchain.toml selects
   1.99.0.
5. **Signing.** None. Windows and macOS installers will trigger SmartScreen and Gatekeeper
   warnings. The Android release APK is signed with the debug key (`build.gradle`).
6. **Windows screenshots.** The screenshot needs an unlocked console session on coffee. A
   rendering check needs either an unlocked session or an in-app self-capture hook.

## 8. Next steps

1. **core-runtime**: `agents.agent.set-ui-policy` + depth on `call-context`; `tools.override`
   render-only registration and guest `render` for built-in tools (utility render styles);
   `summarize-default` with a previous summary. (Bash detach, file checkpoints, single-item
   queue commands, `disabledExtensions`, topic / agent feeds landed in core-gui-gaps.)
2. **modes**: `oino serve` + `--remote` (TCP+TLS, pairing) with `_oino/list_sessions`,
   `_oino/get_keybindings`, `_oino/render`; TUI core settings through `_oino/set_setting`;
   ACP exit on `shutdown_requested`.
3. **release**: raise `RLIMIT_NOFILE` at `oino` startup (macOS); rebuild alpha.2 with
   `cargo xtask remote-build {local,coffee,mac}`; iOS simulator build; signing / notarisation;
   tag so CI assembles the GitHub release.
4. **wit-v1 / host**: wasmtime Pulley for iOS; `mcp.json` in the agent preopen; TLS
   fingerprinting; process stats on macOS / Windows; `host-info` UTC offset; task notification
   `deliverAs: followUp`.
5. **gpui**: desktop perf (cheaper per-pane layout), remaining review P2 items, StatusPanel /
   Queue keys; mobile tablet layout and perf measurement; move `EmbeddedHost` into a shared crate.
6. **acp**: publish `mcpServers` on `oino:acp:mcp-servers`; refusal stop reason; tool kinds;
   try Zed by hand.
7. **integration**: `cargo xtask layering`, `cargo deny`; golden e2e suite under `tests/e2e/`
   with `PARITY.md` (quality §7); criterion benches (quality §5); `oino-protocol` contract crate
   (architecture appendix N1); live OAuth checks.
