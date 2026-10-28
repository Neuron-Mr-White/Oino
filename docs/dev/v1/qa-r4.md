# QA round 4 — desktop GUI live verification (`[qa-r4]`)

Date: 2026-10-29/30 · branch `v1-alpha` @ `a1d7063` (HEAD at start) · host: `ti`
(SSH + private `Xvfb :93` + lavapipe, release build of HEAD), driver
`scripts/gui-drive.py` over `OINO_DESKTOP_AUTOMATION` (docs/dev/v1/gui-driver.md).
Real model: `oino/deepseek/deepseek-flash` (auth.json copied by `cargo xtask ti start`).

**Status: IN PROGRESS** — this file is committed early and updated as evidence lands.

Plan (owner task):

1. Build HEAD on `ti` (`cargo xtask ti sync` + `ti build`).
2. Full driver suites: `cargo xtask gui suite unipi` (28 scenarios, real model) and the
   ZCode/desktop scenarios (`zcode` suite + the non-suite desktop scenarios).
3. `soak.sh` 30 min (faux) — background + poll; `perf.sh` 4 panes.
4. Exploratory pass through every screen with the driver (click everything; focus,
   keyboard, empty/error states, light + dark themes).
5. Look comparison side by side against `docs/dev/v1/design/ref/unipi/*` and
   `/tmp/zcode-src` docs/screens.
6. This report: verdict, per-unipi-package live table, ZCode checklist table, regressions,
   new defects P0/P1/P2 with repro + file hints, soak/perf numbers; screenshots under
   `docs/dev/v1/qa-r4/`.

## 0. Host availability — `ti` blocked, charter local fallback used

- 2026-10-29 ~03:00–03:45: every `ssh ti@ti` (also key-auth to the tailnet IP, `tailscale
  status` shows the node up) prints **"Tailscale SSH requires an additional check"** with a
  fresh approval URL per attempt (surfaced to the owner twice; not approved in time).
  Same block as the `[zcode-perf]` round (ti.md §6).
- Per CHARTER.md ("GUI testing runs on `ti` (preferred) or headless on a private `Xvfb` —
  on this PC only when `ti` is unreachable"), the round ran **locally**: one app instance at
  a time on a private `Xvfb :95` + own dbus + lavapipe, `nice -n 10`, release build of HEAD
  `27ac8fa` (= `a1d7063` + this round's QA-only commits), driven through the same automation
  socket. Runner: `docs/dev/v1/qa-r4/local-suite.py` (mirrors `local-app.sh` + the suite
  logic of `cargo xtask gui suite`, incl. per-scenario `settings`). No window ever opened on
  the owner's display (`:0`); nothing was left running.
- **Re-run on `ti` is still open** (§ Risks): the numbers below are the local-fallback
  equivalents of the ti runs (same binary, same driver, same scenarios).

## 1. Verdict

TBD

## 2. Per-unipi-package live table (suite `unipi`, real model)

TBD — filled from `target/gui-suite/unipi.json`.

## 3. ZCode checklist (design.md §8 / zcode-details.md) live table

TBD

## 4. Regressions

TBD

## 5. New defects

TBD

## 6. Soak / perf numbers

TBD

## 7. Exploratory pass notes

TBD

## 8. Evidence index (docs/dev/v1/qa-r4/)

TBD
