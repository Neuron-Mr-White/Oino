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
