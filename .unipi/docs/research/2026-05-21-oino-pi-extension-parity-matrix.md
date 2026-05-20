# Oino ↔ Pi Extension Capability Parity Matrix

Date: 2026-05-21
Plan: `.unipi/docs/plans/2026-05-21-oino-extension-kernel-roadmap-plan.md`

## Scope and decision

Oino targets semantic parity with the useful extension capabilities in Pi, but **does not target Pi TypeScript extension compatibility**. Pi packages/extensions run in Node with full host permissions; Oino's extension kernel is registry-first, Rust-native, and WASM-first for untrusted community extensions.

Status legend:

- **implemented-now** — Oino already has a native capability that should be registered as a built-in contribution.
- **planned** — included in this roadmap.
- **deferred** — intentionally left for a later plan after core contracts stabilize.
- **rejected** — not an Oino goal.

## Matrix

| Pi capability | Pi source / behavior | Oino-native equivalent | Status | Roadmap task(s) | Rationale |
|---|---|---|---|---|---|
| Global/project extension discovery | `~/.pi/agent/extensions`, `.pi/extensions`, `settings.extensions`, `pi -e` | Oino-owned global/project/dev extension/package sources under explicit `.oino`/`~/.oino` layouts | planned | 4, 7, 9, 20 | Keep deterministic Oino-owned discovery; do not silently load Pi paths. |
| Hot reload | `/reload` reloads auto-discovered extensions/resources/themes | Extension Manager reload with registry snapshot diffs and diagnostics | planned | 5, 9, 10, 21, 24 | Needed for local authoring and safe package lifecycle. |
| Custom model-visible tools | `pi.registerTool()` with TypeBox schema and execution handler | Tool registry contributions bridged into `oino_agent_loop::Tool` | planned | 6, 8, 12, 13, 14 | Core extension vertical slice and model-visible integration. |
| Dynamic tool registration | Extensions can call `registerTool()` at runtime | Manifest-first tool contributions; dynamic registration deferred until runtime policy is stable | deferred | 14, 23 | Manifest-first enables permission review before execution. |
| Tool-call block/patch hooks | `tool_call`, `tool_result`, execution update hooks can block or mutate | Typed hook registry with cancellable/mutable tool events and typed patches | planned | 11, 13, 14, 24 | Preserve safety with deterministic fallback and timeout diagnostics. |
| Input interception | `input` event can transform/handle user input | Input/command hook group returning typed action or patch | planned | 11, 17 | Useful, but must avoid arbitrary mutation surprises. |
| Context/system-prompt mutation | `before_agent_start`, `context`, compaction hooks | Typed context/prompt patch hooks, compaction/tree/session hooks | planned | 11, 19, 24 | Oino already has some harness hook seams; registry externalizes them. |
| Provider request/response hooks | `before_provider_request`, `after_provider_response`, payload mutation | Provider/model registry plus privacy-gated typed provider hooks | planned | 11, 17 | High privacy risk, so typed policy-gated mutation only. |
| Provider/model registration | `pi.registerProvider()`, `models.json`, custom streaming providers | Provider/model registry entries, provider metadata, future custom provider adapters | planned | 6, 8, 11, 17 | Oino already has OpenRouter provider boundaries; extensibility should be registry-backed. |
| OAuth/login provider flows | Pi provider OAuth callbacks and `/login` integration | Oino auth/provider contribution category with explicit credential permissions | deferred | 17, 23 | Needs careful auth UX and secret handling after core provider registry. |
| Custom slash commands | `pi.registerCommand()` | Command registry contributions bridged into app/TUI command handling | planned | 6, 8, 14, 18 | Important for user-invoked extension UX. |
| Extension shortcuts/keybindings | `registerShortcut()`, keybinding ids in `keybindings.json` | Keymap registry contributions with conflict/provenance badges | planned | 6, 8, 15, 17, 18 | Oino already has keymap concepts; conflicts need registry diagnostics. |
| Flags/settings contributions | `registerFlag()`, settings arrays | Settings-page and extension policy registries with persisted enablement/overrides | planned | 6, 7, 15, 18 | Oino should centralize extension configuration and provenance. |
| TUI custom components | `ctx.ui.custom()` arbitrary component render/input | Core-owned first-class surfaces plus constrained declarative UI; no extension code in Ratatui render path | planned | 15, 16, 17, 18 | Direct arbitrary rendering is rejected initially for safety/perf. |
| Overlay/floating UI | Pi overlay options, anchors, responsive visibility | Floating panel/overlay surface contributions with layout/focus/tiny-terminal policy | planned | 15, 16, 18 | Needs deterministic focus and layout behavior. |
| Footer/status/widget/header/titlebar UI | `setStatus`, `setWidget`, custom footer/header examples | Footer/status/sidebar/header-like surfaces as registry-backed state | planned | 15, 16, 18 | Useful for extension health and progress surfaces. |
| Custom tool renderers | Extension renderers for tool calls/results | Tool renderer registry using core-owned render adapters and declared schemas | planned | 15, 16, 24 | Maintain render safety; avoid arbitrary runtime code in render paths. |
| Message renderers/custom messages | Custom message rendering and session custom entries | Transcript/message renderer registry plus typed custom session entries | planned | 15, 16, 19 | Must replay safely without loading unsafe extension code. |
| Themes | `~/.pi/agent/themes`, `.pi/themes`, packages, hot reload | Theme-token registry and Oino-owned theme packages | planned | 6, 8, 15, 17, 20, 25 | Oino already themes TUI; packages should contribute tokens/themes. |
| Prompt templates | Pi `prompts/` and package entries | Oino resource registry for prompt templates under `.oino`/`~/.oino` | implemented-now | 6, 8, 20, 25 | Oino already has explicit prompt resources; migrate to registry snapshots. |
| Skills | Pi skills and package entries | Oino skill resource registry under explicit Oino paths | implemented-now | 6, 8, 20, 25 | Oino has skills/prompts resource layer; no implicit Pi/Claude/agents paths. |
| Resource discovery hooks | `resources_discover` can add skills/prompts/themes | Resource registry contributions and controlled resource discovery hooks | planned | 6, 8, 11, 20 | Keep explicit source provenance and avoid hidden path loading. |
| Session persistence | `appendEntry()`, custom entries survive restart | Typed extension persistence/session APIs with migrations and cleanup | planned | 4, 11, 19, 24 | Must preserve replay safety and extension ownership boundaries. |
| Session lifecycle hooks | `session_start`, switch/fork/compact/tree/shutdown | Session hook group with cancellable/mutable typed events | planned | 11, 19 | Useful but must avoid corrupting session state. |
| User prompts/notifications | `ctx.ui.confirm/select/input/notify` | Host UI capability broker and command/tool interaction surfaces | planned | 13, 15, 18, 23 | Needs permission and non-interactive fallback policy. |
| Long-running tool updates/progress | Tool update callbacks and UI status widgets | Runtime progress channel, diagnostics, UI state updates | planned | 12, 13, 14, 16 | Natural boundary for WASM runtime and tool bridge. |
| Package install/remove/update | `pi install/remove/update/list`, npm/git/local sources | Oino package lifecycle service, local/installed package layouts, registry metadata fixtures | planned | 20, 21, 22 | Oino must add trust/permission prompts before broad distribution. |
| npm/git package compatibility | Pi packages with `package.json` `pi` key and Node deps | Oino package manifests with explicit Oino-owned assets; import tooling may copy later | rejected | 20, 22, 25 | Direct compatibility would import full Node trust model and Pi conventions. |
| Community package gallery | `pi-package` npm keyword/gallery metadata | Oino registry metadata, categories/search, review/signing/checksum/advisories | planned | 22 | Implement metadata/trust locally before hosted registry. |
| Native extension code with full host access | Pi TS extensions run with full system permissions | WASM-first untrusted runtime; trusted native/sidecar only behind later explicit policy | deferred | 12, 13, 22, 23 | Native sidecars are useful but risky; not a default community path. |
| Arbitrary filesystem/process/network imports | Node built-ins and npm deps available directly | Named host capabilities brokered through permissions, limits, timeouts, audit | planned | 4, 12, 13, 24 | Oino must not expose raw host power by default. |
| Custom compaction/tree behavior | Pi compaction/tree events can customize summaries | Typed compaction/tree hooks and custom session entries | planned | 11, 19 | Keep deterministic fallback and auditability. |
| Extension examples/devkit | Many TS examples under `examples/extensions` | Oino manifest generator, validator, SDKs, examples, test harness, local reload | planned | 23, 25 | Needed after contracts stabilize. |
| Direct Ratatui rendering from extension code | Not Pi/Rust-specific; Pi components render strings with input handlers | Extension code running inside Ratatui render paths | rejected | 15, 16 | Violates Oino deterministic state-machine render direction. |
| Pi TypeScript extension API | `@earendil-works/pi-coding-agent` imports | Compatibility shim for Pi API | rejected | 2, 25 | Oino should document semantic parity, not API compatibility. |

## First-phase coverage

Tasks 3–4 intentionally cover the foundation rows only: identity, protocol, source, manifest, package, permissions, provenance, diagnostics, compatibility, and conflicts. Later tasks migrate built-ins, add registry composition, connect runtime execution, and expose UI/package/community layers.
