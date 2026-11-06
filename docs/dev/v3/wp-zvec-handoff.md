# wp-zvec / OIN-10

## Implementation

- Embedded Apache-2.0 `zg-engine`, pinned to upstream commit `407a38512bb0b778819e38035d87fb6ac229a4eb`. No zg process or daemon. Cargo feature `zvec` is optional and disabled by default in tools/core/host.
- `search.semanticEnabled` defaults false. Runtime appends `oino.search-snapshot` once; reload retains it, legacy conversations without a snapshot remain exact. Conditional grep metadata agrees with the tool catalog.
- Settings › Search is a drop-in feature component. Toggle changes future sessions. Project settings exposes an explicit consent checkbox and Build index. The host-only `build_search_index` command is absent from session/model tool catalogs.
- Index and its `*` gitignore live in the canonical project `.zvec-grep/`. Model cache is explicitly `<agent>/zvec/models`; no process-global environment mutation. Only local/potion-code-16m-v2, CPU, is admitted for queries.
- Query preflight refuses missing local index/model with Settings › Search › 'Build index'. Refresh is Off, auto_update false. No parent-index fallback. Symlinked index/manifest/storage and out-of-project query paths are refused. Results are scoped and byte-capped.
- Engine operations serialize. Generation-checked timers close the engine after five idle minutes; a drop guard handles failed/cancelled operations too.

## Verification

- `CARGO_BUILD_JOBS=8 cargo test -p oino-tools --test v3_zvec`: 4 passed (default build); `--features zvec -- --test-threads=1`: 4 passed with a real model download, real index, semantic/hybrid CPU queries and simulated idle time.
- `cargo test -p oino-tools --test v3_fs`: 7 passed.
- `cargo test -p oino-core --test v3_search_snapshot`: 2 passed, including legacy history byte preservation and reload/toggle freezing.
- `cargo test -p oino-settings v3_search`: 1 passed.
- Strict clippy: oino-tools default and zvec, oino-settings, oino-core passed. Host `--features zvec --no-deps` passed. Dependency-inclusive host strict clippy hits the existing large-enum diagnostic in unipi-proto control.rs.
- `cargo check --manifest-path apps/unipi/Cargo.toml -p unipi-host --features zvec`: passed. Protocol settings tests: 4 passed.
- Frontend `npx tsc --noEmit`: passed. SearchSettings Vitest: 3 passed.
- `windows_msvc_cpu` exists under cfg(windows), not executed on Linux. Actual Windows acceptance remains a release gate. Desktop/phone rendered inspection not performed.

## Ownership exceptions

Minimal settings module export; tools configuration and manifests/locks; core builtin.rs/runtime.rs snapshot and trusted Settings bridge; host Cargo feature and host_cmd.rs; host protocol settings.rs; dedicated core snapshot test; this handoff. No compaction/session implementation or Superagent UI files edited.

Root lock updates were narrowed to required futures/futures-util, getrandom 0.4, ignore, cc, and tokio-util updates plus new engine dependencies. tokio-util >=0.7.20 is needed by upstream's derived equality on cancellation-token options. Optional dependencies enter lockfiles but do not enter default compiled graphs.

## CPU release packaging

Build host with `cargo build --manifest-path apps/unipi/Cargo.toml -p unipi-host --release --features zvec`; omit feature for unchanged exact-only distributions. Never enable upstream cuda/vulkan features. Upstream uses verified CPU native SDKs; Python 3/curl are build-time prerequisites, not query subprocesses.

Upstream engine build.rs stages ONNX Runtime 1.21 shared libraries beside binaries. Release installers must preserve those staged runtime files beside the host (`libonnxruntime*.so*` Linux, `.dylib` macOS, corresponding ONNX DLLs Windows), not just copy the executable. Respect native SDK licenses/notices and ship Apache-2.0 attribution. The default Potion model uses model2vec; do not pre-download or bundle model weights as a substitute for consent.

Build and smoke-test separately on Linux x64/arm64, macOS x64/arm64 (merge compatible binaries and runtime libraries for universal delivery), Windows MSVC x64/arm64. Run `cargo test -p oino-tools --features zvec --test v3_zvec` on actual Windows; the named Windows case executes indexing and hybrid CPU inference. Validate clean-machine loader behavior from the installed directory with no development-library environment paths. Installer changes and cross-platform release artifacts are not produced by this package.

## Open

- Actual Windows/MSVC and macOS runtime packaging remain unverified.
- Project-scoped Settings is required to select the build root; global Search explains how to open it.
- Semantic result counts count indexed entities, not exact matching lines. Exact mode retains ripgrep semantics.
