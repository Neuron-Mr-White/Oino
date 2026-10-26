# Deviations: unipi-core

Owner: `unipi-core`. Format per README §5: section, change, reason, impact.

Scope of this entry: `extensions/sdk/unipi-kit` (shared guest kit: unipi `packages/core` +
the umbrella) and `extensions/unipi/core` (`unipi.core`). `unipi.utility` is not started.

## Contract differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | unipi-port §2.4 | jev key order is: resolved settings `apiKey` → secret `decision-model/apiKey` **in the caller's own secret store** → env → `credential-for`. | Secrets are namespaced per extension (extension-api §5.9), so the `decision-model` namespace's secret field (owned by `unipi.core`) is not readable by other extensions. | A key entered once in the hub only reaches `unipi.core`. See request R2. |
| 2 | unipi-port §2.2 / §2.1 | New bus topic `unipi:work-tray:changed` (one-shot) and the Oino holder topics `unipi:bg:tasks`, `unipi:subagents:list`, `unipi:footer:tps` (sticky) and `unipi:subagents:run-done` are named in `unipi_kit::topics`; `topics::STICKY` = unipi's 5 + those 3. | The tray needs a change signal (unipi used `subscribeWorkChanges`). | Producers publish via `unipi_kit::bus::emit_raw`, which applies stickiness automatically. |
| 3 | unipi-port §2.1 "work list + tray" | Work-tray contract: fan-out `work-tray.tab(TabRequest{open, initialId?})` → `TrayTab` or `[TrayTab]` (`{id, label, shortLabel?, order, counts{total,running}, visible, body?: OusNode}`); interactive `actionId`s inside `body` are `<tabId>:<action>`; `unipi.core` forwards them to the tab owner as point service `work-tray.action{tab, actionId, value?}` → `{close, refresh}`. Tab switches use the `tabs` node `onChange: "$tab"`. | The spec names the service but not its payloads. | background-tasks, subagents and dream implement these (`unipi_kit::tray`). |
| 4 | unipi-port §3.1 (start header) | The header is `setHeader{key: "unipi.core/header", spec}` with `spec.t = "responsive"`: `variants: [{minWidth: 72, node: large}, {minWidth: 40, node: compact}]`, `fallback: large`. | Extensions do not know the frontend width; OUS v1 has no width-dependent node. Frontends that do not know `responsive` render `fallback` (ui-protocol §6.1 unknown-node rule). | Request R4. |
| 5 | unipi-port §3.1 (art) | Unicrab art is OUS half-block text with one tone per glyph (palette → `error`/`warning`/`default`; black outline and shell shadow transparent); `hints.crab = image` uses an OUS `image` node with the unipi PNG. | OUS spans carry no colours, only tones (ui-protocol §6.5). | Visual only. |
| 6 | audit-unipi §1.3.13 | Catalog is unipi's `lines.ts` converted to `assets/hints.json`: **132** hints (the audit's 127 is approximate). `match` closures became declarative predicates (`{field, eq}`, `{field, in}`, `{field, wordPrefix}`, `{all}`). Two texts changed: `/unipi:kanboard open` "launches the task board in your browser" → "shows the task board panel" (D22: no web UI), "how fast Pi got ready" → "how fast Oino got ready" (D13). | WASM has no closures in data; parity with D13/D22. | — |
| 7 | audit-unipi §1.3.13 (moment tips) | The late-night tip never fires. | `host-info` exposes no local time zone; UTC would fire at the wrong hours. | Request R3. |
| 8 | audit-unipi §1.3.13 (browser) | `/unipi:hint` opens an overlay panel `unipi.core/hints` (filter `input` + one `section`/`list` per category, status badge seen/unseen/learned). Filtering applies on submit, not per keystroke. | Panels exchange actions, not keystrokes. | — |
| 9 | unipi-port §3.1 (perms) | Manifest permissions: `services.call: ["*/work-tray.tab", "*/work-tray.action"]`; `osNotify` dropped. | The tray calls producers; nothing in `unipi.core` raises OS notifications. | Minimal grant set. |
| 10 | unipi-port §3.1 (tray ↓) | `emptyArrowDown` is answered `{consume: true}` after the tray panel opened (one fan-out round trip). | No safe way to spawn the open after replying (imports need the call context). | If the round trip exceeds the frontend's 50 ms window the frontend also runs its default (a no-op on an empty composer). |
| 11 | extension-api §9.3 | Tests: pure logic is unit-tested natively (`cargo test -p unipi-kit -p unipi-core` in `extensions/`); the component is tested in the wasm host by the standalone crate `extensions/unipi/core/harness` (own `[workspace]`, run with `harness/run.sh`), with a test-only producer component `harness/probe`. `export_extension!` is gated on `target_arch = "wasm32"` in `unipi.core`. | No `MockHost` (wit-v1 deviation 13); native `cdylib` links fail on the `:`-qualified export symbols; the extensions workspace must not depend on wasmtime. | Request R5, R6. |
| 12 | runtime §10.3 | `unipi_kit::settings::Store` is read-only: values come from `InitInput.settings` and, on `oino_settings_changed`, from `control.get-settings()["extensionSettings"]` layered over the registered defaults. | No guest-side settings write import; the changed event carries keys, not values. | Request R1. |
| 13 | audit-unipi §1.2 | Umbrella behaviours not in the kit: contribution ledger/load times (`_oino/extensions`), `simple` render-style tool wrapper (utility B, render hints), command echo (host), `migrateState`/`sweepOrphanSessions` (host state GC, D10). The kit's `suite` module carries the load order, ids and namespace owners. | Host features per unipi-port §2.1. | — |
| 14 | unipi-port §5 (`fmt`) | `unipi_kit::fmt::visible_width` counts chars (no East Asian width / combining marks). | No unicode-width dependency in guests; frontends do exact layout. | Hint-line fitting is approximate for CJK labels. |

## Requests for other work

| # | To | Request |
|---|---|---|
| R1 | core-runtime / wit-v1 | A guest settings write path (e.g. `registry.set-extension-setting(namespace, key, value?, scope)`) and either values in `oino_settings_changed` or an `extension-settings(namespace)` read; confirm `control.get-settings()` includes `extensionSettings`. |
| R2 | core-runtime | Resolve `secret` fields of a registered namespace for consumers that declare it (e.g. let any extension read `decision-model/apiKey` through the settings registry), or a shared-secret grant. |
| R3 | wit-v1 | `host-info`: local UTC offset (minutes) or local wall-clock time, for time-of-day features (hints late-night, notify quiet hours). |
| R4 | spec / ui-protocol (integration at P5) + tui-client + gpui-desktop | OUS node `responsive {variants: [{minWidth, node}], fallback}` (first variant whose `minWidth` ≤ available width; none ⇒ render nothing). |
| R5 | wit-v1 | Gate the exports inside `export_extension!` on `target_arch = "wasm32"` so every extension crate can run native unit tests; or ship `testing::MockHost`. |
| R6 | integration | CI: run `extensions/unipi/core/harness/run.sh` (and `cargo test -p unipi-kit -p unipi-core` natively in `extensions/`); regenerate `extensions/dist/checksums.json` with `cargo xtask wasm` now that `unipi.core` exists; add `unipi.core` first in the bundle order (`unipi_kit::suite::LOAD_ORDER`). |
