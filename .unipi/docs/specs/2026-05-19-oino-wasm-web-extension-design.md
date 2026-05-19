---
title: "Oino WASM Web Extension"
type: brainstorm
date: 2026-05-19
---

# Oino WASM Web Extension

## Problem Statement

Oino needs a native extension platform that can grow from user customization into powerful developer plugins while preserving Rust-level performance, strong safety boundaries, and polished TUI UX. The first dogfood extension should validate the platform in a disposable branch/worktree by adding a simple agent-usable `web_search` tool whose results are visible in a sidebar.

The immediate goal is not to port Pi extensions or build a full marketplace. The immediate goal is to prove Oino's own extension shape: repo-local extension discovery, WASM-first execution, explicit capabilities, model-visible tool registration, and declarative sidebar UI.

## Context

Oino already has many extension-shaped seams, but no external extension loader yet:

- `oino-agent-loop` has `Tool`, `ToolDefinition`, `ToolExecutionMode`, `ToolUpdateCallback`, `AbortSignal`, `StreamProvider`, `AgentEvent`, context transforms, and before/after tool-call hooks.
- `oino-harness` has `HookRegistry`, notification hooks, mutating hooks, dynamic `set_tools`, session binding, resource binding, and auth resolver plumbing.
- `oino-session` uses append-only JSONL session trees and already supports `Custom` and `CustomMessage` entries.
- `oino-resource` discovers explicit `.oino` resources: global system prompt, project instructions, prompts, skills, settings, and exports.
- `oino-tui` has static commands, actions, settings pages, keymaps, and tool enablement UI that can become registry-backed.
- `oino-app` currently wires providers, tools, resources, settings, and commands statically. This is the main bottleneck to extension loading.

Research during brainstorming found relevant Rust HTTP / anti-detection / fingerprinting crates:

- `wreq 6.0.0-rc.28` — ergonomic HTTP client with TLS JA3/JA4 fingerprinting; current-looking but release-candidate.
- `newwreq 5.1.7` — same repository lineage as `wreq`, stable-looking package, TLS fingerprinting, Rust 1.85.
- `chromimic 0.12.1` — Chrome/OkHttp impersonation using Boring TLS.
- `primp 1.3.0` — browser impersonation wrapper around reqwest features.
- `clawser-fetch 0.2.3` — Chromium network stack / antidetect fetch.
- `crawlex 1.0.4` — heavier stealth crawler with Chrome-perfect TLS/H2, render pool, queue, and hooks.
- `nab 0.10.3` — LLM-oriented clean markdown fetcher with impersonation features.

A key constraint: most anti-detection and browser-fingerprint HTTP crates are native-networking crates. They are not good fits for a pure sandboxed WASM module that owns raw sockets/TLS. Therefore the chosen design keeps the extension WASM-first but moves actual HTTP/search execution into a native host capability.

## Chosen Approach

Use a WASM-first extension with a native Oino host capability for web search.

The first extension package lives under repo-local `extensions/web/`. It contains a WASM module and an `oino.extension.json` manifest. The extension registers a model-visible `web_search` tool and a declarative sidebar contribution. When the agent calls `web_search`, Oino's extension tool bridge invokes the WASM tool handler. The WASM module validates arguments and calls a native host capability, `host.web.search`, rather than performing raw network requests itself.

The native `host.web.search` capability is implemented in Rust on the Oino side. It owns HTTP backend selection, anti-detection/browser-impersonation configuration, rate limits, caching policy, timeout handling, and normalized result shaping. The initial implementation branch should validate `wreq`/`newwreq` first, with `chromimic` or a simpler client as fallback if build/runtime constraints are poor.

The web search results are returned both as a normal model-visible tool result and as sidebar state. The TUI renders the sidebar declaratively; extension code does not run inside the Ratatui render hot path.

All implementation work for this experiment must happen in a disposable branch/worktree so the idea can be dropped without affecting `main`.

## Why This Approach

This approach balances the user's goals:

- **High customizability:** extensions can contribute tools and UI surfaces, starting with a sidebar.
- **High performance:** Oino keeps hot render/agent paths in Rust; WASM is invoked at natural boundaries such as tool execution.
- **Good UX:** web results are visible in a sidebar while the agent receives a normal tool result.
- **Safety:** WASM is sandboxed and gets only explicit capabilities. It does not get shell, filesystem, secrets, or raw network by default.
- **Realistic networking:** browser/TLS fingerprinting stays native, where the researched Rust crates can actually work.
- **Future registry compatibility:** repo-local `extensions/` can later generalize to project/global installs and a hosted Oino registry.

Alternatives considered:

1. **Native Rust web extension first** — easiest way to use HTTP crates directly, but weakens the WASM-first platform goal and multi-language/sandbox story.
2. **Pure WASM extension that performs web requests** — clean sandbox story, but raw network/TLS fingerprinting does not map well to portable WASM today.
3. **WASM controller plus native sidecar inside the extension package** — powerful, but more moving parts for the first vertical slice.
4. **Resource-only extension first** — safer and simpler, but does not validate agent tools, host capabilities, or sidebar UI.

## Design

### Architecture

Oino adds an extension substrate with these responsibilities:

- discover repo-local extension packages;
- parse extension manifests;
- load WASM modules;
- negotiate protocol/capability versions;
- collect tool and UI contributions;
- enforce permissions;
- bridge extension tools into the existing `Tool` trait;
- route host capability calls such as `host.web.search`;
- render extension UI contributions through core-owned TUI surfaces.

The first dogfood extension is `extensions/web/`. It requests permissions for:

- registering the `web_search` tool;
- invoking `host.web.search` network capability;
- contributing sidebar UI state.

It should not request shell, filesystem, secrets, arbitrary provider payload mutation, or unrestricted network access.

### Components

1. **`oino-extension-core`**
   - Manifest types.
   - Extension IDs and versions.
   - Capability and permission types.
   - Contribution types for tools and panels.
   - Protocol message types shared by host and SDK.

2. **`oino-extension-wasm`**
   - WASM module loader.
   - Host function registration.
   - Capability checks.
   - Tool invocation boundary.
   - Extension lifecycle: initialize, call, cancel, shutdown.

3. **Local extension loader**
   - Discovers `extensions/*/oino.extension.json`.
   - Validates manifest compatibility.
   - Produces diagnostics instead of aborting Oino startup.
   - Later evolves into project/global install and registry support.

4. **`oino-web-capability`**
   - Native host-side search capability.
   - Backend abstraction for search providers/fetch clients.
   - Candidate backend validation for `wreq`/`newwreq`, `chromimic`, or simpler fallback.
   - Timeout, rate-limit, result-size, and error mapping policy.

5. **`extensions/web/`**
   - WASM extension package.
   - Registers `web_search` tool.
   - Maintains search state.
   - Emits sidebar updates.
   - Formats tool results for model consumption.

6. **TUI sidebar/panel registry**
   - Allows extension-provided sidebar/floating-panel contributions.
   - V1 web sidebar states: idle, loading, results, error.
   - V1 sidebar content: query, status, title, URL, snippet, rank/source.
   - Rendering remains deterministic and core-owned.

7. **Harness/tool bridge**
   - Converts extension tool declarations into `ToolDefinition` values.
   - Executes extension tool calls through the WASM host.
   - Propagates abort signals and updates where possible.
   - Returns normal `ToolResult` values to the agent loop.

### Data Flow

#### Startup

1. Oino scans `extensions/`.
2. Oino finds `extensions/web/oino.extension.json`.
3. Oino validates manifest compatibility and requested permissions.
4. Oino loads the WASM module.
5. The extension declares contributions: `web_search` tool and web sidebar.
6. Oino registers the tool and sidebar if permissions are allowed.
7. The sidebar appears idle/empty.

#### Agent tool call

1. Model calls `web_search({ query, max_results? })`.
2. Oino's extension tool bridge invokes the web extension's WASM handler.
3. WASM validates tool arguments.
4. WASM requests `host.web.search` with normalized arguments.
5. Native host capability performs the search.
6. Native host returns normalized results to WASM.
7. WASM returns model-visible content through normal `ToolResult`.
8. WASM emits a sidebar state update.
9. TUI renders the result list in the sidebar.

#### Persistence

- The tool result is persisted as normal conversation history.
- Sidebar state is ephemeral in v1.
- Later, extension search history or citations may be persisted through session `Custom` entries.

### Error Handling and Safety

- Oino must start normally if an extension fails to load.
- Broken extensions are marked unhealthy with actionable diagnostics.
- Tool failures return error `ToolResult`s and update the sidebar error state.
- `host.web.search` enforces timeout, max result count, response size limit, and rate limits.
- WASM cannot access shell, filesystem, secrets, or raw network by default.
- Network access is mediated through named host capabilities.
- Anti-detection/fingerprinting is framed as compatibility/resilience for public web access, not captcha solving or bypassing protected private content.
- Captcha-solving and Cloudflare-bypass daemons are out of scope for v1.
- Safe mode should allow Oino to start with extensions disabled or one extension disabled.

### Testing and Validation

- Manifest parsing and validation tests.
- Permission/capability authorization tests.
- Tool contribution to `ToolDefinition` conversion tests.
- WASM fixture tests for initialize, register tool, call host capability, return tool result, and deny unauthorized host calls.
- Mock web search backend tests for result normalization and error mapping.
- Optional ignored/env-gated live smoke tests for the selected HTTP backend.
- TUI tests for sidebar idle/loading/results/error rendering.
- Integration smoke in disposable worktree/branch: agent calls `web_search`, transcript receives tool output, sidebar displays results.

## Implementation Checklist

- [x] Create a disposable branch/worktree for the extension experiment. — covered by plan Task 1
- [x] Define the repo-local extension package layout under `extensions/`. — covered by plan Tasks 3 and 10
- [x] Add extension manifest schema and validation types. — covered by plan Task 2
- [x] Add capability and permission model for tool registration, sidebar UI, and named host capabilities. — covered by plan Tasks 2 and 5
- [x] Add a local extension discovery path for `extensions/*/oino.extension.json`. — covered by plan Task 3
- [x] Add WASM host lifecycle for loading, initializing, invoking, cancelling, and shutting down extensions. — covered by plan Task 4
- [x] Add extension-to-tool bridge that exposes extension tools as Oino `Tool` implementations. — covered by plan Task 6
- [x] Add native `host.web.search` capability interface and mock backend. — covered by plan Task 5
- [x] Validate Rust HTTP backend candidates, starting with `wreq`/`newwreq`, then fallback candidates. — covered by plan Task 11
- [x] Add the `extensions/web/` WASM package and `web_search` tool contribution. — covered by plan Task 10
- [x] Add declarative sidebar/panel contribution model. — covered by plan Task 8
- [x] Render web search sidebar states in TUI: idle, loading, results, error. — covered by plan Task 8
- [x] Connect web search tool execution to sidebar state updates. — covered by plan Task 9
- [x] Add settings/diagnostics surface for extension health and permissions. — covered by plan Task 12
- [x] Add tests for manifest validation, capability checks, WASM invocation, tool bridge, web capability, and sidebar rendering. — covered by plan Task 13
- [x] Add documentation for the experimental extension layout and dogfood workflow. — covered by plan Task 14

## Open Questions

- Which WASM interface should Oino use first: raw Wasmtime host functions, WASI Preview 2/component model, or a simpler serialized-call ABI?
- Should `extensions/` discovery be enabled by default in development builds only, or always with explicit permission prompts?
- Should the web sidebar auto-open on search, or stay visible only when the user enables/pins it?
- Which HTTP/search backend should become the default after implementation validation: `wreq`, `newwreq`, `chromimic`, or a simpler fallback?
- Which search source should v1 use: direct HTML search scraping, a public API, self-hosted search proxy, or configurable providers?
- Should sidebar state remain ephemeral for v1, or should search history be stored as session `Custom` entries immediately?
- How should extension permissions be represented in project/global settings before a full package registry exists?

## Out of Scope

- Porting Pi extensions.
- Hosted extension marketplace/registry implementation.
- NPM, pip, or Cargo install flows.
- Dynamic Rust library loading.
- Arbitrary extension-owned Ratatui rendering.
- Full floating panel framework beyond the first sidebar-oriented panel surface.
- Captcha solving, account-authenticated scraping, or bypassing protected private content.
- Provider plugins, OAuth flows, MCP, memory extensions, and background agents.
- Persistent extension state synchronization beyond minimal sidebar/tool state.
