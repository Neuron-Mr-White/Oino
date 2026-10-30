# Wave 2 QA report — unipi-host H1–H10, ext-abi X4–X6, ui-protocol U3–U8

Date: 2026-10-30/31 · Branch: `v2` · Rust evidence verified on HEAD `e3a42a3`
(`e3a42a38…`, full hash via `git rev e3a42a3`); at report time HEAD is `a99c8a1`, which
differs from `e3a42a3` **only in `apps/unipi` TS/screenshots** (`git diff --stat
e3a42a3..a99c8a1 -- crates/ extensions/ xtask/ wit/ .github/` is empty), so every Rust-side
verdict below holds for `a99c8a1` as well. Reviewer: wave2-qa. Machine: Linux x86_64,
rustc/cargo 1.99.1, 32 CPUs.

Scope: `specs/host.md` §12 WP-H1..H10, `specs/ext-abi.md` WP-X4/X5/X6 (WIT
`oino:extension@2.0.0`), `specs/ui-protocol.md` §14 WP-U3/U4/U5/U6/U7a/U7b/U8, the wave
gates, the **cross-machine iroh proof**, and DIRECTION/RULINGS compliance.

## Verdict

**19/19 WPs PASS on their acceptance rows** (H9's manual tray checklist remains open by
nature — automated rows pass; see P3-2). **Three gate-level defects keep the tree red
overall**: root clippy (P1-1), extensions-workspace clippy (P1-2/P2-3), and the
extensions-workspace test gate broken at HEAD by wave-3a's committed `unipi.file-summary`
(P1-3, cross-wave contamination). No P0. The cross-machine proof over iroh (this PC ↔
fresh Agent37 Debian 12 sandbox) passed end to end: pair → `sessions_list` →
`session_new` + streamed faux reply → terminal echo, relay path, and the sandbox was
deleted afterwards.

| WP | Verdict | Key evidence (re-run by QA on this machine) |
|---|---|---|
| H1 move + proto v2 | PASS | `unipi-proto/src/version.rs:6` `PROTO_VERSION="2.0.0"`; control pruned to Hello/Ok/Err/Resize/Exit/Title/BrowserCmd/BrowserEvent; `session.rs`/`term.rs`/`settings.rs` modules exist; root workspace check/test/clippy-minus-P1-1 green; TS suite `npm test` 1781/1782 (single failure is in-flight wave-3a WP-A1, P3-3) |
| H2 skeleton + transport | PASS | `local_socket` 2/2, `instance` 3/3 (load-flaky once, see P2-2), `pairing_atomic` 2/2, `transport` 1/1, `pair_json` 3/3, `admin_ops` 1/1; ALPN `unipi/oino/1` (`endpoint.rs:18`); `~/.oino/host` layout `data_dir.rs` ($OINO_HOST_DIR override, never `~/.unipi`); Windows pipe path exercised in-CI only via the integration job (P3-1) |
| H3 session hub | PASS | `sessions_hub` 3/3: `create_prompt_concurrent_sessions_crash_marks_one_and_archive_moves_to_trash`, `cards_are_grouped_by_project_and_close_keeps_the_file`, `host_notices_are_retained_for_late_clients` |
| H4 session protocol + TS | PASS | `session_stream` 4/4 incl. `five_thousand_deltas_coalesce_into_few_frames`, `resume_replays_from_the_ring_reset_after_a_dispose_and_two_subscribers_agree`; TS `packages/protocol session.ts` ([host-h4] e6d4018) tests green inside the 1781 |
| H5 takeover | PASS | `session_takeover` 4/4: `tui_style_holder_loses_the_session`, `second_host_marks_the_first_stream_taken_over`, `tui_style_holder`, `takeover_labels_follow_the_holder_mode` |
| H6 terminal | PASS | `term` 8/8: open→write→read, `resize_is_throttled_last_wins`, `detach_then_reattach_keeps_the_process_and_replays_nothing`, `kill_ends_the_stream_and_forgets_the_id`, `cap_evicts_the_oldest_idle_terminal` |
| H7 host services | PASS | git 9/9 (incl. `turn_files_restricts_status_and_diffs_to_the_given_paths`), fs_scope 1, forward 1, push 3, notify_config 4, sharing 1, sharing_leave 5, admin_ops 1, blob 11 unit, `settings` 3/3 |
| H8 cron merge | PASS | `cron_ops` 4/4, `cron` 6/6, `cron_notify` 6/6, `cron_wake` 1/1; host embeds unipi-cron (`cron_serve.rs`); integration cron scenario green (104 ms add+run_now); `oino cron` gone (see X4) |
| H9 updater + lifetime | PASS\* | `update::` 8/8 (faux feed, restart-exit, version refuses); TRAY_TITLE `UniPi (oino)` (`apps/unipi/src-tauri/src/desktop.rs:24` + tests); app config dir `dev.unipi.oino` (`lifecycle.rs:30`); \*manual tray checklist rows in `qa/host-h9-tray.md` still unticked and the tauri crate test run stays deferred behind in-flight wave-3a (P3-2) |
| H10 integration + perf | PASS | `cargo test -p unipi-host --test integration -- --ignored` green ×2 (QA re-run): startup 209 ms; 400 k-char stream → 27 frames/115 events, delta frames 14 ≤ limit 27; 50-session list 3.99 ms (≤150 ms); RSS 10 idle sessions Δ0 MB; numbers also recorded in `qa/host-perf.md`; CI jobs `v2-host-integration` (Linux, blocking) + `…-windows` (secondary) wired in `ci.yml:74-110` |
| X4 cron as host service | PASS | crates `unipi-cron`/`unipi-cron-schedule` (schedule builds for `wasm32-wasip2`: `cargo check -p unipi-cron-schedule --target wasm32-wasip2` ok); `jobs/<id>.json` layout + `jobs.json.imported` migration + watcher (`paths.rs`, `embedded.rs`; `serve.json`/installers deleted); sandbox host log: `host services registered names=["host","kanboard","cron"]`; `oino --help` has no cron/browser subcommands and `oino cron <x>` parses as a prompt (subcommand removed) |
| X5 browser host service | PASS | `extensions/unipi/browser` id `unipi.browser`, manifest `"hostServices":["browser"]`, transport `host_service::call("browser",…)`/`available` (`src/lib.rs:60,74`); `wit/oino-extension` has no browser/cron/kanboard interface; `crates/oino/src/browser.rs` deleted; no `oino-browser`/`-protocol` deps in `oino`/`oino-extension-api`/`oino-wasm-host`; `BROWSER_SERVICE="browser"` (`sessions/services.rs:46`); `browser.rs` host test green (flaky once, P2-1) |
| X6 WIT v2 cutover | PASS | `wasm-tools component wit wit/oino-extension` validates; every `.wit` package line `oino:extension@2.0.0`; `API_VERSION="oino.extension/2"` with the 1.x→"rebuild" diagnostic (`manifest.rs:502,755-768`); `host-info` reports `WIT_VERSION="2.0.0"` (`imports/core.rs:99`); `cargo xtask wasm` + `git diff --exit-code extensions/dist/checksums.json` clean (38 components); rebuilt bundle exercised by `rpc`/`frontend` tests (`builtin:ui-probe`) green in the workspace run |
| U3 registry + wire | PASS | `oino-core ui::` 23/23 (ordering/seq, coalescing, stack slots, dispose, foreign-id rejection, reserved gate, `a_revision_gap_requests_a_resync`, `reserved_fallbacks_are_set_by_update_panel`, `pi_rpc_extension_ui_examples_round_trip_byte_for_byte`); `oino-rpc` ui_wire 2/2 (`v2_hello_receives_registry_patches`, `pi_clients_never_see_registry_patches`) + `degradation_follows_the_declared_capabilities` |
| U4 TUI renderer | PASS | `ui_nodes` golden suite + 156 insta snapshots; `ui_keys` 15/15, `ui_slots` 11/11, `snapshots` 5/5; **bench**: `cargo bench -p oino-tui --bench ui_render` → **1000-node panel median 0.90 ms, p95 1.44 ms** (budget 2 ms); `ui_probe_round_trips_every_request` green (`oino-wasm-host --test probes`) |
| U5 oino-view | PASS | 41/41; registry mirror driven by the shared `crates/oino-ui/fixtures/registry/*.json` corpus (`ui_state.rs:1530`) |
| U6 SDK | PASS | `cargo xtask wasm` builds all 38 components incl. `fixture-ui-probe`; doc-tests in the ext workspace run; `unipi-kit` OSS-2 builder compiles |
| U7a migrate panels | PASS | `scripts/ui-v1-gate.sh` → "no OUS v1 node trees in extensions" (exit 0); harnesses green (below); reserved ids `unipi.kanboard/board`, `unipi.info-screen/dashboard`, `unipi.long-horizon/visualize` with fallbacks (`oino-ui/src/slot.rs:64-78`, registry tests) |
| U7b render specs / cards | PASS | same gate green; utility harness 10/10 (render rows + TUI golden at 40/100 cols); `ui-probe` emits every node/slot (probes test); 156 TUI snapshots updated with badge-span cards ([ui-u7b] commits) |
| U8 settings OSS-2 | PASS | `oino-view` 41 (settings_hub incl. `fixtures/registry/snapshot.json`), `oino-core settings` 9/9 (layers, invalid→fallback+diagnostic, default-write=unset), `unipi-kit` oss2 incl. `v1_builder_namespaces_load_unchanged_in_oss2` (green as a test; its *lint* is P1-2); core `oino.*` namespace round-trip e2e green |

## Full gates (re-run by QA)

| Gate | Command | Result |
|---|---|---|
| Format (root) | `CARGO_BUILD_JOBS=8 cargo fmt --all --check` | **PASS** (exit 0) |
| Format (extensions) | `cargo fmt --manifest-path extensions/Cargo.toml --all --check` | **PASS** (exit 0) |
| Lints (root) | `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL** — `crates/oino/src/lib.rs:314` `large_futures` 20304 B (P1-1); also fails CI's `core workspace` job which adds `-W clippy::pedantic` |
| Lints (extensions) | `cargo clippy --manifest-path extensions/Cargo.toml --workspace --all-targets -- -D warnings` | **FAIL** — `unipi-kit` `settings/oss2.rs:715` `too_many_lines` 119/100 (P1-2); pre-existing `fixture-panic-probe`/`fixture-hibernate-probe` (`unused_async`, panic-only-if) (P2-3) |
| Tests (root) | `CARGO_BUILD_JOBS=8 cargo test --workspace` (env cleaned: `UNIPI_KANBOARD_*` unset) | **PASS** — exit 0, 176 green test binaries, 0 failures |
| Tests (extensions) | `cargo test --manifest-path extensions/Cargo.toml --workspace` | **FAIL at HEAD** — `unipi-file-summary` native cdylib link: `rust-lld: …list:3: ; expected, but got :` (P1-3, wave-3a `[app-a26]`). **PASS with `--exclude unipi-file-summary`**: 68 binaries, 0 failures |
| Wasm dist | `cargo xtask wasm` + `git diff --exit-code extensions/dist/checksums.json` | **PASS** — 38 components (37 wave-2 + file-summary), checksums idempotent |
| Harnesses | `sh extensions/unipi/{ask-user,core,kanboard,notify,subagents,utility,workflow}/harness/run.sh` | **PASS** — 3+3+6+5+6+10+7 = 40 tests, 0 failures; (wave-3a `file-summary` harness also 10/10) |
| ui-v1 gate | `scripts/ui-v1-gate.sh` | **PASS** — no OUS v1 node trees |
| pi oracle | `node scripts/pi-oracle/core/session-load.mjs` | **PASS** — "all checks passed" (7 checks); pi rpc-extension-ui examples byte-for-byte via `oino-core ui::bridge` test |
| Docs | `sh scripts/docs-check.sh` | **PASS** |
| WIT | `wasm-tools component wit wit/oino-extension` | **PASS** — `oino:extension@2.0.0` |
| H10 integration | `cargo test -p unipi-host --test integration -- --ignored --nocapture` | **PASS** — numbers in the H10 row |

Note (env hazard, confirmed again): a QA/agent shell exporting `UNIPI_KANBOARD_MAX_SESSIONS`
breaks `unipi-kanboard --test board::eight_threads_race_but_the_two_session_cap_holds`
(wave-1 documented); run gates with those vars unset.

## Cross-machine proof (required) — iroh between this PC and a fresh Linux machine

Method: fresh Agent37 sandbox (`a37 create agent37-base` → id `8224z9z8nw`, Debian 12,
gVisor, us region) as the **host machine**; this PC as the **client**. Host binary built
here for `x86_64-unknown-linux-musl` (fully static, zig cc for C deps) from the `v2`
workspace, stripped+xz (21.7 MB), uploaded via chunked `a37 exec` base64 (`a37 put`'s
300 KB chunks exceed the 128 KB per-arg exec limit); sha256 verified on both sides.
On the sandbox: isolated `--data-dir /home/node/qa2/host`, sandboxed `OINO_AGENT_DIR`
staging `extensions/dist/oino.openai-compat` (127.0.0.1 host allowed) + `models.json`
pointing at a python SSE faux provider on the sandbox loopback; `serve --mode manual
--no-extensions` (n0 relay preset). Nothing on this PC touched `~/.unipi`, `~/.oino/host`
or the owner's running host (client only makes outbound iroh connections).

Client: `/tmp/qa2/client` (scratch cargo bin: iroh 1.4.1/iroh-tickets 1.0.0 — same as the
repo lock; speaks the frame protocol + `unipi/oino/1` ALPN directly; transcript in
`/tmp/qa2/crossmachine.log`). Pairing via the `unipi-host pair --json` link+code
(`unipi://pair?t=…&c=JSC47JGKDF`), consumed by the client exactly like the phone app.

```
client endpoint: b912991b…ad55927
  op pair -> "ok" in 1283 ms (connect 909 ms)
PAIRED with host "8224z9z8nw" in 1.284s: "9015aea6c2…1795078dec"
  op sessions_list -> "ok" in 519 ms (connect 227 ms)   [1 session from the 1st run]
SESSION_NEW ok in 524 ms: 01a18cc2-e47f-73f3-971e-748c3f7c58e4
SET_MODEL faux/faux-1 ok ; PROMPT reply ok (correlated id=p)
STREAM ok: 17 events (5 deltas, 900 chars) first-event 233 ms, total 591 ms; seq 1..26
TERMINAL echo OK in 744 ms (term_id 761da7eccdec83d2)   [echo WAVE2-TERM-OK over host PTY]
PATH: Relay(https://use1-1.relay.n0.iroh.link./) usage=Active
ALL CROSS-MACHINE STEPS COMPLETE
```

- **Steps proven**: `pair` (device registered host-side: `devices` shows
  `wave2-qa-laptop`), `sessions_list`, `session_new` + `set_model` + `prompt` with the
  **faux loopback provider streaming through the host** (seq strictly monotonic,
  `message_update` deltas coalesced to 5, `agent_settled` observed), `term_open` +
  input frame + marker echoed back from the sandbox PTY.
- **Path type**: **relay** (`use1-1.relay.n0.iroh.link` active; the gVisor sandbox
  cannot hole-punch — `netlink: Not supported` — so no direct UDP path was available;
  this is the relay fallback case working as designed).
- **Latency**: connect 227–909 ms; one-op RTT ~0.5 s; first streamed event 233 ms after
  prompt; full 900-char stream 591 ms.
- **Cleanup**: host + SSE server killed on the sandbox, `a37 rm 8224z9z8nw` →
  `{"deleted":true}`, `a37 ls` empty.

## DIRECTION / RULINGS compliance

| Rule | Evidence |
|---|---|
| oino carries no unipi-specific WIT | `wit/oino-extension` has **no** `kanboard`/`cron`/`browser` interface (only `host-service`); kanboard/cron/browser are host services (`sessions/services.rs`) |
| `oino cron` removed | `oino --help` lists no cron; `oino cron x` parses as a prompt; scheduler lives in unipi-host (`cron_serve.rs`) |
| Host data under `~/.oino/host` | `data_dir.rs` (`$OINO_HOST_DIR` default `~/.oino/host`; `~/.unipi` never opened — R8) |
| identifier `dev.unipi.oino` | `apps/unipi/src-tauri/src/lifecycle.rs:30-36` (+ mac variant) |
| tray label `UniPi (oino)` | `desktop.rs:24` `TRAY_TITLE` + tooltip tests |
| no service install | `main.rs:26,95` ("v2 has no service install"); unipi-cron has no `install.rs` (systemd/launchd/schtasks deleted) |

## Defects

- **P1-1 root clippy gate red at HEAD** — `cargo clippy --workspace --all-targets -- -D
  warnings` fails: `error: large future with a size of 20304 bytes` at
  `crates/oino/src/lib.rs:314` (`modes::interactive::run(...)`). Deterministic (re-ran
  `-p oino` twice). Repro: `cargo clippy -p oino --all-targets -- -D warnings`.
  Expected: green. Actual: exit 101. Breaks the CI `core workspace` job. Likely cause:
  the interactive-mode future grew past the 16 KiB `large-futures` threshold in wave-2
  commits touching `crates/oino/src/lib.rs`/`modes/` (`d55a7db` [abi-x5], `416ee6e`
  [abi-x4] are the last commits touching `lib.rs`). Fix: `Box::pin` the call or shrink
  the options struct.
- **P1-2 extensions clippy red (wave-2 file)** — `unipi-kit` `src/settings/oss2.rs:715`
  `fn v1_builder_namespaces_load_unchanged_in_oss2` is 119 lines (`clippy::too_many_lines`
  100). Repro: `cargo clippy --manifest-path extensions/Cargo.toml -p unipi-kit
  --all-targets -- -D warnings`. Introduced by `[ui-u8]` `cc2ee52` (committed). Fix:
  split the test or `#[allow]` with reason.
- **P1-3 extensions test gate broken at HEAD by committed wave-3a code** — native build
  of `unipi-file-summary` (`[app-a26]` `6bf3927`/`728cab8`, committed) fails to link:
  wit-bindgen 0.62 exports guest fns as `[async-lift]oino:extension/guest@2.0.0#…` and
  rust-lld cannot parse those names in the cdylib `--version-script`
  (`rust-lld: error: …/list:3: ; expected, but got :`; symbol dump captured via a linker
  wrapper). Repro: `cargo build --manifest-path extensions/Cargo.toml -p
  unipi-file-summary`. Wave-2 extensions are unaffected (gate passes with
  `--exclude unipi-file-summary`), but the shared extensions gate is red for everyone.
  Likely fix: stop building a native cdylib for wasm-only extensions (`crate-type` cfg)
  or gate the async-lift exports to the wasm target.
- **P2-1 flaky `browser.rs::remote_browser_streams_frames_and_takes_input`** — failed
  once inside a loaded workspace run (`Stopped(0)` at `tests/browser.rs:132`), green on
  immediate retry and later runs. Timing/startup race around the snap Chromium +
  headless adaptation. Repro: run under load; not deterministic.
- **P2-2 flaky `instance.rs` under parallel load** — 2/3 tests timed out (30 s) once
  while a 12-job build ran concurrently; 3/3 green on 4 consecutive solo runs. Spawned
  real binaries + lock timeouts under contention.
- **P2-3 extensions clippy failures on wave-1-era fixtures** — `fixture-panic-probe`
  (`only_a_panic_in_if`, `unused_async`) and `fixture-hibernate-probe` (`unused_async`).
  Pre-existing (ext-workspace clippy was not a wave-1 gate); fix alongside P1-2.
- **P3-1 H2 Windows `local_socket` CI row** — no dedicated Windows job runs
  `--test local_socket` ("coffee"); the named-pipe path is covered indirectly by
  `v2-host-integration-windows` (runs the integration e2e on the pipe,
  `continue-on-error`). Consider an explicit `coffee` run or accept the integration job.
- **P3-2 H9 manual tray checklist open** — `qa/host-h9-tray.md`'s Linux+Windows manual
  rows are still unticked and `cargo test --manifest-path apps/unipi/src-tauri/Cargo.toml`
  remains deferred (blocked by in-flight wave-3a app modules, per that doc).
- **P3-3 TS suite 1 failure (not wave-2)** — `npm test` in `apps/unipi`: 1781/1782;
  the single failure is `mock.test.ts > the A1 scenarios load into a world` (timeout) in
  files modified by in-flight wave-3a WP-A1 (`src/dev/mock/*`, `src/lib/host/mock*`).
  H1/H4 protocol tests themselves pass.

## Notes

- The two `extensions/unipi/{kanboard,workflow}/harness/Cargo.lock` modifications in the
  working tree are harness `run.sh` lock-refresh churn from parallel workers (pre-dated
  this QA), not from wave-2 code.
- Cross-machine scratch artifacts (client source, logs) live under `/tmp/qa2/`
  (`crossmachine.log`, `host-musl.log`, gate logs) for follow-up inspection; the sandbox
  itself is deleted.
