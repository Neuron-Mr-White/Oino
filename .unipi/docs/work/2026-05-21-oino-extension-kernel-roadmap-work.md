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
