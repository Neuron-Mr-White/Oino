# App performance log (§3.6)

Budgets come from app.md §3.6. Measured on the capture harness
(`vite.config.capture.ts`, `?scenario=` + mock host) — absolute numbers include
dev-server/module cost; the in-app rows isolate view cost.

## WP-A29 — phone pass (2026-10-31)

Device: **Redmi Note 7** (Android 10, Chrome 156 — below the Pixel-7a-class
budget target; R11 says treat misses here as early warnings, file fixes for A30).
Driven over `adb reverse` + `chrome_devtools_remote` by
`docs/dev/v2/design/shots/app/a29-device.mjs`.

| §3.6 budget | Measured (Redmi Note 7) | Verdict |
|---|---|---|
| 2 000-turn session opens ≤ 120 ms to first paint | nearest scenario is `timeline-many` (**80** settled turns + live tail): cold dev-server open ≈ **10.0 s** (module + mock dominated, not representative); in-app open `/chat` → `/s/` first turn ≈ **8.0 s** | **miss** — the in-app subscribe→parse→render path is the slow part on this CPU; 2000-turn row still unmeasured (no 2000-turn scenario yet, A30) |
| scroll ≥ 50 fps phone, no long task > 50 ms | p50 frame **16.7 ms** (~60 fps) but p95 **150 ms**; **27 long tasks > 50 ms** during a ~4 s fast scroll | **miss** on the tail — virtualized turns keep p50 fine; long tasks likely stream/markdown work on the weak CPU |
| streaming 100 k reply input latency < 50 ms | `insertText`→paint during a live stream: samples 12–86 ms, **max 86 ms** | **miss** on this device |
| DOM nodes ≤ 3 000 with 2 000 turns | **546** nodes with the 80-turn virtualized session (6 turns mounted) | pass at 80 turns; 2000-turn row open (A30) |
| switching open sessions ≤ 60 ms | **not measurable** — `/s/s001` → `/s/s002` unmounts the conversation and the new session never rendered (selector timeout > 10 s) | **blocked by the known conversation-unmount crash** (Solid `cleanNode`, also hits jsdom — owned by A6/A9/A12 in-flight work; re-measure when it lands) |

### Back stack + IME (same run)

- App-Back (the Tauri plugin path, Escape-equivalent): ⋯ Panes → Git layer ✓ closes; tree sheet ✓ closes; stack left empty (`backDepth() === 0`, also unit-tested).
- OS Back (`input keyevent 4`): conversation route → `/chat` ✓. An open IME consumes the first Back to close the keyboard (standard Android), hash unchanged.
- IME: tapping the composer opens the keyboard; `visualViewport` 718 → 464 px, `--vvh`/`--vv-bottom` track it (inset **254 px**); sheets/dock ride the inset.
- Two `pushBack` stack leaks found and fixed this pass: `FilesLayer` and `StatusPaneLayer` never popped their handler on unmount — a stale handler would have eaten a later Back.

## Desktop

(filled by A30)
