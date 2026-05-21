# Oino Extension Kernel

This document is the maintainer and author reference for the extension-kernel work on the `feat/extension-kernel-roadmap` branch.

Oino targets **semantic parity** with useful Pi extension capabilities, but it does **not** support Pi TypeScript extension API compatibility. Oino extensions use Oino-owned manifests, registries, permissions, and package layouts.

Companion guides:

- User install/manage guide: `docs/extension-kernel/user-guide.md`
- Developer author/test/publish guide: `docs/extension-kernel/developer-guide.md`
- SDK/devkit notes: `docs/extension-sdk/README.md`

## Architecture

The extension kernel is split into intentionally narrow crates:

- `oino-extension-core` — data-only contracts: identifiers, manifests, package metadata, permissions, contributions, registries, provenance, diagnostics, persistence records, UI surface contracts, and community registry metadata.
- `oino-extension-builtins` — maps existing built-in Oino surfaces into typed registries so built-ins and external contributions share policy/composition behavior.
- `oino-extension-manager` — discovers Oino-owned extension/package roots, validates manifests, composes registry snapshots, tracks safe mode, reload diffs, management rows, diagnostics, package lifecycle operations, and fixture registry metadata.
- `oino-extension-runtime` — JSON-v1 runtime lifecycle, hook execution, capability broker, and adapters that bridge active extension tools/commands into existing Oino tool and command paths.
- `oino-extension-sdk` — author-facing templates, validators, Rust JSON-v1 helpers, test harness, devkit CLI, and parity coverage gates.
- `oino-app` / `oino-tui` — runtime wiring and visible management/registry-backed UI surfaces (`/extensions`, settings pages, keymaps, theme tokens, autosuggest, renderer/provider metadata badges).

Render paths remain host-owned. Extension code must never run inside Ratatui rendering; extensions publish declarative contributions and validated state updates, and Oino renders those states.

## Source layout and precedence

Oino only auto-discovers explicit Oino-owned paths. It intentionally ignores implicit Pi, Claude, or generic agent conventions unless a future import command copies data into Oino paths.

| Scope | Path | Kind |
|---|---|---|
| Global local extensions | `~/.oino/extensions/*/oino.extension.json` | local extension |
| Global installed packages | `~/.oino/extension-packages/*/oino.package.json` | installed package |
| Global registry fixtures | `~/.oino/extension-registry/*/oino.package.json` | registry package fixture |
| Project local extensions | `<project>/.oino/extensions/*/oino.extension.json` | local extension |
| Project installed packages | `<project>/.oino/extension-packages/*/oino.package.json` | local package |
| Project WASM bundles | `<project>/.oino/wasm-extensions/*/oino.extension.json` | WASM extension |
| Session extensions | `<project>/.oino/session-extensions/*/oino.extension.json` | session scope |
| Development extensions | `<project>/.oino/dev/extensions/*/oino.extension.json` | development scope |

Default source precedence is built-in, global, project, session, then development. `RegistryPolicy` can override source precedence and explicit enable/disable state for extensions, packages, contributions, and registry entries.

Unknown external contributions default to pending review through `RegistryPolicy::safe_defaults()`. Built-ins stay enabled unless explicitly disabled.

## Contributions and authoring

An extension manifest (`oino.extension.json`) declares runtime, permissions, and contributions before any code is executed. Supported contribution families include:

- model-visible tools
- slash commands
- keymaps / shortcuts
- hooks
- UI surfaces
- settings pages
- themes
- provider/model metadata
- prompt/skill/resource entries
- autosuggest providers
- transcript/message/tool renderers
- persistence records
- diagnostics and health metadata

Authoring commands:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- template-extension
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- template-package
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-extension path/to/oino.extension.json
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- parity-check .unipi/docs/research/2026-05-21-oino-pi-extension-parity-matrix.md
```

Example package: `examples/extensions/rust-wasm-fixture`.

Authoring SDK notes and multi-language roadmap: `docs/extension-sdk/README.md`.

## Permissions and capability broker

Install-time manifest review is not sufficient for privileged operations. Runtime capability calls remain brokered by `oino-extension-runtime::CapabilityBroker`.

Permission categories include tools, commands, host capabilities, UI surfaces, filesystem, shell/process, raw network, secrets, persistence scopes, provider mutation, and package management. Capability calls carry extension id, contribution id, payload, timeout budget, response-size limit, provenance, and an audit record.

Current built-in brokered capabilities include:

- `host.test.echo`
- `host.web.search` (mock fixture capability)
- `host.persistence.read`
- `host.persistence.write`
- `host.persistence.delete`

Denied, oversized, timed-out, unknown, malformed, and unhealthy capability calls become diagnostics and health transitions.

## Diagnostics, health, safe mode, and management UX

The Extension Manager never crashes the app for bad external inputs. It collects diagnostics for manifest parse, compatibility, permissions, registry conflicts, runtime execution, UI update, package lifecycle, persistence, and community registry validation.

`/extensions` opens the management overlay. It exposes extensions, packages, contributions, scope, state, health, diagnostics, conflicts, provenance, and global/project enablement actions.

Core developer/community install flow from the panel:

1. Open `/extensions`.
2. Press `i` to install a package into the current project, or `I` to install globally.
3. Type a local package path such as `examples/extensions/rust-wasm-fixture`, a GitHub shorthand such as `owner/repo`, or a Git URL such as `https://github.com/owner/repo.git#v1.0.0`, then press Enter.
4. Oino resolves local paths or clones Git sources, validates, installs, enables the package in the selected scope, reloads the Extension Manager, and refreshes model-visible tools/UI surfaces.
5. Select a package row and press `u` or `x` to uninstall; Enter/Y confirms and Esc/N cancels.
6. Use `p`/Enter to toggle project enablement and `g` to toggle global enablement for the selected extension, package, or contribution.

Safe mode disables all non-built-in contributions while retaining diagnostics. Reload APIs return typed registry diffs so package lifecycle, hot reload, and author workflows can explain what changed.

## UI surfaces

UI contributions are declarative. A surface declares kind, title, state schema, layout slot, focus policy, visibility, key dispatch scopes, tiny-terminal fallback, and conflict policy. Runtime updates are validated for:

- known surface id
- correct extension ownership
- declared state shape
- non-empty action ids/labels
- declared key scopes

Supported surface kinds cover sidebar, floating panel, footer/status, main panel, settings page, autosuggest, overlay, theme, transcript/message renderers, tool renderers, notification, and health summaries.

## WASM ABI and runtime lifecycle

The v1 runtime boundary is `wasm-json-v1`: initialize, invoke, progress, cancel, shutdown, and health. The host owns JSON payload validation, timeouts, cancellation, diagnostics, and capability imports. The current runtime implementation includes fixture modules for deterministic tests and SDK harnesses; untrusted community execution should remain WASM-first.

ADR: `.unipi/docs/adr/2026-05-21-extension-wasm-json-v1-abi.md`.

## Persistence and sessions

Extensions can declare persistence contributions with scope, key, schema version, size limit, migration policy, cleanup policy, and conflict policy. `ExtensionPersistenceStore` enforces owner, key, scope permission, size, migration, and cleanup rules. Session replay uses typed extension custom entries so persisted data can be inspected without loading extension code.

## Packages and community registry

A package manifest (`oino.package.json`) can include extension manifests, resources, assets, examples, docs, dependencies, package-level permissions, Oino compatibility, and trust metadata.

`PackageLifecycleService` supports local install, update, remove, and fixture-registry install. The app-level `/extensions` panel additionally resolves Git/GitHub sources into temporary checkouts before invoking package lifecycle validation/install. Preflight checks cover manifest validity, compatibility, dependency availability/version compatibility, install scope, checksums, signature/review policy, and rollback/cleanup on write failures.

Community registry metadata is local/fixture-based for now. It models package id, publisher, version, description, categories, license, source link, assets, compatibility, dependencies, permissions, trust/review/signing/checksum status, update policy, changelog, deprecation, and advisories.

Policy doc: `.unipi/docs/specs/2026-05-21-oino-extension-community-registry-policy.md`.

## Pi parity and non-goals

Tracked matrix: `.unipi/docs/research/2026-05-21-oino-pi-extension-parity-matrix.md`.

Explicit non-goals:

- No Pi TypeScript extension API compatibility shim.
- No npm package compatibility; Git installs are supported only for Oino package repos containing `oino.package.json`.
- No direct Ratatui rendering from extension code.
- No raw filesystem/process/network access without explicit brokered permissions.

Deferred areas include dynamic runtime registration, OAuth/login provider flows, fully hosted community registry operations, and production-grade multi-language SDKs.
