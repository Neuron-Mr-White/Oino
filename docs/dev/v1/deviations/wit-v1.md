# Deviations: wit-v1

Settled rows #1–#19 are folded into
[extension-api.md](../extension-api.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

| # | Item | Notes |
|---|---|---|
| O1 | TLS fingerprinting | `http` `impersonate` is accepted but served by reqwest (`impersonated: false`); no `wreq`; no OS part in the profile (unipi-integrations R2). |
| O2 | Process stats | ✅ Resolved 2026-10-29: Linux `/proc`, macOS `ps`, Windows `Get-CimInstance`/`taskkill`; byte counters and process state are platform-limited (documented in `process.rs`). |
| O3 | Bash adoption | `tasks.adopt-bash` forwards to core, which has no bash detach yet (core-runtime O1). |
| O4 | `testing::MockHost` | Not implemented; components are tested through `oino_wasm_host::testing::Harness`. Gate `export_extension!` on `wasm32` so native unit tests link (unipi-core R5). |
| O5 | iOS | Select wasmtime Pulley on no-JIT targets so components load on iOS devices (gpui-mobile). |
| O8 | Contract crate | `core` feature of `oino-wasm-host` stays until core re-exports `oino-extension-api` (#11). |
