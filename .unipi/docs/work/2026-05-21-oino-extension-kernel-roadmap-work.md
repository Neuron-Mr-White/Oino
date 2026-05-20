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
