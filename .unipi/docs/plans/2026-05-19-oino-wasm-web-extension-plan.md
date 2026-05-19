---
title: "Oino WASM Web Extension — Implementation Plan"
type: plan
date: 2026-05-19
workbranch: "feat/wasm-web-extension"
specs:
  - .unipi/docs/specs/2026-05-19-oino-wasm-web-extension-design.md
---

# Oino WASM Web Extension — Implementation Plan

## Overview

Build the first disposable vertical slice of Oino's native extension platform on branch/worktree `feat/wasm-web-extension`.

This plan covers a repo-local WASM extension package under `extensions/web/` that contributes a model-visible `web_search` tool and a declarative TUI sidebar. The WASM extension must not perform raw networking. It calls a native Oino host capability, `host.web.search`, and Oino renders sidebar state in core-owned Ratatui code.

Scope cuts for this first implementation:

- Local development discovery only: `extensions/*/oino.extension.json`.
- Manifest-declared contributions first. Runtime `registerTool()`-style dynamic registration is deferred.
- One required host capability family first: `host.web.search`.
- Sidebar state is ephemeral v1; session `Custom` entries are deferred.
- No hosted marketplace, package installer, Cargo/NPM/Pip install flow, or Pi compatibility layer.
- No arbitrary extension-owned Ratatui rendering.
- No captcha-solving or protected-content bypass.
- Web backend starts with a mock/deterministic backend; live HTTP backend is added after crate validation.

The implementation should keep Oino startup resilient: extension load failures become health diagnostics, not fatal app errors. It should also keep a safe-mode path such as `OINO_DISABLE_EXTENSIONS=1` so this branch remains easy to test and abandon.

## Tasks

- unstarted: Task 1 — Create Disposable Worktree and Capture Baseline
  - Description: Start all implementation in an isolated worktree/branch and establish baseline build/test behavior before changing code.
  - Dependencies: None.
  - Acceptance Criteria:
    - Work is on `feat/wasm-web-extension`, not `main`.
    - Current branch/worktree path is recorded in the work notes or first work commit message.
    - Baseline `cargo test --workspace` result is recorded before extension changes. If baseline fails, failures are documented before proceeding.
  - Steps:
    1. Create or enter a disposable worktree for `feat/wasm-web-extension`.
    2. Confirm `git branch --show-current` reports `feat/wasm-web-extension`.
    3. Run baseline formatting/check/test commands used by this repo.
    4. Record any pre-existing failures in the work log.

- unstarted: Task 2 — Add Extension Core Contracts
  - Description: Introduce shared extension contract types without wiring them into runtime behavior yet.
  - Dependencies: Task 1.
  - Acceptance Criteria:
    - Workspace contains an `oino-extension-core` crate.
    - Manifest, permission, contribution, diagnostic, and health-state types are serializable/deserializable.
    - Unit tests cover valid and invalid manifest parsing.
    - No TUI/app behavior changes yet.
  - Steps:
    1. Add `crates/oino-extension-core` to the workspace.
    2. Define manifest types for id, display name, version, Oino compatibility, runtime, entrypoint, permissions, and contributions.
    3. Define v1 contribution types for tools and sidebar/panel declarations.
    4. Define v1 permission types for tool registration, sidebar UI, and named host capabilities.
    5. Define diagnostic/health types for load errors and runtime errors.
    6. Add tests using inline JSON fixtures.

- unstarted: Task 3 — Implement Local Extension Discovery
  - Description: Discover and validate repo-local extension manifests under `extensions/*/oino.extension.json`.
  - Dependencies: Task 2.
  - Acceptance Criteria:
    - A local discovery API returns loaded manifest metadata plus non-fatal diagnostics.
    - Missing `extensions/` is treated as no extensions, not an error.
    - Bad manifests produce diagnostics and do not abort app startup.
    - Tempdir tests cover empty, valid, invalid, and incompatible extension directories.
  - Steps:
    1. Add an extension discovery module or crate that depends on `oino-extension-core`.
    2. Walk only one package level under `extensions/` for v1.
    3. Validate manifest ids, runtime entrypoints, permissions, and contribution names.
    4. Return diagnostics instead of panicking or failing the entire discovery call.
    5. Add tests for discovery behavior.

- unstarted: Task 4 — Choose and Wrap the WASM Runtime ABI
  - Description: Pick the smallest viable WASM ABI and wrap it behind an Oino runtime abstraction.
  - Dependencies: Task 2.
  - Acceptance Criteria:
    - A short ADR or module-level doc records the selected v1 ABI and why.
    - The runtime wrapper can load a fixture WASM module, call an exported tool handler, and receive JSON output.
    - Unauthorized host calls are denied in tests.
    - The runtime crate does not expose filesystem, shell, secrets, or arbitrary network access by default.
  - Steps:
    1. Spike Extism with default HTTP/filesystem features disabled versus direct Wasmtime JSON ABI.
    2. Prefer the option that gives easiest multi-language WASM authoring and host functions without widening permissions.
    3. Add `oino-extension-wasm` or equivalent runtime crate.
    4. Define a stable v1 call shape: initialize, execute tool, host capability call, sidebar update, shutdown.
    5. Add a tiny fixture module or test fixture to prove load/call/deny behavior.

- unstarted: Task 5 — Add Host Capability Broker and Mock Web Search Capability
  - Description: Create the host-side capability layer used by WASM extensions, starting with `host.web.search` backed by deterministic mock search.
  - Dependencies: Task 2.
  - Acceptance Criteria:
    - Host capability calls include extension id, capability name, request payload, and permission check result.
    - `host.web.search` accepts query/max-results style input and returns normalized results.
    - Mock backend tests cover success, timeout/error mapping, and result limit enforcement.
    - Permission denial produces a clear extension/runtime error.
  - Steps:
    1. Define a `HostCapabilityBroker` abstraction.
    2. Define `WebSearchRequest`, `WebSearchResult`, `WebSearchResponse`, and `WebSearchError` types.
    3. Implement a deterministic mock backend for tests and first UI wiring.
    4. Enforce max result count, timeout budget, and response-size limits at the capability boundary.
    5. Wire the capability broker into the WASM runtime wrapper from Task 4.

- unstarted: Task 6 — Bridge Extension Tools into Oino Tools
  - Description: Convert manifest-declared extension tool contributions into `oino_agent_loop::Tool` implementations.
  - Dependencies: Tasks 3, 4, and 5.
  - Acceptance Criteria:
    - `web_search` can be represented as a normal Oino `ToolDefinition`.
    - Executing the bridge calls the extension runtime and returns a normal `ToolResult`.
    - Abort signals and runtime errors are translated into existing Oino loop errors/tool errors.
    - Unit tests cover successful execution, guest error, host capability denial, and abort-before-execute.
  - Steps:
    1. Add an `ExtensionTool` wrapper that implements `Tool`.
    2. Map manifest tool schemas and execution modes into `ToolDefinition` and `ToolExecutionMode`.
    3. Pass `ToolCall`, `ToolUpdateCallback`, and `AbortSignal` into the runtime boundary where possible.
    4. Normalize extension results into `ToolResult::text` or error results.
    5. Keep tool provenance metadata available for diagnostics/settings even if not model-visible.

- unstarted: Task 7 — Integrate Extension Tool Discovery into App Tool Assembly
  - Description: Replace the app's static-only tool assembly with a merged built-in + extension tool registry.
  - Dependencies: Tasks 3 and 6.
  - Acceptance Criteria:
    - Built-in tools and `set_session_title` keep current defaults.
    - Extension tools appear in tool settings and can be enabled/disabled with existing global/project settings.
    - Non-interactive and TUI runs both receive the same enabled tool set.
    - `OINO_DISABLE_EXTENSIONS=1` disables discovery/runtime and leaves built-in tools intact.
  - Steps:
    1. Add an app-level extension manager initialized near resource/session setup.
    2. Add a helper that builds available tools from built-ins, session title tool, and extension tools.
    3. Update tool settings item generation to include extension tool names from discovered contributions.
    4. Update `apply_tool_settings_to_harness` to use the merged tool set.
    5. Add tests around known tool names and settings defaults for extension tools.

- unstarted: Task 8 — Add Declarative Sidebar/Panel State Model
  - Description: Add core-owned TUI state and rendering for extension sidebar panels, with the web panel as the first concrete panel.
  - Dependencies: Task 2.
  - Acceptance Criteria:
    - `TuiState` can store extension panel definitions and panel states.
    - Render code supports idle, loading, results, and error states for the web search panel.
    - Rendering does not call extension code.
    - Existing transcript/composer layout remains usable at narrow widths.
    - Render tests or deterministic state tests cover all web sidebar states.
  - Steps:
    1. Add panel/sidebar structs to `oino-tui` with stable, serializable state shapes.
    2. Add methods on `TuiState` to register panels and update panel state.
    3. Update layout to allocate a right sidebar only when a panel is active and terminal width allows it.
    4. Render web search title, query, status, ranked result titles, snippets, and URLs.
    5. Add fallback behavior for small terminal widths.

- unstarted: Task 9 — Route Extension UI Updates into the TUI
  - Description: Deliver sidebar updates emitted during extension tool execution to the running TUI state.
  - Dependencies: Tasks 5, 6, and 8.
  - Acceptance Criteria:
    - `web_search` execution can set sidebar loading/results/error state while preserving normal tool output.
    - TUI auto-shows or highlights the web sidebar after a search runs.
    - Non-interactive runs ignore sidebar updates safely.
    - Tests cover state update routing independent of terminal rendering.
  - Steps:
    1. Extend the app runtime event channel with an extension UI/sidebar update event.
    2. Let the extension runtime or host broker emit declarative panel state updates through a callback/channel.
    3. Apply panel updates in `apply_tui_runtime_event`.
    4. Ensure updates from background tool execution are thread-safe and non-blocking.
    5. Decide and implement v1 behavior for auto-opening/pinning the web sidebar.

- unstarted: Task 10 — Add the `extensions/web` WASM Package
  - Description: Add the dogfood web extension manifest and guest source/package that registers/handles `web_search`.
  - Dependencies: Tasks 4, 5, 6, and 8.
  - Acceptance Criteria:
    - `extensions/web/oino.extension.json` validates through local discovery.
    - The package requests only tool registration, sidebar UI, and `host.web.search` permissions.
    - The WASM guest validates `query` and `max_results` arguments.
    - With the mock backend, an agent/tool call returns model-visible search results and updates sidebar state.
    - Extension build instructions are documented if the WASM artifact is not built by normal workspace tests.
  - Steps:
    1. Create the repo-local extension package layout under `extensions/web/`.
    2. Add a manifest declaring the `web_search` tool schema and web sidebar contribution.
    3. Add guest WASM source using the selected ABI/PDK from Task 4.
    4. Format tool output as concise markdown or text suitable for model context.
    5. Emit loading/results/error sidebar state around the host capability call.
    6. Add fixture/integration tests that do not require live internet.

- unstarted: Task 11 — Validate and Integrate a Live Web Search Backend
  - Description: Test candidate Rust HTTP/search backends and add a live backend behind explicit limits/fallbacks.
  - Dependencies: Task 5.
  - Acceptance Criteria:
    - Candidate decision is documented in the branch notes or module docs.
    - Live backend is optional/configurable and does not break offline tests.
    - Ignored/env-gated smoke test can perform a live query when enabled.
    - Backend enforces public-web-only boundaries: no captcha solving, no account scraping, no protected-content bypass.
  - Steps:
    1. Validate `wreq`/`newwreq` build and runtime fit first.
    2. If unsuitable, validate `chromimic`, `primp`, or a simpler `reqwest` fallback.
    3. Add a backend trait implementation for the selected source/provider.
    4. Keep mock backend as default in unit tests.
    5. Add ignored/env-gated live smoke tests and clear error mapping.

- unstarted: Task 12 — Add Extension Health, Diagnostics, and Safe Mode UX
  - Description: Surface extension load/runtime status without making failures fatal.
  - Dependencies: Tasks 3, 4, 7, and 8.
  - Acceptance Criteria:
    - Extension load failures are visible in TUI status/settings/diagnostics instead of crashing Oino.
    - `OINO_DISABLE_EXTENSIONS=1` or equivalent safe-mode path disables all extension loading.
    - Tool settings can show extension-provided tools with enough provenance to distinguish them from built-ins.
    - Runtime errors include extension id and actionable message.
  - Steps:
    1. Store extension diagnostics in app state after discovery/load.
    2. Expose diagnostics through an existing settings or inspect surface, or a minimal extension diagnostics panel.
    3. Add safe-mode env flag and tests for disabling extension discovery.
    4. Include extension provenance in tool settings labels or details where feasible.
    5. Ensure unhealthy extension tools are not registered as callable tools.

- unstarted: Task 13 — End-to-End Smoke and Regression Tests
  - Description: Verify that the vertical slice works without regressing existing Oino behavior.
  - Dependencies: Tasks 7, 9, 10, 11, and 12.
  - Acceptance Criteria:
    - `cargo fmt --check` passes.
    - `cargo test --workspace` passes, excluding explicitly ignored live tests.
    - Manual or automated smoke proves `web_search` returns a transcript tool result and sidebar results with mock backend.
    - Existing built-in tools still appear and execute normally.
    - Oino starts cleanly when `extensions/` is missing, extension WASM is missing, and extensions are disabled.
  - Steps:
    1. Run formatting and workspace tests.
    2. Run extension-specific unit/integration tests.
    3. Run a TUI/manual smoke with the mock backend.
    4. Run missing/bad extension startup smoke.
    5. Run optional live backend smoke only when env-gated credentials/network are available.

- unstarted: Task 14 — Document the Experimental Extension Workflow
  - Description: Document how to build, run, test, disable, and dispose of the extension experiment.
  - Dependencies: Tasks 10, 11, 12, and 13.
  - Acceptance Criteria:
    - Documentation explains repo-local extension layout and manifest fields used by v1.
    - Documentation explains how to build the web WASM artifact if needed.
    - Documentation explains how to enable/disable extensions and read health diagnostics.
    - Documentation explicitly states v1 limitations and deferred work.
  - Steps:
    1. Add docs under `.unipi/docs/quick-work/` or project docs appropriate for experiments.
    2. Include the branch/worktree disposal path.
    3. Include command examples for building guest WASM and running tests.
    4. Include safety/permissions notes.
    5. Link back to the brainstorm spec and this plan.

## Sequencing

1. Task 1 establishes disposable branch and baseline.
2. Tasks 2 and 3 build the static extension metadata foundation.
3. Tasks 4 and 5 build the execution/capability boundary.
4. Tasks 6 and 7 make extension tools usable by the agent loop and settings.
5. Tasks 8 and 9 add the sidebar state/render/update path.
6. Task 10 adds the actual dogfood web extension against the mock backend.
7. Task 11 adds the live backend after the platform works with deterministic tests.
8. Task 12 hardens diagnostics/safe mode.
9. Tasks 13 and 14 verify and document the branch.

Dependency graph:

```text
1
└─ 2 ─┬─ 3 ───────┬─ 7 ─────┐
      ├─ 4 ─┬─ 6 ─┘         ├─ 13 ─ 14
      ├─ 5 ─┘  └─ 9 ───────┘
      └─ 8 ───────┘
5 ─ 11 ───────────────┘
3/4/7/8 ─ 12 ─────────┘
```

## Risks

- **WASM ABI complexity:** Direct Wasmtime host functions can become custom ABI work; Extism can simplify the PDK story but must be configured without broad HTTP/filesystem features. Task 4 is the decision gate.
- **Build ergonomics for guest WASM:** Normal `cargo test --workspace` should not require every developer to have WASM targets installed unless clearly documented or gated.
- **TUI layout regression:** A sidebar can hurt small terminals or transcript performance. Task 8 must keep rendering deterministic and avoid extension code in render paths.
- **Tool settings assumptions:** Current app code assumes built-in/static tool names in several helper functions. Task 7 must centralize merged tool assembly to avoid drift.
- **Live search fragility:** HTML search scraping and fingerprinting crates may break or add heavy dependencies. Task 11 must keep mock tests primary and live tests env-gated.
- **Permission model scope creep:** V1 should authorize only the exact named capabilities needed by `extensions/web`; broad filesystem/shell/network permissions are intentionally deferred.
- **Startup reliability:** Bad manifests, missing WASM files, or runtime initialization failures must be diagnostics only.
- **Branch disposability:** Avoid unrelated refactors so `feat/wasm-web-extension` can be abandoned cleanly if the experiment is not worth keeping.
