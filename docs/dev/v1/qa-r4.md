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

## 0. Host availability

- 2026-10-30: `ssh ti@ti` prints "Tailscale SSH requires an additional check"
  (URL changes per attempt) — waiting for the owner to approve; retrying.

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
