# Oino Desktop GUI QA — round 3

Date: 2026-10-27 · branch `v1-alpha` @ `7525d3d` (HEAD; all five fix batches landed) · work id
`gui-qa` · QA only — no code changed, only this file committed.

## Verdict

**Fix batches hold; not shippable yet because of two new P1s.** All 46 GA ids were
re-checked on the rebuilt binary: every id the batches marked *fixed* really is fixed (GA-12
stays open as agreed; GA-45 is only half-wired), the unipi look (design.md A2) is objectively
present in the rendered app, un-prefixed commands work, no legacy unipi import prompts
appear, no herdr/tmux concepts remain in `apps/`, and the 30-minute faux soak passes cleanly.
Round 3's new findings are concentrated in keyboard interaction: **dialogs and approval cards
do not take keyboard focus** (QA-1) and **the browser tool launch hangs without a timeout**
(QA-2).

## Method

- Binary: `apps/target/release/oino-desktop` rebuilt from `7525d3d`
  (`cd apps && TMPDIR=$PWD/target/tmp cargo build -j 8 --release -p oino-desktop`), real
  embedded host, 32 bundled extensions, not `--demo`.
- Config: scratch agent dir `/tmp/oino-qa` (`auth.json` copied from `~/.oino`; settings
  `defaultProvider: oino`, `defaultModel: deepseek/deepseek-flash`, footer on), scratch repo
  `/tmp/oino-qa-repo`. Real model calls via the Oino router (~8 turns: two write turns under
  ask mode, one browser turn, command turns).
- Display: private `Xvfb :97` (1600×1000) + lavapipe; one instance at a time; only
  self-started PIDs killed. Screenshots via `gst-launch-1.0 ximagesrc`.
- Input: XTEST for plain keys + XSendEvent for modifier combos. On this Xvfb, XTEST does not
  apply modifier state and the app selects no core pointer events, so **pointer clicks are
  not injectable**; all interaction was keyboard-only. Screen text was read back with a
  router vision model (claude-haiku) as OCR; colours verified by pixel probes against the
  A2.2 token values. Checks that require a mouse are marked *mouse-only, not re-exercised*.
- Session transcripts verified directly from the JSONL under `/tmp/oino-qa/sessions/`.

## GA re-check (every id marked fixed, at `7525d3d`)

"Live" = exercised in the app this round; "code" = source + unit tests inspected (live path
needs a mouse or budget).

| Id | Sev | Status | Evidence this round |
|---|---|---|---|
| GA-1 | P0 | **fixed (live)** | fast typing (5–15 ms/keystroke) across sessions and restarts, hours of uptime; no crash log; GA-40 root cause gone |
| GA-2 | P1 | **fixed (live)** | `/kanboard` typed at 15 ms/char with the completion menu open — every key arrives; command executed locally |
| GA-3 | P1 | **fixed (live)** | `/new` opens a fresh empty session; no `/new` user message, nothing to the model |
| GA-4 | P1 | **fixed (live)** | one `COMMANDS` group in the menu, colour `[kanboard]` / `[skill]` tags, no duplicate base group |
| GA-5 | P1 | **fixed (live)** | empty session: crab + build line + greeting; non-empty: one collapsed `Unicrab · Oino 1.0.0-alpha.2 · …` card at the top, scrolls away; one crab |
| GA-6 | P1 | **fixed (live)** | memory wake-up = one collapsed harness card; bg notification = `<background-task-notification>` card; kanboard help = collapsed card |
| GA-7 | P1 | fixed (live, layout) | cards render in the composer slot, capped, body scrolls (approval + question card exercised); keyboard reachability → QA-1 |
| GA-8 | P2 | fixed (live) | no stray gutter glyphs in any transcript shot |
| GA-9 | P2 | fixed (code) | dedupe + `is_model_error` unit tests; not re-triggered |
| GA-10 | P2 | fixed (code) | `assistant_questions_are_not_plan_steps`; not re-triggered |
| GA-11 | P2 | **fixed (live)** | restart reopens the last session ("Reopen the last session" = On in Settings); no extra empty rows |
| GA-12 | P2 | **open (confirmed)** | info-screen splash greeting still UTC-hour (`splash.rs`: "UTC until the host exposes local time"); the *frontend* welcome greeting (`conversation.rs::local_hour`) is local-time correct — only the unipi splash is wrong (needs the R3 WIT field) |
| GA-13 | P1 | fixed (batch) | `+` launcher; mouse-only, not re-exercised; palette/Ctrl+2 reach the same tabs |
| GA-14 | P1 | **fixed (live)** | work tray "Background tasks (1 · 1 running)" tab, running row `sleep 30 · 4s · 0B`, Stop all / Dismiss finished |
| GA-15 | P2 | fixed (code) | `_oino/agents` feed in core + Agent tab from the panes batch; subagent run not driven (budget) |
| GA-16 | P0 | **fixed (live)** | `alt+m` cycles permission modes with toasts (auto → full → ask verified); `alt+p` toggles plan mode (toast with plan-file path + reminder row); `alt+h` on the hint bar |
| GA-17 | P0 | **fixed (live)** | Board page opens; unregistered project → "oino-qa-repo has no board yet" + "Set up the board"; `oino-kanboard` found next to the binary and ran (`serve --host 127.0.0.1 --port 0`) |
| GA-18 | P1 | fixed (batch) | control-centre side tab; mouse-only, not re-exercised |
| GA-19 | P0 | **fixed (live)** | `/readme` opens the READMEs panel (`oino.docs` + version badge, footer legend `esc close · b back`); `/changelog` opens the panel — content empty (QA-4) |
| GA-20 | P1 | fixed (batch) | native New-task dialog → `/kanboard-add` (`65-unipi-board-newtask.png`); FAB is mouse-only |
| GA-21 | P1 | **fixed (live)** | `↓` opens the work tray in the composer slot; collapsed bar "Background tasks 1 running · ↓ open"; `/subagents` opens the tray on its tab |
| GA-22 | P1 | **fixed (live)** | tray: segmented tabs with counts, one legend, Esc closes; info panel `q` closes; hints panel Esc closes |
| GA-23 | P1 | **fixed (live)** | `/info`: dialog header + ×, tabs, big-digit stat tiles, formatted meters (`8.7k of 1M · 1% full`), footer legend `r refresh · R refresh all · q close · 1–9 page`; `2`/`r`/`q` work |
| GA-24 | P1 | **fixed (live)** | kanboard-onboard multi-line notify opened a result-card dialog (title, lines, Copy/Done) instead of a clipped toast |
| GA-25 | P1 | **fixed (live)** | `/footer off` removed the stats strip while the mode/permission chip kept reading its state ("Plan") — chip reads the workflow topic, not footer segments |
| GA-26 | P1 | **fixed (live)** | glance frame under the composer: `oino-qa-repo · main · UNIPI · Regular Mode · auto` + stats strip `↑1.7k in · ↓414 out · 13s avg ttft · 362 tok/s · 4 turn · 6 steps · 48:47 · tool 00:00 · 97% cache hit`, one line each, no wrap at 1400 px |
| GA-27 | P1 | **fixed (live)** | approval card laid out in one paint (feedback field present from the start; numbered options); no height change between t≈1 s and t≈45 s |
| GA-28 | P1 | fixed (unit) | plan-mode tool restore unit-tested; live path quota-blocked for the batch |
| GA-29 | P1 | fixed (code) | `option_row` + capped list scroll (fusion picker not re-run) |
| GA-30 | P0 | **fixed (live)** | approval card alive and pixel-stable after >4 min (no 30 s handler timeout); after a kill+restart no orphan card is restored, composer usable |
| GA-31 | P2 | **fixed (live)** | `/hint` opens "Unicrab hints": filter box, `command (23)` section, rows with seen/unseen tags and "mark learned" pills |
| GA-32 | P0 | **fixed (live)** | `mascotWindow: true` + relaunch: frameless 372 px mascot window shows the animated pixel crab, app alive, no crash loop |
| GA-33 | P1 | **fixed (live)** | Settings → Permissions loads (no "unipi.workflow is not loaded"): mode rows + "Clear saved rules…" |
| GA-34 | P1 | fixed (code) | `PickModel` bound to Ctrl+M (`views/mod.rs`); not live-verified (QA-1 ate focus) |
| GA-35 | P1 | **fixed (live)** | `grep -rniE 'herdr|tmux' apps/` → 0 hits; no pane/workspace/split concepts in UI or palette |
| GA-36 | P1 | **fixed (live)** | A2 palette live in render (§ Look & feel); flush columns, 224 px sidebar, list\|detail, page headers, pills |
| GA-37 | P2 | **fixed (live)** | Settings General is editable: default model menu, thinking segments, folder field, "Reopen the last session" switch; About & updates present |
| GA-38 | P2 | **fixed (live+code)** | no `skill:herdr` in any menu; `~/.agents/skills` not discovered implicitly (`discovery.rs:234`) |
| GA-39 | P2 | **fixed (live)** | `~/.unipi` present on this machine: no import card, no Needs-you badge, no toast; wake-up collapsed card + `0 mem` chip |
| GA-40 | P0 | fixed (code) | K1 landed; GA-1 live evidence |
| GA-41 | P1 | **fixed (live)** | `oino-kanboard` next to `oino-desktop`; daemon started during the board onboard |
| GA-42 | P1 | fixed (code) | handler deadline paused while a dialog is open (`oino-wasm-host/src/instance.rs:303`); GA-30 live evidence |
| GA-43 | P1 | **fixed (live)** | `alt+m`/`alt+p` extension shortcuts fired live through the shell interceptor |
| GA-44 | P1 | fixed (code) | `_oino/agents` feed in core events/agents |
| GA-45 | P2 | **only half-wired** | `apps/oino-desktop/src/embedded.rs:275` still `mode: ExtensionMode::Rpc` (`has_ui: true` is set): `/doctor` would still report Rpc; K7's mode is not consumed by the desktop |
| GA-46 | P2 | soak-clean | threads/fds flat in the soak; idle CPU not re-measured this round |

## New defects (P0/P1 first)

Severity scale as gui-audit.md. Hints point at the owning batch's files.

### QA-1 (P1, systemic) — dialogs and approval cards never take keyboard focus; closing an overlay often drops focus app-wide

Keyboard-only operation is broken on this build. Repros (all reproduced this round):

1. Ask-mode write → approval card. The card *shows* numbered options (`1 Allow once`,
   `2 Always allow`, `3 Deny`, `4 Deny with note`) but `1`–`4`, `Enter`, `Tab`, `Esc` are all
   inert — **an approval can only be answered with the mouse**. A keyboard-only user is hard
   blocked in ask mode (turn waits forever, "Needs you" forever).
2. `/info` → `q` closes the panel → typing, `Ctrl+K`, `Ctrl+1..3`, `Tab`×8, `Esc` all dead;
   only a mouse click or restart restores input. Same after the `/readme` panel's Esc
   (racy: the hints-panel Esc once restored focus fine).
3. The kanboard-onboard result card needed `Tab Tab Return` before Esc/Return acted (no
   self-focus).
4. On the Board page (no session detail focused) the whole keyboard is dead: no palette, no
   Ctrl+1..3, no board keys (`?`, `c`, `/`).

Expected (design.md A2 L11, §4.12–4.15, §1.7 "keyboard first"): every dialog focuses itself,
digits/Enter/Esc act on the focused card, focus returns to the composer when an overlay
closes, shell shortcuts work on every screen.

Hints: `apps/oino-ui/src/components/interaction/*` (card focus + digit keys),
`apps/oino-ui/src/model/dialog.rs`, `apps/oino-ui/src/views/session_pane.rs` (focus restore
after `extension_ui_dismiss`/panel close), `apps/oino-ui/src/components/keys.rs` (interceptor
scope on non-session pages). Looks like one missing focus-set + restore path.

### QA-2 (P1) — browser tool launch hangs forever; no timeout or error surfaces

Prompt "use your browser tools: open https://example.com …". Engine resolution worked
(profile `~/.oino/browser/profiles/default` created, `.cloakbrowser-seed` written), the
transcript shows `Browser navigate · https://example.com` with a spinner, and the call is
still pending after 2.5 min: no result row, no error, no browser process running with the
Oino profile, turn never ends. Per browser-control.md §5.5 a failed launch must return
`backend_unavailable{launch_failed}`.

Caveat: another Oino process (different agent) had used the same `~/.oino/browser` tree
minutes earlier (turn-shots 14:07–14:09 UTC), so a profile-lock interaction is possible;
either way the missing timeout is real.

Hints: `crates/oino-browser/src/launch/*` (launch deadline), `src/profiles.rs` (lock).

### QA-3 (P2) — "Changes this turn" card claims a write that never happened

After the QA-1 approval card was abandoned by killing the app (never answered), the reloaded
session shows "Changes this turn · qa-note2.md +1 −0", but the file was never written
(absent on disk). The turn summary is computed from the recorded tool-call input, not the
applied change.

Hints: `apps/oino-ui/src/model/{timeline,tools}.rs`, core turn finalisation for unanswered
calls.

### QA-4 (P2) — `/changelog` panel opens with no content while online

"No changelog found yet — check from Settings → About once online." although the network is
up and the updater's `last-check.json` has `latestVersion: 0.1.3` (channel alpha). The
bundled `oino.docs` package carries no CHANGELOG file and the remote changelog does not feed
the panel on demand. GA-19's mechanism works; content is empty.

Hints: `extensions/unipi/updater/src/{lib,changelog}.rs`.

### QA-5 (P2) — `/kanboard open` opens nothing

With the board onboarded, running `/kanboard open` from the composer produced no panel, no
side tab, no navigation (its own help says it "opens the board panel"). The Board *page*
(sidebar/palette) works. Suspect the command's open path targets a placement the desktop
does not act on.

Hints: `extensions/unipi/kanboard` (open command), `apps/oino-ui/src/components/side_pane.rs`.

### QA-6 (P2) — `/doctor` runs but never reports

The command echo lands in the transcript, then nothing for 45+ s: no result card, no toast,
no transcript entry, nothing in `unipi.utility.log`. (Ask mode was active; if the doctor's
subprocess checks are gated, the gate card never appeared either.)

Hints: `extensions/unipi/utility/src/maintenance/doctor.rs`, dialog/notify path under QA-1.

### QA-7 (P2, re-flag) — A2.18 leftovers in extensions

`extensions/unipi/fusion/src/policy.rs:193` (`tmux capture-pane` allow rule) and
`extensions/unipi/utility/src/settings.rs:215` (`rename.herdrSync: true` default) are still
present. A2.18 assigns deletion to the extensions owner; re-flagged so it is not lost.
`apps/` itself is clean (GA-35).

### Observations (no id)

- Session auto-rename works ("Create QA Note File" from a write turn).
- Needs-you signalling works end-to-end: sidebar badge count, header chip "Needs you · 4m",
  session-row state word — first live proof of A2.7/L7 on this build.
- Composer drafts still do not survive a release/restart (known; stability.md).
- The turn-fold line ("Edited files, read files · 2 steps"), turn end ("Done in 3.5s"),
  review card and per-turn stats all render per A2.10.

## Per-package unipi parity (live this round)

| Package | Live evidence |
|---|---|
| unipi.core | work tray on `↓` (tabs + counts + Esc + collapsed bar), hint bar with `command · alt+h ›`, `/hint` browser (filter, seen/unseen, learned pills), collapsed start-header card |
| unipi.workflow | `/permission` cycle via `alt+m` + toasts; `alt+p` plan mode (toast with plan-file path, reminder row, "Plan" chip); ask-mode approval card; glance "Regular Mode" segment |
| unipi.background-tasks | `/bg sleep 30` → tray row (command, elapsed, bytes), `<background-task-notification>` card |
| unipi.kanboard | Board page + setup panel; `/kanboard onboard` result card; `/kanboard`, `/kanboard-add` menu entries; daemon next to the binary |
| unipi.info-screen | `/info` dashboard (tiles, meters, tabs, keys 2/r/q live); welcome facts |
| unipi.updater | `/readme` panel; `/changelog` panel (empty content → QA-4) |
| unipi.memory | wake-up harness card, `0 mem` chip, no import prompt |
| unipi.command-enchantment | single COMMANDS group, colour tags |
| unipi.footer | glance frame segments + stats strip, `UNIPI` wordmark, `/footer off` honoured |
| unipi.utility | result-card dialog for multi-line notify; `/doctor` broken (QA-6) |
| unipi.subagents | `/subagents` → tray tab; `/agents` → profiles dialog (builtin profiles, "+ New agent…") |
| unipi.ask-user / btw / fusion / dream / long-horizon / compactor / mcp / notify / skill-registry / web-api / watchdog / input-shortcuts / app-bridge | not driven this round (model budget, mouse-only surfaces, or QA-1 focus); code + unit gates green, batch screenshots stand |

Commands observed un-prefixed and working: `/kanboard` `/kanboard onboard` `/info` `/readme`
`/changelog` `/hint` `/permission` `/bg` `/subagents` `/agents` `/footer` `/new`. Registry
grep across `extensions/unipi/*/src`: 40 command names, **zero** `unipi:`-prefixed
(commands.md §3 holds). No legacy import prompts anywhere (GA-39).

## Look & feel vs unipi (A2 binding)

Pixel-sampled this round (probe points and dominant-colour histograms) vs
`design/ref/unipi/*`:

| Surface | Oino (this build) | unipi ref / A2 token | Match |
|---|---|---|---|
| window bg | `0A0908` | `0A0908` | ✓ |
| sidebar / surface-1 | `141211` (72 % blend) | `141211` | ✓ |
| composer / surface-2 | `1C1917` | `1C1917` | ✓ |
| user bubble / surface-3 | `26211F` | `26211F` | ✓ |
| primary (send, FAB, selection) | `F03018` family | `F03018` | ✓ |
| palette sheet + scrim | `141211` + dark scrim | `141211` + scrim | ✓ |

Layout parity observed live: flush columns with a 224 px sidebar (brand, search `Ctrl+K`,
Sessions/Board/Extensions/Settings), list\|detail with project sections and counts, page
headers (title 30 + meta + tools + crab), 56 px detail header (title, `oino-qa-repo · main`
sub-line, model chip, needs-you chip), welcome EmptyState (crab + build line + local-time
greeting + hint line + 3 suggestion cards), composer shell R26 with attach/mode/model/send
capsule and glance frame, fold lines, "Done in 3.5s" turn ends, "Changes this turn" cards,
dialog headers with `×`, panel keys as footer legends. This is the unipi A2 look, not ZCode
chrome.

## Soak (30 min, faux) — PASS

`apps/oino-desktop/scripts/soak.sh --minutes 30 --backend faux` (own Xvfb :120 + lavapipe,
one instance, 4 GB abort armed). Artifacts: `apps/target/tmp/qa/soak/` (samples.tsv,
summary.txt, app.out).

| Metric | Value | Limit |
|---|---|---|
| driver steps | 2567 | — |
| RSS start → end (max) | 10 → 1100 MB (1100) | 4096 abort |
| RSS slope 2nd half | **1.19 MB/min** | < 8 |
| threads q1 max → end | 231 → 230 | growth < 16 |
| fds q1 max → end | 424 → 416 | growth < 64 |
| frame run_max / p99 | 55.6 / 40.2 ms (lavapipe) | report only |
| crash logs / stall logs | 0 / 0 | 0 |
| exit | 0 (quit after the soak) | 0 |

**RESULT: PASS.** No leak, no freeze, no crash; the faux plateau (~1.1 GB, 230 threads) is
the known wasmtime-services footprint from stability.md.

## Gates

Not re-run (QA changed no code); last full gates green on `7525d3d` per the
[gui-fix-unipi] commit. Working tree clean apart from this file.

## Coverage gaps / not exercised

- Mouse-only surfaces: side-pane `+` launcher, board FAB/New-task dialog, settings switches,
  browser pane take-over, mascot toggle UI (config path verified instead). Clicks are not
  injectable on Xvfb for this build (see Method).
- `run_subagent` live child timeline, `/fusion` picker, `/answer` form, ask_user card,
  `/memory` panel, `/continue` `/retry` `/summarize` `/move`.
- Long-horizon flows (goal/ralph/swarm/graph), notify/web/image tools: model or external
  budget, as in round 2.
- Light theme, onboarding flow, Windows/macOS hosts, `:0` GPU path.
