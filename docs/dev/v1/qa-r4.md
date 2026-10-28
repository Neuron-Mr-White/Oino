# QA round 4 — desktop GUI live verification (`[qa-r4]`)

Date: 2026-10-29/30 · branch `v1-alpha` (HEAD `a1d7063` + QA-only commits) · driver
`scripts/gui-drive.py` over `OINO_DESKTOP_AUTOMATION` (docs/dev/v1/gui-driver.md).
Real model: `oino/deepseek/deepseek-flash` (auth.json + cached model catalog; the router
402 credit block of §1b.9 is gone — real turns answered, incl. a vision turn).

**Status: complete.** Verdict in §1.

## 0. Host availability — `ti` blocked, charter local fallback used

- 2026-10-29 ~03:00–03:45 (+rechecks): every `ssh ti@ti` — also key-auth to the tailnet IP,
  `tailscale status` shows the node up and direct — prints **"Tailscale SSH requires an
  additional check"** with a fresh approval URL per attempt (surfaced to the owner twice,
  latest `https://login.tailscale.com/a/l23da5fb3a3fe8`; not approved in time). Same block
  as the `[zcode-perf]` round (ti.md §6).
- Per CHARTER.md ("GUI testing runs on `ti` (preferred) or headless on a private `Xvfb` —
  on this PC only when `ti` is unreachable"), the round ran **locally**: one app instance
  at a time on a private `Xvfb :95` + own dbus session + lavapipe, `nice -n 10`, release
  build of HEAD, driven through the same automation socket and the same scenario files.
  Runner: `docs/dev/v1/qa-r4/local-suite.py` (mirrors `apps/oino-desktop/scripts/local-app.sh`
  plus the per-scenario `settings`/clean-slate logic of `cargo xtask gui suite`). No window
  ever opened on the owner's display; nothing was left running (see §8 cleanup).
- Consequences: (a) numbers below are local-fallback equivalents (Xvfb+lavapipe, like the
  `[zcode-perf]` round), (b) three scenario defects that hardcode `ti` particulars showed
  up (§5 H-1/H-2/H-4) — they are real harness portability bugs, not product bugs, and they
  would also bite any non-ti re-run.

## 1. Verdict

**PASS with defects** — no P0. The desktop app at HEAD is functionally where STATUS.md §1b
says it is: all 28 unipi-suite scenarios pass (4 needed the harness fixes/retries of §5),
the zcode behaviours re-verify (5/6 first pass; the 6th is a dropped-click race, the
feature itself verified), the desktop sweep passes 11/12 (one product papercut QA-8, one
flaky scenario), soak and perf hold the stability/perf bars. New findings: 1 product
defect (P2), 4 harness defects (P2), plus the standing `ti` Tailscale block. A `ti` re-run
of the three host-coupled scenarios remains outstanding (§ Risks).

| Suite | Result |
|---|---|
| `unipi` (28 scenarios, real model) | **28/28 verified** (24/28 first pass; 4 needed §5 fixes/retries) |
| `zcode` (6 scenarios) | **5/6** first pass + 1 verified manually (`zcode-row-actions`, §5 H-3) |
| `qa-r4` desktop sweep (12 scenarios, new manifest `docs/dev/v1/gui/suites/qa-r4.json`) | **11/12** first pass + 1 flaky retry (`turn-undo` PASS on re-run; `subagent-real` → QA-8) |
| soak 30 min `faux` | PASS (§6) |
| perf 4 panes | PASS (§6) |
| exploratory pass (every screen, light+dark) | no new defects (§7) |

## 2. Per-unipi-package live table (suite `unipi`, real model)

All rows re-verified at HEAD locally (same binary the suites ship). "Pass" column: first
pass unless noted.

| Package | Scenario | Result | Notes |
|---|---|---|---|
| core | `unipi-core` | PASS (2 s) | hint strip, `alt+h`, `/hint`, work tray |
| info-screen | `unipi-info-screen` | PASS (2 s) | dashboard tabs, `r` refresh |
| footer | `unipi-footer` | PASS (1 s) | glance frame, `/footer` off/on |
| workflow | `unipi-workflow` | PASS (3 s) | mode chip, `/permission ask`, approval allow, `/plan` |
| ask-user | `unipi-ask-user` | PASS (3 s) | question card, answer, turn resumes |
| long-horizon | `unipi-long-horizon` | PASS (1 s) | `/goal`, `/visualize-progress`, `/regular` clears |
| memory | `unipi-memory` | PASS (9 s) | status, search, `memory_store`, consolidate (markdown-only; no MemPalace here, as on ti) |
| dream | `unipi-dream` | PASS (1 s) | status, report, approve/reject, tray |
| skill-registry | `unipi-skill-registry` | PASS (1 s) | `/skills` page, exposure switch |
| subagents | `unipi-subagents` | PASS after H-1 variant (73 s→) | run_subagent row, tray, profiles; step 18 host-coupled (H-1), relaxed variant passes |
| (subagents live) | `agents-live` | PASS (5 s) | AgentRow live, Agent tab, Stop, btw child |
| background-tasks | `unipi-background-tasks` | PASS (4 s) | `/bg`, tray, Stop-all, completion |
| btw | `unipi-btw` | PASS (1 s) | side-chat answer, message count |
| web-api | `unipi-web-api` | PASS (8 s) | `web_search` row, `/info` Web tab |
| mcp | `unipi-mcp` | PASS (1 s) | `/mcp-status`, settings page (no live server, as on ti) |
| notify | `unipi-notify` | PASS (1 s) | list + toggle, settings page |
| kanboard | `unipi-kanboard` | PASS after env fix (60 s→3 s) | onboard, Board, FAB, `/kanboard status`; first FAIL was my broken `oino-kanboard` symlink beside the binary |
| command-enchantment | `unipi-command-enchantment` | PASS (2 s) | `/` menu, package tags, Tab |
| compactor | `unipi-compactor` | PASS (2 s) | stats/doctor/recall/vcc/by-llm |
| updater | `unipi-updater` | PASS (2 s) | `/readme`, `/changelog`, About |
| input-shortcuts | `unipi-input-shortcuts` | PASS (3 s) | `alt+h`/`alt+m`/`alt+p`, `/ho` |
| fusion | `unipi-fusion` | PASS (1 s) | lead→effort→confirm, stats |
| watchdog | `unipi-watchdog` | PASS on retry (61 s→3 s) | `/bg-detach` adopts running bash; first FAIL = scenario race H-2 |
| app-bridge | `unipi-app-bridge` | PASS (2 s) | `/app` side tab, four tabs |
| utility | `unipi-utility` | PASS (1 s) | `/doctor`, `/cleanup`, `/move`, `/answer`, hub |
| utility (attach) | `image-attach` | PASS via local variant (H-4) | drop → `Image #1` → real vision answer "red" |
| cron | `unipi-cron` | PASS (1 s) | `/cron` status, Automations page, new-job dialog |
| browser | `unipi-browser` | PASS (7 s) | `/browser` status, `browser_open` tool row (`OINO_BROWSER=ephemeral`, hidden engine — nothing on the owner's display) |

## 3. ZCode checklist (design.md §8 / zcode-details.md) live status

Driven through the `zcode` suite + manual driver checks; look stays A2 (unipi) per the
charter, so rows are behaviour checks.

| zcode-details row | Suite/manual evidence | Status |
|---|---|---|
| M1–M6 motion (enter/exit/shimmer/reduce/ticker/static icon) | `zcode` suite green paths; exit fade seen on toasts; `OINO_REDUCE_MOTION=1` in every run | ✅ |
| T1–T2 scroll pinning / jump pill | code-verified (driver cannot wheel-scroll, D11); jump pill seen in exploratory pass | ✅ (D11 driver gap) |
| T4 thinking label `Thinking`→`Thought for Ns` | `zcode-row-actions` step 6 (`Thought for`) | ✅ |
| T5 fold `Ran commands · 2 steps` | `approval-allow`/`approval-deny` folds | ✅ |
| T6 copy glyph→check 1.2 s | verified manually (click after settle flips `Copy answer`→`Copied` <100 ms, reverts); scenario step 9 hits H-3 | ✅ (feature) / harness H-3 |
| T7 hover bars | `zcode-row-actions` step 7 hover | ✅ |
| T8 inline diff / Review / TurnFooter | `turn-undo`, `approval-allow` | ✅ |
| T10 query rail + tooltip + detach-first | exploratory pass (2-prompt transcript) | ✅ |
| T11 find bar ⌘F | exploratory pass | ✅ |
| T12 virtualised rows | all suites (transcript tree reads) | ✅ |
| T13 arrival time labels | `zcode-row-actions` (`as-time` `Answer time`) | ✅ |
| C1 state placeholders | `zcode` suite; placeholders seen idle/running | ✅ |
| C2 model-switch toast | `zcode-model-menu` (round 2 scenario) | ✅ |
| C3 model menu drill-down + back | `zcode-model-menu` | ✅ |
| C4 `alt-t` thinking cycle | `zcode-thinking-cycle` | ✅ |
| C5–C6 history/drafts | `queue-edit`, exploratory | ✅ |
| C7 mentions/completions | `unipi-command-enchantment`, `new-session-prompt` | ✅ |
| C8 attachments | `image-attach-local` (real vision turn) | ✅ |
| C9 queue panel | `queue-edit`, `zcode-queue-guard` | ✅ |
| C10/C11 steer/queue + recall guard | `zcode-queue-guard` | ✅ |
| N1–N2 palette scopes + history chips | `zcode-palette-history` | ✅ |
| N3 tooltips / long-title tooltip | exploratory | ✅ |
| N6 platform modifier | `cmd-*` mapped to Ctrl locally (assertions use `secondary-*`) | ✅ |
| N7 session context menu | `zcode-session-menu` | ✅ |
| F1–F6 toasts/cards/empty/error/unread | exploratory pass + suites | ✅ |

No ZCode-behaviour regressions found at HEAD vs zcode-details.md §7 (the round-2 PASS set).

## 4. Regressions (vs STATUS.md §1b/§7 records)

1. **`zcode-row-actions` no longer passes as a scenario** (passed for `[zcode-perf]`,
   fails at HEAD deterministically for this round *and* the previous QA attempt). Root
   cause is the click race of H-3, not a behaviour change — the copy feature itself works
   (manual verification). Filed as harness H-3, watch for recurrence on `ti`.
2. Nothing else regressed: unipi 24/28 == the ti record (the 4 failures are §5 harness/env,
   all green after fixes), `turn-undo`/`queue-edit`/`status-panel`/`agents-live` (the
   `[gui-wiring]` four) all PASS.

## 5. New defects

**Product**

- **QA-8 (P2)** — AgentRow shows the generic name when the model omits the `agent` arg.
  Repro (driver): run `subagent-real` (or any prompt where the model calls `run_subagent`
  with only `title`/`task`): the extension defaults the profile (`subagent-step` events
  carry `profile: "subagent_explore"`, label `Explore subagent · Read README first line`),
  but the row label falls back to `tool_label("run_subagent")` = "Run subagent" because no
  `agent`/`profile`/`name`/`subagent` key is in the args; the sidebar/StatusPanel then
  show no profile either. Evidence: session
  `target/tmp/local-run/agent/sessions/…01a18418….jsonl` entries 8–13, shot
  `docs/dev/v1/qa-r4/shots/fails/desktop-subagent-real-FAIL-06.png`.
  Hints: `apps/oino-ui/src/model/timeline.rs` `agent_line` (args-only name), the
  `subagent-step` custom rows carry the resolved profile; or `extensions/unipi/subagents`
  could echo the resolved profile into the call args/details.

**Harness / scenarios** (GUI-driver scenarios are QA-owned; fixes below are one-liners the
next round can take, not applied here to keep product commits clean)

- **H-1 (P2)** — `unipi-subagents` step 18 hardcodes ti's project label `"ti project"` and,
  worse, asserts a **nondeterministic auto-generated session name** (the rename model
  picked "Subagent README First Line" here, "README First Line Subagent" on retry). Fails
  off-ti and can fail on ti. File: `docs/dev/v1/gui/scenarios/unipi-subagents.json` step 18.
- **H-2 (P2)** — `unipi-watchdog` step 6 waits for text `"sleep 60"`, which matches the
  **user prompt row** (the prompt contains the string) before the bash call starts; if the
  model is slow to call the tool, `/bg-detach` runs with nothing running and step 10 fails.
  Flaky (passed on retry in 3 s). Fix: wait for the tool row (`id: "tool-0"`).
  File: `docs/dev/v1/gui/scenarios/unipi-watchdog.json` step 6.
- **H-3 (P2)** — **clicks dispatched in the ~0.5–1 s window after `busy=false` are
  dropped** when the post-turn rows land (memory-save card row inserts; toast entrance):
  the injected MouseDown/Up miss the just-shifted pinned transcript. Deterministic for
  `approval-deny` step 7 (fold click) and `zcode-row-actions` step 9 (copy click) — both
  failed for the previous QA attempt too, at the same steps; both features verified working
  when clicked after the window (second click always works). A user can also miss a click
  on a moving target, so impact is QA-first; the product-side trigger (late row insertion
  shifting a bottom-pinned transcript right at turn end) may deserve a look regardless.
  Hints: `apps/oino-ui/src/automation/{driver,input}.rs` (frame cadence between dispatch
  and hit-test), `apps/oino-ui/src/model/transcript.rs` (insert-driven relayout).
- **H-4 (P2)** — `image-attach` hardcodes ti's drop path `/home/ti/oino-test/run/attach-red.png`;
  the `drop` RPC silently ignores missing files, so off-ti it fails at step 7 with no
  diagnostic. Fix: resolve the scratch png relative to the run dir, or have `drop` error on
  missing paths. Files: scenario step 6; `apps/oino-ui/src/automation/driver.rs` (`drop`).
- **H-5 (P2)** — `apps/oino-desktop/scripts/local-app.sh` **faux mode is broken**: it passes
  `--faux`, which the binary rejects (`Error: unknown argument: --faux` — only `--demo`
  exists), and it copies `extensions/dist/provider-faux.wasm`, which does not exist (the
  fixture ships as the directory `extensions/dist/oino.fixture.provider-faux/`). The `||
  true`/`set -e` mix hides both. Found while driving faux locally; the soak runs faux
  through its own path and is unaffected.

## 6. Soak / perf numbers (local Xvfb + lavapipe, release build)

**Soak 30 min, `faux` backend** (`apps/oino-desktop/scripts/soak.sh --minutes 30 --backend faux`,
evidence `docs/dev/v1/qa-r4/soak-faux-30min/`):

```
backend=faux minutes=30 samples=360 steps=2567
rss_mb start=10 end=1098 max=1099 slope_2nd_half_mb_per_min=0.88   (limit 8)
threads first_quarter_max=228 end=229                              (limit +16)
fds     first_quarter_max=438 end=430                              (limit +64)
frame_ms run_max=46.7 run_p99=32.4                                 (report-only on Xvfb)
crash_logs=0 stall_logs=0 exit=0
RESULT: PASS
```

Same profile as the recorded round-3 runs (≈1.1 GB wasmtime plateau, flat after the pane
cap); slope 0.88 MB/min is better than the 2.80 MB/min of the last recorded run.

**Perf, 4 streaming panes** (`perf.sh`, headless lavapipe, 20 s, `BUDGET_MS=10`):

```
windows=8 frames=1200 (~60 fps rendered) dropped(>16.7ms)=0 over_8.3ms=3
p50 median=2.15ms  window p99 median=3.79ms  worst window p99=4.11ms  max=4.72ms
RUN p99 (all frames)=3.90ms  budget=10ms                         -> PASS
latency first frame 130.4ms · keystroke→paint median 8.4ms max 12.5ms · session switch 8.5ms
idle CPU (welcome, after settle): 0.80 % UI thread, 13.6 % process
```

Matches (slightly better than) the recorded 4.1–4.9 ms p99 and the 0.8–1.0 % idle CPU of
DESIGN-STATUS.

## 7. Exploratory pass notes (every screen, light + dark)

Driver: `docs/dev/v1/qa-r4/exploratory.py` (one app instance per theme, demo + real
backends) plus targeted manual steps. Shots under `shots/explore-{dark,light}/`.

Walked: sessions home (hero, suggestions, needs-you strip), sidebar nav → Board /
Automations / Extensions / Settings (16 nav rows) / back; board page (lanes, toolbar, FAB,
New-task dialog + Esc); automations page; extensions page (`Install from file…`, `Open
folder`, per-extension switches — `ext-toggle-0` flip + restore verified); settings
(sections, hub); palette (opens with ⌘K **from every route incl. Settings**, empty-query
sections, no-results state, Esc); session view (composer focus, typing lands after every
overlay — the QA-1 fix holds: `/info` + Esc, dialog close, palette close all return focus
to the composer); side pane + `+` launcher (Review/Files/Plan/Tree/Browser/Terminal/
Kanboard rows); mode-chip and model-chip menus; `/info` dashboard on a **real** core
(`panel:unipi.info-screen/dashboard` opens, Esc closes).

Findings:

- **Unknown slash commands are sent to the model as plain text** (no local "unknown
  command" error). This is pi's behaviour (parity, not a defect); the completion menu only
  offers known commands.
- **Local `--faux` startup is broken in the fallback script** (H-5 below); the faux checks
  were covered by the soak (faux in-process) and the suites' real backend instead.
- **Look, dark**: pixel probes of live shots resolve exactly to the A2.2 tokens — window
  `#0A0908` dominant, surfaces `#141211/#1C1917/#26211F`, accents shell `#F03018` (211
  sampled px on the welcome shot), amber (needs-you/hints) and success green present; no
  off-token dominant colour on any probed screen.
- **Look vs unipi refs**: dominant-token fingerprints match the reference shots'
  families and rank order on every compared pair (sessions 60/54 % window · 25/29 %
  surface-1; palette 63/73 % · 25/18 %; settings 78/73 % · 17/21 %; board 64/78 % · 23/15 %
  — ours are emptier screens, same palette). Oino's chat/board shots read "darker" only
  because the reference screens are denser.
- **Look, light**: derived light tokens render exactly (`#FAF7F2` bg dominant,
  `#F2EDE6` surfaces; probes d≤4 on every region) — A2 light is not blocking.
- The QA-1 focus guarantees re-verified on this build: every overlay/page returns keyboard
  focus to the composer (`typing lands` checks passed in both themes).

## 8. Evidence index (docs/dev/v1/qa-r4/)

- `shots/unipi-*/` — per-package screenshots (fetched from the local run)
- `shots/agents-live/`, `shots/image-attach*/`, `shots/zcode-*/`, `shots/gui/` — suite shots
- `shots/fails/` — FAIL screenshots of every failing step (renamed `<suite>-<scenario>-FAIL-NN.png`)
- `local-suite.py` — the local fallback runner (QA tooling)
- `exploratory.py` — the every-screen walk (dark + light)
- `look-probe.py` — pixel probe comparing renders against the A2.2 palette
- `soak-faux-30min/` — soak summary + samples (raw soak output)
- `scenarios/*-local.json` — host-neutral variants (subagents relaxed assertion, watchdog
  tool-row wait, image-attach absolute path)
- logs: `target/tmp/qa-r4-{unipi,zcode,desktop,repro}*.log`, suite json
  `target/tmp/local-run/suite-*.json`

Cleanup at end of round: `apps/oino-desktop/scripts/local-app.sh stop`, soak helpers kill
their own Xvfb/dbus/app; verified `pgrep -af 'oino-desktop|Xvfb :9|Xvfb :120'` empty (see
final commit message).

## 9. Resolution (`[qa-r4-fix]`, 2026-10-30)

All five defects of §5 fixed (QA-8 + H-1…H-5), each verified by its driver scenario, then the
full suites, soak and perf re-run. Commits `6eeecdb`…`e17f93a` on `v1-alpha`.

**Host**: `ti` stayed Tailscale-blocked the whole round ("additional check" with a fresh
approval URL per attempt, latest `https://login.tailscale.com/a/lca9869a3a0c73`, surfaced to
the owner; re-checked at start, mid and end). Everything below ran on the charter's local
fallback (private `Xvfb :95`, one app instance, release build, `nice -n 10`), like §0. The
`ti` re-run remains outstanding; every fix is host-neutral by construction (that was the
point of H-1/H-4).

### Fixes

| Defect | Fix | Verification |
|---|---|---|
| QA-8 AgentRow generic name | The subagents extension echoes the resolved profile into the streamed tool details (`profileLabel`, e.g. "Explore subagent", + raw `profile` id) — `view::details` and the `wait_foreground` progress line; `timeline::agent_line` resolves the name args(`agent`/`name`/`subagent`) → details `profileLabel` → details `profile` → args `profile` → label fallback. StatusPanel/side pane share `agent_line`, so they follow. | unit tests (timeline, subagents view); `subagent-real` PASS — row shows `Explore subagent Read README.md…`, Agent tab opens, turn settles |
| H-1 host-coupled subagents assertion | Every harness writes the same scratch README first line ("oino test project": `ti start`, `local-app.sh`, `local-suite.py`); step 18 waits for that text; step 15 waits for the AgentRow (role row, label "subagent") instead of a raw profile id | `unipi-subagents` PASS (11 s), twice |
| H-2 watchdog prompt-row race | Step 6 waits for the running fold row (`role row`, label `sleep 60`) — the user prompt row can no longer satisfy it — plus a 4 s settle before `/bg-detach` (the live fold row shows during Preparing too, before the bash process exists) | `unipi-watchdog` 3/3 standalone + in-suite |
| H-3 dropped clicks at turn end | Two causes, both in `automation/driver.rs`: (1) selector bounds were resolved before the settle frames and a late row insert shifts the bottom-pinned target — `pointer`/`scroll`/`drop` now re-resolve against the last published frame immediately before dispatch; (2) Move+Down+Up dispatched as one batch hit-test against the pre-move frame (a target that just moved, e.g. the hover bar opening, is missed at *correct* coordinates) — each event now gets a frame of its own, the order a person produces | `approval-deny` 3/3 (step 7 fold click), `zcode-row-actions` 6/6 (step 9 copy click) — both previously deterministic FAILs |
| H-4 hardcoded drop path | Scenario drops `$RUN/attach-red.png`; `gui-drive.py` resolves `$RUN` to the app host's run dir (ti: `~/oino-test/run`, local: `target/tmp/local-run`) and creates the red PNG when missing; the app-side `drop` now refuses paths that are not files on the app host instead of a silent no-op | `image-attach` PASS (PNG created on demand, real vision turn answers "red") |
| H-5 local-app.sh faux | Faux mode mirrors `cargo xtask ti start --faux`: settings select `faux/faux-1`, the fixture directory `extensions/dist/oino.fixture.provider-faux` is copied to `<agent>/extensions/provider-faux` (there is no `provider-faux.wasm`), auth gets the `faux` key, and no `--faux` flag is passed (the binary only knows `--demo`/`--cwd`/`--soak`). Missing prerequisites fail loudly; `local-suite.py` fixed the same way | `local-app.sh start faux` → real faux session, `hi` → "Hello from faux." |

### Verification (all on the charter local fallback, release build)

| Suite | Result |
|---|---|
| `unipi` (28) | **28/28** (`target/tmp/qa-r4-fix-unipi4.log`) |
| `zcode` (6) | **6/6**, incl. the previously failing `zcode-row-actions` |
| `qa-r4` desktop sweep (12) | **12/12**, incl. `subagent-real` (QA-8), `approval-deny` (H-3), the previously flaky `turn-undo` first-try |
| soak 30 min `faux` | **PASS** — 2566 steps, RSS 10→1127 MB, slope **1.07 MB/min** (limit 8), threads 233→232, fds 438→430, 0 crash/stall |
| perf 4 panes | **PASS** — run p99 **4.50 ms** (budget 10), 1 dropped frame, keystroke→paint 7.6 ms, idle CPU 0.80 % UI thread |
| gates | `cargo fmt --check`, `clippy -D warnings -W clippy::pedantic` (oino-ui, unipi-subagents, xtask), `cargo test -p oino-ui --lib` (121) and `-p unipi-subagents` (14) green; `cargo xtask wasm -p unipi-subagents` rebuilt into `extensions/dist` |

Evidence: `docs/dev/v1/qa-r4/qa-r4-fix/` (suite summaries, soak summary + samples, perf
numbers); logs `target/tmp/qa-r4-fix-*.log`; refreshed shots under
`docs/dev/v1/design/shots/desktop|gui/`. Nothing left running (`pgrep` clean, soak removed
its own Xvfb/dbus/app).

### New findings from the fix round

- **H-6 (P3, harness)** — extension **panel bodies** (e.g. the updater's `/changelog`) render
  visually but their OUS text nodes never register in the automation tree (cached-view
  registration gap), so a scenario cannot assert panel body text; `unipi-updater` now asserts
  the panel title (`overlay-panel-title`). Same family as the §7b cached-view finding.
- **M-1 (harness/environment)** — the unipi **memory** extension talks to a host **MemPalace
  daemon** when one is reachable through `PATH`: on this PC `/memory-search` answered from
  (and `memory_store` wrote into) the *owner's* daemon store — the round-4 pass was luck
  (markdown mode until the daemon was probed). `local-app.sh`/`local-suite.py` now run the
  app with `PATH=/usr/bin:/bin` so scratch runs stay markdown-only, exactly like `ti` (no
  mempalace there). **Residue:** one QA test drawer (`project_mascot_pixel_crab_unicrab`,
  project "project") was written into the owner's store (round 4 and this round); the
  product-path delete (`memory_delete`) no-ops because the clean-slate removed the source md
  the daemon matches by. It is inert; remove via unipi's memory UI if undesired.
- **Network quota flake (harness)** — `unipi-updater`'s changelog body comes from GitHub
  (tag → branch → releases API, unauthenticated 60/h): the old `0.` assertion passed only
  while quota lasted. The scenario now asserts the rendered panel (title) instead;
  `assert`/`wait_state` gained `any_text` for either-branch checks.
- `subagent-real`/`unipi-subagents` prompts no longer name the tool arg in the prompt text —
  the prompt leaks into the sidebar session-row label and made `role`+`label` selectors
  ambiguous with it.

### Outstanding

- The `ti` re-run (suites + soak + perf on the reference host) once Tailscale is approved —
  the three host-coupled originals of §5 are now host-neutral, so the same scenario files
  must pass there unchanged.
- H-6 (panel-body registration) left as a small follow-up for the automation layer.
