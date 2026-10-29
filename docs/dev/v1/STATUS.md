# Oino v1.0.0-alpha.4 — status

Date: 2026-10-30 · branch `v1-alpha` · round 4 (`round4-integration`: version bump, all gates,
desktop launcher, alpha.4 release builds local+coffee+mac; on top of round 3 + `harness-gaps`
(`round3-integration`: gui-qa + gui-qa-fix, stability, browser-impl, unipi-native + commands,
cron, mempalace-rs, cross-platform smoke; on top of round 2 `integration-2`: core-agents,
core-settings, frontend-cmds, providers-more, unipi-utility, design-spec / design-desktop(-fix)
/ design-mobile, release-builds), and the QA round 4 + fix (`qa-r4` / `qa-r4-fix`, see
[qa-r4.md](qa-r4.md)).
Parity targets: `@earendil-works/pi-coding-agent` **1.4.0**, unipi **3.0.0**, ZCode design
([design.md](design.md)).
Legend: ✅ done · 🟡 partial · ❌ missing · ⛔ diverges by design. Settled deviations are folded
into the appendix of each spec; open items per work item stay in [deviations/](deviations/).

## 1. Gates (this commit)

All run locally with `TMPDIR=$PWD/target/tmp`, `nice -n 10`, `-j 4`. Script:
`scripts/round4/gates.sh` (log `target/gate-round4.log`). Test counts sum every `test result`
line (lib + integration + doc-tests), 0 failures across all three workspaces.

| Workspace | fmt | clippy `-D warnings` (pedantic) | tests | other |
|---|---|---|---|---|
| root (`crates/*`, `xtask`) | ✅ | ✅ | ✅ **1,683 passed** (`cargo test --workspace`, incl. `crates/oino/tests/bundled.rs`, wasm-host e2e with the real unipi agents / settings components) | ✅ rustdoc `-D warnings` (`RUSTDOCFLAGS`); ✅ `cargo xtask wit`; ✅ `scripts/docs-check.sh`; ✅ `scripts/install-smoke.sh` (source install + faux prompt); all crates except `oino-wasm-host` / `xtask` on `[lints] workspace = true` |
| `extensions/` | ✅ | ✅ (`--target wasm32-wasip2`) | ✅ **680 native** (4 ignored); ✅ 7 wasm-host harness suites (`extensions/unipi/*/harness/run.sh`: ask-user, core, kanboard, notify, subagents, utility, workflow) | ✅ `cargo xtask wasm` (39 components at 1.0.0-alpha.4: 8 providers + `oino.browser`, 24 unipi, 5 fixtures) + `cargo xtask checksums` |
| `apps/` (GPUI) | ✅ | ✅ | ✅ **141 passed** (`cargo test --workspace`, incl. the automation, status-topic, agent-feed, chord-parsing and qa-r4-fix tests) | ✅ **rustdoc `-D warnings` now gates apps too** (new this round: fixed broken/private doc links + `doc_markdown` in `oino-ui`/`oino-mobile`); ✅ the unipi suite re-verified in [qa-r4.md](qa-r4.md) §9 |

Red gates found and fixed this round: an `extensions/` fmt drift from `[qa-r4-fix]`, the
never-before-gated `apps/` rustdoc (`-D warnings`), and — found by the **coffee** release
build — the automation Unix-socket server was not `cfg(unix)`-gated, so the Windows desktop
binary did not compile (fixed; Windows gets a no-op `install`, see [§1a](#1a-round-4-summary)).
`cargo xtask layering` and `cargo deny` (quality §1, §6) still do not exist.

## 1a. Round 4 summary (work id `round4-integration`, 2026-10-30, `v1-alpha`, version **1.0.0-alpha.4**)

Round 4 is the alpha.4 consolidation round: version bump across all three workspaces,
every gate green (incl. two new ones), the `mise` desktop launcher verified, and the alpha.4
release artifacts built and smoked locally + on `coffee` and `desmondgiam`. Product code is
unchanged except the Windows build fix; the functional baseline is the QA round-4 verification
of HEAD (`qa-r4` + `qa-r4-fix`, all defects fixed and driver-verified).

| Area | State |
|---|---|
| unipi in desktop (live) | **24 ✅ live · 0 🟡 · 0 ❌** ([§1b.1](#1b1-unipi-300-in-the-desktop-app-incl-cron) table unchanged); driver suite `unipi` **28/28 PASS** with the real model at HEAD, re-verified by `[qa-r4-fix]` (qa-r4.md §9) after the QA-8 + H-1…H-5 fixes; the same scenario files also pass on the charter local fallback (Xvfb+lavapipe) |
| ZCode checklist | desktop **52 ✅ · 1 ⛔ (#50 ZCode product features) · 1 n/a (#51 mobile)**; `zcode` driver suite **6/6 PASS** incl. the previously failing `zcode-row-actions` (H-3 fix); counts per `apps/oino-desktop/DESIGN-STATUS.md` |
| perf / soak | 30-min faux soak **PASS** (2566 steps, RSS slope **1.07 MB/min**, limit 8; threads 233→232, fds 438→430, 0 crash/stall) and 4-pane perf **PASS** (run p99 **4.50 ms**, budget 10; keystroke→paint 7.6 ms; idle CPU 0.80 % UI thread) — qa-r4.md §9, Xvfb+lavapipe numbers |
| harness gaps | **closed**: pointer injection + semantic tree ([gui-driver.md](gui-driver.md), §1b.7 items 1–2), host-neutral scenarios (H-1/H-2/H-4), click racing at turn end (H-3), local faux lifecycle (H-5). **Remaining**: H-6 panel-body OUS nodes never register in the tree (assert the panel title instead); D11 synthetic wheel-scroll not accepted by GPUI's list hitbox; no mouse drag / touch / multi-window / webview content |
| test tooling | `cargo xtask gui suite unipi\|zcode\|qa-r4` manifests + `scripts/gui-drive.py` (SSH-bridged automation socket); charter local fallback `apps/oino-desktop/scripts/local-app.sh` (demo/faux/real, minimal-PATH isolation) + `docs/dev/v1/qa-r4/local-suite.py`; `scripts/round4/gates.sh` (all gates, incl. new apps rustdoc); host suites in `extensions/unipi/*/harness/run.sh` |
| `mise` launcher | `mise run desktop\|start\|stop` (`scripts/desktop.sh`) verified on a private Xvfb :97 (never the owner's display): `start` builds + backgrounds the release app with a scratch config, `stop` stops it cleanly, `desktop` runs foreground (`--demo` exercised; 100 KB rendered screenshot kept under `target/` during the check) |
| Windows build | the `[gui-driver]` automation Unix-socket server broke the Windows desktop build (never compiled there before); now `cfg(unix)` with a no-op install on Windows — the alpha.4 `coffee` build proves it |
| release artifacts | 1.0.0-alpha.4 built on this PC (Linux CLI+desktop), `coffee` (NSIS + CLI smoke) and `desmondgiam` (.app/.dmg + CLI smoke) — [§7](#7-release-builds-10000-alpha4-2026-10-30-v1-alpha--845d31e-not-tagged-or-published); AppImage/deb assigned to `ti`, which stayed **Tailscale-blocked** the whole round (fresh approval URL per attempt, surfaced to the owner), so the Linux packages were built with the charter's `remote-build local` fallback and the `ti` re-run (suites + soak/perf on the reference host) stays outstanding |
| gates | all green at `845d31e` — see [§1](#1-gates-this-commit) |

## 1b. Round 3 summary (work id `round3-integration`, 2026-10-28, `v1-alpha`)

Round 3 closed the desktop QA defects, the stability/soak work, browser control, the command
rename + `unipi-migrate` skill, cron (Oino-native), the mempalace-rs integration and the
cross-platform smoke. Parity targets unchanged: pi 1.4.0, unipi 3.0.0, ZCode design.
Version: **1.0.0-alpha.3**.

### 1b.1 unipi 3.0.0 in the desktop app (incl. cron)

Every unipi surface (design.md §7) reaches exactly one primary rendering; the desktop now
renders each one natively or through the shared OUS/docked-card machinery. Since
`[unipi-live]` (2026-10-29) "live" means: exercised on `ti` by a **driver scenario with a real
model** (`oino/deepseek/deepseek-flash`) that asserts the rendered panel text, the element
tree and `state.chrome`/`state.open` — the scenarios below are the regression suite
`cargo xtask gui suite unipi` ([gui-driver.md](gui-driver.md) §4). Screenshots per scenario:
`docs/dev/v1/design/shots/desktop/<scenario>/`.

| Package | Desktop surface | Status | Scenario (proof) |
|---|---|---|---|
| core | hint strip (`alt+h`), hints browser (`/hint`), welcome hero / slim header card, work tray (`↓`) | ✅ live | `unipi-core` |
| info-screen | `/info` dashboard dialog (tabs, stat tiles, meters, `r` refresh) | ✅ live | `unipi-info-screen` |
| footer | glance frame under the composer + stats strip (`/footer off` hides it) | ✅ live | `unipi-footer` |
| workflow | mode chip + menu, `/permission`, `alt+m` cycle, `alt+p` plan mode, approval card (allow) | ✅ live | `unipi-workflow`, `unipi-input-shortcuts` |
| ask-user | `ask_user` tool → question card → answer → turn resumes | ✅ live | `unipi-ask-user` (the tool is off by default, like unipi; the scenario enables it through `settings`) |
| long-horizon | `/goal` continuation + `Goal` status, `/visualize-progress`, `/regular` clears the mode, `/ralph`/`/graph` help | ✅ live | `unipi-long-horizon` (`/regular` left the chip behind — fixed) |
| memory | `/memory` status card, `/memory-search`, `/global-memory-list`, `memory_store` tool, `/memory-consolidate` | ✅ live | `unipi-memory` (markdown-only on `ti`: no MemPalace install) |
| dream | `/dream` status card, report, approve/reject, work-tray tab | ✅ live | `unipi-dream` |
| skill-registry | `/skills` → Settings hub "Skills" page, exposure switch | ✅ live | `unipi-skill-registry` |
| subagents | `run_subagent` row → Agent tab, `/agents` profile manager, `/subagents` tray tab | ✅ live | `unipi-subagents`, `agents-live` |
| background-tasks | `/bg`, tasks tray tab, tray strip, Stop all (two-step confirm), `bg_run` completion notification | ✅ live | `unipi-background-tasks` |
| btw | `/btw` side-chat panel, session message count unchanged | ✅ live | `unipi-btw` |
| web-api | `web_search` tool row, `/info` Web tab | ✅ live | `unipi-web-api` |
| notify | `/notify-event` list + toggle, Settings → Notify page | ✅ live | `unipi-notify` |
| kanboard | `/kanboard onboard`, native Board page, New-task dialog, `/kanboard status` | ✅ live | `unipi-kanboard` (the board panel follows the daemon SSE and can lag a capture; the board's Refresh is the workaround — gap 17) |
| mcp | `/mcp-status` panel, Settings → MCP page | ✅ live | `unipi-mcp` (no live MCP server on `ti`; the stdio/HTTP paths stay covered by the harness e2e) |
| command-enchantment | `/` completion menu with package colour tags, filter, Tab accept | ✅ live | `unipi-command-enchantment` |
| compactor | `/compact-stats`, `/compact-doctor`, `/session-recall`, `/compact-vcc`, `/compact-by-llm`, `/compact-help` | ✅ live | `unipi-compactor` (a tiny session reports "Nothing to compact"; the LLM summary path needs a context-sized session) |
| updater | `/readme`, `/changelog`, Settings → About & updates | ✅ live | `unipi-updater` |
| input-shortcuts | `alt+h`, `alt+m` (ask→auto→full→ask), `alt+p`, `/ho` completion | ✅ live | `unipi-input-shortcuts` |
| fusion | `/fusion` lead → effort → confirm, `/fusion-stats` | ✅ live | `unipi-fusion` |
| watchdog | `/bg-detach` adopts a running bash call, stuck chip, tray row | ✅ live | `unipi-watchdog` |
| app-bridge | `/app` control-centre side tab (Session/Work/Modes/BTW/Info) | ✅ live | `unipi-app-bridge` |
| utility | `/doctor`, `/cleanup`, `/move`, `/answer`, Settings hub, image attachments | ✅ live | `unipi-utility`, `image-attach` |
| **cron** (Oino-native) | **Automations page** (sidebar nav, job rows, run history, add/edit/enable/disable/run/pause) | ✅ live | `unipi-cron` (+ `scripts/round3/cron-e2e.sh --real`) |
| browser | `/browser` status (engine, display, profile, tabs), `browser_*` tool rows | ✅ live | `unipi-browser` |

Count: **24 ✅ live, 0 🟡, 0 ❌** for the 24 bundled packages + cron. Run the same suite to
re-check: `cargo xtask gui suite unipi` (report in `target/gui-suite/unipi.json`).

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

ZCode parity ([design.md](design.md) §8, 54 rows): desktop 50 ✅ · 2 🟡 (motion,
performance) · 1 ⛔ · 1 n/a after §1b.9. Open: 4-pane p99 under the 10 ms budget, exit
animations, mobile tablet layout and the mobile halves of rows 23–25, 31, 33, 40. `apps/oino-desktop/DESIGN-STATUS.md` is the row-by-row source.

### 1b.9 `[gui-wiring]`: the core features reach the GUI

Rows 23, 24–25, 31, 33 and 40 of the desktop checklist are closed on desktop
(`apps/oino-desktop/DESIGN-STATUS.md`); GA-41 packaging now ships `oino-kanboard` beside
`oino-desktop`. Verified on `ti` (release build, Xvfb :93, lavapipe) with four new driver
scenarios — `agents-live` (20 steps), `status-panel` (31), `turn-undo` (13), `queue-edit`
(19) — all **PASS**; screenshots under
`docs/dev/v1/design/shots/desktop/<scenario>/`. The real-provider scenarios
(`image-attach`, `subagent-real`, `rewind-real`) are written and their GUI half runs (the
drop attaches `Image #1`, the prompt is sent), but the model call fails: the Oino Router
account is out of OpenRouter credits (HTTP 402, "can only afford 25079 tokens") — see
[gui-driver.md](gui-driver.md) §7b. GA-46 (idle cost) stays as measured: the scoped ticker
stops when nothing animates; no new timer or task was added by this round.

### 1b.10 `[unipi-live]`: every unipi package driven live in the desktop app

Goal (owner priority #1): each unipi 3.0.0 feature works **live** in Oino Desktop with the
behaviour of the reference app. This round wrote one driver scenario per package
(`docs/dev/v1/gui/scenarios/unipi-*.json`, listed in
[docs/dev/v1/gui/suites/unipi.json](gui/suites/unipi.json), runner `cargo xtask gui suite unipi`)
and drove each on `ti` (Xvfb :93, lavapipe, release build) against a real provider
(`oino/deepseek/deepseek-flash`), asserting rendered panel text, the element tree and
`state.chrome`/`state.open` — no OCR. 28 scenarios: 24 for the bundled packages,
`image-attach` (utility attachments), `agents-live` (subagent timeline fixtures) and cron.
Full suite on `ti` (release build, Xvfb :93): **28/28 PASS** (report `target/gui-suite/unipi.json`).

Why the earlier rounds could not drive real-model turns: the `ti` harness wrote a `model:`
setting (core reads pi's `defaultProvider`/`defaultModel`), uploaded `auth.json` with the
legacy `router` credential name (the provider id is `oino`) and never copied the router's
dynamic model catalog — so the app always opened with "No model selected", which round 3 read
as a router credit block. With that fixed, every scenario runs a real turn.

Harness work needed to make the surfaces assertable (`apps/oino-ui`, all inert while
automation is off): OUS text nodes (`ous/<key>`), `state.chrome` (statuses, widgets, frame
segments), `state.open` naming open extension panels (`panel:<id>`), the result-card body
(`result-body`), every modal title (`<id>-title`), the AgentRow (`<name> <task>`), the
completion menu and its rows (package tag in the label), toast messages (`toast-<id>`), the
board's New-task inputs, and the composer's mode/model chips. `+` chords (`alt+h`) now parse,
and a scenario can carry `settings` (extension switches) and assert `chrome`/`chrome_absent`/
`open`.

Defects found live and fixed:

1. **`/regular` left long-horizon mode on** — `publish` falls back to the last resolved mode
   when no owner is active, and the command did not clear it, so the `Goal` chip and the
   long-horizon status survived the stop (`extensions/unipi/long-horizon/src/lib.rs`).
2. **`ti` never built `oino-kanboard`** (nor linked it beside `oino-desktop`), so the native
   Board view sat on "Starting the kanboard daemon…" forever (`xtask/src/ti.rs`).
3. The driver injected `alt+h`/`alt+m`/`alt+p` as literal text instead of a chord
   (`apps/oino-ui/src/automation/input.rs`).

Per-package status and the scenarios that prove each one: [§1b.1](#1b1-unipi-300-in-the-desktop-app-incl-cron).

Open gaps (recorded, not fixed here):

17. **Board refresh latency**: the native Board renders the extension panel, which the kanboard
    extension refreshes from the daemon's SSE stream; a task captured from another view can
    take tens of seconds to appear (the board's own Refresh shows it at once). Worth a
    follow-up in `apps/oino-ui/src/views/board.rs` (poll or subscribe to the kanboard topic).
18. **MCP** has no live server in the `ti` scratch dir: `/mcp-status` and the settings page are
    live-verified, the stdio/HTTP tool paths stay covered by the wasm-host e2e tests.
19. **MemPalace** is not installed on `ti`, so `/memory` runs markdown-only there (the palace
    paths are verified separately, [§1b.5](#1b5-mempalace-rs-the-rust-port)).
20. **LLM compaction** needs a context-sized session; the scenario exercises the command
    surface and the "Nothing to compact" answer on a small one.
21. **Browser**: the engine status and a `browser_open` tool row are live-verified; page
    interaction, screencast and take-over remain WP5 items ([§5b](#5b-browser-control)).
22. **Mobile/TUI** are not part of this suite (desktop only).

## 2. End-to-end smoke (release `oino` 1.0.0-alpha.2, temp agent dir, bundle of 33 packages)

Re-checked on **1.0.0-alpha.4** (2026-10-30, local release build): `oino --version`,
`oino-kanboard --version`, `oino-desktop --version` → `1.0.0-alpha.4`; faux print run →
`Hello from faux.` (`scripts/install-smoke.sh` locally, plus every host's smoke.txt);
`oino-desktop --demo` verified through `mise run start` on a private Xvfb :97 + lavapipe
(rendered screenshot). The table below is the round-2 record and stays valid.

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
| long-horizon | ✅ | goal continuation through the arbiter verified (e2e + smoke + the live desktop scenario); `/regular` now clears the mode (fixed in `[unipi-live]`); graph runs not e2e-tested beyond `/graph` help |
| memory | ✅ | markdown memories, tools, recall, side-save ✅ (live desktop scenario); MemPalace verified against **both** implementations (Python 3.11.0 and mempalace-rs) on palace copies; `mempalaceBin` selects a binary or folder with a `PATH` (Python) fallback — [§1b.5](#1b5-mempalace-rs-the-rust-port) |
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
| kanboard | ✅ | CLI/daemon, panel, desktop native board, monitor nudges through the arbiter (smoke + the live desktop scenario); TUI renders the board panel generically; `/kanboard open` opens the side pane at any window width ([gui-qa-fix] QA-5). Open: a capture can take tens of seconds to show in the native board (SSE-driven refresh, §1b.10 gap 17) |
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
Desktop: 52 ✅ · 1 ⛔ (#50 ZCode product features) · 1 n/a (#51 mobile) — the [zcode-perf] rounds
(motion, exit animations, timestamps, model-menu drill-down, palette history, pacing) closed
rows 6 and 52; details + measured numbers in `apps/oino-desktop/DESIGN-STATUS.md`.

| Group (design.md §8 rows) | Desktop | Mobile | Gaps |
|---|---|---|---|
| Theme, type, spacing, icons (1–5) | ✅ | ✅ | unipi warm OLED dark + derived light (A1) |
| Motion (6) | ✅ | 🟡 | desktop: enter/exit animation pairs (`design::motion::enter/exit`), run-state/fold/thinking shimmer, ticker stops when idle; mobile still lacks exit animations |
| Frames, sidebar, header, welcome (7–13) | ✅ | ✅ (M1/M2 app bars, sections, swipe rows) | tablet layout on mobile ❌ |
| Timeline, tool rows, diffs, bash, thinking (14–22, 26) | ✅ | ✅ (touch rows) | `Thought for Ns` timing + hover-bar arrival times on desktop; history rows carry no timing (session messages have no timestamps) |
| Subagent rows (23) | ✅ | 🟡 | [gui-wiring] AgentRow live status + Agent tab child timeline from `_oino/agents` / `agent_messages`, Stop agent, `btw` / dream children from the StatusPanel Run section; scenario `agents-live` (mobile: drawer rows still show task + result only) |
| Turn undo / rewind (24–25) | ✅ | 🟡 | [gui-wiring] turn footer Undo (files) and user-bubble Rewind (fork + composer) behind a confirmation (`turn-undo`); mobile keeps Edit & resend only |
| Composer, model menu, context ring, mentions, history (27–30, 32, 34) | ✅ | ✅ | provider drill-down submenu (←/Esc back, one provider = flat list), alt-t thinking cycle, state placeholders ([zcode-perf]) |
| Attachments (31) | ✅ | 🟡 | [gui-wiring] pasted (⌘V) and dropped images become image attachments sent with the prompt, other files `@path` mentions, an attachment strip shows the payload; scenario `image-attach` (real provider blocked by the router balance, see §1b.9) |
| Queue (33) | ✅ | 🟡 | [gui-wiring] ↑/↓ reorder, send now, edit in place, recall to composer, remove (`_oino/queue_edit`); scenario `queue-edit` |
| Permission, guard, elicitation, plan, waiting badge (35–39) | ✅ | ✅ (docked + sheet promotion) | — |
| Status panel (40) | ✅ | 🟡 | [gui-wiring] `rebuild_status` merges session state with the sticky topics (long-horizon state/progress, `bg:tasks`, `fusion:status`, `kanboard:status`, `oino:git`), Run rows open the child Agent tab / Tasks tab, ⌘⇧S expand, ↑/↓, Enter, Delete (confirm), Esc; scenario `status-panel` |
| Side pane, browser, PTY terminal (41–43) | ✅ | ✅ browser (Custom Tabs / SFSafariViewController); terminal ❌ on mobile | — |
| Command center, modifiers, settings + hub, toasts, split panes, board (44–49) | ✅ | ✅ (search, stacked settings + hub, lane pager) | — (history chips, ⌥1–⌥3, per-chip forget on desktop) |
| Performance (52) | ✅ | 🟡 | desktop 4-pane run p99 4.1–4.9 ms (budget 10, per-pane phase pacing), idle CPU 0.8–1.0 % UI thread, threads/fds flat (Xvfb+lavapipe numbers; `ti` blocked by a Tailscale re-auth, GPU re-run pending); mobile 120 Hz not measured |
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
12. **Release artifacts** in §7 are 1.0.0-alpha.4 (2026-10-30, `845d31e`), built locally
    (Linux CLI+desktop+AppImage/deb) and on `coffee` (NSIS) and `desmondgiam` (.app/.dmg),
    each with its host smoke; the host build outputs stay on the hosts in `~/oino-release`.
    The `ti` packaging + GUI re-run is blocked by the Tailscale check (see §7). Nothing is
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
| GA-46 | ✅ | no refresh loop when idle (scoped ticker ends one tick after the last animation) | measured 2026-10-29: idle CPU 0.8–1.0 % UI thread after settle, threads 180 / fds 43 flat across idle ↔ 2 streaming sessions (64 llvmpipe + 32 GPUI + 84 app threads), 12-min faux soak PASS (threads 233→231, RSS slope −0.1 MB/min) — [zcode-perf], DESIGN-STATUS perf section |

## 7. Release builds (1.0.0-alpha.4, 2026-10-30, `v1-alpha` @ `845d31e`, not tagged or published)

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
universal build needs ~6 GB more than the ~10 GB free on that host after cleanup);
`cargo xtask remote-build local --jobs 4 -- --formats appimage,deb` — the AppImage/deb run
on `ti` could not run (Tailscale check never approved this round either), so the Linux
packages come from the same script locally (charter fallback, built at `781fb00`, one commit
after the hosts: a `build.sh` packaging fix only, no product change).

| Target | Artifact | Size |
|---|---|---|
| x86_64-unknown-linux-gnu (local) | `oino` | 83.6 MiB |
| | `oino-kanboard` | 5.5 MiB |
| | `oino-desktop` | 139.1 MiB |
| x86_64-pc-windows-msvc (`coffee`) | `oino.exe` | 66.7 MiB |
| | `oino-kanboard.exe` | 4.2 MiB |
| | `oino-desktop.exe` | 93.7 MiB |
| | `oino-desktop_1.0.0-alpha.4_x64-setup.exe` (NSIS, per-user) | 28.9 MiB |
| x86_64-apple-darwin (`mac`) | `oino` | 74.2 MiB |
| | `oino-kanboard` | 4.8 MiB |
| | `oino-desktop` | 101.5 MiB |
| | `Oino.app` (ad-hoc signed, not notarized) | 103 MiB |
| | `Oino_1.0.0-alpha.4_x64.dmg` | 44.9 MiB |

Smoke results (see `target/release-artifacts/<target>/smoke.txt`): Windows ✅ CLI (`--version`,
faux print with 35 bundled packages) / 🟡 desktop alive but the console is locked (PARTIAL, as
in round 3); macOS ✅ CLI + `.app` bundle version / 🟡 desktop alive after 20 s but no
screenshot this time (no Screen Recording grant over SSH / display asleep); Linux ✅ (local
run: `--version` ×3, faux print, desktop `--demo` 20 s on its own Xvfb with a screenshot).

Problems found: the **Windows desktop build failed** on the automation server's Unix-only
imports (`std::os::unix`, never compiled on Windows before the `[gui-driver]` round) — fixed
with a `cfg(unix)` gate and a no-op install on Windows (`845d31e`); `ti` stayed
Tailscale-blocked all round, so the Linux AppImage/deb come from the local fallback run and
the `ti` re-run (build + `unipi` suite + soak/perf on the reference host) is outstanding.
Still open: no signing/notarisation (Windows SmartScreen, macOS Gatekeeper), no Android build
this round, `create-dmg` still needs Finder over a GUI session (this run used the `hdiutil`
fallback). The host build outputs stay on `coffee`/`mac` in `~/oino-release` for reuse; the
mac was trimmed back to ~10 GB free (stale `target/debug`, `aarch64-apple-darwin`, old
artifacts removed).

## 8. Next steps

0. **`ti` re-run (blocked since 2026-10-29)** — once the owner approves the Tailscale check:
  `cargo xtask ti sync && cargo xtask ti build`, the `unipi`/`zcode`/`qa-r4` suites, `perf.sh`/
  `soak.sh` and the `remote-build ti@ti --dir oino-test/oino --jobs 6 -- --formats appimage,deb`
  packaging on the reference host (all local numbers since are Xvfb+lavapipe fallback). The
  ZCode desktop rows are otherwise done ([zcode-perf], `zcode` suite 6/6); remaining from the
  old list: mobile tablet layout, i18n + Windows/macOS rendering checks, real-provider
  scenario runs once the router balance allows (§1b.9).
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
