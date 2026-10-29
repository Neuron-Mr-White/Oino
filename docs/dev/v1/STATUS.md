# Oino v1.0.0-alpha.5 — status

Date: 2026-10-30 · branch `v1-alpha` · round 5 (`round5-integration`: version bump, gates,
D-9 suite assembly, alpha.5 release builds; on top of the round-5 work packages
(`wp-extensions`/`wp-chat`/`wp-composer`/`wp-shell`/`wp-settings`/`wp-kanboard`/`wp-editor`),
QA round 5 (`qa-r5`: 27/27 issues verified + defect list; `qa-r5-fix`: D-1…D-8/D-10 fixed,
suites 24/24 · 12/12 · 5/5) and the full gap audit (`gap-audit`, [GAPS.md](GAPS.md)); round 4
(`round4-integration`: gates, launcher, alpha.4 builds), round 3 (`round3-integration` +
`harness-gaps`: gui-qa, stability, browser-impl, unipi-native, cron, mempalace-rs), round 2
(`integration-2`), round 1 (`integration`)).
Parity targets: `@earendil-works/pi-coding-agent` **1.4.0**, unipi **3.0.0** (harness parity
only — D1–D4), ZCode design ([design.md](design.md)).
Legend: ✅ done · 🟡 partial · ❌ missing · ⛔ diverges by design. Settled deviations are folded
into the appendix of each spec; open items per work item stay in [deviations/](deviations/).

## 0. Open gaps — read this first (from [GAPS.md](GAPS.md), the audit of record)

Why this section exists: the owner found that the phone remote story (no `oino serve`, no
remote attach, no mobile rebuild since alpha.1) was **never surfaced in any round summary**
until the [gap audit](GAPS.md) (77 gaps: 2 blockers, 17 majors, 58 minors; audited at
`9b447a0`, two majors — DG-1/DG-2 — have since closed). Every round summary from now on
starts here. Full list with evidence: [GAPS.md](GAPS.md).

**Blockers (product capability promised, absent):**

| ID | Gap | Owner |
|---|---|---|
| RM-1 | **`oino serve` / `SessionHub` / hub protocol do not exist**: no `_oino/open_session/attach/detach/close_session/hub_sessions`, no TLS/token/pairing server, no multi-client fan-out, no first-answer-wins, no attach replay; `oino-client` has no `RpcBackend` (modes.md §5/§8) | round 6 |
| RM-2 | **No iroh / P2P anywhere** (0 code hits); modes.md §5 (TCP+TLS+token) is superseded by the owner's [round6-direction.md](round6-direction.md) (iroh + opt-in daemon) — spec and code both pending | round 6 (spec first) |

**Majors:**

| ID | Gap | Owner |
|---|---|---|
| CM-2 | `--remote <url>` refuses in interactive mode (silently ignored with `-p`) | round 6 |
| RM-3 | Remote TUI/GUI attach (`--remote`, host switcher → served host) blocked by RM-1 | round 6 |
| RM-4 | **Phone remote attach is UI-only**: pairing stored, token discarded (no keystore), hosts show "not connected", sessions run embedded on the phone | round 6 |
| RM-5 | **Desktop host switcher remote pick is inert**: stores the selection and toasts that the app keeps running locally (`oino serve` does not exist) | round 6 |
| RM-6 | No `RpcBackend` transport in `oino-client` (client side of RM-1) | round 6 |
| RM-8 | Opt-in remote daemon (Settings "Remote access" toggle, consent prompt, per-OS service install, paired-devices revoke) — nothing exists | round 6 |
| MB-1 | **No mobile build since alpha.1** (Oct 26–27 APKs): browser control, cron page and every round-5 UI change never reached a phone | round 6 |
| MB-2 | **Mobile UI misses all round-5 changes** (no mode switcher, model-picker modal, new-chat page, Files/LSP; the old composer footer survives) and still codes surfaces for extensions D1 deleted (footer segments, hint strip, app-bridge tab) | round 6 |
| MB-3 | Mobile is embedded-core in reality but spec'd remote-only (`RpcBackend`, gpui-app.md §4/D16) — divergence never settled | round 6 spec |
| MB-5 | iOS: simulator-only, wasmtime Pulley not wired (devices can't JIT), no `.ipa` ever built | round 6 stretch |
| EX-1 | Zed (or any real ACP client) never tried by hand — ACP is fixture-verified only | verification |
| PR-1 | Nothing signed or notarised (Windows SmartScreen, macOS Gatekeeper) | release |
| PR-2 | Nothing tagged or published; the CI release jobs (`ci.yml`, `v*` tags) never exercised | release |
| PR-3 | Reference-host re-run outstanding: `ti` is reachable again (Tailscale approved), so what remains is the `ti` AppImage/deb packaging + real-GPU perf numbers (suites/soak ran on ti in qa-r5/qa-r5-fix) | verification |
| VF-1 | OAuth flows (Anthropic, Copilot, Codex, ChatGPT) verified only with fixtures/faux — never against live accounts | verification (owner-assisted) |

The 58 minors (harness core, providers, CLI stubs, desktop polish, verification debt) stay in
[GAPS.md](GAPS.md) §2 — nothing there is new this round. Round 6 direction (owner decision
2026-10-29, [round6-direction.md](round6-direction.md)): remote anywhere via the opt-in daemon
+ iroh transport, mobile catch-up.

## 1. Gates (this commit, 1.0.0-alpha.5)

All run locally with `TMPDIR=$PWD/target/tmp`, `nice -n 10`, `-j 4`. Script:
`scripts/round5/gates.sh` (log `target/gate-round5.log`). Test counts sum every `test result`
line (lib + integration + doc-tests), 0 failures across all three workspaces.

| Workspace | fmt | clippy `-D warnings` (pedantic) | tests | other |
|---|---|---|---|---|
| root (`crates/*`, `xtask`) | ✅ | ✅ | ✅ **773 passed** (`cargo test --workspace`, incl. `crates/oino/tests/bundled.rs`, wasm-host e2e with the real unipi agents / settings components) | ✅ rustdoc `-D warnings`; ✅ `cargo xtask wit`; ✅ `scripts/docs-check.sh`; ✅ `scripts/install-smoke.sh` |
| `extensions/` | ✅ | ✅ (`--target wasm32-wasip2`) | ✅ **609 native**; ✅ 7 wasm-host harness suites (`extensions/unipi/*/harness/run.sh`: ask-user, core, kanboard, notify, subagents, utility, workflow) | ✅ `cargo xtask wasm` (35 components at 1.0.0-alpha.5: 8 providers + `oino.browser`, 21 unipi, 5 fixtures) + `cargo xtask checksums` |
| `apps/` (GPUI) | ✅ | ✅ | ✅ **171 passed** | ✅ rustdoc `-D warnings` |

Gates drift found and fixed this round (`966e2fb`): a 1-in-5 flake in
`oino-acp` `session_new_publishes_mcp_servers` (the mcp-servers topic was published *after*
the `session/new` response — now published before responding), and five rustdoc
`-D warnings` breaks from the round-5 work packages (`oino-kanboard` `Exec::process` link
from the daemon removal; `oino-ui` `DiagnosticsRx`/`subscribe`/`AppShell::open`/`CURRENT_PC`
links). qa-r5-fix had already fixed the fmt drift qa-r5 found (D-5).

GUI suites (driver, release build, `ti` Xvfb :93): **unipi 24/24 · qa-r4 12/12 · zcode 5/5**
(re-verified by `[qa-r5-fix]`, reports `target/gui-suite/*.json`); the new round-5 acceptance
suite `cargo xtask gui suite r5` (25 scenarios, [§1a](#1a-round-5-summary)) — **25/25 PASS**
on `ti` at `966e2fb` (fresh sync + release build; report `target/gui-suite/r5.json`; a first
run hit one board-watcher flake in `r5-board-inprocess` step 9 — the re-seeded card did not
appear within 15 s — re-verified PASS solo and in the full re-run; same class as the known
startup flake qa-r5 D-8).
`cargo xtask layering` and `cargo deny` (quality §1, §6) still do not exist (VF-8).

`mise run desktop|start|stop` verified on a private Xvfb :97 + lavapipe (never the owner's
display): `start` built the alpha.5 release app and rendered a 205 KB frame with the scratch
config, `stop` stopped it cleanly (pidfile removed), `desktop -- --demo` ran foreground with
a 103 KB rendered frame (shots kept under `target/tmp/` during the check).

## 1a. Round 5 summary (work id `round5-integration`, 2026-10-30, `v1-alpha`, version **1.0.0-alpha.5**)

Round 5 was the GUI-first/ZCode-UX/harness-only-parity round ([round5.md](round5.md), owner
direction D1–D4 in [round5-feedback.md](round5-feedback.md)): seven work packages, then QA
(`[qa-r5]` 27/27 issues driver-verified + 16 defects, `[qa-r5-fix]` fixed D-1…D-8/D-10),
then this integration (alpha.5 bump, D-9 suite assembly, `SETTINGS_*` prune, gates, builds,
STATUS/GAPS sync). The product is the GPUI desktop; `crates/oino-tui` is frozen (deletion
candidate, charter D3).

### 1a.1 The 27 issues + D1–D3 (QA verdicts, [qa-r5.md](qa-r5.md) §3; fixes in its §8)

Key: ✅ acceptance met with driver/scenario evidence; ⚠️ met with a recorded caveat.

| # | Issue | Verdict | Evidence |
|---|---|---|---|
| D1 | TUI-only extensions removed | ✅ | `r5-tui-removal` (ext-row probes added by qa-r5-fix D-10); 35 components; repo grep clean |
| D2 | ZCode UX direction | ✅ | ZCode tokens + type scale; `r5-settings-zcode`, `r5-new-chat-page`, `r5-contrast-tokens` |
| D3 | Harness parity only | ✅ | every kept unipi package driver-reachable; `oino-tui` still compiles (frozen) |
| 1 | Modal model picker | ✅ | `r5-model-picker-modal` (real backend, ti) |
| 2 | Mode switcher up front | ✅ | `r5-mode-switcher` |
| 3 | One card + context inside | ✅ | `r5-composer-context-ring` |
| 4 | No duplicate sessions | ✅ | `r5-no-dup-sessions` |
| 5/13 | Missing icons | ✅ | `every_referenced_icon_is_embedded` unit test + `r5-icons-automations` |
| 6 | New-chat page | ✅ | `r5-new-chat-page` |
| 7 | In-process kanboard | ✅ | `r5-board-inprocess` + live CLI write → GUI refresh on ti |
| 8 | Draggable cards | ✅ | `r5-board-drag-pc` (refused-lane `<60px` mid-drag bound not encoded — D-11) |
| 9 | Modal formatting | ⚠️ | FAB dialog fields + creation proven (D-1 focus fix); **not** rebuilt on gpui-component `form/` (mechanism deviation, recorded); scenario now committed (`r5-board-modal-form`) |
| 10 | Phone side panel + unipi PC board | ⚠️ | `r5-board-side-phone` PASS; PC toolbar carries extra `Move`/`Refresh` and no `Clear` chip — D-12, owner visual pass |
| 11 | Overlay shows full text | ✅ | `qa-r5-board-fulltext`; scenario now committed (`r5-board-fulltext`) |
| 12 | Central host switcher | ✅ | `r5-host-switcher` ("Current PC", Add-host persists, honest remote toast — see [RM-5](#0-open-gaps--read-this-first-from-gapsmd-the-audit-of-record)) |
| 14 | ZCode settings layout | ✅ | `r5-settings-zcode` |
| 15 | Esc twice stops | ✅ | `r5-esc-twice-stop` + `r5-esc-dialog-guard` |
| 16 | Virtualization kept | ✅ | contract documented in `components/conversation.rs`; exercised by `r5-bounce-watch` |
| 17 | unipi markdown palette | ✅ | palette unit tests + `r5-markdown-palette` shots |
| 18 | One footer row | ✅ | `r5-composer-context-ring` + `r5-tui-removal` |
| 19 | Project tab labels project | ✅ | `r5-settings-trim` (no-session disabled state code-verified — D-13) |
| 20 | No Extensions page in Settings | ✅ | `r5-settings-trim` |
| 21 | Chat bounce | ✅ | `r5-bounce-watch` (zero gaps while streaming) |
| 22 | Every setting does something | ✅ | `r5-settings-trim` (dead keys gone; no-op-write toast code-verified — D-13) |
| 23 | Scroll containment | ✅ | `qa-r5-scroll-containment` (D11 synthetic-wheel caveat recorded); scenario now committed (`r5-scroll-containment`) |
| 24 | Files view + editor + LSP | ✅ | `r5-files-editor` + `r5-editor-lsp` (rust-analyzer on ti) |
| 25 | Contrast floor | ✅ | `contrast_floor_r5` unit test (both palettes) + `r5-contrast-tokens` shots |
| 26 | Real logo | ✅ | `r5-logo-real` |
| 27 | Refresh button removed | ✅ | `r5-refresh-gone` |

**24 ✅ + 3 ⚠️** (caveats recorded as D-9/D-11/D-12/D-13; D-9 closed by this integration —
see 1a.3).

### 1a.2 Work packages

| Package | Issues | Outcome |
|---|---|---|
| `wp-extensions` | D1/D3 | deleted `unipi.footer`, `unipi.command-enchantment`, `unipi.input-shortcuts`, `unipi.app-bridge` + `unipi.core` hints half; 39 → 35 components; settings namespaces gone |
| `wp-chat` | #15 #16 #17 #21 #25(conv) #26 #27 | list discipline + virtualization contract, two-stage Esc, markdown palette, contrast remap, real logo, refresh-button removal |
| `wp-composer` | #1 #2 #3 #18 | modal model picker (search/pins/fusion), agent-mode switcher, one-card composer with context ring |
| `wp-shell` | #4 #5 #6 #12 #13 #25(app) | ZCode tokens + contrast floor, session dedup, new-chat page, host switcher, icon audit |
| `wp-settings` | #14 #19 #20 #22 | settings rebuild on gpui-component `setting/`, project scope labels, dead keys/no-op fixes |
| `wp-kanboard` | #7 #8 #9 #10 #11 #23 | daemon deleted (in-process service + WIT `kanboard` host import), unipi drag UX, layouts, modal/full-text, scroll containment, driver `drag` step |
| `wp-editor` | #24 | Files page + editor + rust-analyzer LSP (diagnostics, hover, format-on-save) on the in-pin `gpui-base` engine |

### 1a.3 QA + integration closure

- `[qa-r5]` (27/27 + 16 defects D-1…D-16) and `[qa-r5-fix]` (D-1…D-8, D-10 fixed; suites
  **unipi 24/24, qa-r4 12/12, zcode 5/5**; `/kanboard status` copy; `xtask wasm`/`checksums`
  round-trip): see [qa-r5.md](qa-r5.md) §6/§8.
- `[round5-integration]` (this): version **1.0.0-alpha.5** everywhere + dist/checksums
  rebuilt; **D-9 closed** — `suites/r5.json` (25 scenarios) + the three spec'd board
  scenarios (`r5-board-modal-form`, `r5-board-fulltext`, `r5-scroll-containment`) committed
  and run: **r5 suite 25/25 PASS on `ti`** (§1); `SETTINGS_*` dead constants pruned from
  `design/tokens.rs`; ACP flake + 5 rustdoc drifts fixed (§1); gates green in all three
  workspaces; alpha.5 artifacts (§7); STATUS/GAPS sync (§0, stale rows fixed).
- Perf/soak on `ti` (qa-r5 §5): 30-min faux soak **PASS** (2560 steps, RSS slope
  **2.08 MB/min** ≤ 8, threads 116→104, fds 399→399, 0 crash/stall); 4-pane perf **PASS**
  (whole-run p99 **9.90 ms** ≤ 10 budget, p50 4.07 ms, keystroke→paint 5.2 ms median).
- P3s left to owners (qa-r5 §8): D-11 (mid-drag bound needs a driver pause primitive),
  D-12 (owner visual pass on the board toolbar), D-13 (zero-pane startup state), D-14 (ti
  fixture model config), D-15 (docs drift, partially carried), D-16 (wasm build
  nondeterminism — every rebuild rewrites all 70 hashes).
- Previous round (alpha.4, `round4-integration`): version bump, all gates green incl. the
  new apps rustdoc gate, Windows `cfg(unix)` build fix (automation server), `mise` launcher
  verified, alpha.4 artifacts local+coffee+mac (mac built then; **unreachable this round**).
  Detail: git history + [qa-r4.md](qa-r4.md); superseded rows below were updated in place.

## 1b. Round 3 summary (work id `round3-integration`, 2026-10-28, `v1-alpha`)

Round 3 closed the desktop QA defects, the stability/soak work, browser control, the command
rename + `unipi-migrate` skill, cron (Oino-native), the mempalace-rs integration and the
cross-platform smoke. Parity targets unchanged: pi 1.4.0, unipi 3.0.0, ZCode design.
Version then: **1.0.0-alpha.3**.

### 1b.1 unipi 3.0.0 in the desktop app (incl. cron)

Every kept unipi surface (design.md §7) reaches exactly one primary rendering; the desktop
renders each one natively or through the shared OUS/docked-card machinery. "Live" means:
exercised on `ti` by a **driver scenario with a real model**
(`oino/deepseek/deepseek-flash`) that asserts the rendered panel text, the element tree and
`state.chrome`/`state.open` — the scenarios are the regression suite
`cargo xtask gui suite unipi` ([gui-driver.md](gui-driver.md) §4). Screenshots per scenario:
`docs/dev/v1/design/shots/desktop/<scenario>/`. Round 5 (D1) removed the four presentational
packages (`footer`, `command-enchantment`, `input-shortcuts`, `app-bridge`, see 1a.2) and
the `hints` half of core — **the suite is 24 scenarios now** and stays 24/24.

| Package | Desktop surface | Status | Scenario (proof) |
|---|---|---|---|
| core (work tray) | work tray (`↓`); hints/start-card deleted by D1 (native welcome instead, round5 #26) | ✅ live | `unipi-core` |
| info-screen | `/info` dashboard dialog (tabs, stat tiles, meters, `r` refresh) | ✅ live | `unipi-info-screen` |
| workflow | mode chip + menu, `/permission`, `alt+m` cycle, `alt+p` plan mode, approval card (allow) | ✅ live | `unipi-workflow`, `unipi-input-shortcuts`→deleted (mode surface now `r5-mode-switcher`) |
| ask-user | `ask_user` tool → question card → answer → turn resumes | ✅ live | `unipi-ask-user` (tool off by default, like unipi) |
| long-horizon | `/goal` continuation + `Goal` status, `/visualize-progress`, `/regular` clears the mode, `/ralph`/`/graph` help | ✅ live | `unipi-long-horizon` (mode status now a transcript row — qa-r5-fix D-2) |
| memory | `/memory` status card, `/memory-search`, `/global-memory-list`, `memory_store` tool, `/memory-consolidate` | ✅ live | `unipi-memory` (markdown-only on `ti`: no MemPalace install) |
| dream | `/dream` status card, report, approve/reject, work-tray tab | ✅ live | `unipi-dream` |
| skill-registry | `/skills` → Settings hub "Skills" page, exposure switch | ✅ live | `unipi-skill-registry` |
| subagents | `run_subagent` row → Agent tab, `/agents` profile manager, `/subagents` tray tab | ✅ live | `unipi-subagents`, `agents-live` |
| background-tasks | `/bg`, tasks tray tab, tray strip, Stop all (two-step confirm), `bg_run` completion notification | ✅ live | `unipi-background-tasks` |
| btw | `/btw` side-chat panel, session message count unchanged | ✅ live | `unipi-btw` |
| web-api | `web_search` tool row, `/info` Web tab | ✅ live | `unipi-web-api` |
| notify | `/notify-event` list + toggle, Settings → Notify page | ✅ live | `unipi-notify` |
| kanboard | native Board page (in-process since round 5 #7), New-task dialog, `/kanboard status` | ✅ live | `unipi-kanboard` + `r5-board-*` |
| mcp | `/mcp-status` panel, Settings → MCP page | ✅ live | `unipi-mcp` (no live MCP server on `ti`; stdio/HTTP paths covered by the harness e2e) |
| compactor | `/compact-stats`, `/compact-doctor`, `/session-recall`, `/compact-vcc`, `/compact-by-llm`, `/compact-help` | ✅ live | `unipi-compactor` |
| updater | `/readme`, `/changelog`, Settings → About & updates | ✅ live | `unipi-updater` |
| fusion | `/fusion` lead → effort → confirm, `/fusion-stats`; round 5 #1 folded fusion into the model picker | ✅ live | `unipi-fusion`, `r5-model-picker-modal` |
| watchdog | `/bg-detach` adopts a running bash call, stuck chip, tray row | ✅ live | `unipi-watchdog` |
| utility | `/doctor`, `/cleanup`, `/move`, `/answer`, Settings hub, image attachments | ✅ live | `unipi-utility`, `image-attach` |
| **cron** (Oino-native) | **Automations page** (sidebar nav, job rows, run history, add/edit/enable/disable/run/pause) | ✅ live | `unipi-cron` (+ `scripts/round3/cron-e2e.sh --real`) |
| browser | `/browser` status (engine, display, profile, tabs), `browser_*` tool rows | ✅ live | `unipi-browser` |

Count: **20 ✅ live + cron** for the 20 kept bundled packages (footer /
command-enchantment / input-shortcuts / app-bridge deleted by D1 — do not resurrect them).
Run the same suite to re-check: `cargo xtask gui suite unipi` (report in
`target/gui-suite/unipi.json`).

### 1b.1b Harness gaps closed (work id `harness-gaps`, 2026-10-29)

Ten request rows from the unipi matrices that needed a core/WIT or component change, closed in
that round (`[harness-gaps]` commits):

| # | Gap | What landed | Tests |
|---|---|---|---|
| 1 | mcp `mcp.json` + ACP `mcpServers` (unipi-integrations R1/R3) | `/agent/mcp.json` read-only preopen; `unipi.mcp` merges pi global → `/ext-global/mcp.json` → trusted project file; ACP publishes `session/new`/`session/load` `mcpServers` as sticky `oino:acp:mcp-servers` | `paths::tests::agent_view_exposes_config_files_only`, `config::tests::pi_file_is_merged…` (29 mcp tests), `acp::session_new_publishes_mcp_servers` (flake fixed this round: publish before responding) |
| 2 | compactor previous summary (unipi-context R1) | `models.summarize-default(…, previous-summary)` throughout | `compaction_llm::summarize_default_updates_a_previous_summary` |
| 3 | fusion startup model (unipi-agents R4) | `control.set-default-model` writes core settings without switching | `session_flow::set_default_model_persists_core_settings` |
| 4 | task notification delivery | host `task_finished` → `<background-task-notification>` `deliverAs: followUp` | `probes::services_bus_and_tasks_between_instances` |
| 5 | utility render styles (U1–U2) | `tools.override` render-only overrides + `_oino/render` (`RenderRow`/`RenderRequest`) | `session_flow::builtin_tool_render_overrides` |
| 6 | watchdog process stats | `proc-stats`/`process-tree` on macOS (`ps`) / Windows (`Get-CimInstance`), `taskkill /T` | `process::tests::{ps,cim}_output_parsing` |
| 7 | `host-info` UTC offset | `info.local-offset-minutes`; SDK `host::local_hour` — **closes GA-12** (splash greeting now local; qa-r5-fix verified) | core/notify guest tests, `unipi.core`/`info-screen` suites |
| 8 | `oino:settings:<ns>` topic | every settings write publishes the sticky namespace topic | `extension_settings::registry_serves_guests_and_frontends` |
| 9 | `--session-id` (known issue 14) | `SessionSource::NewWithId`; cron runs with the run id | `print::session_id_creates_the_session_like_pi` |
| 10 | ask-user forms over ACP (acp #11) | `form` → `elicitation/create` one object schema per question; `FormAnswers` rebuilt | `ui::tests::form_schema_and_answers` |

Not closed: the GPUI desktop renders tool rows natively (`render` overrides are honoured
host-side) and `custom`/clipboard dialogs stay cancelled over ACP.

### 1b.2 Stability / soak ([stability.md](stability.md))

Seven root causes fixed (GPUI background-executor panic on editor events, extension install
on the foreground executor, panes never dropped, no shutdown at quit, file-tree walk on the
render thread, unbounded `ous::Drains`, soak-driver double lease). Diagnostics: panic hook
writes `~/.oino/logs/desktop-crash-*.log`; watchdog writes `desktop-stall-*.log` after 2 s
of UI silence.

30-minute faux soaks on the release binary, one instance, own Xvfb + lavapipe:

| Run | Steps | RSS start→end (max) MB | Slope 2nd half | Threads | fds | Verdict |
|---|---|---|---|---|---|---|
| before fixes (`--max-panes 1000`) | ≈2560 | 5→1731 | **19.9 MB/min** | 276→333 | **500→842** | FAIL |
| after fixes, run 2 | 2564 | 4→1030 (1072) | −3.4 MB/min | 225→226 | 405→396 | PASS |
| demo backend | 2552 | 6→316 (360) | −1.8 MB/min | 184→182 | 73→61 | PASS |
| round-4 final (qa-r4-fix) | 2566 | 10→1120 | **2.80 MB/min** | 230→232 | 424→416 | PASS |
| round-5 (qa-r5, on `ti`) | 2560 | 11→588 (748) | **2.08 MB/min** | 116→104 | 399→399 | **PASS** |

Limits: RSS slope < 8 MB/min, thread growth < 16, fd growth < 64, 4 GB abort, 0 crash/stall
logs — all met. The faux plateau is the wasmtime `Services` footprint of ~30 bundled
components × live panes, flat once the pane cap is reached. Soak script:
`apps/oino-desktop/scripts/soak.sh --minutes 30 --backend demo|faux [--out DIR]`.

### 1b.3 Browser control ([browser-control.md](browser-control.md))

| WP | Status | What landed |
|---|---|---|
| WP1 contracts | ✅ | `crates/oino-browser-protocol`, `BrowserHost` + manifest permission, WIT `browser` |
| WP2 `oino-browser` | ✅ | in-house CDP client (unix pipe / loopback ws), engine detection, hidden Xvfb display, AX snapshot with stable refs, trusted input, dialogs/popups/downloads, screenshot ladder, screencast, residency LRU, default-deny site permissions; real-Chromium test `crates/oino-browser/tests/real_browser.rs` |
| WP3 host wiring | ✅ | wasm-host import, `LocalServices::set_browser`, nested-agent scope walk, `OINO_BROWSER=off\|ephemeral\|headless\|window` |
| WP4 `oino.browser` | ✅ | 19 `browser_*` tools, approval dialog, prompt section, turn-tail screenshot, `/browser`, `skill:browser` |
| WP5 desktop | 🟡 | side-pane screencast tab with agent/user tabs, take-over strip, dialog + handoff cards, mouse/wheel/key/clipboard; IME, file drag-drop, site-permissions popover, Fit-to-pane and a Settings page open (settings via `settings.json` `browser.*` only — DG-3) |
| WP6 mobile / WP7 TUI / WP8 integration | ❌ | WP6 is **round 6's** (mobile catch-up, [round6-direction.md](round6-direction.md) §8); WP7 TUI is obsolete (charter D3 froze the TUI) |

Verified end to end: `oino -p` with a real provider opened a fixture page, clicked twice,
read `clicked 2`; the desktop showed the agent's tab and user input reached the page.

### 1b.4 Commands: rename + `unipi-migrate` ([commands.md](commands.md))

Feature commands carry no `unipi:` prefix and no aliases. `/unipi:settings` and
`/unipi:changelog` merged into `/settings` and `/changelog`; the Fusion picker became
`/fusion` (round 5 #1 folded fusion picking into the model-picker modal too). Bundled
opt-in skill **`unipi-migrate`** rewrites `/unipi:*` references (fixtures:
`scripts/unipi-migrate-fixture.sh`).

### 1b.5 mempalace-rs, the Rust port

Upstream ([mempalace-rs](file:///home/oi/Projects/Personal/llm/mempalace-rs) branch
`oino-parity`): **99.7% parity** (1,194 of 1,198 items). Perf: CLI search cold 1.04 s/332 MB
→ 0.21 s/222 MB; wake-up 0.79 → 0.03 s; warm MCP reader startup 528 → 2 ms; 2 h soak daemon
RSS slope −38/−9 MB/h (Rust) vs +238/+49 (Python). Oino integration: `unipi.memory` has a
**`mempalaceBin`** setting (binary or folder; empty = `PATH`) with a Python fallback;
verified by `scripts/round3/memory-mempalace-e2e.sh` (4 cases, **RESULT: PASS**).
Still on Python by design: maintenance commands, `mempalace-light-mcp`, coordination tools,
hook saves. **The switch is the owner's action** (see known issue 15).

### 1b.6 Cross-platform smoke (round 3; `ti` unreachable then, re-verified since)

| Host | Build | `oino --version` | faux print | `oino cron status` | Desktop `--demo` 20 s |
|---|---|---|---|---|---|
| `coffee` (Windows) | ✅ CLI + NSIS desktop | ✅ | ✅ | ✅ | 🟡 alive; locked console (no visual check) |
| `desmondgiam` (macOS) | ✅ .app + .dmg | ✅ | ✅ | ✅ | ✅ alive + rendered screenshot |
| `ti` (Ubuntu) | — | — | — | — | skipped that round (Tailscale check); **reachable again** — suites/soak/perf ran there in qa-r5/qa-r5-fix; the `ti` packaging + GPU re-run remain (PR-3) |

### 1b.7 Open harness gaps (round 3)

1. ~~Pointer injection~~ **Fixed 2026-10-29** ([gui-driver.md](gui-driver.md)): JSON-lines
   RPC on a Unix socket (`OINO_DESKTOP_AUTOMATION`), events through `Window::dispatch_event`;
   round 5 added the `drag` step (board drag scenario).
2. ~~Screen reading~~ **Fixed 2026-10-29**: `tree` returns the semantic element list from a
   per-frame registry (`apps/oino-ui/src/automation/`).
3. **Windows**: screenshots need an unlocked console session on `coffee`; verification is
   "process alive after 20 s" plus the build's own smoke.txt.
4. **macOS**: no Screen Recording grant over SSH → "alive after 20 s", no screenshot;
   `ulimit -n 4096` required before smoke runs (known issue 10).
5. **GPU path**: soak/perf numbers are lavapipe; the `:0`/real-GPU re-run has not happened
   (owner's workstation rule; `ti` re-run outstanding — PR-3/VF-4).
6. **Real providers** are not soaked (faux shares the code path except the HTTP component).
7. **Embedded browser** is not part of the soak (WebKitGTK cannot embed on Xvfb).
8. **Cron** e2e runs in a scratch HOME; `install` is verified by the unit file.
9. ~~`ti` Tailscale~~ approved — reachable again (this round's r5 suite + qa-r5-fix suites
   ran there); the outstanding `ti` items are the AppImage/deb packaging and GPU perf.
10. **Release signing** is still manual (PR-1); the CI release jobs exist but were never
    exercised (PR-2).

### 1b.8 ZCode parity (design.md §8)

Desktop: **52 ✅ · 1 ⛔ (#50 ZCode product features) · 1 n/a (#51 mobile)** after round 5
(the round-5 WPs closed rows 1–6, 14, 27–34 etc.; `apps/oino-desktop/DESIGN-STATUS.md` is
the row-by-row source). Mobile rows lag — see MB-2. Perf: 4-pane run p99 4.1–9.9 ms
(budget 10), idle CPU 2.0 % UI thread (ti, lavapipe).

### 1b.9 `[gui-wiring]`: core features reach the GUI

GA rows 23–25, 31, 33, 40 closed on desktop; GA-41 packaging shipped `oino-kanboard` beside
`oino-desktop` (now CLI-only relevance — the board is in-process, [§6.1](#61-gui-audit-area-e-core-gui-gaps)).
Real-provider scenarios: the round-3 router-credit block (HTTP 402) is gone — qa-r5 ran the
real-model scenarios on `ti` successfully (`r5-model-picker-modal`, `rewind-real`); the old
blocked pair (`image-attach`, `subagent-real`) was never re-run (VF-3).

### 1b.10 `[unipi-live]`: every unipi package driven live

One driver scenario per package (`cargo xtask gui suite unipi`), driven on `ti` (Xvfb :93,
lavapipe, release build) against a real provider (`oino/deepseek/deepseek-flash`), asserting
rendered panel text, the element tree and `state.chrome`/`state.open` — no OCR. Harness work
that made the surfaces assertable: OUS text nodes, `state.chrome`, `state.open`, result-card
body, modal titles, AgentRow, completion menu, toasts, board inputs, composer chips, `+`
chords, scenario `settings`, `chrome`/`chrome_absent`/`open` assertions.

Defects found live and fixed: `/regular` leaving long-horizon mode on; `ti` never building
`oino-kanboard`; driver injecting chords as literal text.

Open gaps (recorded, not fixed): 18. MCP has no live server on `ti` (harness e2e covers the
tool paths). 19. MemPalace not installed on `ti` (markdown-only; verified separately,
[§1b.5](#1b5-mempalace-rs-the-rust-port)). 20. LLM compaction needs a context-sized session
(the scenario covers the surface). ~~17. board SSE refresh latency~~ fixed by round 5 #7
(in-process watcher refresh; `r5-board-inprocess` proves CLI writes appear). ~~21. Mobile/TUI
not in this suite~~ mobile is round 6 (MB-2); the TUI is frozen.

## 2. End-to-end smoke (release `oino`, temp agent dir, bundle)

Re-checked on **1.0.0-alpha.5** (2026-10-30, local release build): `oino --version`,
`oino-kanboard --version`, `oino-desktop --version` → `1.0.0-alpha.5`; faux print run →
`Hello from faux.` (`scripts/install-smoke.sh` in the gates, plus every host's smoke.txt);
`oino-desktop --demo` verified through `mise run start` on a private Xvfb :97 (rendered
screenshot, §1). The table below is the round-2 record and stays valid.

| Area | Result | How |
|---|---|---|
| print | ✅ `Hello from faux.` | `-e extensions/dist/oino.fixture.provider-faux --model faux/faux-1 -p hi` |
| JSON | ✅ header + full event stream; unipi tools (`run_subagent`, `memory_*`, `web_search`, `image_*`, `plan_submit`, …) in the request | `--mode json` |
| RPC | ✅ 17/17 checks | `_oino/hello`, `prompt`, `get_commands`, `get_messages` |
| panel button action | ✅ | `/info` → `openPanel` → `ui_action refresh` → `close` |
| settings write | ✅ | `_oino/get_settings_schema` (25 namespaces), `set_extension_setting` round-trip |
| btw | ✅ | `/btw …` answers in the panel; session message count unchanged |
| long-horizon continuation | ✅ | `/goal-start {objective}` + prompt → `unipi:lh-continue` nudge |
| kanboard nudge | ✅ | `oino-kanboard` CLI: onboard, add, autowork → `unipi:kanboard-next` nudges (in-process since round 5) |
| ACP | ✅ | `initialize`, `session/new`, `session/prompt`, `end_turn`, `usage_update`, `available_commands_update`; `mcpServers` sticky topic |
| TUI (frozen, charter D3) | ✅ compiles; pty smoke from round 2 | typed prompt, reply, chrome, `/quit` — no feature work, deletion candidate |
| real provider | ✅ | Oino Router: `oino/deepseek/deepseek-flash` print with a `bash` call (ti suites) |
| subagents (real) | ✅ | `run_subagent` foreground → child answered; `child:Subagent` usage |
| fusion (real) | ✅ | sidekick tool → persistent child answered; `child:Sidekick` usage |
| install | ✅ | `scripts/install-smoke.sh` (source install + faux prompt) |

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
| Built-in tools `read write edit bash grep find ls` (+`powershell`) | ✅ | Windows paths untested (VF-2); image bytes differ from Photon |
| Bash detach / background adoption | ✅ | `control.bash-detach`, `tasks.adopt-bash` |
| Resources: AGENTS.md / SYSTEM.md / APPEND_SYSTEM.md, skills, prompt templates, trust | ✅ | |
| Themes | 🟡 | custom theme files discovered, only built-in themes rendered (DG-4) |
| System prompt (sections, docs section, D13) | ✅ | docs root = bundled `oino.docs` |
| Settings manager + extension settings registry | ✅ | schema, layers, scoped writes, secret fields, change events, `oino:settings:<ns>` topics |
| Models: registry, `models.json`, `auth.json`, scopes, virtual models | ✅ | dynamic catalogs refreshed at startup (not in RPC background — HC-8) |
| Providers (pi-ai wires) | ✅ | 8 components; 165 pi oracle fixtures |
| Provider transports / auth edge cases | 🟡 | WebSockets ❌, Anthropic federation ❌, Bedrock SSO/IMDS ❌ (PA-2…PA-5) |
| OAuth `/login` `/logout` | 🟡 | flows implemented + fixture-tested; **never run against live accounts** (VF-1) |
| CLI flags and help | ✅ | `--session-id` creates like pi; `--remote` refuses (CM-2); `--browser` ❌ (CM-7) |
| Print / JSON modes | ✅ | goldens `print`, `json-mode` |
| RPC mode + extension UI sub-protocol | ✅ | golden `rpc`; `_oino/list_sessions` + `_oino/get_keybindings` ❌ (CM-6) |
| `_oino/*` frontend commands | ✅ | completions, ui_action, editor_event, shortcuts, settings, service_call, auth, topics, bus_get, agents, queue_edit, checkpoints/rewind, running_bash, extensions, render |
| Interactive TUI | ⛔ frozen | charter D3: no feature work, deletion candidate; round-2 chrome still works (compiles, pty smoke) |
| Package manager + `oino update` | ✅ | dir/archive/https/git; self-update from `release-manifest.json`; `auth`/`ext`/`sessions`/`mcp` stubbed (PA-1/7, CM-4) |
| Extension API (pi `ExtensionAPI`) via WIT | 🟡 | events, tools, commands, shortcuts, flags, renderers, providers, virtual models, UI, settings r/w, `models.*`, `control.*`, nested agents, turn arbiter, bash detach ✅; provider hooks routing, `models.classify/generate-image`, `set-ui-policy` ❌ (HC-1/2/4) |
| ACP (Zed etc.) | ✅ | protocol v1, permissions, elicitation (forms), client fs/terminal, `--model`, `mcpServers`; refusals ❌ (EX-2); **Zed never tried by hand (EX-1)** |
| `oino serve`, `--remote`, multi-client | ❌ | **RM-1 blocker** — round 6 (see §0) |
| `/share`, `/bug`, pi telemetry, pi-mcp, codemode | ⛔ | out of scope (charter; MCP is `unipi.mcp`) |

## 4. Parity matrix — unipi 3.0.0

**20 packages are ported and bundled** (load order `unipi_kit::suite::LOAD_ORDER`) plus
Oino-native cron (`oino-cron`, extension `unipi.cron`, `oino cron` CLI, `/cron`, Automations
page). Round 5 (D1) **deleted** the presentational packages `unipi.footer`,
`unipi.command-enchantment`, `unipi.input-shortcuts`, `unipi.app-bridge` and the `hints`
half of `unipi.core` (round5.md §3) — their rows are removed from this table and must not
come back. Not ported by decision (D22): browser skills/state, app push encryption, Herdr,
the unipi desktop app. Since `unipi-native`: feature commands have plain names, `/settings`
and `/changelog` are core built-ins, the Fusion picker is `/fusion` and the model-picker
modal (round 5 #1); no legacy unipi support (no `~/.unipi` reads, imports, migrate prompts,
legacy argument shapes, renderers or settings keys).

| Package | Status | Missing / next step |
|---|---|---|
| core (work tray) | ✅ | hints/start-card deleted (D1); native welcome is the desktop's |
| utility | ✅ | settings, auto-rename, `/answer`, attachments, image tools, `/move`, doctor; render styles for built-in tools ✅; trust.json move (U5) open |
| workflow (permissions, plan mode) | ✅ | mode and rules in settings files; the composer mode switcher is round5 #2 |
| long-horizon | ✅ | goal continuation through the arbiter; mode status prints as a transcript row (qa-r5-fix D-2) |
| memory | ✅ | markdown memories, tools, recall, side-save; MemPalace via `mempalaceBin` (both implementations verified, §1b.5) |
| dream | ✅ | nested-agent dream runs with resume (e2e) |
| skill-registry | ✅ | |
| info-screen | ✅ | dashboard panel + actions verified in the smoke; splash greeting local (GA-12 closed) |
| subagents | ✅ | foreground/background/read verified; UI policy fixed at spawn (R2); `set-ui-policy` open (HC-4) |
| background-tasks | ✅ | tasks, tray, notifications `deliverAs: followUp`, bash adoption |
| btw | ✅ | verified in the smoke |
| web-api | ✅ | no TLS fingerprinting (`impersonated: false`, PA-5) |
| ask-user | ✅ | forms map to ACP `elicitation/create`; `custom`/clipboard dialogs stay cancelled |
| mcp | ✅ | stdio + HTTP ✅; pi `~/.oino/mcp.json` preopen; ACP `mcpServers` forwarded |
| notify | ✅ | wizards and secrets in settings |
| kanboard | ✅ | **in-process since round 5 #7** (daemon/SSE/token deleted, `bb853e3`): native Board page + side phone layout + drag, CLI `oino-kanboard` against the same files, watcher refresh picks up CLI/agent writes; `/kanboard status` names the board again (qa-r5-fix D-6) |
| compactor | ✅ | `llm` method passes the previous summary (`summarize-default`) |
| updater | ✅ | never installs (by design); `/changelog` falls back to GitHub release notes |
| cron (Oino-native) | ✅ | scheduler crate + daemon + user service install, `/cron` + `cron_*` tools + skill + settings namespace, Automations page; `scripts/round3/cron-e2e.sh` |
| fusion | ✅ | sidekick handoff verified; startup pair persisted via `control.set-default-model`; the modal picker owns fusion picking now (round5 #1) |
| watchdog | ✅ | stuck detection + session caps + kill / background; process stats on all three OSes |

## 5. Parity matrix — ZCode design (design.md §8, desktop + mobile)

Source of truth: `apps/oino-desktop/DESIGN-STATUS.md`, `apps/oino-mobile/DESIGN-STATUS.md`,
[design/review-desktop.md](design/review-desktop.md). Desktop after round 5:
**52 ✅ · 1 ⛔ (#50) · 1 n/a (#51 mobile)**. **Mobile lags the desktop by a full round**
(MB-2 — see §0): no mode switcher / model-picker modal / new-chat page / Files+LSP, and the
old composer footer survives.

| Group (design.md §8 rows) | Desktop | Mobile | Gaps |
|---|---|---|---|
| Theme, type, spacing, icons (1–5) | ✅ | ✅ | unipi warm OLED dark + derived light |
| Motion (6) | ✅ | 🟡 | mobile lacks exit animations |
| Frames, sidebar, header, welcome (7–13) | ✅ | ✅ | tablet layout on mobile ❌ (MB-6) |
| Timeline, tool rows, diffs, bash, thinking (14–22, 26) | ✅ | ✅ (touch rows) | |
| Subagent rows (23) | ✅ | 🟡 | mobile drawer rows show task + result only |
| Turn undo / rewind (24–25) | ✅ | 🟡 | mobile keeps Edit & resend only |
| Composer, model menu, context ring, mentions, history (27–30, 32, 34) | ✅ | 🟡 | mobile predates round-5 composer (MB-2) |
| Attachments (31) | ✅ | 🟡 | |
| Queue (33) | ✅ | 🟡 | |
| Permission, guard, elicitation, plan, waiting badge (35–39) | ✅ | ✅ | |
| Status panel (40) | ✅ | 🟡 | |
| Side pane, browser, PTY terminal (41–43) | ✅ | ✅ browser; terminal ❌ mobile | |
| Command center, modifiers, settings + hub, toasts, split panes, board (44–49) | ✅ | 🟡 | board phone layout landed desktop-side (round 5 #10) |
| Performance (52) | ✅ | 🟡 | desktop 4-pane run p99 4.1–9.9 ms ≤ 10 budget, idle CPU 2.0 % (ti lavapipe); mobile 120 Hz not measured (MB-8); real GPU pending (VF-4) |
| i18n, window chrome (53–54) | ✅ | ✅ | Windows/macOS rendering not visually checked (DG-8) |
| Remote attach (mobile M10, D16) | — | ❌ UI-only | **RM-4** — pairing stored, token discarded, needs `oino serve` (RM-1); §0 |
| unipi surfaces (design.md §7) | ✅ | 🟡 | mobile still codes deleted D1 surfaces (MB-2) |

## 5b. Browser control ([browser-control.md](browser-control.md), work id `browser-impl`)

Round-3 outcome in brief: [§1b.3](#1b3-browser-control-browser-controlmd). Open items:
WP5 leftovers (IME, file drag-drop, site-permissions popover, Fit-to-pane, Settings →
Browser section — DG-3); WP6 mobile is round 6's; ambient context (EX-5); `--browser`
CLI flag (CM-7). Verified end to end on `ti` and locally (Xvfb); no browser/Xvfb process
left after runs.

## 6. Known issues

1. **Bash detach** is wired; hosts without the WASM host task service report `unsupported`.
2. **Shutdown**: RPC, TUI and print/JSON exit once idle; **ACP does not** (client
   lifecycle) — deviations/modes #11 narrowed to ACP only.
3. **Nested agents**: `UiPolicy` fixed at spawn (no `set-ui-policy`), child depth not on
   `call-context`, `oino_agent` events reach only the spawning extension (HC-4).
4. **Remote**: `oino serve` / `--remote` do not exist — mobile remote attach and a remote
   client are unavailable. **§0 / RM-1** (round 6).
5. **OAuth** sign-in is verified only with the faux provider and recorded fixtures (VF-1).
6. **First start** after an update compiles ~24 MB of WASM (≈4 s on 32 cores, longer on
   laptops); later starts use `~/.oino/cache/wasmtime` (PR-6).
7. `models.classify` / `generate-image` unsupported; TUI inline images are placeholders
   (HC-1).
8. `/tmp` quota on the dev machine: run builds/tests with `TMPDIR=$PWD/target/tmp`.
9. **iOS devices** cannot JIT: components need wasmtime Pulley (not wired); iOS is
   simulator-only (MB-5).
10. **macOS EMFILE**: `oino` does not raise its open-file limit; under the 256-descriptor
    soft limit of SSH/nohup shells every bundled component fails to init (`os error 24`).
    Works with `ulimit -n 4096` (PR-5).
11. ~~Desktop perf~~ **closed** (`[zcode-perf]` + round 5): 4-pane run p99 4.1–9.9 ms,
    budget 10; idle CPU 0.8–2.0 %; threads/fds flat (ti + lavapipe numbers, qa-r5 §5).
    Real-GPU numbers pending `ti` (VF-4).
12. **Release artifacts** in §7 are 1.0.0-alpha.5 (2026-10-30, `966e2fb`), built locally
    (Linux CLI+desktop+AppImage/deb) and on `coffee` (NSIS); `desmondgiam` was
    **unreachable** this round (asleep — charter says skip). Nothing is signed, tagged or
    published (PR-1/PR-2); the CI release jobs exist but were never exercised.
13. ~~GA-12 splash greeting UTC~~ **closed 2026-10-29** (harness-gaps #7:
    `host-info.local-offset-minutes` → `host::local_hour`; splash greeting now local).
    Round-3 QA defects QA-1…QA-7 + GA-45 are fixed ([gui-qa.md](gui-qa.md) Resolution);
    round-5 defects D-1…D-8/D-10 are fixed ([qa-r5.md](qa-r5.md) §8); D-11…D-16 stay with
    their owners (P3).
14. **`--session-id`** (fixed 2026-10-29): a missing id creates the session with that
    header id; `--fork` + `--session-id` stays unsupported (HC-10).
15. **MemPalace**: the Rust port is 99.7% parity but **not switched** (owner action,
    §1b.5); maintenance commands, `mempalace-light-mcp`, coordination tools and hook saves
    stay on Python. Rust daemon 260–360 MB under load; bulk mines ~1.5× wall time at
    default threads. Turn *MemPalace auto-update* off in both callers before switching.
16. **`ti`**: Tailscheck approved — reachable again; the remaining `ti` items are the
    AppImage/deb packaging and real-GPU perf (PR-3).

## 6.1 gui-audit area E (core-gui-gaps)

| Id | Status | Core / backend side ([frontend-api.md](frontend-api.md)) | GUI side still open |
|---|---|---|---|
| GA-40 | ✅ | `InProcessBackend` runs commands on its runtime when awaited elsewhere | C1: drop the `Tokio::spawn` workaround if wanted |
| GA-41 | ✅ (rescoped) | host finds bare programs next to the running executable | board is in-process since round 5 (#7); shipping `oino-kanboard` beside `oino-desktop` matters for **CLI users** only |
| GA-42 | ✅ | handler deadlines pause while a dialog is open | C2: drop cards on `extension_ui_dismiss` |
| GA-43 | ✅ | core fires retained `setShortcuts` | S3/GA-16 closed (palette + settings pages bind keys) |
| GA-44 | ✅ | `_oino/agents`, `agents_changed`, `agent_event`, `_oino/agent_messages` | P3/GA-15 closed (Agent tab timeline) |
| GA-45 | ✅ | `ExtensionMode::Desktop`/`Mobile`; `host-info.frontends` | none — verified via `/doctor` |
| GA-46 | ✅ | no refresh loop when idle | measured again in qa-r5 §5 (idle CPU 2.0 %) |

## 7. Release builds (1.0.0-alpha.5, 2026-10-30, `v1-alpha` @ `966e2fb`, not tagged or published)

Local builds (this machine, `nice -n 10`, `-j 4`):

```
TMPDIR=$PWD/target/tmp cargo build --release -p oino -p oino-kanboard
cd apps && TMPDIR=../target/tmp cargo build --release -p oino-desktop
```

Host builds use `cargo xtask remote-build <host|local> [opts] [-- build-host args]`
(`xtask/src/remote.rs`): HEAD synced as a `git bundle` into `~/oino-release` (detached
checkout) + an `extensions/dist` tarball (no wasm toolchain needed on the hosts);
`scripts/release/build-host.sh` runs detached and is polled; artifacts fetched to
`target/release-artifacts/<target>/`.

Commands: `cargo xtask remote-build local --jobs 4 -- --formats appimage,deb` (Linux,
this machine — the `ti` AppImage/deb run is the PR-3 remainder, ti was used for the suites
instead); `cargo xtask remote-build coffee --os windows --jobs 4 --timeout-mins 75 --
--formats nsis` (~13 min on the host). **`desmondgiam` (macOS) was skipped: unreachable**
(SSH connect timeout, host asleep — two attempts; charter says skip); its `~/oino-release`
still holds the alpha.4 outputs for reuse. `ti` was reachable again this round (Tailscale
approved): r5 suite + suites/soak/perf ran there, but the packaging re-run stayed local.

| Target | Artifact | Size | Smoke |
|---|---|---|---|
| x86_64-unknown-linux-gnu (local) | `oino` | 84.7 MiB | ✅ 5/5 (`--version` ×3, faux print 31 packages, desktop `--demo` + 97 KB screenshot) |
| | `oino-kanboard` | 2.6 MiB | |
| | `oino-desktop` | 143.5 MiB | |
| | `oino-desktop-1.0.0-alpha.5-linux-x86_64.tar.gz` | 80.6 MiB | |
| | `oino-desktop_1.0.0-alpha.5_amd64.deb` | 49.1 MiB | |
| | `oino-desktop_1.0.0-alpha.5_x86_64.AppImage` | 116.1 MiB | |
| x86_64-pc-windows-msvc (`coffee`) | `oino.exe` | 67.3 MiB | ✅ 4 + 🟡 1 (desktop alive; locked console — known since round 3) |
| | `oino-kanboard.exe` | 2.0 MiB | |
| | `oino-desktop.exe` | 96.2 MiB | |
| | `oino-desktop_1.0.0-alpha.5_x64-setup.exe` (NSIS, per-user) | 29.0 MiB | |
| x86_64-apple-darwin (`desmondgiam`) | — | — | **skipped: host unreachable** (asleep); `.app`/`.dmg` remain at alpha.4 on the host |

Smoke details: `target/release-artifacts/<target>/smoke.txt` + SHA256SUMS + build-info.txt
per target. Problems found this round: none (the alpha.4 Windows `cfg(unix)` fix holds;
the ACP mcp-servers flake and 5 rustdoc drifts were fixed before the builds, §1). Still
open: no signing/notarisation (PR-1), nothing tagged/published (PR-2), no mobile build
(MB-1), `create-dmg` Finder fallback note is moot until the mac is back.

## 8. Next steps

0. **Round 6 = remote anywhere + mobile catch-up** (owner direction, [round6-direction.md](round6-direction.md)):
   write the serve/iroh spec (superseding modes.md §5) → implement `oino serve` +
   `SessionHub` + `RpcBackend` (RM-1/RM-3/RM-6) → opt-in daemon with Settings consent +
   device revoke (RM-8) → phone attach for real (RM-4) and desktop host-switcher attach
   (RM-5); in parallel, mobile UI catch-up to round-5 desktop + rebuild APK (MB-1/MB-2).
1. **`ti` leftovers** (PR-3): `remote-build ti@ti --dir oino-test/oino -- --formats
   appimage,deb`, real-GPU perf (`SOAK_MAX_FRAME_MS`), re-run of the router-blocked pair
   (`image-attach`, `subagent-real`).
2. **core-runtime**: `agents.agent.set-ui-policy` + depth on `call-context`;
   `control.move-project-state` and tmp GC (HC-5).
3. **release**: raise `RLIMIT_NOFILE` at `oino` startup (PR-5); signing/notarisation
   (PR-1); tag + exercise the CI release jobs (PR-2); macOS universal (PR-4); mobile
   rebuild (MB-1), release-key APK signing (MB-7).
4. **verification**: live OAuth (VF-1, owner-assisted); Windows tool paths (VF-2); golden
   e2e suite `tests/e2e/` + `PARITY.md` (VF-6); criterion benches (VF-7); `cargo xtask
   layering` / `cargo deny` (VF-8).
5. **wit-v1 / host**: wasmtime Pulley for iOS (MB-5); TLS fingerprinting (PA-5).
6. **gpui**: browser WP5 leftovers (DG-3); custom theme rendering (DG-4); keyboard/P2
   leftovers (DG-5/6); automation gaps (DG-7).
7. **acp**: refusal stop reason (EX-2); tool kinds (EX-3); `custom`/clipboard dialogs
   (EX-4); try Zed by hand (EX-1).
