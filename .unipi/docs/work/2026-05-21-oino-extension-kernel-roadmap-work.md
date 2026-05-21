# Oino Extension Kernel Roadmap — Work Notes

## 2026-05-21 — Task 1 baseline

Worktree/branch:

- Path: `/home/pi/project/oino/.unipi/worktrees/feat/extension-kernel-roadmap`
- Branch: `feat/extension-kernel-roadmap`
- Starting commit: `f81995a` (`docs: plan Oino extension kernel roadmap`)
- Starting status: clean

Baseline crate graph:

- `crates/oino-types`
- `crates/oino-agent-loop`
- `crates/oino-agent`
- `crates/oino-session`
- `crates/oino-harness`
- `crates/oino-resource`
- `crates/oino-env`
- `crates/oino-tools`
- `crates/oino-auth`
- `crates/oino-provider-openrouter`
- `crates/oino-tui`
- `crates/oino-app`

Baseline validation:

- `cargo fmt --all --check` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

No pre-existing validation failures were observed before extension-kernel changes.

## 2026-05-21 — Tasks 3–4 extension core contracts

Added `crates/oino-extension-core` and registered it in the workspace. The crate is intentionally data-oriented and has no dependency on TUI, harness, providers, runtime hosts, or filesystem discovery code.

Implemented contract coverage:

- Strongly typed `ExtensionId`, `PackageId`, and `ContributionId` with serde validation.
- `ProtocolVersion`, `OinoCompatibility`, `SourceScope`, `SourceKind`, `SourceDescriptor`, and `LifecycleState`.
- `ExtensionManifest` and `RuntimeDescriptor` for WASM, built-in, and explicitly trusted/future native-sidecar runtimes.
- `ExtensionPermissions` vocabulary for tools, commands, host capabilities, UI surfaces, filesystem, shell/process, raw network, secrets, session persistence, provider mutation, and package management.
- Manifest contribution shapes for tools, commands, keymaps, hooks, UI surfaces, settings pages, themes, providers, resources, autosuggest providers, and renderers.
- `PackageManifest` with package extension/resource/asset refs, dependencies, trust metadata, compatibility, and source descriptors.
- `Provenance`, `ExtensionDiagnostic`, `DiagnosticPhase`, `HealthState`, `ConflictStrategy`, and `ConflictPolicy`.

Verification after Tasks 3–4:

- `cargo fmt --all --check` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Task 5 generic contribution registry

Added generic registry primitives to `oino-extension-core`:

- `RegistryEntryKey`, `ContributionMetadata`, and `RegistryEntry<T>`.
- `ContributionRegistry<T>` with register/unregister/iterate/compose APIs.
- `RegistryPolicy` for disabled extensions/packages/contributions/entries and user-selected overrides.
- `RegistrySnapshot<T>` and `RegistryDiff<T>` for immutable active/inactive snapshots and reload/install/update/remove diffs.
- Composition support for deterministic source-scope ordering, priority, compatibility filtering, permission decisions, health/lifecycle filtering, duplicate-id diagnostics, namespaced defaults, first/last/user-override conflict strategies, and error conflicts.

Task 5 validation:

- Added deterministic registry tests for source registration/unregistration, ordering, duplicate namespacing, user overrides, disable policy, incompatible entries, permission pending/denied entries, diagnostics, and snapshot diffs.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Task 6 specialized registry types

Layered typed registries on the generic registry engine:

- Added `RegistryFamily`, `RegistryContribution`, `RegistryValidationError`, and `TypedContributionRegistry<T>`.
- Added typed aliases/constructors for tool, command, keymap, hook, UI surface, settings page, theme, provider/model, resource, autosuggest, transcript renderer, message renderer, tool renderer, diagnostic, and health registries.
- Added diagnostic and health contribution shapes.
- Registered contributions now validate family-specific required fields and then flow through the same metadata, provenance, conflict, enablement, snapshot, and diff model from Task 5.
- Renderer registries validate that renderer targets match transcript/message/tool renderer family constraints.

Task 6 validation:

- Added typed registry tests that accept one valid contribution for every specialized registry family.
- Added typed registry tests that reject one invalid contribution for every specialized registry family.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Task 7 registry configuration and persistence

Added registry policy configuration and persistence support:

- Extended `RegistryPolicy` with explicit enabled/disabled extension, package, contribution, and registry-entry sets.
- Added persisted override support for conflict winners and priority/order overrides.
- Added source-scope policy defaults so built-ins stay enabled while unknown external contributions require review or can be disabled by policy.
- Added `ExtensionPolicySettings`, `SourceScopePolicySettings`, `PolicyToggle`, and `UnknownContributionPolicy` to model global/project Oino settings and merge project settings over global settings.
- Added `ExtensionPolicySettings::from_optional_json`, `merge`, `to_registry_policy`, and `merged_registry_policy` helpers.
- Extended app `UserSettings` with an `extensions` field backed by `oino-extension-core`, so global/project `.oino/settings.json` files can persist extension policy state alongside existing settings.

Task 7 validation:

- Added tests for global/project merge precedence, source-scope precedence overrides, missing settings JSON, invalid override ids, conflict winner override reload, priority override reload, invalid override diagnostics, safe built-in defaults, and external review/disable/explicit-enable policy.
- Added app settings round-trip coverage for persisted extension policy settings.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Task 8 built-in registry migration

Moved built-in contribution surfaces onto registry-backed metadata:

- Added `crates/oino-extension-builtins` as the bridge between existing hardcoded Oino surfaces and extension-kernel registries.
- Represented built-in tools as `ToolRegistry` contributions generated from live `ToolDefinition` values and execution modes.
- Represented built-in slash commands, prompt/skill include prefixes, keymap actions/default bindings, settings pages, chat-style theme metadata, OpenRouter provider/model metadata, and Oino resource paths as typed registries.
- Updated app tool wiring so the existing global/project tool settings are converted into a `RegistryPolicy`, composed through the built-in `ToolRegistry`, and then used to filter actual harness tools.
- Kept existing TUI/non-interactive defaults: read/bash/edit/write remain enabled, session-title remains disabled unless explicitly enabled, and existing tool settings still control the same behavior.

Task 8 validation:

- Added built-in catalog tests covering tools, slash commands, keymaps, settings pages, themes, provider metadata, resources, and contribution-id slugging.
- Added app regression test proving existing global/project tool settings map to registry policy enable/disable decisions.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 9–10 extension manager and management snapshots

Added `crates/oino-extension-manager` for discovery, load/reload, safe mode, and read-only management state:

- Defined deterministic Oino-owned discovery roots for global, project, session, development, local-package, installed-package, registry-package, and WASM extension sources.
- Added manifest/package discovery for `oino.extension.json` and `oino.package.json` with deterministic scope/kind/path ordering.
- Implemented manifest/package parsing, compatibility validation, runtime-entry validation, permission-denial diagnostics, contribution registration, and registry snapshot composition without crashing on bad inputs.
- Added safe-mode behavior that disables all non-built-in registry entries and reflects safe mode in extension health/lifecycle state.
- Added reload APIs that preserve the previous snapshot and return typed registry diffs.
- Added management snapshot types exposing extensions, packages, contributions, lifecycle/health/permission/provenance/compatibility state, diagnostics, and grouped diagnostics.
- Added health event transitions for runtime crashes, permission denials, hook timeouts, and invalid UI updates.

Tasks 9–10 validation:

- Added tests for deterministic discovery ordering, manifest load/parse/runtime/compatibility/permission diagnostics, snapshot composition, safe-mode disables, reload diffs, diagnostic grouping, and health state transitions.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 11–12 hooks and runtime ABI

Added `crates/oino-extension-runtime` for hook/event execution and the v1 runtime boundary:

- Expanded `HookEventKind` to cover thinking selection, tree, install, update, and remove events in addition to the existing startup/resource/session/input/command/agent/context/provider/message/tool/model/compaction/reload/package lifecycle groups.
- Added built-in no-op hook contributions for every declared hook event group through `oino-extension-builtins` and threaded those into manager built-in registries.
- Added typed hook event payloads, typed hook patches, hook decisions, hook execution records, timeout diagnostics, fallback/cancellation semantics, mutable patch application, deterministic priority ordering, and unhealthy hook isolation.
- Added `wasm-json-v1` ABI decision and runtime lifecycle in `oino-extension-runtime`: initialize, invoke, progress, cancel, shutdown, health, structured errors, payload validation, denied imports, timeout, crash, and malformed payload behavior.
- Added ADR: `.unipi/docs/adr/2026-05-21-extension-wasm-json-v1-abi.md` comparing simple Wasmtime JSON, WASI Preview 2/component model, and Extism-style options. Accepted `wasm-json-v1` as the v1 host-owned ABI.

Tasks 11–12 validation:

- Added hook tests for ordering, typed patch application, cancellation, timeout diagnostics, and unhealthy isolation.
- Added runtime tests for initialize, execute with progress, cancel, timeout, crash recovery, unauthorized imports, allowed brokered imports, malformed payloads, unsupported ABI, and shutdown.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 13–14 capability broker and runtime bridge

Added capability enforcement and runtime adapters for extension tools/commands:

- Added `CapabilityBroker`, `CapabilityRequest`, `CapabilityResponse`, `CapabilityAudit`, `CapabilityDecision`, and typed `CapabilityError` values to `oino-extension-runtime`.
- Capability calls now carry extension id, optional contribution id, capability name, payload, timeout budget, payload/response size limits, provenance, and audit records.
- Broker permission checks use `ExtensionPermissions::allows_host_capability`, typed denial/timeout/invalid/oversized/unhealthy errors, and diagnostic conversion.
- Added initial safe capabilities: `host.test.echo` and mock `host.web.search`.
- Added `ExtensionToolAdapter` implementing `oino-agent-loop::Tool` so active extension tool contributions become normal model-visible tools routed through the runtime boundary with progress and normal Oino tool results.
- Added `ExtensionCommandAdapter` for command contributions routed through runtime handlers.
- Wired app startup/tool refresh to load extension manager snapshots from Oino-owned global/project paths, compose extension policy from global/project settings, and add active external extension tools to the harness tool map.
- Added non-interactive extension command fallback for unknown slash commands when an active extension command contribution matches the slash command id.

Tasks 13–14 validation:

- Added broker tests for allow, deny, timeout, invalid payload, oversized payload, audit capture, and unhealthy extension behavior.
- Added bridge tests for tool success, tool runtime errors, cancellation, command success/error, disabled extension contributions, and non-interactive command execution helper behavior.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 15–16 UI surface contracts and TUI wiring

Added explicit extension UI contracts in `oino-extension-core`:

- Extended `UiSurfaceContribution` with layout, visibility, focus, key-dispatch, tiny-terminal fallback, slot ownership, and conflict policy data.
- Added UI surface kinds for sidebar, floating panel, footer/status/notification/health, main panel, settings page, autosuggest, overlay, theme, transcript/message renderers, and tool call/result renderers.
- Added `UiSurfaceStateUpdate`, `UiSurfaceAction`, layout-decision helpers, conflict detection by surface/slot, ownership validation, schema-shape validation, and key-scope validation.
- Added Display/Error support for UI validation errors so app/TUI paths can propagate diagnostics cleanly.

Wired registry-backed surfaces into TUI state/rendering:

- Added `ExtensionUiState` to `TuiState` for active registry snapshots, state summaries, focus, actions, and conflict badges.
- Added state APIs for applying validated extension UI updates, focusing extension-owned surfaces, and dispatching extension UI actions through `TuiAction::RunExtensionUiAction`.
- Rendered core-owned, state-driven extension surfaces without calling extension code in render paths: sidebar, main panel, footer/status surfaces, floating/overlay panels, settings-page badges, autosuggest/renderer badges, tiny-terminal fallbacks, and conflict indicators.
- Wired app startup/tool-policy refresh to synthesize TUI-visible surfaces from extension manager snapshots, including settings pages, themes, autosuggest providers, transcript/message/tool renderers, diagnostics, and health contributions.

Tasks 15–16 validation:

- Added core tests for invalid ownership, conflicting slots, bad state shapes, undeclared key scopes, and tiny-terminal fallback decisions.
- Added TUI render/state tests for registry-backed surfaces, focus/action dispatch, tiny fallbacks, and conflict badges.
- `cargo fmt --all` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 17–18 extensibility surfaces and management UX

Completed interactive extensibility surfaces beyond basic panels/tools:

- Extended extension contracts for keymaps, autosuggest providers, theme token packs, and provider/model metadata.
- Added provider privacy policy metadata so extension-provided model entries are listed only when they do not request prompt/tool/request mutation access by default.
- Added static autosuggest items and trigger metadata, with TUI-side fuzzy matching through existing `nucleo`-backed `fuzzy_indices` outside render paths.
- Added extension shortcut state that parses registry keymap snapshots, preserves built-in key precedence, exposes built-in/extension conflicts, and dispatches non-conflicting extension shortcut actions.
- Added extension theme token application through the existing `Theme` boundary with a safe token allowlist and ignored-token warnings.
- Synthesized TUI-visible metadata surfaces for settings pages, themes, autosuggest providers, transcript/message/tool renderers, diagnostics, and health contributions.
- Merged safe extension provider/model metadata into model catalog updates without replacing existing OpenRouter catalog behavior.

Added many-extension management UX:

- Added `/extensions` command and an Extensions overlay.
- Added searchable management state covering extensions, packages, and contributions with scope, family, health, state, permission, provenance, diagnostics, conflicts, and global/project enablement.
- Added project/global enable-disable actions from the overlay (`p`/Enter for project, `g` for global) backed by persisted `ExtensionPolicySettings`.
- Search/filtering is precomputed in TUI state with `nucleo` fuzzy matching and not rescored in render paths.
- App adapters now project `ExtensionManagerSnapshot` records into TUI management rows and refresh the TUI/harness snapshot after enablement changes.

Tasks 17–18 validation:

- Added tests for extension shortcut dispatch/conflict exposure, autosuggest cache refresh, theme token plumbing via render path, extension management search/toggle actions, diagnostic/conflict rendering, and policy-setting persistence.
- `cargo fmt --all` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 19–20 persistence/session APIs and package layouts

Completed extension persistence and session API foundations:

- Added persistence contribution contracts with scope, key, schema version, optional schema marker, size limits, migration policy, cleanup policy, and conflict policy.
- Added typed `PersistenceRecord` and `ExtensionSessionEntry` data shapes so extension-owned state can be reconstructed without loading or executing extension code.
- Added `ExtensionPermissions::allows_persistence_scope` checks and validation that persistence contributions declare matching `session_persistence` permission scope.
- Added `PersistenceRegistry` and wired persistence contributions through extension manager registries, snapshots, diffs, diagnostics, and management contribution records.
- Added `ExtensionPersistenceStore` under Oino-owned state roots with read/write/delete/list/migrate/cleanup APIs, owner/key validation, size limits, permission enforcement, tombstone cleanup, delete-on-uninstall cleanup, migration copy-forward, and corrupted-state errors.
- Added session JSONL support for extension custom entries via `SessionEntryKind::ExtensionCustom`, `append_extension_custom`, and `extension_custom_entries`.
- Added runtime capability broker entries for `host.persistence.read`, `host.persistence.write`, and `host.persistence.delete` so runtime access is gated by host capability permissions.

Completed package layout standardization:

- Added `ExtensionLayoutPaths` to centralize Oino-owned global/project roots for local extensions, installed packages, registry metadata fixtures, WASM bundles, session extensions, dev extensions, package assets, and extension state.
- Refactored discovery to build roots from `ExtensionLayoutPaths` while preserving deterministic scope/kind/path ordering.
- Extended `PackageManifest` with `examples[]` and `docs[]`; packages are valid if they include extensions, resources, assets, examples, or docs.
- Documented accepted package layout ADR: `.unipi/docs/adr/2026-05-21-extension-package-layouts.md`.

Tasks 19–20 validation:

- Added tests for persistence registry validation, permission-denied persistence access, migration, uninstall cleanup/tombstones, corrupted state, session replay of extension custom entries, persistence capability gating, missing/valid package roots, docs/examples manifest entries, ignoring implicit foreign files, and scope ordering.
- `cargo fmt --all` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 21–22 package lifecycle and community registry policy

Completed package lifecycle operations:

- Added `PackageLifecycleService` for local install, update, remove, and registry-fixture install flows against `ExtensionLayoutPaths`.
- Added lifecycle reports with operation, package/version/destination, permission/trust prompt data, diagnostics, and extension-manager reload diffs.
- Added install preflight checks for manifest validity, Oino compatibility, dependency availability/version compatibility, install scope, trust checksums, and signature/review policy.
- Added deterministic package-directory checksums that normalize mutable trust checksum/signature manifest fields.
- Added copy/replace helpers with cleanup/rollback behavior so failed writes do not leave partial installs or clobber previous installs.
- Connected successful lifecycle operations to `ExtensionManager::reload` so package/contribution state and registry diffs are refreshed immediately.

Completed community registry metadata and policy foundations:

- Added typed community registry index metadata contracts for package id, publisher, version, description, categories, license, source link, package path/artifacts, assets, compatibility, dependencies, permissions, trust metadata, update policy, changelog, deprecation, and security advisories.
- Added `FixtureRegistryClient` for local/fixture indexes with latest-package lookup, search/category filtering, compatibility filtering, and advisory lookup.
- Added publishing validation with configurable trust policy gates for review, checksum, signature, deprecation, compatibility, and high/critical advisories.
- Documented trust, review, checksum/signature, advisory, deprecation, takedown, compatibility, and publishing policy in `.unipi/docs/specs/2026-05-21-oino-extension-community-registry-policy.md`.

Tasks 21–22 validation:

- Added tests for install/update/remove reload behavior, permission prompt data, registry diff output, dependency conflict handling, checksum failure preserving installed packages, fixture registry search/category/compatibility filtering, metadata validation, deprecation, advisories, and signature requirements.
- `cargo fmt --all` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

## 2026-05-21 — Tasks 23–24 author SDK/devkit and coverage gates

Completed authoring support:

- Added `crates/oino-extension-sdk` with manifest/package template generation, manifest/package JSON validators, package-directory validation, Rust JSON-v1 `WasmSdk` helpers, `ExtensionTestHarness`, parity coverage gates, and a small `oino-extension-devkit` binary.
- The SDK example template covers tools, slash commands, keymaps, sidebar/floating/footer UI surfaces, theme tokens, autosuggest, provider/model metadata, mutable tool hooks, and project persistence.
- Added local test harness support for runtime tool/command calls, host capability mocks, permission denials, UI state update validation, persistence write/read, package validation, and package lifecycle smoke tests.
- Added an author-facing fixture at `examples/extensions/rust-wasm-fixture` and SDK notes at `docs/extension-sdk/README.md`, including the TypeScript/JavaScript, Go, and Python binding roadmap without promising Pi API compatibility.

Completed coverage/parity gates:

- Added `REQUIRED_COVERAGE_GATES` and `validate_parity_matrix` to keep the tracked Pi parity matrix connected to tests.
- Added SDK tests for safe mode, hot reload, multi-extension conflict errors, package lifecycle install/reload diffs, author examples, host capability denial, UI snapshots, persistence, and parity matrix coverage.
- Updated `.unipi/docs/research/2026-05-21-oino-pi-extension-parity-matrix.md` with the automated coverage gate note.

Tasks 23–24 validation:

- `cargo test -p oino-extension-sdk --no-fail-fast` — passed
- `cargo fmt --all` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed
