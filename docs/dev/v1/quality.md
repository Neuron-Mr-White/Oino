# Quality: gates, tests, CI

Status: binding for every work item. Owner of shared config/CI: `integration`.

## 1. Gates (charter, per commit, for touched crates)

Root workspace:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings          # pedantic is enabled via workspace lints
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo xtask layering                                            # architecture §3 forbidden edges
```

Extensions workspace (`cd extensions`): `cargo fmt --check`, `cargo clippy --target
wasm32-wasip2 --all-targets -- -D warnings`, native unit tests `cargo test` (SDK `testing`
feature), `cargo xtask wasm --check` (build + checksum diff).
Apps workspace (`cd apps`): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`.

A work item reports which gates it ran. A commit that touches only docs runs none.

## 2. Toolchain and shared configuration

- `rust-toolchain.toml`: `channel = "1.99.0"`, `components = ["rustfmt", "clippy"]`,
  `targets = ["wasm32-wasip2"]`. Mobile targets are added by the mobile build script.
- Edition 2024, `rust-version = "1.99"` in every crate (`[workspace.package]`).
- Workspace lints (root, extensions and apps `Cargo.toml`):

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"
unreachable_pub = "warn"
unused_qualifications = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
todo = "deny"
unimplemented = "deny"
dbg_macro = "deny"
print_stdout = "warn"     # binaries allow at the output layer with an inline #[allow(.., reason = "…")]
print_stderr = "warn"
```

- `clippy.toml`: `allow-unwrap-in-tests = true`, `allow-expect-in-tests = true`,
  `allow-panic-in-tests = true`, `allow-print-in-tests = true`.
- Every `#[allow]`/`#[expect]` carries `reason = "…"` (Rust 1.81+ syntax).
- Crates opt into `#![warn(missing_docs)]` via the workspace lint; crate-level docs (`//!`)
  describe the boundary and an API map (charter).
- Errors: `thiserror = "2"`; `anyhow` only in `crates/oino`, `xtask`, `apps/oino-desktop`,
  `apps/oino-mobile`.
- Dependency policy: caret requirements in `[workspace.dependencies]`, except exact pins:
  `wasmtime*`, `wit-bindgen*`, `wasm-tools`/`wit-component`, `agent-client-protocol*`, `gpui*`,
  `gpui-kit`. `cargo deny check` (licenses: MIT, Apache-2.0, BSD-2/3, ISC, Zlib,
  Unicode-3.0, MPL-2.0 (file-level), CC0; advisories deny; no git deps outside `apps/vendor`).
- `serde_json` with feature `preserve_order` (D25).
- Temp files in tests: never `/tmp` directly (user quota on the dev machine). Use
  `tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR"))` in integration tests and
  `tempfile::tempdir_in(std::env::var("OINO_TEST_TMP")…)` helpers in unit tests (falls back to
  `target/tmp`). Tests must pass with `cargo test -j 4` under memory pressure.

## 3. Test layers

| Layer | Where | Tooling |
|---|---|---|
| Unit | inline `#[cfg(test)]` modules | std test, `proptest` for parsers (session lines, template args, truncation) |
| Crate integration | `crates/<c>/tests/` | real files in temp dirs; faux provider; `FakeExtensionRuntime` |
| Golden vs pi | `crates/<c>/tests/golden/` + fixtures in `crates/<c>/tests/fixtures/` | §4 |
| WASM host | `crates/oino-wasm-host/tests/` | fixture components from `extensions/dist` (built by `cargo xtask wasm`; tests skip with a clear message when dist is missing locally, never in CI) |
| Extension | per component: native `MockHost` tests + harness tests | extension-api §9.3 |
| E2E | `tests/e2e/` (integration) | drives the `oino` binary in print/json/rpc/acp modes with the faux provider component |
| Client contract | `crates/oino-client/tests/` | same suite over InProcess and Rpc backends |
| UI | `crates/oino-tui/tests` (ratatui `TestBackend` snapshots with `insta`), `apps/oino-ui` view tests | |

## 4. Golden testing against pi 1.4.0

### 4.1 Faux script (shared by Rust faux provider and the pi oracle)

```json
{
  "model": { "provider": "faux", "id": "faux-1", "api": "faux", "contextWindow": 200000, "maxTokens": 8192, "reasoning": true },
  "responses": [
    { "content": [{"type": "thinking", "thinking": "plan"}, {"type": "text", "text": "Hello"}], "stopReason": "stop",
      "usage": {"input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0} },
    { "content": [{"type": "toolCall", "id": "call_1", "name": "read", "arguments": {"path": "a.txt"}}], "stopReason": "toolUse" },
    { "error": "rate limit exceeded (429)", "errorKind": "retryable" },
    { "error": "prompt is too long: 250000 tokens > 200000", "errorKind": "overflow" }
  ],
  "chunk": { "text": 4, "thinking": 4, "toolArgs": 8 }
}
```

Each request consumes the next response; streamed as `start`, `*_start/_delta/_end` with the
given chunk sizes, then `done`/`error`. Costs computed from model cost (zero by default).
Implementations: `oino_agent::testing::FauxStreamFn` (native), `extensions/fixtures/provider-faux`
(component), and `scripts/pi-oracle/faux-provider.ts` (a pi extension using
`pi.registerProvider("faux", {streamSimple})`).

### 4.2 Oracle

`scripts/pi-oracle/` (Node 24 + `@earendil-works/pi-coding-agent@1.4.0` installed in
`scripts/pi-oracle/node_modules`, version-locked by `package-lock.json`):

- `record-json.ts <scenario>` — runs `pi --mode json` with the faux extension, cwd = a fixture
  workspace, `PI_CODING_AGENT_DIR` = temp agent dir → `*.json.jsonl`.
- `record-rpc.ts <scenario>` — feeds scripted RPC commands to `pi --mode rpc` → transcript.
- `record-print.ts` — stdout/stderr/exit code.
- `context.ts <session.jsonl>` — `buildSessionContext`/projection output via pi's
  `session-manager.js`.
- `prompt.ts <options.json>` — `buildSystemPromptSections` output.
- `tools.ts <case>` — runs pi tool definitions directly (read/write/edit/bash/grep/find/ls)
  against fixture files.
- `compaction.ts <case>` — `prepareCompaction`/`findCutPoint` results.

Scenarios live in `scripts/pi-oracle/scenarios/*.json` (faux script + inputs). Outputs are
committed under the consuming crate's `tests/fixtures/pi/`. `cargo xtask golden --regen
<suite>` reruns the oracle (needs Node); CI only compares.

### 4.3 Normalisation and comparison

Before comparing, both sides replace: entry ids → `E1, E2…` (by first appearance), session id
→ `S`, ISO timestamps → `T`, ms timestamps → `0`, `durationMs`/`wall_time_seconds` → `0`,
absolute fixture paths → `$CWD`, agent dir → `$AGENT`, temp file names (`pi-bash-*.log` /
`oino-bash-*.log`) → `$TMPLOG`, and apply the D13 product substitutions (`Oino`↔`pi`, docs
section, env guideline) on the pi side. Comparison = parsed JSON equality per line (key order
ignored), plus exact line count and order.

### 4.4 Required suites (owner)

| Suite | Owner | Scope |
|---|---|---|
| `session-context` | core-types-session | ≥ 30 generated sessions (branches, compactions incl. retain-none, context edits incl. branch-relative, labels, custom messages, bash executions, legacy pi v1/v2) |
| `legacy-migration` | core-types-session | Oino v1/v2 files → v3 goldens (generated from `legacy/` test data) |
| `tools` | core-runtime | every string in audit-pi §10 + images + truncation + fuzzy edit + CRLF/BOM |
| `system-prompt` | core-runtime | default, custom SYSTEM, append, context files, skills (read/bash/indirect), custom sections, forced prompt |
| `compaction` | core-runtime | ported `compaction.test.ts` cut-point cases, split turns, file tracking |
| `json-mode` | modes | 15 scenarios: plain reply, tool calls (parallel/sequential), steering, follow-up, abort, retry, overflow recovery, compaction, bash `!`, skills, templates, extension command (faux ext), errors |
| `rpc` | modes | every command success + error, queue semantics, UI sub-protocol round trip |
| `print` | modes | exit codes and stdout/stderr |
| `extension-events` | wit-v1 + core-runtime | `echo` fixture payload log vs a pi recording extension over the same scenarios |
| `acp` | acp | JSON-RPC transcripts for initialize, new, prompt with tools, cancel, load (replay), permissions |

## 5. Performance benchmarks (criterion, `benches/`, run on demand + weekly CI)

| Bench | Target (dev machine) |
|---|---|
| `SessionManager::open` 10 000 entries | < 60 ms |
| `build_session_context` 10 000 entries | < 15 ms |
| first event after `prompt` (faux provider, in-process) | < 5 ms |
| wasm cold start, all bundled components, warm compile cache | < 150 ms |
| wasm event dispatch round trip (`echo`, `turn_end`) | < 200 µs median |
| TUI frame render, 2 000-row transcript | < 4 ms |
| JSON mode throughput | ≥ 50 000 events/s |

Regressions > 20 % fail the weekly job.

## 6. CI (GitHub Actions, `.github/workflows/`, integration)

| Job | Runs on | Steps |
|---|---|---|
| `fmt` | ubuntu | fmt checks for all three workspaces |
| `clippy` | ubuntu | root + extensions (wasm32-wasip2) + apps (Linux deps installed) |
| `test` | ubuntu | `cargo test --workspace -j 4` (root), extension native tests |
| `wasm` | ubuntu | `cargo xtask wasm --check` (rebuild, checksum diff) then wasm-host + e2e tests |
| `docs` | ubuntu | rustdoc `-D warnings`; `wasm-tools component wit wit/oino-extension` |
| `layering` + `deny` | ubuntu | `cargo xtask layering`, `cargo deny check` |
| `golden` | ubuntu | all §4.4 suites (part of `test`, listed for visibility) |
| `bench` | ubuntu, weekly | §5 |
| `release` | manual | `cargo xtask remote-build <host\|local>` (`xtask/src/remote.rs` → `scripts/release/build-host.sh`): git bundle + `extensions/dist` over ssh, detached build on coffee (Windows) / mac (macOS universal), Linux + Android locally; smoke tests; artifacts in `target/release-artifacts/<target>/`; assembles GitHub release |

Pull requests are not used (agents commit to `v1-alpha`); CI runs on every push to `v1-alpha`.
The graph owner treats a red CI on `v1-alpha` as a blocking defect for the item that broke it.

## 7. Release checklist (P4)

1. All gates and CI jobs green; no open deviations without an integration decision.
2. Golden suites cover every row of audit-pi §§4–17 marked ❌/⚠️/🟡 (tracked in
   `tests/e2e/PARITY.md`, integration).
3. All 23 unipi components bundled, each with its acceptance (unipi-port §6) reported.
4. Desktop builds on Linux/Windows/macOS and mobile on one Android and one iOS device pass
   gpui-app §8.
5. User docs rewritten (integration) with the exact file names referenced by the system
   prompt docs section (runtime §9.4).
6. `legacy/` deleted (P5) and deviations folded into specs.
