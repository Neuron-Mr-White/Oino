# Oino v1.0.0-alpha.3 — status

Date: 2026-10-29 · branch `v1-alpha` · round 3 + `harness-gaps` (`round3-integration`: gui-qa + gui-qa-fix,
stability, browser-impl, unipi-native + commands, cron, mempalace-rs, cross-platform smoke;
on top of round 2 `integration-2`: core-agents, core-settings, frontend-cmds, providers-more,
unipi-utility, design-spec / design-desktop(-fix) / design-mobile, release-builds).
Parity targets: `@earendil-works/pi-coding-agent` **1.4.0**, unipi **3.0.0**, ZCode design
([design.md](design.md)).
Legend: ✅ done · 🟡 partial · ❌ missing · ⛔ diverges by design. Settled deviations are folded
into the appendix of each spec; open items per work item stay in [deviations/](deviations/).

## 1. Gates (this commit)

All run locally with `TMPDIR=$PWD/target/tmp`, `nice -n 10`, `-j 4`; components rebuilt with
rustc 1.99.1. Script: `scripts/round3/gates.sh` (log `target/gate-round3.log`).

| Workspace | fmt | clippy `-D warnings` (pedantic) | tests | other |
|---|---|---|---|---|
| root (`crates/*`, `xtask`) | ✅ | ✅ | ✅ **826 passed** (`cargo test --workspace`, incl. `crates/oino/tests/bundled.rs`, wasm-host e2e with the real unipi agents / settings components; +10 by `harness-gaps`) | ✅ rustdoc `-D warnings` (`RUSTDOCFLAGS`); ✅ `cargo xtask wit`; ✅ `scripts/docs-check.sh`; ✅ `scripts/install-smoke.sh` (source install + faux prompt); all crates except `oino-wasm-host` / `xtask` on `[lints] workspace = true` |
| `extensions/` | ✅ | ✅ (`--target wasm32-wasip2`) | ✅ **679 native** (4 ignored); ✅ 7 wasm-host harness suites, **36 passed** (`extensions/unipi/*/harness/run.sh`: ask-user 5, core 3, kanboard 3, notify 5, subagents 5, utility 9, workflow 6) | ✅ `cargo xtask wasm` (39 components: 8 providers + `oino.browser`, 24 unipi incl. `unipi.cron`, 5 fixtures) + `cargo xtask checksums` |
| `apps/` (GPUI) | ✅ | ✅ | ✅ **120 passed** | — |

Red gates found and fixed this round: the doc gate (`cargo doc … -- -D warnings` no longer
passes trailing args on cargo 1.99 → `RUSTDOCFLAGS`), three rustdoc links/HTML tags in
`oino-cron`, and the notify harness's updater command assertion (the updater's hidden
`changelog` runner added by `7525d3d` was missing). `cargo xtask layering` and `cargo deny`
(quality §1, §6) still do not exist.

## 1b. Round 3 summary (work id `round3-integration`, 2026-10-28, `v1-alpha`)

Round 3 closed the desktop QA defects, the stability/soak work, browser control, the command
rename + `unipi-migrate` skill, cron (Oino-native), the mempalace-rs integration and the
cross-platform smoke. Parity targets unchanged: pi 1.4.0, unipi 3.0.0, ZCode design.
Version: **1.0.0-alpha.3**.

### 1b.1 unipi 3.0.0 in the desktop app (incl. cron)

Every unipi surface (design.md §7) reaches exactly one primary rendering; the desktop now
renders each one natively or through the shared OUS/docked-card machinery. "Live" = exercised
in the round-3 / round-4 QA runs on the release binary (private Xvfb + lavapipe) or by the
round-3 harness scripts; "code" = unit/e2e tests only this round.

| Package | Desktop surface | Status | Evidence |
|---|---|---|---|
| core | hint strip (`alt+h`), hints overlay, welcome hero / slim header card, work tray (`↓`, `/subagents`) | ✅ live | GA-5, GA-16, GA-21, GA-22, GA-31 |
| info-screen | `/info` dashboard dialog (tabs, stat tiles, meters, `1–9` pages, `r` refresh) | ✅ live | GA-23 |
| footer | glance frame under the composer + stats strip (`/footer off` keeps the mode chip) | ✅ live | GA-25, GA-26 |
| workflow | approval card (keyboard + focus, >4 min alive), plan review card, `/permission`, `alt+m`/`alt+p`, Permissions page | ✅ live | GA-16, GA-27, GA-30, GA-33, QA-1 |
| ask-user | question card, tool rows, OS notification | 🟡 code | forms render (GA-7); a live `form` turn was not re-driven (budget) |
| long-horizon | `/goal` continuation, `/ralph` `/swarm` `/graph`, visualise dialog, StatusPanel goal row | 🟡 code | `/goal-start` e2e + smoke; graph runs not e2e-tested |
| memory | recall/save cards, badge rows, forget/consolidate dialogs, `memory_*` tools | 🟡 code | MemPalace verified with both implementations ([§1b.5](#1b5-mempalace-rs-the-rust-port)); `/memory` in the desktop not driven this round |
| dream | report card with proposals, tray tab, StatusPanel Run row | 🟡 code | nested-agent dream e2e with resume; Accept/Reject buttons not clicked live |
| skill-registry | `/skills` → Settings hub "Skills" page, vault discovery | ✅ live | GA-31 (`/hint`), skills page |
| subagents | `run_subagent` rows, `/subagents` tray tab, `/agents` dialog | 🟡 code | GA-21 tray; live child timeline still open (GA-15 / GA-44 feed exists) |
| background-tasks | `/bg`, tasks tray tab (`sleep 30 · 4s · 0B`), Stop all / Dismiss, completion card | ✅ live | GA-14, GA-21, GA-22; notification `deliverAs: followUp` ([harness-gaps]) |
| btw | `/btw` side-chat panel, session message count unchanged | ✅ live | smoke (`/btw …` in the panel) |
| web-api | `web_search` / read tool rows, confirm dialogs | ✅ live | tool rows in the round-2/3 runs |
| notify | wizards (`form`/`select`), secret fields, toasts, OS notifications | ✅ live | kanboard-onboard dialog (GA-24), notify settings |
| kanboard | native Board page, `/kanboard open` side pane, New-task dialog, claims badge | ✅ live | GA-17, GA-20, QA-5; `65-unipi-board-newtask.png` |
| mcp | `/mcp-status` panel, `<server>__<tool>` rows, add/settings flows | ✅ code | stdio + HTTP e2e; pi `~/.oino/mcp.json` via `/agent`, ACP `mcpServers` on `oino:acp:mcp-servers` ([harness-gaps]) |
| command-enchantment | completion menu with package colour tags, one COMMANDS group | ✅ live | GA-2, GA-4 |
| compactor | `/compact*`, `/session-recall`, SummaryCard | ✅ code | commands + tests; LLM summary takes the previous summary like pi ([harness-gaps]) |
| updater | `/readme`, `/changelog` (GitHub release notes fallback), update badge | ✅ live | GA-19, QA-4 |
| **cron** (Oino-native) | **Automations page** (sidebar nav, job rows with each job's own last run, run history, add/edit/enable/disable/run/pause) | ✅ live | `66-automations.png`; `scripts/round3/cron-e2e.sh --real` → PASS (every-minute job 3 runs + sessions, `at:` job, pause/resume, missed-run catch-up, install/uninstall in a scratch HOME, one real `oino/deepseek/deepseek-flash` run) |
| input-shortcuts | composer editor intents, `setShortcuts` → shell interceptor | ✅ live | GA-16, QA-1 repro 4 |
| fusion | `/fusion` picker, sidekick AgentRow, wake widget | ✅ code | sidekick e2e + live handoff; startup pair persisted to core settings ([harness-gaps]) |
| watchdog | stuck detection, session caps, `/bg-detach`, `setStatus` chip | ✅ code | `bash-detach` e2e; process stats on Linux/macOS/Windows ([harness-gaps]) |
| app-bridge | `/app` control-centre side tab | ✅ live | GA-18 |

Count: 12 ✅ live, 11 🟡 code, 0 ❌. "🟡 code" means gates green and the surface renders, but
the round-3 QA budget did not re-drive that turn live; the gaps are itemised in
[gui-qa.md](gui-qa.md) "Coverage gaps".

### 1b.1b Harness gaps closed (work id `harness-gaps`, 2026-10-29)

Ten request rows from the unipi matrices that needed a core/WIT or component change, closed in
this round (`[harness-gaps]` commits):

| # | Gap | What landed | Tests |
|---|---|---|---|
| 1 | mcp `mcp.json` + ACP `mcpServers` (unipi-integrations R1/R3) | `/agent/mcp.json` read-only preopen (`state/agent-view`, refreshed copies of `mcp.json` only, never `auth.json`/`secrets.json`; a watcher follows edits, cap-std refuses symlinks that escape a preopen); `unipi.mcp` merges the pi global file, then `/ext-global/mcp.json` (same-name entry wins), then the trusted project file. ACP publishes `session/new`/`session/load` `mcpServers` as sticky `oino:acp:mcp-servers` (`{servers}`, publisher `oino.acp`); the "ignored" notice only fires without `unipi.mcp` | `paths::tests::agent_view_exposes_config_files_only`, `config::tests::pi_file_is_merged_with_the_unipi_user_file` (29 mcp tests), `acp::session_new_publishes_mcp_servers` |
| 2 | compactor previous summary (unipi-context R1) | `models.summarize-default(messages, instructions, previous-summary)` throughout (WIT → ops → core → SDK); `unipi.compactor` passes `preparation.previousSummary` / the last compaction summary instead of an instruction | `compaction_llm::summarize_default_updates_a_previous_summary` |
| 3 | fusion startup model (unipi-agents R4) | `control.set-default-model(provider, id, thinking)` writes `defaultProvider`/`defaultModel`/`modelThinkingLevels` without switching the session; `fusion` calls it on "Confirm and use at startup" | `session_flow::set_default_model_persists_core_settings` |
| 4 | task notification delivery (unipi-agents R1) | host `task_finished` sends `<background-task-notification>` with `deliverAs: followUp` | `probes::services_bus_and_tasks_between_instances` |
| 5 | utility render styles (unipi-utility U1–U2) | `tools.override` is enforced for render specs of built-in tools (init + dynamic, diagnostic when dropped); `ExtensionInstance::render` + `AgentSession::render_spec` + `_oino/render` (`RenderRow`/`RenderRequest` with `density`/`width`); the TUI offers every tool row (`LocalHost::with_render`) | `session_flow::builtin_tool_render_overrides` |
| 6 | watchdog process stats (unipi-agents) | `proc-stats`/`process-tree` on macOS (`ps`) and Windows (`Get-CimInstance`), `taskkill /T` for kills; pure parsers unit-tested on Linux | `process::tests::{ps_output_parsing,cim_output_parsing}` |
| 7 | `host-info` UTC offset (unipi-core R3) | `info.local-offset-minutes`; SDK `host::local_hour`; late-night hint, info-screen splash greeting and notify test-notification time are local | core/notify guest tests, `unipi.core`/`info-screen` suites |
| 8 | `oino:settings:<ns>` topic | every settings write (extension or frontend) publishes the sticky `oino:settings:<namespace>` topic (`{namespace, keys, scope, extensionId?}`) | `extension_settings::registry_serves_guests_and_frontends` |
| 9 | `--session-id` (known issue 14) | `SessionSource::NewWithId`; an unknown id creates the session with that header id; `oino-cron` runs with `--session-id <run id>` | `print::session_id_creates_the_session_like_pi`, `oino-cron` 64 tests |
| 10 | ask-user forms over ACP (acp #11) | `form` → `elicitation/create` with one object schema per question; `FormAnswers` rebuilt (`{answers:[{id, selected}], action?}`); clients without elicitation still get `cancelled` | `ui::tests::form_schema_and_answers` |

Not closed: the GPUI desktop still renders tool rows natively (`apps/` is owned by another
work id) and `custom`/clipboard dialogs stay cancelled over ACP.

### 1b.2 Stability / soak ([stability.md](stability.md))

Seven root causes fixed (GPUI background-executor panic on editor events, extension install
on the foreground executor, panes never dropped → RSS/fd/thread growth, no shutdown at quit,
file-tree walk on the render thread, unbounded `ous::Drains`, soak-driver double lease).
Diagnostics: panic hook writes `~/.oino/logs/desktop-crash-*.log` and aborts after logging on
main/worker threads; a watchdog writes `desktop-stall-*.log` after 2 s of UI silence.

30-minute faux soaks on the release binary, one instance, own Xvfb :120 + lavapipe:

| Run | Steps | RSS start→end (max) MB | Slope 2nd half | Threads | fds | Frame max/p99 ms | Verdict |
|---|---|---|---|---|---|---|---|
| before fixes (`--max-panes 1000`) | ≈2560 | 5→1731 | **19.9 MB/min** | 276→333 | **500→842** | 195/18.7 | FAIL |
| after fixes, run 2 | 2564 | 4→1030 (1072) | −3.4 MB/min | 225→226 | 405→396 | 112/34.4 | PASS |
| demo backend | 2552 | 6→316 (360) | −1.8 MB/min | 184→182 | 73→61 | 92/15.9 | PASS |
| post gui-qa-fix round 4 (final) | 2566 | 10→1120 (1120) | **2.80 MB/min** | 230→232 | 424→416 | 64.0/43.6 | **PASS** |

Limits: RSS slope < 8 MB/min, thread growth < 16, fd growth < 64, 4 GB abort, 0 crash/stall
logs — all met. The ~1.1 GB faux plateau is the wasmtime `Services` footprint of ~30 bundled
components × 8 live panes, flat once the pane cap is reached. Soak script:
`apps/oino-desktop/scripts/soak.sh --minutes 30 --backend demo|faux [--out DIR]`.

### 1b.3 Browser control ([browser-control.md](browser-control.md))

| WP | Status | What landed |
|---|---|---|
| WP1 contracts | ✅ | `crates/oino-browser-protocol`, `BrowserHost` + manifest permission, WIT `browser` |
| WP2 `oino-browser` | ✅ | in-house CDP client (unix pipe / loopback ws), engine detection (CloakBrowser → Chrome/Chromium/Edge/Brave → Playwright), hidden Xvfb display, AX snapshot with stable refs, trusted input, dialogs/popups/downloads, screenshot ladder, screencast, residency LRU, default-deny site permissions; real-Chromium test `crates/oino-browser/tests/real_browser.rs` |
| WP3 host wiring | ✅ | wasm-host import, `LocalServices::set_browser`, nested-agent scope walk, `OINO_BROWSER=off\|ephemeral\|headless\|window` |
| WP4 `oino.browser` | ✅ | 19 `browser_*` tools, approval dialog, prompt section, turn-tail screenshot, `/browser`, `skill:browser` (replaces unipi's agent-browser skill) |
| WP5 desktop | 🟡 | side-pane screencast tab with agent/user tabs, take-over strip, dialog + handoff cards, mouse/wheel/key/clipboard; IME, file drag-drop, site-permissions popover, Fit-to-pane and a Settings page are open |
| WP6 mobile / WP7 TUI / WP8 integration | ❌ | next rounds |

Verified end to end: `oino -p` with `oino/deepseek/deepseek-flash` opened a local fixture page,
clicked twice, read `clicked 2`; the desktop app (Xvfb + lavapipe) showed the agent's tab,
frames `clicked 1`, user clicks/typing reached the page; no browser or Xvfb process left
afterwards. QA-2 fix: launch is bounded (10 s handshake, 90 s launch) and a wedged engine
returns `backend_unavailable{launch_failed}`.

### 1b.4 Commands: rename + `unipi-migrate` ([commands.md](commands.md))

Feature commands carry no `unipi:` prefix and no aliases (40 names, registry test asserts no
collision with built-ins). `/unipi:settings` and `/unipi:changelog` merged into the core
`/settings` and `/changelog`; `/unipi:model` renamed `/fusion`; hidden runners keep plain names
(`goal-start`, `mcp-sync`, …). The bundled opt-in skill **`unipi-migrate`**
(`extensions/unipi/core/skills/unipi-migrate/`, user doc
[docs/migrate-from-unipi.md](../../migrate-from-unipi.md)) rewrites `/unipi:*` in prompts,
skills, subagent profiles, ralph files and `AGENTS.md`/`CLAUDE.md` from
`references/commands.tsv` + `references/state-map.md`; session history is not rewritten.
Fixture check: `scripts/unipi-migrate-fixture.sh`.

### 1b.5 mempalace-rs, the Rust port

Upstream ([mempalace-rs](file:///home/oi/Projects/Personal/llm/mempalace-rs) branch
`oino-parity` @ `d18684d`, `docs/PARITY-REPORT.md` + `docs/SWITCH-GUIDE.md`): **99.7% parity**
(1,194 of 1,198 compared items) on palace copies — CLI help 63/63, CLI commands 152/152 equal,
wake-up 89/89, MCP read tools 220/220, writes + knowledge graph 34/34, MCP protocol 61/61,
search 297/300 identical ranking (Python vs Python: 296/300), mining 100%, cross-compat 100%,
daemon HTTP 15/15, oino/pi caller contract 28/28. Both implementations share the palace,
`~/.mempalace/daemon/<key>/endpoint.json` and the MCP tool surface.

Perf (same palace, copies): CLI search cold 1.04 s/332 MB → 0.21 s/222 MB; wake-up 0.79 → 0.03 s;
status 0.71 → 0.06 s; warm MCP reader startup 528 → 2 ms, search p50 58 → 40 ms, CPU per 100
searches 101 → 11.5 s, 152 → 4 threads; 2 h soak: daemon RSS 344→2,048 MB (Python) vs
278→331 MB (Rust), slope +238/+49 MB/h vs −38/−9 MB/h, daemon CPU 6,550 s vs 1,579 s.

Oino integration (this round, `96137f5`): `unipi.memory` has a **`mempalaceBin`** setting
(binary **or** folder; empty = `PATH`). The extension probes the candidates with `--version`
(`MemPalace 3.11.0 (rs …)` ⇒ Rust), expands `~/` through the granted `~/.mempalace` preopen,
takes `mempalace-mcp` from the same folder and **falls back to `PATH`** (Python) when the
configured path is stale; the daily `uv`/PyPI auto-update is skipped for the Rust port and for
an explicit path. `/memory` prints the resolved binary. Verified by
`scripts/round3/memory-mempalace-e2e.sh` — four cases on palace copies (Rust on `PATH`,
Python on `PATH`, `mempalaceBin: "~/mp-bin"`, stale path → Python fallback): backend line
`(rust)`/`(python)`, `memory_store` filed through the daemon job, `memory_search` found the
note through the warm reader, `memory_delete` filed, drawer gone from the copy, live
`mempalace.service` MainPID unchanged → **RESULT: PASS**.

Still on Python by design: `audit`/`migrate`/`compress`/`repair`/`serve`/`task`/`artifact`
and friends, `mempalace-light-mcp`, the 8 coordination tools, BM25 fallback on a diverged
index and the stop/session-end/precompact hook saves. The switch itself is the owner's action
(`scripts/switch/switch-to-rs.sh --apply` in mempalace-rs; rollback
`rollback-to-py.sh --apply`); turn the oino toggle *MemPalace auto-update* off first
(Settings → unipi.memory) and restart oino/pi afterwards so the warm readers respawn.

### 1b.6 Cross-platform smoke (one host at a time, `-j 4`, `cargo xtask remote-build`)

| Host | Build | `oino --version` | faux print | `oino cron status` | Desktop `--demo` 20 s |
|---|---|---|---|---|---|
| `ti` (Ubuntu) | — | — | — | — | **skipped**: SSH printed "Tailscale SSH requires an additional check" (owner must approve); the Linux desktop demo ran locally on a private Xvfb instead |
| `coffee` (Windows, 16 threads, 380 GB free) | ✅ 21 min, commit `5d5d06f` (CLI + NSIS desktop); artifacts fetched | ✅ `1.0.0-alpha.3` (also kanboard, desktop) | ✅ `Hello from faux.` (35 bundled packages) | ✅ "scheduler not running … not paused concurrency 2" | 🟡 alive after 20 s in the console session through a scheduled task; `desktop-demo.png` is the locked console (no visual check) |
| `desmondgiam` (macOS 15.6.1, 8 threads, 13 GB free after cleanup) | ✅ 39 min, commit `f934b81` (first attempt aborted in `build.sh` — bash 3.2 empty-array bug, fixed in `f934b81`), .app + hdiutil/cargo-packager `.dmg`; artifacts fetched | ✅ `1.0.0-alpha.3` (also kanboard, desktop, `.app` binary) | ✅ `Hello from faux.` (35 bundled packages) | ✅ "scheduler not running … not paused concurrency 2" | ✅ alive after 20 s **and** a rendered screenshot (`desktop-demo.png`, 8.9 MiB) |

Both hosts: smoke.txt (in `target/release-artifacts/<target>/` and on the host), SHA256SUMS,
build-info.txt, then the scratch `%TEMP%\oino-*` dirs (coffee) and the packaging `dist/` plus temp
staging (mac) removed; `~/oino-release` is kept for reuse (it grew ~4 GB from the rebuild).
`ti` remains the preferred GUI host once its Tailscale check is approved.

### 1b.7 Open harness gaps (round 3)

1. ~~**Pointer injection**: XTEST does not apply modifier state on the QA Xvfb and the app
   selects no core pointer events, so mouse-only surfaces (board FAB, `+` launcher, settings
   switches, browser take-over, mascot toggle) are keyboard- or config-verified only.~~
   **Fixed 2026-10-29** ([gui-driver.md](gui-driver.md)): the app serves a JSON-lines RPC on a
   Unix socket (`OINO_DESKTOP_AUTOMATION`) and injects pointer/keyboard events through
   `Window::dispatch_event`. Board FAB, `+` launcher, the task detail's move button and the
   settings switches are click-driven in the shipped scenarios on `ti`; the browser take-over
   strip is the same `Btn` path but only renders while an agent drives a tab, so it is not
   exercised by a scenario yet.
2. ~~**Screen reading** is a vision model (OCR) on Xvfb screenshots; colours are pixel probes.
   No accessibility-tree read-back for the GPUI shell.~~ **Fixed 2026-10-29**: `tree` returns
   the semantic element list (id, role, label, state, bounds) from a per-frame registry
   (`apps/oino-ui/src/automation/`), and transcript rows are readable text.
3. **Windows**: screenshots need an unlocked console session on `coffee`; verification is
   "process alive after 20 s" plus the build's own smoke.txt.
4. **macOS**: no Screen Recording grant over SSH and the display may be asleep → "alive after
   20 s", no screenshot; `ulimit -n 4096` is required before smoke runs (known issue 10).
5. **GPU path**: soak/perf frame numbers are lavapipe; the `:0` GPU soak with
   `SOAK_MAX_FRAME_MS` has not been run (owner's workstation rule).
6. **Real providers** are not soaked (faux shares the code path except the HTTP component).
7. **Embedded browser** is not part of the soak: WebKitGTK cannot embed on Xvfb without a
   session bus.
8. **Cron** e2e runs in a scratch HOME; `systemd --user` is usually absent in a headless
   shell, so `install` is verified by the unit file, not by a running service.
9. **`ti`** is unreachable without the owner approving a Tailscale browser check.
10. **Release signing** (Windows/macOS) and CI release assembly are still manual.

### 1b.8 Next round: ZCode parity

ZCode parity ([design.md](design.md) §8, 54 rows) is the next round: 44 ✅ · 8 🟡 · 1 ⛔ ·
1 n/a on desktop (perf row 🟡). Priorities: core subagent/agent feed wiring (GA-15/44),
turn undo/rewind buttons on the core file checkpoints, queue-edit wiring, StatusPanel +
topics feed, attachments from pasted images, 4-pane p99 under the 10 ms budget, mobile tablet
layout, exit animations. `apps/oino-desktop/DESIGN-STATUS.md` is the row-by-row source.

## 2. End-to-end smoke (release `oino` 1.0.0-alpha.2, temp agent dir, bundle of 33 packages)

Re-checked on **1.0.0-alpha.3** (2026-10-28, local release build): `oino --version`,
`oino-kanboard --version`, `oino-desktop --version` → `1.0.0-alpha.3`; faux print run →
`Hello from faux.`; `oino cron status` → "scheduler not running …"; `oino-desktop --demo` on a
private Xvfb :98 + lavapipe stayed up 20 s with a rendered screenshot (110 KB). The table below
is the round-2 record and stays valid.

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
| Settings manager + extension settings registry | ✅ | schema, layers, scoped writes, secret fields, change events; every write also publishes the sticky `oino:settings:<ns>` topic |
| Models: registry, `models.json`, `auth.json`, scopes, virtual models | ✅ | dynamic catalogs refreshed at startup |
| Providers (pi-ai wires) | ✅ | 8 components: chat completions (+Copilot, Cloudflare), Responses (+ChatGPT sign-in), Anthropic, Codex, Azure, Google (Gemini, Vertex), Bedrock, Oino Router; 165 pi oracle fixtures |
| Provider transports / auth edge cases | 🟡 | WebSockets ❌, Anthropic federation ❌, Bedrock SSO/IMDS ❌ (`extensions/providers/README.md` M1–M10) |
| OAuth `/login` `/logout` | 🟡 | flows for Anthropic, Copilot, Codex, ChatGPT implemented and fixture-tested; never run against live accounts |
| CLI flags and help | ✅ | `--session-id` creates the session with that header id like pi (known issue 14 fixed) |
| Print / JSON modes | ✅ | goldens `print`, `json-mode` |
| RPC mode + extension UI sub-protocol | ✅ | golden `rpc`; RPC / TUI / print exit on `ctx.shutdown()` once idle (`shutdown_requested`); ACP leaves it to the client |
| `_oino/*` frontend commands | ✅ | completions, ui_action, editor_event, shortcuts, settings, service_call, auth, topics, bus_get, agents, queue_edit, checkpoints / rewind, running_bash, extensions — in-process (runtime-agnostic) and RPC (`FrontendApi`, [frontend-api.md](frontend-api.md)) |
| Interactive TUI | 🟡 | full chrome, dialogs, panels with buttons, settings hub, completions, editor intents, `/login`, regular mode ✅; inline images ❌; core settings still via `LocalHost` |
| Package manager + `oino update` | ✅ | dir / archive / https / `git+https`; self-update from `release-manifest.json`; `oino auth`, `ext`, `sessions`, `mcp` ❌ |
| Extension API (pi `ExtensionAPI`) via WIT | 🟡 | events, tools, commands, shortcuts, flags, renderers (incl. render-only overrides of built-in tools), providers, virtual models, UI, settings r/w, `models.complete/stream/summarize-default` (with previous summary), `control.*` (incl. `set-default-model`), nested agents, turn arbiter, bash detach, `disabledExtensions`, dialog-aware deadlines ✅; provider hooks routing, `models.classify/generate-image`, `set-ui-policy` ❌ |
| ACP (Zed etc.) | ✅ | protocol v1, permissions, elicitation (forms included), client fs/terminal, `--model`, `mcpServers` forwarded on `oino:acp:mcp-servers`; refusals ❌; Zed not tried by hand |
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
| core (hints, header, work tray) | ✅ | late-night hint, splash greeting and notify times use the host's UTC offset (`host-info.local-offset-minutes`, R3) |
| utility | 🟡 | settings, auto-rename (verified live), `/answer`, attachments, image tools, `/move`, doctor ✅; render styles for built-in tools ✅ (core `tools.override` render-only overrides + `_oino/render`; the TUI asks for every tool row, the GPUI app still renders natively); trust.json move (U5) |
| workflow (permissions, plan mode) | ✅ | mode and rules in the settings files |
| long-horizon | ✅ | goal continuation through the arbiter verified (e2e + smoke); graph runs not e2e-tested |
| memory | ✅ | markdown memories, tools, recall, side-save ✅; MemPalace verified against **both** implementations (Python 3.11.0 and mempalace-rs) on palace copies; `mempalaceBin` selects a binary or folder with a `PATH` (Python) fallback — [§1b.5](#1b5-mempalace-rs-the-rust-port) |
| dream | ✅ | nested-agent dream runs with resume (e2e) |
| skill-registry | ✅ | |
| info-screen | ✅ | dashboard panel + actions verified in the smoke |
| subagents | ✅ | foreground/background/read verified (e2e + live); UI policy fixed at spawn (R2) |
| background-tasks | ✅ | tasks, tray, notifications delivered with `deliverAs: followUp`, adopting a running bash call (`tasks.adopt-bash`) |
| btw | ✅ | verified in the smoke |
| web-api | ✅ | no TLS fingerprinting (`impersonated: false`) |
| ask-user | ✅ | forms map to ACP `elicitation/create` (one object schema → `FormAnswers`); `custom`/clipboard dialogs stay cancelled |
| mcp | ✅ | stdio + HTTP ✅; pi `~/.oino/mcp.json` read through the `/agent` preopen, project `.oino/mcp.json`, ACP `mcpServers` forwarded on `oino:acp:mcp-servers` |
| notify | ✅ | wizards and secrets in settings |
| kanboard | ✅ | CLI/daemon, panel, desktop native board, monitor nudges through the arbiter (smoke); TUI renders the board panel generically; `/kanboard open` opens the side pane at any window width ([gui-qa-fix] QA-5) |
| command-enchantment | ✅ | TUI and desktop merge `complete-input` |
| compactor | ✅ | `llm` method passes the previous summary (`summarize-default(…, previousSummary)`, pi's update prompt) |
| footer | ✅ | |
| updater | ✅ | never installs (by design); `/changelog` falls back to GitHub release notes when no `CHANGELOG.md` exists ([gui-qa-fix] QA-4) |
| cron (Oino-native) | ✅ | scheduler crate + daemon + user service install, `/cron` + `cron_*` tools + skill + settings namespace, desktop Automations page (`docs/dev/v1/design/shots/desktop/66-automations.png`); verified by `scripts/round3/cron-e2e.sh` with the faux provider (every-minute job: 3 runs, sessions written; `at:` job; pause/resume; missed-run catch-up after the daemon stopped; install/uninstall in a scratch HOME) plus one real `oino/deepseek/deepseek-flash` run ([docs/cron.md](../../../docs/cron.md)) |
| input-shortcuts | ✅ | TUI and desktop send editor intents; no composer-history push (Q2) |
| fusion | ✅ | sidekick handoff verified (e2e + live); the startup pair is persisted to core settings (`control.set-default-model`, R4) |
| watchdog | ✅ | stuck detection + session caps + kill / background via `bash-detach` ✅; process stats + tree on Linux, macOS (`ps`) and Windows (`Get-CimInstance`/`taskkill`) |
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

Round-3 outcome in brief: [§1b.3](#1b3-browser-control-browser-controlmd); the work-package
detail follows.

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
12. **Release artifacts** in §7 are 1.0.0-alpha.3, built locally and (smoke only) on `coffee`
    and `desmondgiam`; the host build outputs stay on the hosts in `~/oino-release`. Nothing is
    signed, tagged or published.
13. **Desktop GUI QA**: the round-3 defects QA-1…QA-7 and GA-45 are fixed and live-verified
    ([gui-qa.md](gui-qa.md) Resolution; commits `d94a75f`…`1e1242c`); the 30-min faux soak passes
    (2.80 MB/min, [stability.md](stability.md)). GA-12 (unipi splash UTC-hour greeting) stays
    open until `host-info` grows the R3 local-offset field. Mouse-only surfaces and the
    real-provider path are still keyboard/config-verified only (§1b.7).
14. **`--session-id`** (fixed 2026-10-29): a missing id now creates the session with that
    header id, like pi (`SessionSource::NewWithId`); `--fork` + `--session-id` stays unsupported.
    Cron passes `--session-id <run id>`, so the run id is both the header id and part of the file
    name.
15. **MemPalace**: the Rust port is 99.7% parity but **not switched** (owner action, §1b.5);
    maintenance commands, `mempalace-light-mcp`, the coordination tools and the hook saves stay
    on Python. The Rust daemon sits at 260–360 MB under load (above the 250 MB idle target, flat),
    and bulk mines take ~1.5× the wall time at the default 4 embedding threads
    (`embedding_threads` / `MEMPALACE_EMBEDDING_THREADS`). Turn *MemPalace auto-update* off in
    both callers before switching, or `uv tool upgrade` silently restores Python.
16. **`ti`** printed "Tailscale SSH requires an additional check" this round, so the Linux
    GUI/desktop smoke ran locally on a private Xvfb (charter order); the owner has to approve
    the Tailscale check for `ti` to be usable again.

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

## 7. Release builds (1.0.0-alpha.3, 2026-10-27/28, `v1-alpha` @ `f934b81`, not tagged or published)

Local builds (this machine, `nice -n 10`, `-j 4`):

```
TMPDIR=$PWD/target/tmp cargo build --release -p oino -p oino-kanboard
cd apps && TMPDIR=../target/tmp cargo build --release -p oino-desktop
```

Host builds use `cargo xtask remote-build <host> [--os …] [--jobs 4] [--timeout-mins n] [-- <build-host.sh args>]`
(`xtask/src/remote.rs`):

1. **Sync.** `HEAD` goes to the host as a `git bundle` (incremental) into `~/oino-release`
   (detached checkout), plus a tarball of `extensions/dist` (the hosts have no wasm toolchain).
2. **Build.** `scripts/release/build-host.sh` runs detached (`nohup` on Unix, WMI
   `Win32_Process.Create` → Git Bash on Windows) and is polled once a minute, bounded by
   `--timeout-mins`.
3. **Fetch.** `~/oino-release/target/release-artifacts/*` → `target/release-artifacts/<target>/`,
   build log → `target/release-artifacts/logs/<host>.log`.

`build-host.sh` checks `cargo xtask checksums`, builds `oino` / `oino-kanboard`, runs
`apps/oino-desktop/scripts/build.sh --release --no-wasm`, and writes `smoke.txt`, `SHA256SUMS`
and `build-info.txt` per target.

Commands used: `cargo xtask remote-build coffee --os windows --jobs 4 -- --formats nsis`;
`cargo xtask remote-build mac --os macos --jobs 4 -- --formats app,dmg` (thin x86_64 — a
universal build needs ~6 GB more than the 17 GB free on that host).

| Target | Artifact | Size |
|---|---|---|
| x86_64-unknown-linux-gnu (local) | `oino` | 83.5 MiB |
| | `oino-kanboard` | 5.6 MiB |
| | `oino-desktop` | 139.1 MiB |
| x86_64-pc-windows-msvc (`coffee`) | `oino.exe` | 66.4 MiB |
| | `oino-kanboard.exe` | 4.2 MiB |
| | `oino-desktop.exe` | 93.0 MiB |
| | `oino-desktop_1.0.0-alpha.3_x64-setup.exe` (NSIS, per-user) | 28.8 MiB |
| x86_64-apple-darwin (`mac`) | `oino` | 74.0 MiB |
| | `oino-kanboard` | 4.8 MiB |
| | `oino-desktop` | 100.7 MiB |
| | `Oino.app` (ad-hoc signed, not notarized) | 102 MiB |
| | `Oino_1.0.0-alpha.3_x64.dmg` | 44.6 MiB |

Smoke results (see §1b.6 and `target/release-artifacts/<target>/smoke.txt`): Linux ✅ (print,
`cron status`, desktop `--demo` 20 s on Xvfb :98 + lavapipe with a screenshot); Windows ✅ CLI /
🟡 desktop alive but the console is locked; macOS ✅ CLI and desktop rendering (screenshot).

Problems found: the macOS **desktop** build aborted in `apps/oino-desktop/scripts/build.sh`
(`target_args[@]: unbound variable` — macOS `/bin/bash` 3.2 with `set -u`), fixed with the
`${arr[@]+"${arr[@]}"}` idiom used by the mobile script. Still open: no signing/notarisation
(Windows SmartScreen, macOS Gatekeeper), no Android build this round, `create-dmg` still needs
Finder over a GUI session (this run succeeded; the `hdiutil` fallback stays for SSH).

## 8. Next steps

0. **ZCode parity round** (design.md §8; `apps/oino-desktop/DESIGN-STATUS.md`): agent/subagent
   timeline on the `_oino/agents` feed, TurnFooter undo + UserBubble rewind on the core file
   checkpoints, queue-edit GUI wiring, StatusPanel + `_oino/topics`, pasted-image attachments,
   model-menu submenus, palette history chips, 4-pane p99 under 10 ms, mobile tablet layout,
   exit animations / run-label shimmer, i18n + Windows/macOS rendering checks.
1. **core-runtime**: `agents.agent.set-ui-policy` + depth on `call-context`; `control.move-project-state`
   (trust.json + sessions dir) and tmp GC. (Bash detach, file checkpoints, single-item queue
   commands, `disabledExtensions`, topic / agent feeds, built-in render overrides,
   `summarize-default` with a previous summary and `control.set-default-model` landed.)
2. **modes**: `oino serve` + `--remote` (TCP+TLS, pairing) with `_oino/list_sessions`,
   `_oino/get_keybindings`, `_oino/render`; TUI core settings through `_oino/set_setting`;
   ACP exit on `shutdown_requested`.
3. **release**: raise `RLIMIT_NOFILE` at `oino` startup (macOS); rebuild alpha.2 with
   `cargo xtask remote-build {local,coffee,mac}`; iOS simulator build; signing / notarisation;
   tag so CI assembles the GitHub release.
4. **wit-v1 / host**: wasmtime Pulley for iOS; TLS fingerprinting. (`mcp.json` agent preopen,
   process stats on macOS / Windows, the `host-info` UTC offset and the task notification
   `deliverAs: followUp` landed 2026-10-29.)
5. **gpui**: desktop perf (cheaper per-pane layout), remaining review P2 items, StatusPanel /
   Queue keys; mobile tablet layout and perf measurement; move `EmbeddedHost` into a shared crate.
6. **acp**: refusal stop reason; tool kinds; `custom`/clipboard dialogs over ACP; try Zed by
   hand. (`mcpServers` on `oino:acp:mcp-servers` and `form` elicitation landed 2026-10-29.)
7. **integration**: `cargo xtask layering`, `cargo deny`; golden e2e suite under `tests/e2e/`
   with `PARITY.md` (quality §7); criterion benches (quality §5); `oino-protocol` contract crate
   (architecture appendix N1); live OAuth checks.
