---
title: "Oino Extension Kernel Roadmap — Implementation Plan"
type: plan
date: 2026-05-21
workbranch: "feat/extension-kernel-roadmap"
specs:
  - .unipi/docs/specs/2026-05-20-oino-extension-kernel-roadmap-design.md
---

# Oino Extension Kernel Roadmap — Implementation Plan

## Overview

Implement the full registry-first Oino Extension Kernel roadmap on the isolated worktree branch `feat/extension-kernel-roadmap`.

This is intentionally a large, Ralph-assisted roadmap plan rather than a single-session patch plan. The work should still move in strict dependency order, with each task kept independently reviewable and with `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` run at phase boundaries and before final review.

Planning decisions recorded for implementation:

- Oino will not support Pi TypeScript extension compatibility; Pi remains a parity checklist and prior-art benchmark.
- Built-ins should migrate onto the same registry contribution paths used by extensions.
- Untrusted community extensions should be WASM-first. Any trusted native/sidecar mechanism must be explicitly gated behind later policy, warnings, and provenance.
- The initial WASM ABI must be chosen by a short ADR during implementation, comparing the current simple Wasmtime JSON boundary, WASI/component model, and Extism-style host abstractions.
- UI extensibility should use core-owned render paths with fixed schemas for first-class surfaces plus a constrained declarative model for future surfaces.
- Permissions should combine install-time review, per-project/user enablement policy, and capability-call enforcement for high-risk actions.
- Community registry work must not precede local/installed package layouts, permission diagnostics, compatibility checks, and trust metadata.

## Tasks

- completed: Task 1 — Create Roadmap Worktree and Capture Baseline
  - Description: Start implementation in the selected isolated branch and capture the current workspace state before architecture changes.
  - Dependencies: None.
  - Acceptance Criteria:
    - Work happens on `feat/extension-kernel-roadmap` in a dedicated worktree or branch.
    - Baseline repository status, current crate graph, and validation command results are recorded in work notes.
    - Any pre-existing validation failures are documented before extension-kernel changes begin.
  - Steps:
    1. Create or enter the worktree for `feat/extension-kernel-roadmap`.
    2. Confirm branch and repository cleanliness.
    3. Re-read `AGENT.md`, `README.md`, root `Cargo.toml`, and relevant prior extension docs/specs.
    4. Run the repository validation commands and record baseline results.

- completed: Task 2 — Complete Pi-to-Oino Extensibility Parity Matrix
  - Description: Produce the authoritative parity map for Pi extension capabilities and Oino-native equivalents.
  - Dependencies: Task 1.
  - Acceptance Criteria:
    - Matrix covers tools, slash commands, lifecycle hooks, tool hooks, provider/model hooks, keybindings, TUI surfaces, renderers, themes, package install, persistence, hot reload, diagnostics, settings, providers, resources, and SDK/dev tooling.
    - Every row is marked implemented-now, planned, deferred, or rejected with rationale.
    - The matrix explicitly states that Pi extension compatibility is rejected while semantic parity is targeted.
  - Steps:
    1. Review Pi coding-agent docs used by the brainstorm and any newly relevant docs.
    2. Create a tracked parity document under project docs.
    3. Map each Pi capability to an Oino registry contribution, hook, runtime capability, or explicit deferral.
    4. Link each planned capability to later tasks in this plan.

- completed: Task 3 — Add Extension Core Crate and Identity Contracts
  - Description: Introduce a runtime-agnostic `oino-extension-core` layer for extension/package identity and shared serializable types.
  - Dependencies: Task 2.
  - Acceptance Criteria:
    - Workspace contains an extension core crate or equivalent core module with no TUI/provider/runtime dependencies.
    - Types exist for extension ids, package ids, contribution ids, source scopes, source kinds, protocol versions, Oino compatibility ranges, and lifecycle states.
    - Serialization/deserialization tests cover valid and invalid ids, versions, scopes, and source kinds.
  - Steps:
    1. Add the core crate to the workspace.
    2. Define strongly typed identifiers and source metadata.
    3. Define compatibility and lifecycle state types.
    4. Add unit tests and schema/serde fixtures.

- completed: Task 4 — Define Manifest, Package, Permission, Provenance, and Diagnostic Contracts
  - Description: Extend core contracts to describe extension manifests, package metadata, permissions, provenance, health, diagnostics, and conflict policy.
  - Dependencies: Task 3.
  - Acceptance Criteria:
    - Manifest and package metadata cover local extensions, installed packages, WASM modules, built-ins, and future registry packages.
    - Permission vocabulary includes filesystem, shell/process, network/host capability, secrets, session persistence, UI, provider mutation, and package management boundaries.
    - Diagnostics identify extension/package id, contribution id when available, source path/package, failure phase, severity, remediation, and health state.
    - Tests cover permission parsing, diagnostic formatting, compatibility rejection, and conflict-policy defaults.
  - Steps:
    1. Define manifest and package metadata structures.
    2. Define permission and capability request structures.
    3. Define provenance, diagnostic, health, and conflict-policy structures.
    4. Add serde tests and invalid fixture coverage.

- completed: Task 5 — Implement Generic Contribution Registry Engine
  - Description: Build the shared registry engine that all specialized registries use for validation, enablement, ordering, conflicts, provenance, snapshots, and diffs.
  - Dependencies: Task 4.
  - Acceptance Criteria:
    - Registry engine can register/unregister contributions from built-in, global, project, session/dev, local package, installed package, and WASM sources.
    - Composition respects deterministic scope ordering, contribution priority, user enable/disable state, overrides, compatibility, permissions, and conflict policy.
    - Snapshot and diff APIs are immutable/read-only for consumers and suitable for TUI render paths.
    - Tests cover duplicate ids, ordering, disabled contributions, user overrides, incompatible sources, diagnostics, and diffs.
  - Steps:
    1. Define generic contribution metadata and registry entry types.
    2. Implement validation and active-snapshot composition.
    3. Implement diffing between snapshots for reload/install/update/remove.
    4. Add deterministic tests for common and conflict cases.

- completed: Task 6 — Add Specialized Registry Types
  - Description: Layer typed registries on the generic engine for the contribution categories identified by the roadmap.
  - Dependencies: Task 5.
  - Acceptance Criteria:
    - Typed registry wrappers exist for tools, commands, keymaps, hooks, UI surfaces, settings pages, themes, providers/models, resources, autosuggest providers, transcript/message renderers, tool renderers, and diagnostics/health entries.
    - Each wrapper exposes typed contribution schemas and consumes the same provenance/conflict/enablement model.
    - Tests prove at least one valid and one invalid contribution per specialized registry family.
  - Steps:
    1. Define contribution schemas for each registry family.
    2. Implement typed wrappers over the generic registry engine.
    3. Add validation helpers for ids, schema shape, and declared permissions.
    4. Add registry-family tests and fixtures.

- completed: Task 7 — Add Registry Configuration, Enablement, and Override Persistence
  - Description: Persist user/project choices for extension enablement, contribution enablement, conflict overrides, and scope preferences.
  - Dependencies: Tasks 5 and 6.
  - Acceptance Criteria:
    - Global and project settings can enable/disable extensions and individual contributions by id/scope.
    - User-selected conflict winners and ordering overrides survive reload.
    - Safe defaults keep built-ins enabled and unknown external contributions disabled or reviewed according to policy.
    - Tests cover settings merge order, missing settings files, invalid overrides, and project-vs-global precedence.
  - Steps:
    1. Extend Oino-owned settings structures or introduce extension settings files under Oino-owned paths.
    2. Implement loading/merging of global and project extension policies.
    3. Apply policies during registry composition.
    4. Add tests for precedence and invalid settings diagnostics.

- completed: Task 8 — Migrate Built-in Tools, Commands, Keymaps, Resources, Settings, Themes, and Provider Metadata onto Registries
  - Description: Register existing built-ins through the same contribution path used by extensions without changing user-visible defaults.
  - Dependencies: Tasks 6 and 7.
  - Acceptance Criteria:
    - Existing tools (`read`, `bash`, `edit`, `write`, and session-title tool where applicable) are contributed via the tool registry.
    - Existing slash commands, key hints/keymaps, Oino resources, settings pages, theme/default style metadata, and OpenRouter provider/model metadata are represented in registries where practical.
    - TUI and non-interactive behavior remain equivalent to pre-migration behavior.
    - Regression tests cover built-in defaults and disabled/overridden cases.
  - Steps:
    1. Inventory current hardcoded built-in contribution points.
    2. Register each built-in contribution with provenance and default enablement.
    3. Replace direct consumers with registry snapshots incrementally.
    4. Validate no behavior regressions in TUI, harness, and app wiring.

- completed: Task 9 — Implement Extension Manager Discovery, Loading, Safe Mode, and Reload
  - Description: Add the manager that discovers extension sources, validates manifests, initializes runtimes, wires registries, and publishes health/snapshot state.
  - Dependencies: Tasks 4, 5, 6, and 7.
  - Acceptance Criteria:
    - Discovery order is deterministic across built-in, global, project, session/dev, local package, installed package, WASM, and future registry sources.
    - Bad manifests, incompatible packages, denied permissions, and missing runtimes produce diagnostics without crashing startup.
    - Safe mode disables all non-built-in extensions and is visible to app/TUI consumers.
    - Reload computes registry diffs and preserves active session state where possible.
  - Steps:
    1. Define extension source layout discovery under Oino-owned paths.
    2. Implement manifest/package validation and diagnostic collection.
    3. Compose registry snapshots through the manager.
    4. Add safe-mode and reload entry points.

- completed: Task 10 — Expose Extension Health, Diagnostics, and Management Snapshots
  - Description: Provide queryable state for management UX, logs, and diagnostics across app, harness, and TUI.
  - Dependencies: Task 9.
  - Acceptance Criteria:
    - Consumers can list extensions/packages, contributions, states, health, permissions, provenance, conflicts, and diagnostics.
    - Diagnostics are actionable and grouped by extension/package and contribution.
    - Health changes from load errors, runtime crashes, denied permissions, hook timeouts, and invalid UI updates are reflected in snapshots.
    - Tests cover diagnostic grouping and state transitions.
  - Steps:
    1. Define read-only management snapshot types.
    2. Connect registry and manager diagnostics into those snapshots.
    3. Add state transitions for unhealthy and disabled contributions.
    4. Add test fixtures for common failure modes.

- completed: Task 11 — Implement Hook and Event Registry Model
  - Description: Add typed hook groups with observe-only, mutable, cancellable, blocking, ordering, timeout, and fallback semantics.
  - Dependencies: Tasks 6, 9, and 10.
  - Acceptance Criteria:
    - Hook groups cover startup/resource/session, input/command, before/after agent turn, context transform, provider request/response, message stream, tool call/result/update, model/thinking selection, compaction/tree/session, and reload/install/update/remove.
    - Mutable hooks return typed patches rather than arbitrary mutation.
    - Cancellable/blocking hooks have deterministic fallback behavior and timeout diagnostics.
    - Tests cover ordering, timeout, patch application, cancellation, and unhealthy hook isolation.
  - Steps:
    1. Define hook event and patch types.
    2. Implement hook registry composition and runner.
    3. Wire initial no-op/built-in hook points into harness/app boundaries.
    4. Add tests before enabling external hook runtimes.

- completed: Task 12 — Choose WASM ABI and Implement Runtime Lifecycle
  - Description: Stabilize the untrusted extension runtime boundary for initialize, execute, cancel, progress, structured results, shutdown, and crash recovery.
  - Dependencies: Tasks 4, 9, and 10.
  - Acceptance Criteria:
    - ADR records the chosen v1 ABI and rejects alternatives with rationale.
    - Runtime can initialize an extension, invoke a handler, stream progress, cancel execution, shut down, and report structured errors.
    - Filesystem, shell/process, secrets, raw network, and unrestricted host imports are unavailable unless brokered by named capabilities.
    - Tests cover initialize, execute, cancel, timeout, crash recovery, unauthorized imports, and malformed payloads.
  - Steps:
    1. Compare simple Wasmtime JSON, WASI Preview 2/component model, and Extism-style runtime options.
    2. Implement the selected runtime behind an Oino-owned trait/interface.
    3. Add fixture modules for success, error, timeout, and unauthorized import cases.
    4. Route runtime health into the Extension Manager.

- completed: Task 13 — Implement Capability Broker and Permission Enforcement
  - Description: Add the host capability broker used by WASM extensions and future runtimes for privileged behavior.
  - Dependencies: Tasks 4, 7, 10, and 12.
  - Acceptance Criteria:
    - Capability calls include extension id, contribution id when available, capability name, payload, permission decision, timeout budget, size limits, and audit/provenance metadata.
    - Denied capabilities return typed errors and health diagnostics without crashing the runtime.
    - Initial capabilities include safe test/mocking paths plus web/search or similar proof capability from prior extension work.
    - Tests cover allow, deny, timeout, invalid payload, oversized payload, and unhealthy extension behavior.
  - Steps:
    1. Define broker request/response/error types.
    2. Connect permission policy from manifests and settings to broker decisions.
    3. Implement initial named capabilities and mocks.
    4. Add audit/diagnostic emission and tests.

- completed: Task 14 — Bridge Extension Tools and Commands into Oino Runtime Paths
  - Description: Make extension-contributed tools and commands usable through existing agent/harness/TUI flows.
  - Dependencies: Tasks 6, 8, 9, 12, and 13.
  - Acceptance Criteria:
    - Extension tools appear as normal model-visible tool definitions with provenance and user enablement.
    - Tool execution routes through built-in code or extension runtime consistently and returns normal Oino tool results.
    - Extension slash commands route through command registry handlers with validation, diagnostics, and cancellation where applicable.
    - Tests cover success, extension error, permission denial, cancellation, disabled tools/commands, and non-interactive usage.
  - Steps:
    1. Add tool bridge adapters from registry contributions to `oino-agent-loop` tool abstractions.
    2. Add command bridge adapters from registry contributions to TUI/app command handling.
    3. Preserve existing built-in behavior and settings defaults.
    4. Add integration tests for model-visible and user-invoked extension contributions.

- completed: Task 15 — Implement UI Surface Registry Contracts
  - Description: Define extension-visible UI surface contributions without allowing extension code in Ratatui render paths.
  - Dependencies: Tasks 4, 6, and 10.
  - Acceptance Criteria:
    - Contracts exist for sidebar, floating panel, footer sections, main panel, settings pages, autosuggest providers, overlays, theme tokens/packs, transcript/message renderers, tool call/result renderers, and notification/status/health surfaces.
    - Surface ownership, focus, visibility, priority, layout constraints, tiny-terminal fallback, key dispatch, and conflict behavior are explicit.
    - UI state updates are validated against owned surfaces and declared schemas.
    - Tests cover invalid ownership, conflicting slots, bad state shape, and tiny-terminal fallback decisions.
  - Steps:
    1. Define UI contribution and state-update schemas.
    2. Define surface ownership and layout policy types.
    3. Add validation for actions, state updates, and declared key scopes.
    4. Add surface registry tests.

- completed: Task 16 — Wire Registry-backed UI Surfaces into TUI State and Rendering
  - Description: Consume UI registry snapshots in `oino-tui` while preserving deterministic state-machine rendering and no blocking render work.
  - Dependencies: Tasks 8 and 15.
  - Acceptance Criteria:
    - TUI can show registry-backed sidebar, floating panel, footer, main panel, settings page, autosuggest, theme, message renderer, and tool renderer contributions where implemented.
    - Render paths are core-owned and state-driven; extension code is never called during render.
    - Existing composer/transcript/settings/help/session UX remains intact.
    - Render/state tests cover focus, key dispatch, layout constraints, tiny terminals, and conflict badges.
  - Steps:
    1. Add TUI state holders for active registry snapshots and surface state.
    2. Route extension UI updates through actions/events into TUI state.
    3. Extend layout/render modules for each first-class surface in dependency-safe order.
    4. Add state and render tests for each surface family.

- completed: Task 17 — Add Autosuggest, Keymap, Theme, Renderer, and Provider/Model Extensibility
  - Description: Complete the interactive customization surfaces beyond basic panels and tools.
  - Dependencies: Tasks 6, 8, 15, and 16.
  - Acceptance Criteria:
    - Custom keymaps/shortcuts compose with built-ins and expose conflicts/provenance.
    - Autosuggest providers compose with slash/resource/path suggestions without blocking render paths and use high-level `nucleo` where fuzzy matching is needed.
    - Theme packs/tokens can override or extend built-in visual tokens while preserving readability.
    - Provider/model entries and safe provider hooks are registry-backed with privacy-aware constraints.
    - Tests cover conflicts, ordering, theme fallback, provider/model compatibility, and autosuggest refresh behavior.
  - Steps:
    1. Wire key dispatch to keymap registry snapshots.
    2. Wire autosuggest sources to cached state updates outside render paths.
    3. Apply theme tokens through existing TUI theme/style boundaries.
    4. Add provider/model metadata and safe hook integration.

- completed: Task 18 — Add Many-extension Management UX
  - Description: Build user-facing management surfaces for installed extensions and contributed components at scale.
  - Dependencies: Tasks 10, 15, 16, and 17.
  - Acceptance Criteria:
    - TUI/app can show searchable extensions, packages, contributions, permissions, health, conflicts, provenance, scope, and enablement state.
    - Users can enable/disable per extension, contribution, and scope where policy allows.
    - Conflict badges and diagnostic details are visible and actionable.
    - Search/browse interactions use Oino's high-level `nucleo` fuzzy conventions and avoid render-path rescoring.
    - Tests cover filtering, toggles, diagnostics, conflict display, and settings persistence.
  - Steps:
    1. Add management data adapters from Extension Manager snapshots.
    2. Add TUI settings/management pages and commands.
    3. Implement enable/disable and override actions.
    4. Add fuzzy search state and management UX tests.

- unstarted: Task 19 — Add Extension Persistence and Session APIs
  - Description: Let extensions persist typed custom state safely across sessions/projects with migrations and cleanup rules.
  - Dependencies: Tasks 4, 7, 9, 12, and 13.
  - Acceptance Criteria:
    - Extension persistence APIs define scope, owner id, schema/version, migration behavior, cleanup on uninstall, and size limits.
    - Session custom entries are typed, provenance-tagged, and reconstructable without loading unsafe extension code.
    - Permission boundaries prevent extensions from reading or mutating unrelated extension/core state.
    - Tests cover migrations, uninstall cleanup, permission denial, session replay, and corrupted state.
  - Steps:
    1. Define persistence contribution/contracts and session custom entry shapes.
    2. Add storage APIs under Oino-owned paths and session JSONL conventions.
    3. Broker runtime access through capability checks.
    4. Add migration and cleanup tests.

- unstarted: Task 20 — Define Local and Installed Package Layouts
  - Description: Standardize package source layouts under Oino-owned paths before public registry features depend on them.
  - Dependencies: Tasks 4, 7, and 9.
  - Acceptance Criteria:
    - Local project extensions, global installed packages, project installed packages, dev/session extensions, and package assets have documented Oino-owned paths.
    - Package manifests can include extensions, skills, prompts, themes, examples, docs, and assets.
    - Discovery ignores implicit Pi/Claude/AGENTS conventions unless a future migration/import command copies them into Oino paths.
    - Tests cover missing directories, valid packages, invalid packages, duplicate ids, and scope ordering.
  - Steps:
    1. Define directory layouts and manifest references.
    2. Implement discovery/validation updates for local and installed packages.
    3. Document paths and source precedence.
    4. Add filesystem fixture tests.

- unstarted: Task 21 — Implement Package Install, Update, Remove, and Reload Flow
  - Description: Add package lifecycle operations against local files or registry metadata while enforcing compatibility, permissions, trust, and dependency checks.
  - Dependencies: Tasks 10, 13, and 20.
  - Acceptance Criteria:
    - Install/update/remove can operate on local package paths and registry-style metadata fixtures.
    - Oino checks compatibility, dependencies, permissions, conflicts, trust metadata, checksums/signatures where available, and install scope before writing files.
    - User-facing flows present permission/trust changes and reload the Extension Manager after lifecycle operations.
    - Tests cover install, update, remove, rollback/failure, dependency conflict, permission prompt data, and registry diff output.
  - Steps:
    1. Define package lifecycle service APIs.
    2. Implement local package install/update/remove first.
    3. Add registry-metadata fixture support without requiring a hosted service.
    4. Connect lifecycle completion to manager reload and diagnostics.

- unstarted: Task 22 — Design and Implement Community Registry Metadata, Trust, Publishing, and Advisory Policy
  - Description: Add the community-facing registry model and tooling needed before public distribution, without hardcoding a specific hosted backend too early.
  - Dependencies: Tasks 20 and 21.
  - Acceptance Criteria:
    - Registry metadata covers package id, publisher, version, description, categories, license, source link, included assets, compatibility, dependencies, permissions, trust/review/signing status, update policy, changelog, deprecation, and security advisories.
    - Search/category metadata and publishing validation are implemented against local/fixture registry indexes.
    - Trust, review, signing/checksum, security-advisory, and takedown/deprecation policies are documented.
    - Tests cover metadata validation, search/category filtering, compatibility, advisories, deprecated packages, and signature/checksum failure handling.
  - Steps:
    1. Define registry index and package metadata schemas.
    2. Implement local/fixture registry client and search/category filtering.
    3. Add publishing validation tooling for package authors.
    4. Document trust and security policies before enabling public hosting.

- unstarted: Task 23 — Build Author SDK, Devkit, Examples, and Test Harness
  - Description: Provide authoring support targeting the same stable contracts used by Oino core and packages.
  - Dependencies: Tasks 12, 13, 14, 15, 19, 20, and 22.
  - Acceptance Criteria:
    - Manifest generator/validator and package validator are available to extension authors.
    - Rust WASM SDK is usable for at least tool, command, host capability, UI state update, and persistence examples.
    - JavaScript/TypeScript, Go, and Python SDK plans or minimal examples are documented where practical.
    - Local dev reload and test harness support tool calls, host capability mocks, UI surface snapshots, permission denials, persistence, and package validation.
    - Examples cover tools, commands, keymaps, sidebar, floating panel, footer, theme, autosuggest, provider/model metadata, hooks, and persistence.
  - Steps:
    1. Add authoring CLI/devkit commands or examples around the core validators.
    2. Build the Rust SDK and fixture extensions first.
    3. Add test harness helpers for capabilities, UI state, and persistence.
    4. Document multi-language roadmap and provide minimal examples as feasible.

- unstarted: Task 24 — Add Comprehensive Test and Parity Coverage Gates
  - Description: Ensure the extension kernel remains safe, deterministic, and Pi-parity-trackable as features accumulate.
  - Dependencies: Tasks 2 through 23.
  - Acceptance Criteria:
    - Test suites cover contracts, registry composition, built-in migration, manager discovery/reload, safe mode, WASM runtime, capability denial, tool/command bridge, hooks, UI surfaces, keymaps, themes, autosuggest, persistence, package lifecycle, community metadata, hot reload, and multi-extension conflicts.
    - Pi-parity matrix has automated or checklist-backed status for implemented, deferred, and rejected capabilities.
    - Final repository validation passes or documented external blockers are accepted before review.
  - Steps:
    1. Add missing unit/integration tests identified during implementation.
    2. Add E2E smoke tests for safe mode, hot reload, and multi-extension conflict scenarios.
    3. Connect parity matrix status to tests or review checklist.
    4. Run final formatting, clippy, and workspace tests.

- unstarted: Task 25 — Document Architecture, Authoring, Permissions, UI, Packages, and Community Roadmap
  - Description: Publish maintainable docs for Oino core maintainers, extension authors, and users.
  - Dependencies: Tasks 3 through 24.
  - Acceptance Criteria:
    - Docs explain extension-kernel architecture, registries, source precedence, contribution authoring, permissions, diagnostics, safe mode, UI surfaces, WASM ABI, persistence, package lifecycle, community registry policy, and Pi-parity mapping.
    - README and AGENT conventions are updated where user-facing behavior or future-agent conventions change.
    - Docs avoid promising Pi extension compatibility.
    - Examples and docs stay consistent with validators and tests.
  - Steps:
    1. Add architecture and maintainer docs.
    2. Add extension authoring and package publishing docs.
    3. Update README limitations/features as appropriate.
    4. Cross-check docs against tests and examples.

- unstarted: Task 26 — Final Hardening, Review Prep, and Merge Readiness
  - Description: Prepare the long-running roadmap branch for review and eventual merge.
  - Dependencies: Tasks 1 through 25.
  - Acceptance Criteria:
    - Plan task statuses and work notes accurately reflect completed/deferred work.
    - All final validation commands pass or have documented accepted blockers.
    - Extension-kernel rollout risks, migration notes, compatibility guarantees, and follow-up issues are documented.
    - Branch is ready for `/unipi:review-work` and later merge.
  - Steps:
    1. Reconcile plan statuses and docs.
    2. Run final validation and capture results.
    3. Document remaining risks and future follow-ups.
    4. Prepare review handoff summary and commands.

## Sequencing

1. **Foundation:** Tasks 1–7 establish branch safety, parity decisions, core contracts, registry behavior, and persisted enablement policy.
2. **Built-in migration and manager:** Tasks 8–10 prove that Oino itself consumes registry snapshots before broad external extension behavior is enabled.
3. **Hooks and runtime:** Tasks 11–14 add hook semantics, WASM runtime, capability broker, and tool/command bridges.
4. **UI and customization:** Tasks 15–18 add registry-backed UI surfaces, keymaps, autosuggest, themes, provider/model metadata, and many-extension management UX.
5. **Persistence and packages:** Tasks 19–22 add session/custom state, source layouts, package lifecycle, and community registry metadata/trust tooling.
6. **Authoring and quality gates:** Tasks 23–26 complete SDK/devkit support, test coverage, docs, hardening, and review readiness.

Tasks should be executed in order unless a later task explicitly only writes docs/tests that do not depend on unfinished code. For Ralph-assisted work, use phase boundaries as reflection checkpoints and avoid starting a new phase while previous phase diagnostics or validation failures remain unresolved.

## Risks

- **Roadmap size:** This is too large for one normal `/unipi:work` pass. Ralph iterations should stop at phase boundaries for validation and review.
- **Contract lock-in:** Extension ids, permissions, registry snapshots, and WASM ABI become author-facing. ADRs and compatibility tests are required before declaring stability.
- **Built-in migration regressions:** Moving hardcoded behavior behind registries can change defaults. Built-in parity tests must be added before broad rewiring.
- **TUI complexity:** Extension UI must preserve deterministic state-machine rendering and must never call extension code from render paths.
- **Permission ambiguity:** Install-time approval alone is insufficient for high-risk operations. Runtime capability enforcement and diagnostics must remain mandatory.
- **Community registry security:** Trust, signing/checksum, review, deprecation, and advisory flows must exist before public distribution is encouraged.
- **Provider/privacy hooks:** Provider and context mutation hooks can leak sensitive data. Mutability must stay typed, explicit, and policy-gated.
- **Multi-language SDK drift:** SDKs must be generated from or tested against core contracts to avoid incompatible authoring experiences.

## Ralph Loop Guidance

The selected work branch and task granularity are intended for a long-running Ralph loop. Recommended loop cadence:

- Iterations 1–2: Tasks 1–4.
- Iterations 3–5: Tasks 5–8.
- Iterations 6–8: Tasks 9–14.
- Iterations 9–11: Tasks 15–18.
- Iterations 12–14: Tasks 19–23.
- Iterations 15+: Tasks 24–26, validation, docs, review prep.

At each Ralph reflection checkpoint, update this plan's task statuses, record validation results, and decide whether to continue, split a follow-up plan, or pause for review.
