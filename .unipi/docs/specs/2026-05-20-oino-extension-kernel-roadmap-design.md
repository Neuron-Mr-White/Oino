---
title: "Oino Extension Kernel Roadmap"
type: brainstorm
date: 2026-05-20
---

# Oino Extension Kernel Roadmap

## Problem Statement

Oino needs an Oino-native extension kernel for core maintainers: replace hardcoded runtime/TUI/tool/resource behavior with clean registries, preserve the practical extensibility power of `pi-coding-agent` without supporting Pi's TypeScript extension API, enforce safe WASM/capability boundaries, and keep the UX understandable when many extensions are installed.

The root problem is not simply “add plugins.” The root problem is that Oino's core must become registry-driven before a public community and package ecosystem can be healthy. Built-ins, local extensions, WASM extensions, and future registry packages should all contribute through the same contracts, with provenance, permissions, diagnostics, user enablement, and conflict handling attached from the beginning.

## Context

Key Oino context:

- Oino is a Rust workspace with strong existing seams: `oino-agent-loop`, `oino-agent`, `oino-session`, `oino-env`, `oino-tools`, `oino-resource`, `oino-harness`, `oino-auth`, `oino-provider-openrouter`, `oino-tui`, and `oino-app`.
- The current README still lists dynamic plugins/packages and permissions UI as limitations.
- Oino already owns explicit resource conventions under `~/.oino/` and `<project>/.oino/`; future extension discovery should keep this deterministic Oino-owned direction rather than silently loading Pi/Claude conventions.
- Oino TUI conventions require deterministic state-machine rendering, no blocking work in render paths, explicit focus/mode, tiny-terminal fallbacks, and high-level `nucleo` for fuzzy/search features.
- A prior WASM web-extension design and isolated worktree explored a local extension vertical slice: manifest validation, Wasmtime/WAT runtime, `host.web.search`, tool bridge, declarative sidebar state, diagnostics, and safe mode.

Relevant Pi extensibility findings:

- Pi extensions are TypeScript modules with broad capabilities: lifecycle hooks, mutable/cancellable tool hooks, model/provider hooks, custom tools, slash commands, keybindings, flags, TUI components, UI helpers, custom renderers, providers, session persistence, hot reload, package installation, themes, and diagnostics.
- Pi's model is powerful but high-trust: extensions run with full user permissions. Oino should preserve the extensibility semantics while making permissions, capability boundaries, provenance, and diagnostics more explicit.
- Oino will not implement Pi extension compatibility. Pi is used as a parity benchmark and prior-art inventory, not as an API to support.

The request spans several independent subsystems: extension ABI/runtime, contribution registries, UI surface management, tools/commands/keymaps/themes/providers/hooks, package/community registry, SDKs, permissions, and diagnostics. This spec is therefore a roadmap/design for the extension kernel, not a single narrow implementation patch.

## Chosen Approach

Use a **registry-first extension kernel**.

Oino should first define shared contribution contracts and registries, then migrate built-ins onto those registries. WASM runtimes, package installs, and community registry entries become sources of contributions rather than special cases. This makes the core extensible and maintainable before the public extension ecosystem scales.

The roadmap still keeps WASM and community registry goals in view, but sequences them after the core registry layer is coherent enough to support many contribution types safely.

## Why This Approach

This approach best matches the selected success criteria:

- **Core maintainability first:** tools, commands, UI surfaces, keymaps, themes, providers, hooks, resources, settings, and diagnostics stop being scattered hardcoded paths.
- **Pi parity without Pi compatibility:** each Pi capability can be mapped to an Oino-native contribution type, hook, or explicit deferral.
- **Many-extension cleanliness:** registries can enforce namespacing, provenance, conflict diagnostics, ordering, enablement, and searchable management before dozens of extensions are installed.
- **Security by design:** WASM and named host capabilities attach to contribution contracts instead of being retrofitted later.
- **Future community readiness:** package metadata, trust, compatibility, install/update/remove, and SDKs have stable core contracts to target.

Alternatives rejected:

1. **Sandbox-first WASM platform** — strong security foundation, but it would leave Oino internals hardcoded longer and risk designing an ABI before contribution composition is clear.
2. **Community/package-first registry** — good for momentum, but premature if Oino has not stabilized contribution types, conflict policy, permissions, or built-in registry behavior.
3. **Pi-compatible TypeScript extensions** — faster parity on paper, but conflicts with Oino's Rust-native/WASM-first direction and inherits Pi's high-trust arbitrary-code security model.

## Design

### Architecture

Oino adds an Extension Kernel that sits between core systems and all extensibility sources.

All capabilities become registry contributions, including built-ins:

- tools;
- slash commands;
- keybindings and shortcuts;
- lifecycle, context, provider, tool, message, model, and session hooks;
- UI surfaces such as sidebar, floating panel, footer sections, main panel, transcript/message/tool renderers, settings pages, autosuggest providers, and overlays;
- themes and theme tokens;
- provider/model entries;
- resources such as prompts, skills, and future package-provided assets;
- diagnostics and health entries.

Every contribution carries stable metadata:

- contribution id and human label;
- source extension/package id;
- scope: built-in, global, project, session, or dev/explicit path;
- source type: built-in, local directory, installed package, WASM module, or future registry package;
- protocol and Oino compatibility range;
- permission requirements;
- lifecycle state and health;
- conflict policy and priority/order;
- user enablement and override state;
- provenance for diagnostics and UI.

Execution sources feed these registries through one composition path:

1. Oino built-ins.
2. Repo-local/project extensions.
3. Global installed extensions.
4. WASM extensions.
5. Future community registry packages.

The UI model is slot/registry-based rather than arbitrary extension-owned Ratatui rendering in the hot path. Extensions contribute declarative surface definitions, state, actions, and optional renderer descriptors. Oino owns layout, focus, key dispatch, terminal-size behavior, theme application, and render performance.

WASM is the default untrusted extension execution boundary. Privileged behavior goes through named host capabilities with explicit permissions. Trusted built-in/native code may exist, but it should still register through the same contribution contracts so built-ins and extensions are composed consistently.

### Components

#### 1. Extension core contracts

A data-oriented `oino-extension-core` layer defines:

- manifest and package metadata;
- extension ids and contribution ids;
- compatibility and protocol versions;
- permission vocabulary;
- contribution types;
- provenance metadata;
- health and diagnostics;
- conflict policy;
- registry snapshot types shared by app, harness, TUI, and SDKs.

This layer should remain runtime-agnostic and serializable.

#### 2. Contribution registry engine

A generic registry engine provides shared behavior for specialized registries:

- register and unregister contributions;
- validate names, schemas, compatibility, scopes, and permissions;
- compose active snapshots;
- apply user enable/disable and project/global settings;
- resolve conflicts deterministically;
- expose provenance and diagnostics;
- compute diffs for reload/install/update/remove;
- avoid blocking consumers such as TUI render paths.

Specialized registries use this engine for tools, commands, keymaps, hooks, UI surfaces, settings pages, themes, providers/models, resources, autosuggest providers, and renderers.

#### 3. Extension Manager

The Extension Manager owns discovery, loading, runtime lifecycle, registry wiring, health, and diagnostics.

Responsibilities:

- discover sources in deterministic scope order;
- parse manifests and package metadata;
- enforce Oino/protocol compatibility;
- initialize enabled runtimes;
- collect contributions;
- load and unload on hot reload;
- support safe mode;
- maintain health state;
- publish registry snapshots to the app, harness, and TUI;
- surface actionable diagnostics instead of crashing startup.

#### 4. WASM runtime and capability broker

The WASM runtime executes untrusted extensions behind a stable ABI. The capability broker mediates privileged host behavior.

Responsibilities:

- initialize, execute, cancel, and shut down extension instances;
- support progress updates and structured results;
- enforce timeouts and resource limits;
- deny filesystem, shell, secrets, process, and raw network access unless exposed through named capabilities;
- route host capability calls with extension id, permission check, payload validation, and audit/provenance;
- support declarative UI updates and session/custom-state updates through explicit APIs;
- recover from runtime crashes by marking contributions unhealthy.

The earlier web-extension slice can become the first concrete runtime/capability proof point, but the roadmap generalizes it to all registries.

#### 5. UI surface and layout registry

The UI registry owns extension-visible surfaces without letting arbitrary extension code run during render.

Initial surfaces:

- sidebar;
- floating panel;
- footer sections;
- main panel;
- settings pages;
- autosuggest providers;
- theme tokens and theme packs;
- transcript/message renderers;
- tool call/result renderers;
- notification/status/health surfaces.

The registry defines slot ownership, stacking, priority, visibility, focus, key dispatch, layout constraints, tiny-terminal fallbacks, and conflict behavior. Extensions emit state and actions; Oino validates ownership and renders core-owned components.

#### 6. Hook and event registry

Pi parity requires rich hooks, but Oino should make mutability explicit and typed.

Hook groups:

- startup/resource/session hooks;
- input and command hooks;
- before/after agent-turn hooks;
- context-transform hooks;
- provider request/payload/response hooks;
- message stream hooks;
- tool call/result/update hooks;
- model/thinking selection hooks;
- compaction/tree/session hooks;
- reload/install/update/remove hooks.

Each hook declares whether it is observe-only, mutable, cancellable, or blocking. Mutable hooks return typed patch values rather than arbitrary mutation. Hooks run in deterministic order with timeouts and health diagnostics.

#### 7. Package and community registry model

The public community registry is a later phase, but its metadata should be anticipated in core contracts:

- package id, publisher, version, description, categories, license, source link;
- included extensions, skills, prompts, themes, and assets;
- Oino compatibility and protocol compatibility;
- dependency constraints;
- permission manifest;
- trust/review/signing status;
- install scope: global or project;
- update policy and changelog;
- deprecation/security-advisory metadata.

Install/update/remove should resolve metadata, present permission and trust information, write to Oino-owned install locations, and then reload the Extension Manager.

#### 8. SDKs and authoring experience

Authoring support should target the same contracts:

- manifest generator and validator;
- WASM SDKs for Rust first, then JavaScript/TypeScript, Go, and Python where practical;
- local dev reload;
- test harness for tool calls, host capability mocks, UI surface snapshots, and permission denials;
- examples for tools, commands, keymaps, sidebar, floating panel, footer, theme, autosuggest, provider, hooks, and persistence;
- Pi-parity checklist explaining Oino-native equivalents.

### Data Flow

#### Startup and reload

1. Extension Manager discovers sources in deterministic scope order: built-in, global, project, session/dev flags.
2. It parses manifests and package metadata.
3. It validates Oino compatibility, protocol version, contribution schemas, permissions, and user enablement settings.
4. It initializes enabled runtimes, including WASM modules when needed.
5. Each source submits contributions into registries.
6. Registries compose active snapshots with conflicts, overrides, ordering, and diagnostics applied.
7. App, harness, TUI, providers, and resource systems consume snapshots, not extension internals.
8. Reload computes a diff and updates snapshots without losing session state where possible.

#### Tool execution

1. The agent loop sees a registry-backed `ToolDefinition`.
2. A tool call enters the registry-backed tool bridge.
3. Built-in tools execute directly through Oino code; extension tools invoke their runtime handler.
4. Extension handlers may call named host capabilities through the broker.
5. The broker validates permission, payload, limits, timeout, and extension health.
6. The result returns as a normal Oino `ToolResult`.
7. Optional progress, diagnostic, session, or declarative UI updates are routed through typed channels.

#### UI updates

1. An extension declares ownership of a UI surface contribution.
2. Runtime events emit state/action updates for that surface.
3. The UI registry validates that the extension owns the target surface and that the update shape is allowed.
4. TUI state applies the update.
5. Core-owned renderers draw the surface according to layout/focus/theme/tiny-terminal rules.

#### Hooks

1. Runtime event reaches a hook point.
2. The hook registry selects enabled hooks by scope, priority, and type.
3. Observe-only hooks receive event snapshots.
4. Mutable/cancellable hooks return typed patch/block/continue decisions.
5. Timeouts or errors mark hook health and continue with deterministic fallback policy.

#### Package install/update/remove

1. Package manager resolves registry or local metadata.
2. Oino checks compatibility, dependencies, permissions, trust, and conflicts.
3. User approves installation scope and permissions.
4. Package is installed into Oino-owned locations.
5. Extension Manager reloads and registry diffs show added, changed, or removed contributions.

### Error Handling and Safety

- Bad manifests, duplicate ids, incompatible protocol, denied permissions, missing runtimes, runtime crashes, invalid UI updates, hook timeouts, and package errors become health diagnostics by default.
- Unhealthy or unsafe contributions are omitted from active snapshots.
- Built-ins remain available when external extensions fail.
- Safe mode disables all non-built-in extensions and records that state visibly.
- Every privileged host capability has explicit permission checks, typed payload validation, timeouts, size limits, audit/provenance, and denial errors.
- Conflict resolution is deterministic and visible: namespaced ids by default, user override precedence, scope ordering, priority rules, and conflict badges.
- Extension diagnostics must be actionable: identify extension/package id, contribution id, source path/package, failure phase, and remediation.
- Public registry work must include trust signals, review status, signing or checksum verification, and security-advisory metadata before broad distribution.

### Testing and Validation

- Manifest and contract tests for ids, versions, permissions, contribution schemas, compatibility, and package metadata.
- Registry composition tests for enablement, ordering, conflict resolution, provenance, diffs, and user overrides.
- Built-in migration tests proving Oino built-ins still behave after registering through the same registry path.
- Tool bridge tests for built-in tools, WASM extension tools, cancellation, progress, errors, and denied capabilities.
- Hook registry tests for observe-only, mutable, cancellable, timeout, ordering, and fallback behavior.
- TUI state/render tests for sidebar, floating panel, footer, settings pages, autosuggest, themes, renderers, conflict badges, and tiny-terminal behavior.
- WASM runtime tests for initialize, execute, cancel, timeout, crash recovery, unauthorized imports, and host capability calls.
- Package manager tests for install/update/remove, compatibility checks, dependency conflicts, permission prompts, and registry metadata validation.
- End-to-end smoke tests for safe mode, hot reload, extension diagnostics, and multi-extension conflict scenarios.
- Pi-parity coverage tests/checklists that mark each Pi extension capability as implemented, deferred, or rejected with rationale.

### Phased Roadmap

#### Phase 0 — Capability inventory and parity map

Create a Pi-to-Oino extensibility matrix covering tools, commands, hooks, UI, keybindings, themes, providers/models, packages, persistence, hot reload, diagnostics, and settings. Mark each item as Oino-native equivalent, deferred, or rejected.

#### Phase 1 — Registry foundation

Define core contracts and implement the generic contribution registry engine. Migrate Oino built-ins onto registries where possible so the internal architecture proves the model before external extensions depend on it.

#### Phase 2 — Core-maintainer surfaces

Expose stable internal registry snapshots to TUI, harness, app wiring, resource loading, settings, and providers. Remove hardcoded extension-shaped behavior from the main app paths.

#### Phase 3 — WASM execution boundary

Stabilize the WASM ABI, capability broker, permissions, cancellation, progress updates, state updates, diagnostics, local discovery, safe mode, and hot reload behavior.

#### Phase 4 — UI extensibility expansion

Add clean registry-backed UI surfaces: sidebar, floating panel, footer, panel, autosuggest, settings pages, theme tokens, transcript/message renderers, and tool renderers.

#### Phase 5 — Extension author SDK and devkit

Ship manifest validation, SDKs, examples, test harnesses, local dev reload, docs, and authoring guides for the main contribution types.

#### Phase 6 — Community package registry

Add install/update/remove/publish workflows, registry metadata, search/categories, trust/review/signing policy, dependency resolution, compatibility checks, project/global scopes, and security advisory handling.

## Implementation Checklist

- [x] Build a Pi-to-Oino extensibility parity matrix with implement/defer/reject decisions.
- [x] Define extension core contracts for manifests, packages, permissions, contributions, provenance, diagnostics, compatibility, and conflicts.
- [x] Implement the generic contribution registry engine with scopes, ordering, enablement, overrides, conflicts, diagnostics, and snapshot/diff APIs.
- [x] Add specialized registries for tools, commands, keymaps, hooks, UI surfaces, settings pages, themes, providers/models, resources, autosuggest providers, and renderers.
- [x] Migrate Oino built-ins onto registry contribution paths without changing user-visible defaults.
- [x] Add an Extension Manager that discovers sources, validates manifests, loads enabled runtimes, composes registries, supports safe mode, and surfaces health.
- [x] Define the hook/event model with observe-only, mutable, cancellable, ordering, timeout, and fallback semantics.
- [x] Stabilize WASM runtime lifecycle and capability broker contracts for initialize, execute, cancel, progress, host capabilities, UI updates, and diagnostics.
- [x] Expand UI surface contracts and render paths for sidebar, floating panel, footer, panel, settings pages, autosuggest, themes, message renderers, and tool renderers.
- [x] Add many-extension management UX: searchable contributions, grouped settings, per-extension health, conflict badges, per-scope enable/disable, and provenance details.
- [x] Add extension persistence/session APIs with typed custom entries, migrations, and permission boundaries.
- [x] Define local package and installed package source layouts under Oino-owned paths.
- [x] Build author SDK/devkit support: manifest generator, validator, WASM SDKs, examples, test harness, and local reload workflow.
- [x] Design and implement package install/update/remove metadata flow before public registry publishing.
- [x] Design community registry trust, review, signing/checksum, categories/search, publishing, and security-advisory policy.
- [x] Add comprehensive tests for contracts, registry composition, built-in migration, WASM runtime, capability denial, UI surfaces, hooks, package lifecycle, safe mode, hot reload, and Pi-parity coverage.
- [x] Document the extension kernel architecture, contribution authoring model, permissions model, UI surface model, and community roadmap.

## Open Questions

- Which registries should be in the first implementation plan, and which can remain designed but unimplemented until later phases?
- Should community extensions be WASM-only, or should Oino allow trusted native/sidecar extension types with stronger warnings?
- Should permissions be approved per install, per project, per extension version, per capability call, or through a combined policy file?
- What is the first stable WASM ABI target: the current simple Wasmtime JSON boundary, WASI Preview 2/component model, Extism, or another host ABI?
- How generic should extension UI state be: fixed schemas per surface, a constrained declarative UI DSL, or a hybrid?
- How much provider request/payload mutation should extensions get, given safety and privacy risks?
- What compatibility guarantees should Oino give extension authors across minor releases?
- How should extension persistence handle migrations, cleanup on uninstall, and sync across sessions/projects?
- What review/signing/trust policy is required before publishing a community registry?
- How should Oino expose extension telemetry or audit logs without leaking sensitive user/project data?

## Out of Scope

- Supporting Pi TypeScript extensions or Pi extension package compatibility.
- Letting arbitrary extension code render directly inside Ratatui render paths in the initial model.
- Implementing the public hosted registry as the first implementation task.
- Building MCP, memory database, subagents, or workflow automation as part of this roadmap, except as future contribution categories.
- Bypassing protected websites, captcha solving, account-authenticated scraping, or weakening host capability boundaries.
- Replacing Oino-owned resource conventions with implicit Pi/Claude/AGENTS.md discovery.
- Defining every final SDK language binding in this first roadmap; SDKs should follow after the core contracts stabilize.
