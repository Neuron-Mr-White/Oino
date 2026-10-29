# Oino v1.alpha — gap audit

Date: 2026-10-30 · branch `v1-alpha` · audited at HEAD `9b447a0` (round-5 WPs + `[qa-r5]`
report landed; `qa-r5-fix` / `round5-integration` (alpha.5) still pending — rows marked
*in flight* may flip as those land). Method: CHARTER Vision + round5-feedback D1–D4 +
every spec in `docs/dev/v1/*.md` and `deviations/*` compared against the code at HEAD;
stub/refusal greps over `crates/ apps/ extensions/` (no `target/`, no `apps/vendor/`);
cheap live checks (`./target/release/oino serve|auth|ext|sessions|mcp`, `--remote`,
`--browser`) — never on `DISPLAY=:0`.

Why this file exists: the owner found that the phone's remote connection (no `oino serve`,
no remote attach, no mobile rebuild since alpha.1) was **never surfaced in any round
summary**. This is the one complete, honest list.

## 1. One-screen summary

| Area | ⛔ Blocker | 🔴 Major | 🟡 Minor | Total | Biggest hole |
|---|---|---|---|---|---|
| Harness core (pi 1.4.0 parity) | 0 | 0 | 12 | 12 | `models.classify`/`generate-image`, provider-hook routing, `set-ui-policy` |
| Providers / auth | 0 | 0 | 8 | 8 | OAuth never run against live accounts (counted under Verification) |
| CLI modes | 0 | 1 | 6 | 7 | `--remote` refuses; `auth`/`ext`/`sessions`/`mcp` stubbed |
| **Remote / multi-device** | **2** | **5** | **1** | **8** | **`oino serve`/SessionHub does not exist; no iroh code; round6-direction only** |
| Desktop GUI | 0 | 2 | 8 | 10 | 2 P1 defects + round-5 closure (suites red, alpha.5 bump) |
| Mobile | 0 | 4 | 6 | 10 | No build since alpha.1; UI misses all round-5 changes; iOS device never built |
| Extensions / ACP | 0 | 1 | 5 | 6 | Zed never tried by hand; ACP refusals/tool kinds |
| Packaging / release | 0 | 3 | 5 | 8 | Nothing signed, tagged or published; `ti` re-run outstanding |
| Verification | 0 | 1 | 7 | 8 | OAuth live; Windows paths; real GPU; golden e2e suite |
| **Total** | **2** | **17** | **58** | **77** | |

Top 10 (owner-visibility weighted): RM-1, RM-2, RM-4, MB-1, MB-2, RM-5, MB-6, PR-1, VF-1, RM-8.

Post-audit flips: **DG-1 and DG-2 (Desktop GUI majors) closed** on 2026-10-29/30 by
`[qa-r5-fix]` + `[round5-integration]` (see §2.5) — majors now 15, and `ti` is reachable
again (Tailscale approved), so PR-3 shrank to the packaging/GPU re-run. STATUS.md §0 is
the living copy of this summary.

## 2. Full gap list

Severity is **for a GUI-first alpha** (blocker = breaks a promised product capability,
major = promised capability missing/broken or unverified promise, minor = parity/polish).
"Surfaced" = where it appeared before: **yes** (STATUS top sections / round summaries),
**buried** (deep matrix row, known-issue list, deviations file or QA report only),
**no** (nowhere).

### 2.1 Harness core (HC)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| HC-1 | `models.classify` / `models.generate-image` host ops unimplemented (default stubs); `models.*` usage not recorded as `ext:<id>` entries | `crates/oino-core/src/extensions/host.rs:425`; deviations/core-runtime O4; STATUS §3 + known issue 7 | 🟡 | yes | core-runtime, next rounds |
| HC-2 | Provider hooks not routed to guests: `before_provider_request` / `after_provider_response` / `provider_stream_event`; virtual-model `route` reason (`direct` only); tool-result / prompt image normalisation; `project_trust` event | deviations/core-runtime O6; audit-pi §6 rows | 🟡 | buried | core-runtime |
| HC-3 | pi cache warming (runtime §14) and command output capture (runtime §7.4) not implemented (decision hook scaffolding only) | deviations/core-runtime O6; `crates/oino-core/src/extensions/runner.rs:851` | 🟡 | buried | core-runtime |
| HC-4 | Nested agents: `agents.agent.set-ui-policy` (subagents Ctrl+B, fusion non-blocking handoffs), nesting depth on `call-context`, `oino_agent` events reach only the spawning extension | deviations/core-runtime O2, unipi-agents R2; STATUS known issue 3 | 🟡 | yes | core-runtime |
| HC-5 | `control.move-project-state` (trust.json + sessions dir, U5) and tmp GC (U6) missing | deviations/core-runtime O7; STATUS §8.1 | 🟡 | buried | core-runtime |
| HC-6 | Built-in tools keep the first session's `SessionToolContext` after `new_session` / `fork` / `switch_session` | deviations/modes #19 | 🟡 | buried | core-runtime |
| HC-7 | Core loader ignores `disabledExtensions` (desktop filters packages itself); file checkpoints + single-item queue commands landed, but core-side list drifts from GUI assumptions | deviations/core-runtime O9 | 🟡 | buried | core-runtime |
| HC-8 | RPC: no background model-catalog refresh; bus data publish through frontend API (acp #2); `ToolInfo.kind` (acp #7) | deviations/core-runtime O8 | 🟡 | buried | modes / frontend-cmds |
| HC-9 | `--offline`: `OINO_OFFLINE` not exported to the process env | deviations/modes #17 | 🟡 | buried | modes |
| HC-10 | `--fork` with `--session-id` rejected | `crates/oino/src/startup.rs:201`; STATUS known issue 14 tail | 🟡 | yes | modes |
| HC-11 | `route-model` extension export: SDK/host scaffolding exists but **no bundled component implements it** (core default replies "not implemented") | `crates/oino-core/src/extensions/api.rs:481`; grep of `extensions/` → 0 implementers | 🟡 | no | wit-v1 / core-runtime |
| HC-12 | `oino-tui` frozen by charter D3 but still a live workspace member: inline images are `[image]` placeholders, `LocalHost` writes settings outside `_oino/set_setting`, no deletion performed | charter Vision 2; deviations/tui-client open; `crates/oino-tui/` present | 🟡 | buried | owner decision, post-alpha |

### 2.2 Providers / auth (PA)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| PA-1 | `oino auth` CLI subcommand stubbed ("not available in this build yet") — pi parity; GUI/TUI sign-in exists | `crates/oino/src/lib.rs:148`; live: `./target/release/oino auth` | 🟡 | yes | modes |
| PA-2 | WebSocket transports (`StreamOptions.transport`) ignored — OpenAI/Anthropic/Codex SSE only | deviations/providers-v1 open; M1 | 🟡 | buried | providers, next rounds |
| PA-3 | Anthropic workload-identity federation (`ANTHROPIC_FEDERATION_*`) missing | deviations/providers-v1; STATUS §3 | 🟡 | buried | providers |
| PA-4 | Bedrock SSO/credential_process/assume-role/IMDS chains; Vertex external-account ADC; provider credential env not stored; Cloudflare "not configured" check (M4/M6/M7/M10) | `extensions/providers/README.md` M4–M7, M10 | 🟡 | buried | providers |
| PA-5 | TLS fingerprinting: `impersonate` accepted but served by reqwest (`impersonated: false`); no `wreq`; web-api has no fingerprinting | deviations/wit-v1 O1, unipi-integrations R2 | 🟡 | buried | wit-v1 |
| PA-6 | Provider host e2e scripts (`scripts/pi-oracle/providers-{v1,more}/host-e2e.rs`) not committed as `crates/oino-wasm-host/tests/providers.rs` | deviations/providers-v1 open | 🟡 | no | providers |
| PA-7 | `oino mcp` CLI stubbed (MCP works via `unipi.mcp` extension only) | `crates/oino/src/lib.rs:148`; live check | 🟡 | yes | modes, next rounds |
| PA-8 | OAuth live-account verification missing (single entry counted under Verification) | deviations/providers-v1; STATUS known issue 5 | → VF-1 🔴 | yes | verification round |

### 2.3 CLI modes (CM)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| CM-1 | `oino serve` subcommand stubbed (whole remote stack under RM-1; counted there) | `crates/oino/src/lib.rs:148`; live: `oino serve` → "not available in this build yet" | → RM-1 ⛔ | buried | **round 6** |
| CM-2 | `--remote <url>` refuses in interactive mode ("not available in this build") | `crates/oino/src/lib.rs:247-252`; live check | 🔴 | buried (known issue 4) | round 6 |
| CM-3 | `--remote` is **silently ignored** in print/JSON mode (preflight only guards interactive; a local prompt runs instead) | `crates/oino/src/lib.rs:247`; live: `oino --remote oino://x:1?token=t -p hi` ran a local turn | 🟡 | no | round 6 (serve) |
| CM-4 | `oino ext` and `oino sessions` subcommands stubbed (package manager covers install/remove/update/list only) | `crates/oino/src/lib.rs:148`; live checks | 🟡 | yes | modes |
| CM-5 | ACP mode does not exit on `shutdown_requested` (client lifecycle) | STATUS known issue 2; deviations/modes #2 | 🟡 | yes | acp |
| CM-6 | `_oino/list_sessions` and `_oino/get_keybindings` RPC commands not implemented (modes §4/R row; note `_oino/render`, `_oino/topics`, `_oino/agents` now exist — deviations row is stale) | grep `_oino/*` dispatch; deviations/modes R | 🟡 | buried | modes / frontend-cmds |
| CM-7 | `--browser` flag / `oino browser …` CLI surface missing (`OINO_BROWSER` env is the only switch) | live: `oino --browser off -p hi` → "Unknown option: --browser"; STATUS §5b WP3 | 🟡 | yes | browser-impl |

### 2.4 Remote / multi-device (RM) — the cluster the owner could not see

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| RM-1 | **`oino serve` + `SessionHub` + hub protocol do not exist at all**: no `_oino/open_session/attach/detach/close_session/hub_sessions`, no TLS/token/pairing server, no multi-client fan-out, no first-answer-wins, no attach replay. `oino-client` has only `FakeBackend`/`InProcessBackend` — no `RpcBackend` (modes §8 requires contract tests against it) | modes.md §5, §8; grep: 0 hits for `SessionHub`/`_oino/attach`/`RpcBackend` in `crates/ apps/`; `crates/oino-client/src/` = fake.rs + in_process.rs; deviations/modes #20 | ⛔ | buried (STATUS §3 one ❌ row + known issue 4 + §8 next steps; **never in a round summary table**) | **round 6** (spec + impl) |
| RM-2 | **No iroh / peer-to-peer anywhere**: 0 grep hits for `iroh` in code, specs or Cargo manifests. modes.md §5 specifies TCP+TLS+token, which the owner does not want. `round6-direction.md` (commit `9b447a0`, minutes before this audit) decides iroh + opt-in daemon and supersedes §5 — but there is **no spec and no code yet**, and modes.md §5 is now stale | grep `iroh` → 0; modes.md §5.1; docs/dev/v1/round6-direction.md | ⛔ | no (direction only landed 9b447a0) | round 6 (spec first, then impl) |
| RM-3 | Remote TUI/GUI attach (`--remote`, host switcher → served host) blocked by RM-1 | `crates/oino/src/lib.rs:247`; deviations/modes #13 | 🔴 | buried | round 6 |
| RM-4 | **Mobile remote attach is UI-only**: hosts list + "Pair a desktop" page store a pairing (`oino://host:port?token=…`), but the token is **discarded** (no keystore), hosts show "not connected", sessions run embedded on the phone | `apps/oino-ui/src/mobile/hosts.rs:7,47-48,119`; mobile DESIGN-STATUS MG8 + deviations §6.10; `apps/oino-ui/src/mobile/shell.rs:230` | 🔴 | buried (STATUS §5 mobile row 🟡; never in round summaries) | round 6 |
| RM-5 | **Desktop host switcher remote pick is inert**: stores the selection and toasts that the app keeps running locally | `apps/oino-ui/src/components/host_switcher.rs:9-11, 93-101` (round5 #12); qa-r5 #12 ✅ with "honest toast" | 🔴 | buried (reads ✅ in qa-r5) | round 6 |
| RM-6 | No `RpcBackend` transport in `oino-client` (folded evidence of RM-1; separate work item: the client side) | `crates/oino-client/src/{fake,in_process}.rs`; modes §3.3/§8 | 🔴 | buried | round 6 |
| RM-7 | Offline notifications (ntfy / UnifiedPush, E2E-encrypted payloads) — promised by round6-direction §7, nothing exists | round6-direction.md §7 | 🟡 | no | round 6+ |
| RM-8 | Opt-in remote daemon: Settings "Remote access" toggle with consent prompt, per-OS service install (systemd user + linger / LaunchAgent / Windows startup task), paired-devices list with revoke — all missing (cron's user-service install is the only precedent) | round6-direction.md §1, §4, §5; nothing in `apps/oino-ui/src/views/settings_page.rs` | 🔴 | no | round 6 |

### 2.5 Desktop GUI (DG)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| DG-1 | Round-5 P1 defects at HEAD: D-1 New-task dialog Title not focused (typed text lost), D-2 composer loses keyboard focus after a mode-command result card | ✅ **closed 2026-10-29/30 by `[qa-r5-fix]`** (`925b6cd`: repair_focus guard while a field-owning dialog is open; long-horizon mode status as transcript row instead of the keyboard-owning modal) — qa-r5.md §8 | ~~🔴~~ ✅ | yes (qa-r5) | closed |
| DG-2 | Round-5 closure open: fmt red in `oino-kanboard` (D-5), `zcode` suite manifest references deleted scenario (D-3), `unipi` suite 17/24 (stale selectors D-4/D-7), `/kanboard status` stale copy (D-6), spec'd `r5-board-modal-form`/`r5-board-fulltext`/`r5-scroll-containment` scenarios + `suites/r5.json` missing (D-9), `SETTINGS_*` dead-constant prune, alpha.5 version bump + STATUS refresh | ✅ **closed 2026-10-30 by `[qa-r5-fix]` + `[round5-integration]`** (`966e2fb`: suites/r5.json + the three scenarios; SETTINGS_* pruned; alpha.5 bump + STATUS refresh; suites 24/24 · 12/12 · 5/5) | ~~🔴~~ ✅ | yes (qa-r5) | closed |
| DG-3 | Browser WP5 leftovers: IME input, file drag-drop, site-permissions popover, Fit-to-pane, Settings → Browser section (browser settings live only in `settings.json` `browser.*`) | STATUS §5b WP5; `apps/oino-ui/src/views/settings_page.rs` has no Browser page | 🟡 | yes | browser-impl |
| DG-4 | Custom theme files are discovered/loaded but the GUI renders built-in themes only | STATUS §3 Themes 🟡 | 🟡 | buried | gpui, next rounds |
| DG-5 | Keyboard leftovers: StatusPanel / Queue full key tables, DiffViewer `u`, tooltips on keyboard focus | deviations/gpui-desktop open | 🟡 | buried | gpui |
| DG-6 | Review-P2 leftovers: secret reveal toggle, DiffViewer line numbers / per-file actions / split view, compact-now confirmation, updater-driven update badge, skeleton loaders | deviations/gpui-desktop open (zcode-perf closed the rest) | 🟡 | buried | gpui |
| DG-7 | Automation harness gaps: H-6 panel-body OUS nodes never register in the element tree; D11 synthetic wheel-scroll unreliable; no mouse-drag bound assertion (D-11); no touch / multi-window / webview-content automation | STATUS §1a harness gaps; qa-r5 D-11 | 🟡 | yes | gui-driver |
| DG-8 | Windows rendering never visually checked (locked console), macOS rendering never checked over SSH; i18n spot-checks pending | STATUS §1b.7 items 3–4, §8.0 | 🟡 | yes | verification |
| DG-9 | `_oino/service_call` has no native GPUI consumer (its consumer, app-bridge, was deleted by D1) | deviations/frontend-cmds open | 🟡 | buried | frontend-cmds |
| DG-10 | Desktop updater never installs (by design) — release feed + update badge never exercised end to end | STATUS §4 updater row; no tagged release exists (PR-2) | 🟡 | buried | release |

### 2.6 Mobile (MB)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| MB-1 | **No mobile build since alpha.1**: newest artifacts are `oino-mobile-1.0.0-alpha.1-{debug,release}.apk` + iOS-simulator `.app` (Oct 26–27); everything after alpha.1 (browser control, cron page, round-5 UI) never reached a phone | `apps/oino-mobile/dist/` timestamps; STATUS §7 "no Android build this round" | 🔴 | buried | round 6 (round6-direction §8) |
| MB-2 | **Mobile UI lags the round-5 desktop direction**: no mode switcher (#2), no model-picker modal (#1), no new-chat page (#6), no Files/editor/LSP (#24), no context-in-composer (#3); the old composer keeps the footer strip/context header (#18) — and screens still code surfaces for extensions deleted by D1 (footer segments, hint strip, app-bridge drawer tab), which now render nothing | `apps/oino-ui/src/mobile/` has no r5 equivalents (grep for r5 / round5 / zcode in mobile DESIGN-STATUS → 0 hits); DESIGN-STATUS §7 still lists footer/hints/app-bridge surfaces | 🔴 | no | round 6 (round6-direction §8) |
| MB-3 | gpui-app.md §4 (D16) specifies mobile as **remote-only** (`RpcBackend` over TCP+TLS to `oino serve`, keystore tokens); reality is an embedded core. Divergence never folded as a settled deviation; §4 must be rewritten for round 6 | gpui-app.md §4; deviations/gpui-mobile open; `apps/oino-desktop/src/embedded.rs:409` | 🔴 | buried | round 6 spec |
| MB-4 | Remote attach stub on the phone (pairing stored, never connected; counted under RM-4) | see RM-4 | → RM-4 🔴 | buried | round 6 |
| MB-5 | iOS: simulator-only builds; devices cannot JIT (wasmtime **Pulley not wired**), no `OINO_IOS_TEAM`, no `.ipa` ever built | mobile DESIGN-STATUS platform status; STATUS known issue 9; round6-direction §8 stretch | 🔴 | yes | round 6 stretch / wit-v1 |
| MB-6 | Tablet / landscape (≥720dp) layout missing — phone layout everywhere | mobile DESIGN-STATUS deviations §6; STATUS §5 mobile row | 🟡 | buried | design-mobile |
| MB-7 | Release APK signed with the **debug key** | deviations/gpui-mobile open | 🟡 | buried | release |
| MB-8 | Mobile perf never measured (120 Hz scroll / input-to-paint; test device is 60/90 Hz) | mobile DESIGN-STATUS deviations §10.2 | 🟡 | buried | design-mobile |
| MB-9 | Browser control WP6 (mobile screencast/take-over) not started | STATUS §1b.3 / §5b WP6 ❌ "next rounds" (stale after two rounds) | 🟡 | buried | browser-impl, later |
| MB-10 | Android: second activity in one process panics (`android_main` exits when the activity ends) — app dies on some navigation/restart paths | mobile DESIGN-STATUS deviations; vendor gpui-pre 0.3.10 limitation | 🟡 | buried | gpui-mobile |

### 2.7 Extensions / ACP (EX)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| EX-1 | Zed (or any real ACP client) never tried by hand — ACP is fixture/unit-verified only | deviations/acp #16; STATUS §3 ACP row | 🔴 | buried | acp / verification |
| EX-2 | ACP `stopReason: "refusal"` never produced | deviations/acp #3 | 🟡 | buried | acp |
| EX-3 | ACP extension tool kinds are all `other` (no `ToolInfo.kind`) | deviations/acp #7 | 🟡 | buried | acp |
| EX-4 | `custom` / `clipboardReadImage` / `editExternally` dialogs resolve `cancelled` over ACP | deviations/acp #11 | 🟡 | buried | acp |
| EX-5 | Browser ambient context (page state injected into prompts) not implemented | `extensions/oino/browser/DEVIATIONS.md`; STATUS §5b WP4 | 🟡 | buried | browser-impl |
| EX-6 | `ui.editor-history-push` (composer ↑ history) not implemented | deviations/unipi-ux Q2 | 🟡 | buried | frontend-cmds |

### 2.8 Packaging / release (PR)

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| PR-1 | Nothing signed or notarised: Windows SmartScreen exposure, macOS Gatekeeper (`.app` ad-hoc signed only) | STATUS §7 "Still open"; known issue 12 | 🔴 | yes | release |
| PR-2 | Nothing tagged or published; the CI release jobs (`.github/workflows/ci.yml` `release-binaries`/`release-assets` on `v*` tags) have never been exercised — no tag exists | ci.yml:4-6; STATUS §7 "not tagged or published" | 🔴 | yes | release |
| PR-3 | `ti` reference-host re-run outstanding since 2026-10-29 (Tailscale approval pending): suites + soak/perf on the reference host, AppImage/deb built by local fallback instead; all GUI numbers since are Xvfb+lavapipe | STATUS §7, §8.0, known issue 16 | 🔴 | yes | verification / owner action |
| PR-4 | macOS universal (aarch64) build not done — x86_64 thin only (host disk limits) | STATUS §7 | 🟡 | yes | release |
| PR-5 | `RLIMIT_NOFILE` not raised at `oino` startup — macOS SSH/nohup shells hit EMFILE with every bundled component failing to init | STATUS known issue 10; §8.3 | 🟡 | yes | modes / release |
| PR-6 | First start after an update compiles ~24 MB of WASM (~4 s on 32 cores, longer on laptops) | STATUS known issue 6 | 🟡 | yes | core / release |
| PR-7 | `cargo xtask desktop` / `cargo xtask mobile` (gpui-app §6 build matrix) do not exist — `scripts/release/build-host.sh` + `apps/*/scripts/build.sh` are used; spec drift | gpui-app.md §6; `xtask/src/main.rs` task table | 🟡 | no | gpui-app doc fix |
| PR-8 | `create-dmg` needs a GUI Finder session; `hdiutil` fallback used for the .dmg | STATUS §7 | 🟡 | yes | release |

### 2.9 Verification (VF) — implemented but never verified live

| ID | Gap | Evidence | Sev | Surfaced | Owner / round |
|---|---|---|---|---|---|
| VF-1 | **OAuth flows (Anthropic, Copilot, Codex, ChatGPT) verified only with fixtures + faux provider — never against live accounts** | deviations/providers-v1; STATUS known issue 5, §3 | 🔴 | yes | verification round (owner-assisted) |
| VF-2 | Windows path handling for built-in tools (`read`/`edit`/`bash`/`powershell`) untested (built and smoked on `coffee`, but tool paths not exercised) | STATUS §3 built-in tools row | 🟡 | buried | verification |
| VF-3 | Real-provider GUI residuals: `image-attach`, `subagent-real` were router-credit-blocked (HTTP 402) in round 3; qa-r5's real scenarios now pass — the blocked pair was never re-run | STATUS §1b.9; qa-r5 §2 | 🟡 | buried | verification |
| VF-4 | Perf on a real GPU never measured (lavapipe only; blocked by PR-3 `ti`) | STATUS §1b.7 item 5 | 🟡 | yes | verification |
| VF-5 | Soak excludes real providers (HTTP component) and the embedded browser (no session bus on Xvfb) | STATUS §1b.7 items 6–7 | 🟡 | yes | verification |
| VF-6 | Golden e2e suite `tests/e2e/` + `PARITY.md` (quality §7) not created | quality.md §7.2; no `tests/e2e`, no `PARITY.md` in tree | 🟡 | buried | integration |
| VF-7 | Criterion benches (quality §5) not created | quality.md §5; no criterion in any `Cargo.toml` | 🟡 | buried | integration |
| VF-8 | `cargo xtask layering` and `cargo deny` (quality §1/§6) do not exist | STATUS §1 tail; `xtask/src/main.rs` | 🟡 | yes | integration |

## 3. Stale STATUS / doc statements to correct

1. **STATUS §1b.1 table + §4 matrix** list `footer`, `command-enchantment`, `input-shortcuts`, `app-bridge` as "✅ live" — deleted by D1 (commit `b431416`; qa-r5 D-1). The "24 ✅ live · 0 🟡 · 0 ❌" count and "28/28 suite" are now 20 kept packages + cron, 24 scenarios (+22 `r5-*`).
2. **STATUS §4 kanboard row** says "CLI/daemon, panel, desktop native board … SSE refresh (gap 17)" — the daemon was deleted by round-5 #7 (`bb853e3`); refresh is a file-watch; `/kanboard status` still prints a stale `daemon: not running` line (qa-r5 D-6).
3. **STATUS §5 "Remote attach (mobile M10, D16) 🟡 … needs `oino serve`"** is accurate but was never repeated in §1a/§1b round-summary tables — the reason this audit exists. Round summaries must carry a Remote row (or reference §2.4 here).
4. **STATUS §5b / §1b.3 WP6 "mobile … next rounds"** — stale after round 5 picked up nothing; round 6 now owns it.
5. **STATUS known issue 11** ("four streaming panes do not reliably meet the 10 ms p99 budget") contradicts §1a/§1b.8 (perf PASS, p99 3.9–4.9 ms since `[zcode-perf]`).
6. **STATUS known issue 13 / GA-12** ("stays open until `host-info` grows the local-offset field") — landed 2026-10-29 (harness-gaps #7, §1b.1b row 7); GA-12 is closed by the same evidence as the other R3 rows.
7. **STATUS §1b.9 router-credit blockage (HTTP 402)** — stale: qa-r5 ran real-model scenarios on `ti` successfully (`r5-model-picker-modal`, `rewind-real` on re-run).
8. **STATUS §2 TUI row / §3 "Interactive TUI 🟡"** presents the TUI as a live deliverable; charter D3 froze it (deletion candidate). Mark frozen; browser-control WP7 (TUI) and tui-parity rows in specs are obsolete for the same reason.
9. **STATUS §6.1 GA-41** ("packaging must ship `oino-kanboard` beside `oino-desktop`") — the board is in-process since `bb853e3`; the companion matters for the CLI only; re-scope the row.
10. **STATUS §1b.7 item 10** ("CI release assembly still manual") — `.github/workflows/ci.yml` has tag-triggered release jobs; what is missing is *exercising* them (PR-2), not the assembly itself.
11. **deviations/modes.md row R** lists `_oino/render`, `_oino/topics`, `_oino/agents` as missing — all three exist at HEAD (grep in `crates/`); only `_oino/list_sessions`, `_oino/get_keybindings` remain.
12. **deviations/core-runtime O1 + O5** and **deviations/wit-v1 O3** (bash detach missing / settings bus topic not published) — landed (STATUS §3, harness-gaps #8/#9); fold or delete.
13. **deviations/gpui-desktop** open list: perf numbers 9.3–14.9 ms superseded (4.1–4.9 ms), `_oino/topics`/`_oino/agents` landed; only the keyboard/P2/platform rows remain open.
14. **deviations/modes #11** ("no mode exits on `shutdown_requested`") — RPC/TUI/print do exit (known issue 2); only ACP remains.
15. **modes.md §5 (TCP+TLS+token serve)** — superseded by `round6-direction.md` (iroh, opt-in daemon); the spec rewrite must land in round 6 before implementation.
16. **gpui-app.md §4 (D16 mobile remote-only) + §6 (`cargo xtask desktop|mobile`)** — both diverge from the tree (embedded core; script-based builds); rewrite with the round-6 remote design.
17. **STATUS title/date** ("1.0.0-alpha.4 — status, 2026-10-30") predates the round-5 WPs + qa-r5 at HEAD; the round5-integration refresh (including a Remote/mobile section referencing this file) is pending.
18. **round5-feedback.md status line** ("10/27 verified") — superseded by qa-r5 (27/27 + D1–D3); flip at round close.
19. **mobile DESIGN-STATUS §7** still lists footer / hints / app-bridge surfaces as rendering paths — the extensions are gone (D1); the mobile catch-up (MB-2) must prune these.

## 4. Method notes / non-gaps

- `grep -rniE 'iroh' crates apps extensions docs/dev/v1` → 0 hits before `9b447a0` (round6-direction.md is the only mention now).
- Stub sweep (`TODO|FIXME|todo!|unimplemented!`, `not available|not implemented|not supported|not wired|placeholder|stub`) over `crates/ apps/ extensions/` (excl. `target/`, `apps/vendor/`): only the items quoted above plus demo fixtures — the codebase is unusually clean; the gaps are *absent features*, not dead code.
- Live checks (local release binary, alpha.4): `serve`/`auth`/`ext`/`sessions`/`mcp` all print "not available in this build yet"; `--remote` errors in interactive mode and is ignored with `-p`; `--browser` is an unknown option.
- Not counted as gaps (by design): `/share`, `/bug`, pi telemetry, pi-mcp, pi-codemode (charter ⛔); mempalace-rs switch (owner action); `ti` Tailscale approval (owner action, tracked as PR-3).
