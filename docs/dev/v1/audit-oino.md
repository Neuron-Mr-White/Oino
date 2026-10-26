# Audit: current Oino codebase → v1 restructure

Status: audit (work id `audit-oino`, 2026-10-26). Read-only; no code changed.
Baseline: branch `v1-alpha` at `17dcdb7` ("Start v1.alpha: charter; drop revival specs
and P5"). Companion notes: [research-acp.md](research-acp.md),
[research-gpui.md](research-gpui.md). Contract: [CHARTER.md](CHARTER.md).

## 1. Snapshot

| Area | Content | Size |
|---|---|---|
| `crates/` (host workspace, 18 crates) | runtime, TUI, app binary, WASM host | 81.5 k lines of Rust (incl. inline tests) |
| `extensions/` (separate workspace, `wasm32-wasip2`, 15 crates) | guest SDK, provider kit, 4 providers, 6 built-in ext, 2 fixtures | 9.9 k lines |
| `extensions/built-in/` | 14 package dirs (`oino.package.json` + `oino.extension.json`, 7 with committed `plugin.wasm`) | — |
| `wit/` | `extension.wit` (`oino:extension@0.1.0`, 152 lines) + WASI 0.2.12 deps | — |
| `scripts/` | `build-wasm-extensions.sh`, installers (`install.sh`, `install.ps1`, `install-interactive.sh`, `install-smoke.sh`, `install-all-builtins.sh`), `docs-check.sh`, `podman*`, `pi-oracle/` (pi v0.87.1 provider fixtures generator) | — |
| `examples/extensions/rust-wasm-fixture` | sample package (copy of `fixture.wasm`) | — |
| Committed binaries | 13 `.wasm` files (3 provider assets in `crates/oino-app/assets/providers`, 2 host test fixtures, 1 example, 7 built-in packages) | 5.8 MB |

P5 / FrontierHarness: no code or scripts remain (only historical notes under the
git-ignored `.unipi/` working dir). Nothing to delete for P5 in `crates/`.

## 2. Build / test / lint status (measured 2026-10-26)

Toolchain: rustc/cargo **1.99.0** (mise sets `RUSTUP_TOOLCHAIN=1.99.0`; no
`rust-toolchain.toml` in the repo). Machine: 61 GB RAM, heavily loaded (6 GB available).
Separate `CARGO_TARGET_DIR` was used to avoid disturbing concurrent agents.

| Command | Result | Wall time |
|---|---|---|
| `cargo build --workspace` | ✅ | 54 s (cold target dir) |
| `cargo test --workspace --no-fail-fast` | ✅ **534 passed, 0 failed, 1 ignored** (`bench_overhead`) | 78 s with `-j 4` (first attempt at default parallelism was SIGTERM-killed under memory pressure) |
| slowest test binaries | `oino-wasm-host/tests/host.rs` 24 tests **34.1 s**; `oino-app` unit tests 96 tests 18.7 s; `oino-tui` 219 tests 0.07 s | |
| `cargo clippy --workspace --all-targets -- -D warnings` | ❌ **19 errors**, all `clippy::double_must_use` (new in clippy 1.99): 14 in `oino-env/src/lib.rs`, 5 in `oino-agent-loop/src/lib.rs`. With that lint allowed, the rest of the workspace is clean. | 21 s |
| `cargo clippy … -W clippy::pedantic` (charter gate) | ❌ **832 warnings**: oino-tui 265, oino-app 191, oino-harness 83, oino-extension-sdk 48, oino-auth 30, oino-wasm-host 30, oino-extension-manager 27, oino-extension-builtins 26, oino-extension-core 25, oino-extension-runtime 21, oino-session 16, oino-env 15, oino-agent-loop 12, oino-types 11, oino-agent 10, oino-provider-catalog 8, oino-tools 7, oino-resource 7. Top lints: `missing_errors_doc` 139, `redundant_closure` 61, `match_same_arms` 56, `needless_pass_by_value` 31, `unnested_or_patterns` 30 | 15 s |
| `cargo fmt --all --check` | ✅ | 1 s |
| `extensions/`: `cargo build --release --target wasm32-wasip2 --workspace` | ✅ with toolchain 1.98.1 (9 s warm). ❌ with the active 1.99.0: `wasm32-wasip2` std is **not installed** for 1.99.0 | |
| `extensions/`: `cargo clippy --target wasm32-wasip2 --all-targets -- -D warnings` | ✅ (1.98.1) | 8 s |
| `extensions/`: `cargo fmt --check` | ✅ | |

Other quality facts:

- Every crate has `#![forbid(unsafe_code)]`; workspace lints deny `unwrap_used` /
  `expect_used` (so non-test code is free of them); 21 inline `#[allow]`s.
- No crate enables `missing_docs`; rustdoc coverage is uneven (crate-level docs are good
  in tui/wasm-host/extension crates; `oino-app` modules mostly undocumented).
- `thiserror = "1"` (current is 2); `anyhow` only in `oino-wasm-host` (a library — the
  charter wants `anyhow` only at binary edges).
- Re-building the provider components with 1.98.1 gives different sizes from the
  committed assets (e.g. `provider_anthropic.wasm` 688 867 B built vs 689 788 B
  committed) → committed binaries are not reproducibly tied to source/toolchain; there
  is no checksum or CI check.

## 3. Crate map

Legend for *v1 verdict*: **keep** (move/rename, light edits), **rewrite** (concept
stays, code mostly replaced to reach pi 1.4.0 parity), **split**, **delete**.

| Crate | LOC | Responsibility | Key public API | Internal deps → | Used by | v1 verdict |
|---|---:|---|---|---|---|---|
| `oino-types` | 1 070 | Message/content/usage/model types, transcript normalisation | `Message {System,User,Assistant,ToolResult…}`, `ContentBlock`, `AssistantStreamEvent`, `Usage`, `Model`, `ThinkingLevel`, `transcript::{normalize,collapse,render_system_prompt}` | — | 10 crates + provider-kit (wasm) | **rewrite** → `oino-ai` types mirroring pi `AgentMessage`/pi-ai shapes (camelCase serde). Oino's `Message::System` transcript model is not pi's. |
| `oino-agent-loop` | 2 613 | Turn loop: stream provider, tool exec (parallel/sequential), steering/follow-up drains, hooks as closures | `run_agent_loop`-style API, `AgentEvent`, `Tool` trait, `StreamProvider` trait, `StreamRequest`, `AbortSignal`, `BeforeToolCall/AfterToolCall/TransformContext/PrepareRequest` callbacks | types | agent, harness, tools, wasm-host, ext-builtins, ext-runtime, app | **keep** core idea (≈ pi-agent-core `agentLoop`), merge with `oino-agent` into `oino-agent` |
| `oino-agent` | 806 | Stateful `Agent` (state, queue modes, subscribe, steer/follow_up/abort/wait_for_idle) | `Agent`, `AgentState`, `QueueMode` | agent-loop, types | harness | **keep** (≈ pi-agent-core `Agent`), merge |
| `oino-harness` | 3 269 | Agent + session + hooks registry + system prompt sync + deferred tools + cache tracker; e2e test | `Harness {prompt, steer, abort, compact, navigate_tree, set_model, set_tools, sync_system, …}`, `HookRegistry` (`NotificationHook`, `MutatingHook`), `CacheTracker` | agent, agent-loop, env, session, types | app | **rewrite** into `oino-core::AgentSession` (≈ pi `agent-session.ts`); hook registry replaced by pi's extension event model |
| `oino-session` | 1 176 | Tree session (v2 JSONL), entries, branch, context build, repository/list | `SessionManager`, `SessionRepository`, `SessionEntry{Kind}`, `SessionHeader {version: 2}` | types, **extension-core** | harness, app | **rewrite** to pi session v3 exactly + v1/v2 migration; drop extension-core dep |
| `oino-resource` | 976 | Skills, prompt templates, resource catalog, scopes | `ResourceCatalog`, `ResourcePaths`, `Skill`, `PromptTemplate` | — | ext-builtins, app | **keep/extend** → `oino-core::resources` (+ AGENTS.md / SYSTEM.md / settings per pi resource-loader) |
| `oino-env` | 274 | `ExecutionEnv` trait (exec, read/write/stat) + local impl | `ExecutionEnv`, `LocalExecutionEnv`, `CommandOptions/Output` | — | harness, tools, ext-builtins | **keep** as `oino-core::env` — it is the seam for ACP `fs/*`/`terminal/*` delegation |
| `oino-tools` | 1 209 | Built-in tools `read`, `write`, `edit`, `bash` + `tool_search`, `session_title` | `default_tools`, `ReadTool`… | agent-loop, env, types | ext-builtins, app | **keep** the four pi tools in `oino-core::tools` (re-verify vs pi 1.4.0 behaviour); `tool_search`/`session_title` → extensions or delete |
| `oino-extension-core` | 4 600 | Manifest schema, permissions, 30-variant `HookEventKind`, contribution registries (tools/commands/keymaps/hooks/ui surfaces/settings pages/themes/providers/resources), policy, conflicts | `ExtensionManifest`, `ExtensionPermissions`, `*Registry`, `UiSurfaceContribution`, `ProviderRuntimeProtocol` | types | session, tui, ext-builtins, ext-manager, ext-runtime, sdk, wasm-host, app | **rewrite (shrink)**: keep manifest + permissions, drop contribution registries/UI-surface model in favour of pi's runtime registration (`registerTool/Command/Shortcut/Flag`, events, `ctx.ui`) |
| `oino-extension-manager` | 3 633 | Discovery, snapshots, package lifecycle (install/update/remove, checksums, trust), persistence | `ExtensionManagerSnapshot`, `PackageLifecycleService`, `ExtensionPersistenceStore` | **ext-builtins**, ext-core | sdk, app | **split**: package manager → `oino-core::packages` (≈ pi `package-manager.ts`); discovery/policy rewritten |
| `oino-extension-builtins` | 757 | Adapts built-ins into registries + catalog of optional built-in packages | `BuiltinRegistryCatalog`, `optional_builtin_packages` | agent-loop, ext-core, provider-catalog, resource, **tui**, env, tools | ext-manager, app | **delete** (built-ins removed; layering violation: depends on the TUI) |
| `oino-extension-runtime` | 1 580 | Pre-WASM "JSON-v1" runtime trait, `HookRunner`, `CapabilityBroker`, tool/command adapters | `ExtensionRuntime`, `HookRunner`, `CapabilityBroker` | agent-loop, ext-core, types | sdk; app uses only 2 result structs; wasm-host declares dep but **never uses it** | **delete** (dead since P2 WASM host) |
| `oino-extension-sdk` | 1 646 | Devkit: manifest validators, fixture JSON-v1 runtime, coverage matrix, `oino-extension-devkit` bin | `validate_package_dir`, `JsonWasmRuntime`, `WasmSdk`, … | ext-core, ext-manager, ext-runtime | nothing (leaf) | **delete**; replace with guest SDK docs + `oino ext validate` subcommand. NB: misleading name — the real guest SDK is `extensions/crates/oino-extension-guest` |
| `oino-auth` | 1 795 | `auth.json` credential storage, accounts, readiness/validation records, external auth trust | `AuthStorage`, `AuthCredential`, `ProviderAuthAssessment` | — | app | **rewrite (shrink)** → `oino-core::auth` ≈ pi `auth-storage.ts`; OAuth flows belong to provider extensions |
| `oino-provider-catalog` | 1 183 | Static list of ~40 providers (OpenAI-compatible profiles, env keys) | `providers()`, `ProviderDescriptor`, `OpenAiCompatibleProfile` | — | ext-builtins, tui, app | **delete from core**; data moves into provider extension manifests/configs (charter: no provider code in core) |
| `oino-wasm-host` | 2 453 | Wasmtime 48 component host: engine + cache, epoch deadlines, memory limits, capability gating, `WasmStreamProvider`, `WasmTool` | `WasmHostEngine`, `WasmExtension`, `HostCaps`, `SecretResolver/HostUi/HostSession/HostModel`, `CallLimits` | agent-loop, ext-core, ext-runtime (unused), types | app | **keep + extend** for new WIT (`oino:extension@1`): events, commands, UI requests, timers, subprocesses, subagents |
| `oino-tui` | 32 441 (19 files; `render.rs` 7.7 k, `app.rs` 6.8 k, `command.rs` 3.8 k) | Ratatui state machine, rendering, composer, keymap, settings overlays, themes, markdown/mermaid, model selector, extension surfaces, help | `TuiState`, `TuiAction` (54 variants), `render`, `KeymapConfig`, `ThemeCatalog`, `SettingsState` | ext-core, provider-catalog, types | ext-builtins, app | **keep (as client)** but strip built-in specifics (ralph 156, notify 436, router 172, btw 121, search 509, footer 121, preset 105 mentions) and re-target onto the core event API + generic extension UI |
| `oino-app` (bin `oino`) | 19 985 (16 files; **`main.rs` 11 915**: ~7 800 code + ~4 100 tests) | Everything else: CLI, config, auth wiring, headless/print/JSON mode, settings mutation, extension install/update/git, extension→TUI surface glue, router container management, wasm wiring, compaction (LLM + VCC), mode-sandbox profiles, TUI event loop, chat HTML export, file drop/mentions, URL opening/WSL, update | `main()` only | **every crate** | — | **split** (see §4) |

Dependency layering problems (cargo has no cycles, but the arrows point the wrong way):

1. `oino-extension-builtins → oino-tui` (runtime-side crate depends on the UI).
2. `oino-extension-manager → oino-extension-builtins` (generic manager knows built-ins).
3. `oino-session → oino-extension-core` (session format coupled to manifest types).
4. `oino-tui → oino-provider-catalog, oino-extension-core` (UI knows provider lists and
   manifest registries instead of consuming view models).
5. `oino-wasm-host → oino-extension-runtime` (declared, unused).
6. Four overlapping event/hook vocabularies: `oino_agent_loop::AgentEvent`,
   `oino_harness::{NotificationHook, MutatingHook}`, `oino_extension_core::HookEventKind`
   (30 variants), `oino_extension_runtime::HookEventPayload`, plus WIT
   `handle-event(kind: string, payload: json)`. pi has one: the extension event set
   (`session_start`, `before_agent_start`, `context`, `tool_call`, `tool_result`,
   `message_update`, `turn_end`, `agent_end`, …) plus `AgentSessionEvent` for clients.

## 4. How oino-app / oino-tui / oino-harness are entangled

`oino-harness` is reasonably clean (agent + session + hooks). The entanglement is
`oino-app/src/main.rs`, which is simultaneously the CLI, the runtime composition root,
the extension host glue and the TUI controller:

| main.rs region (lines) | What it does | v1 home |
|---|---|---|
| 171–370 `CliArgs`, usage | hand-rolled arg parsing (`--json`, `-p`, `--max-cost`, `--preset`, …) | `crates/oino` CLI (clap), flags per pi `cli.md`; `--max-cost/--preset` die with budget/presets |
| 380–780 `AppConfig`, `build_auth_resolver`, `load_or_create_session` | runtime composition | `oino-core` (`AgentSessionServices` ≈ pi `agent-session-services.ts`) |
| 781–920, 5927–6080 `JsonLinesEventSink`, `headless_*`, `run_non_interactive` | print / JSON mode | `oino-core::modes::{print,json}` |
| 922–1325 `build_harness`, tool settings, notify settings, extension enable/disable | settings mutation | `oino-core::settings` (SettingsManager); notify parts deleted |
| 1325–1918 extension install sources, git clone, update | package manager | `oino-core::packages` |
| 1918–2941 `extension_ui_surfaces`, `ralph_panel_surface`, shortcuts, autosuggest, themes, model options, management items | extension → TUI projection | rewritten as generic extension UI (TUI client); ralph parts deleted |
| 2943–3007 router external endpoint | managed router container | delete (router UI ext); Oino Router stays as a provider extension |
| 3013–3330 `execute_extension_command`, wasm init config, cache warming, `rebuild_wasm`, `apply_tool_settings_to_harness`, ask-user requester | extension runner glue | `oino-core::extensions::runner` |
| 3337–3470 HTML chat export | `export_html` | `oino-core::export` (pi `session-export.ts`) |
| 3472–3500 `system_prompt_parts` | system prompt | `oino-core::system_prompt` |
| **3502–4931 `run_tui`** (~1 430 lines, matches 54 `TuiAction`s) | TUI event loop + runtime calls | `oino-tui::interactive` over the `oino-core` API |
| 4931–5480 TUI session switch/reload, queue drain, `/btw` side harness, `start_prompt` | mixed | queue → core; `/btw` → unipi `btw` extension; UI parts → TUI |
| 5479–5930 mouse, URL/WSL opener, project file scan, dropped files, image attachments | TUI / input | TUI (`@file` / image attachment parsing → core `prompt` input handling) |
| 6081–6300 CLI `@`resource references | prompt expansion | core (pi expands skills/templates in `AgentSession.prompt`) |
| 6304–6605 ralph args, optional package gating, **mode-sandbox profiles** | built-ins | delete (unipi ports replace) |
| 6605–6728 `compact_session_with_vcc/llm`, `recall_session_with_vcc` | compaction | LLM compaction → `oino-core::compaction` (pi `compaction.md`); VCC deleted |
| 6728–7156 `execute_runtime_command` (~430 lines of slash commands) | command dispatch | split: core commands (`compact`, `new`, `fork`, `name`, `session`, `export`, …) in core; UI-only commands in TUI |
| 7156–7316 session list helpers | | core `SessionManager::list` + view model |
| 7316–7700 `TuiRuntimeEvent`, `try_auto_compact`, `spawn_model_catalog_task`, mode hooks, stream hooks, `apply_tui_runtime_event`, credential preflight | runtime↔UI bridge | auto-compaction & model catalog → core; event application → TUI |
| 7715–7807 `TerminalGuard`, OSC 8 links | terminal | TUI |
| 7807–11915 tests | | split with the code they test |

Other `oino-app` modules: `llm_compact.rs` (core), `model_catalog.rs` 1 303 lines
(OmniRoute/OpenRouter enrichment — provider-specific → provider extensions; generic
model registry → core), `provider_config.rs` / `provider_runtime.rs` /
`extension_provider_runtime.rs` (declarative provider → bundled component wiring →
core `models` + wasm host), `wasm_extensions.rs` 1 631 (host bridges: secrets, UI, model,
session → core extension runner), `usage.rs` 881 (usage report → core `usage-totals`;
OpenRouter parts → extension), `auth_readiness.rs`/`extension_readiness.rs`
(readiness UI for removed built-ins → delete or fold into `/login` UX), `updater.rs`
454 (self-update → CLI crate), `user_settings.rs` (compaction settings → core
settings), `router.rs` 1 226 (podman-managed router container → delete),
`ralph_panel.rs`, `notify_types.rs`, `ask_user.rs` (→ delete; unipi ports).

`oino-tui` itself is UI-only by design (its crate docs say so) but its `TuiState` holds
domain state for removed built-ins (ralph panel, notify settings, router readiness,
search settings, footer status, presets, btw) and extension registry types. It never
calls the runtime directly — all runtime work is in `run_tui`. That makes the v1 move
tractable: keep rendering/composer/keymap/theme/markdown, replace the domain state with
view models fed by core events, and move `run_tui` into the TUI crate.

## 5. What is dead after removing built-ins (and P5)

Delete outright:

- Crates: `oino-extension-builtins`, `oino-extension-runtime`, `oino-extension-sdk`
  (+ bin `oino-extension-devkit`), `oino-provider-catalog` (data → provider manifests).
- `oino-app` modules: `router.rs`, `ralph_panel.rs`, `notify_types.rs`, `ask_user.rs`,
  `auth_readiness.rs`, `extension_readiness.rs`; main.rs regions for ralph, VCC,
  mode-sandbox, budget/presets CLI flags, notify settings, router, `/btw`, optional
  built-in package install choices, legacy router migrations
  (`migrate_legacy_router_*`).
- `extensions/built-in/*` (all 14 packages incl. committed `plugin.wasm`), keep only
  provider packages re-homed under `extensions/providers/`.
- `extensions/crates/ext-{budget,notify,presets,ralph,search,vcc}`.
- `oino-tui`: ralph floating panel, notify settings page, router readiness, search
  settings, footer-status surface, preset selector, btw overlay, ask-user overlay
  (re-implemented generically via extension UI `select/input/editor` requests), the
  `UiSurfaceContribution` slot/tab machinery (replaced by `setWidget/setStatus/
  setFooter/setHeader`-style requests).
- `scripts/install-all-builtins.sh`; `docs/built-in-extensions.md`; built-in sections in
  `docs/extension.md`, `docs/command.md`, `docs/configurations.md`.
- P5: nothing in code; `.unipi/` notes only.

Keep but rewrite: `scripts/build-wasm-extensions.sh` (→ `xtask`), installers,
`scripts/pi-oracle` (re-target pi 1.4.0, it is the parity oracle), `podman-test.sh`.

## 6. WIT, WASM host, guest SDK, provider bundling (current)

**WIT `oino:extension@0.1.0`** (`wit/extension.wit`):

- `types`: `json = string`; `model`, `stop-reason`, `content-block` (text/thinking/
  tool-call(json)/image(json)/custom(json)), `tool-declaration`, `usage`,
  `assistant-event` (start/text/thinking/signature/tool-call start-delta-done/usage/
  done/error/aborted).
- `host` imports (manifest-gated): `log`, `secret`, `kv-get/kv-set`, `session-append`,
  `session-entries(all-branches)`, `complete` (non-streaming model call via host,
  `host_capabilities: ["model.complete"]`), `notify`, `confirm`, `tool-progress`,
  `exec(cmd, args, cwd)`.
- `guest` exports: `init(config) -> registration{tools, commands, hooks, providers}`,
  `handle-event(kind, payload) -> json`, `run-tool(name, call-id, args) -> tool-output`,
  `run-command(name, args) -> json`.
- `provider` exports: `list-models`, `stream(request) -> response` resource with pull
  `next() -> option<list<assistant-event>>` and `cancel`.
- Worlds: `extension` (host + wasi http/clocks/filesystem) and `provider-extension`
  (host + wasi http; exports guest + provider).

Gaps vs pi extension API (what unipi needs): no `registerShortcut/Flag`, no
`registerToolRenderer`, no runtime `registerProvider`/`registerVirtualModel`, no UI request surface beyond `notify/confirm` (pi has
`select/input/editor/setStatus/setWidget/setTitle/setFooter/setHeader/custom`), no
`sendMessage/sendUserMessage/appendEntry` parity, no `getActiveTools/setActiveTools`
or model/thinking control, no timers, no streaming subprocess / background
process handles (only blocking `exec`), no subagent spawn (`createAgentSession`), no
event-specific typed records (all `json` strings + string kinds), no provider
registration API beyond static `providers` names. Breaking changes are allowed →
design `oino:extension@1.0.0` from pi's `ExtensionAPI` / `ExtensionContext` surface.

**WASM host (`oino-wasm-host`)**: wasmtime **=48.0.2** (component-model, async,
cranelift, cache, pooling allocator) + wasmtime-wasi / -wasi-http / -wasi-io 48.0.2;
shared `WasmHostEngine` with epoch interruption and on-disk compiled cache; one lazily
instantiated component per extension, calls serialised through a mutex; per-call
deadlines (`CallLimits {hook 2 s, command, tool, provider_next, memory_bytes,
max_restarts}`); capability gating from manifest `ExtensionPermissions` (network
hosts for wasi-http, filesystem preopens, `shell_process`, secrets,
`session_persistence`, `host_capabilities`); `WasmStreamProvider` adapts provider
components to `StreamProvider` with a dedicated pull task; `ProgressRouter` for tool
progress. App surfaces injected via `SecretResolver`, `HostUi`, `HostSession`,
`HostModel`. `forbid(unsafe_code)` holds. Good foundation — keep.

**Guest SDK** (in `extensions/crates`, not `crates/oino-extension-sdk`):
`oino-extension-guest` (wit-bindgen =0.62.0; `Extension` trait + `export_extension!`;
JSON in/out helpers; state in statics because exports are static fns) and
`oino-extension-guest-provider` (separate crate because two `generate!` calls collide).
`oino-provider-kit` (4 250 lines) is a faithful port of the former OpenRouter/OpenAI
wire logic (pi cache hints, SSE parsing, Anthropic, Responses), mirroring
`StreamRequest` from `oino-agent-loop` because tokio can't target wasm; it depends on
host `crates/oino-types` by path. v1: one guest crate (`oino-extension`) with a single
`wit_bindgen::generate!` over a world that has optional provider exports (or a
`provider` feature), and move shared wire types into a tokio-free `oino-ai` crate both
sides can depend on (removes the mirror).

**Providers and bundling**:

- Components: `provider-openai-compat`, `provider-openai-responses`,
  `provider-anthropic`, `provider-oino` (Oino Router; Anthropic Messages or OpenAI
  chat per model), all on `oino-provider-kit`.
- `scripts/build-wasm-extensions.sh` builds the `extensions/` workspace for
  `wasm32-wasip2` release (`opt-level = "s"`, LTO, strip) and **copies** artifacts into
  `crates/oino-wasm-host/tests/fixtures`, `examples/extensions/...`,
  `crates/oino-app/assets/providers/{openai_compat,anthropic,openai_responses}.wasm`
  and each built-in package's `plugin.wasm`. The `.wasm` files are committed.
- `crates/oino-app/src/wasm_extensions.rs:880-892` embeds the three generic provider
  components with `include_bytes!` and selects one by
  `ProviderRuntimeProtocol {OpenAiChatCompletions, AnthropicMessages,
  OpenAiResponses}`. Declarative provider packages (`extensions/built-in/
  provider-openai`, `provider-anthropic`) only carry manifests (`runtime.kind =
  built_in`, protocol, base URL, `api_key` secret, network hosts); the host
  synthesises permissions and instantiates the embedded component per provider
  contribution. `oino-router` ships its own `plugin.wasm`.
- Model metadata: `oino-provider-catalog` (40 static providers) +
  `oino-app/src/model_catalog.rs` (OpenRouter/OmniRoute caches) — provider knowledge
  living in core, contrary to the charter.

v1 recommendation: an `xtask` (`cargo xtask wasm`) builds all components, writes
`extensions/dist/*.wasm` + a `checksums.json`, and a `build.rs`/`include_bytes!` in the
distribution crates (`crates/oino` CLI, `apps/oino-desktop`) bundles the chosen set
(providers + unipi). CI rebuilds and diffs checksums. Pin the wasm toolchain in a
`rust-toolchain.toml` with `targets = ["wasm32-wasip2"]` (today's active 1.99.0 lacks the
target, see §2). Provider catalogs become provider-extension data (`list-models` +
bundled JSON), surfaced to core through a model registry API.

## 7. Restructure recommendations

1. **Make `oino-core` the only composition root.** Everything `main.rs` wires today
   (session services, auth, settings, resources, extension runner, providers via the
   WASM host, compaction, modes) moves behind `oino_core::AgentSession` /
   `AgentSessionRuntime` APIs that mirror pi 1.4.0 (`dist/core/agent-session*.d.ts`,
   `sdk.d.ts`). Clients get a typed command/event API, never internals.
2. **One event model.** Replace the four hook vocabularies with pi's: client-facing
   `AgentSessionEvent` (= `json.md` events) and extension-facing events (pi
   `extensions.md`). WIT v1 gets typed records for them.
3. **Session format v3 now**, before anything else is built on top: flat entries, 8-hex
   ids, `parentId`, ISO timestamps, camelCase, pi `AgentMessage`; migrate v1/v2 on load
   (`oino-session` already has a v1 loader to extend).
4. **Types follow pi.** `oino-ai` = pi-ai message/content/usage/model/stream-event types
   + `Provider` trait (no impls, no tokio) so the WASM provider kit can share them.
5. **Extension host as a trait in core, wasmtime in its own crate.** Core defines
   `ExtensionRuntime` (load, dispatch event, run tool/command, UI request channel);
   `oino-wasm-host` implements it. Core unit tests use an in-memory fake runtime.
6. **Frontend-agnostic extension UI** = pi `rpc-extension-ui` request/response records
   as Rust enums in core; TUI/GPUI/RPC/ACP render them.
7. **Delete before porting**: remove the dead crates/modules listed in §5 first (shrinks
   the clippy-pedantic backlog by ~120 warnings and ~8 k lines) so the new layout is not
   built around legacy seams.
8. **Gates**: add `rust-toolchain.toml` (one toolchain with `wasm32-wasip2`), fix the 19
   `double_must_use` errors, turn on `missing_docs` + pedantic per crate as each crate is
   (re)written, bump `thiserror` to 2, keep `anyhow` to binaries, run wasm-host tests in
   a separate CI job (34 s).
9. **Speed**: keep wasmtime compiled-component cache; pre-compile bundled components at
   install time; keep tests parallel-safe under memory pressure (they were killed at
   default `-j`).

## 8. Proposed target layout (pi-like monorepo)

pi monorepo analogue: `packages/ai` (types + providers), `packages/agent`
(agent-core), `packages/coding-agent` (runtime + modes), `packages/tui`. Oino keeps the
same cut but providers are WASM and the TUI is a client.

```text
Cargo.toml                     # host workspace: crates/* + apps/*  (extensions/ stays separate: wasm32-wasip2)
rust-toolchain.toml            # pinned, targets = [wasm32-wasip2, (mobile targets in apps/)]
wit/
  oino-extension/              # package oino:extension@1.0.0 (types, host, guest, provider, ui)
  deps/                        # WASI 0.2.x
crates/
  oino-ai/                     # ≈ pi-ai types: Message/AgentMessage, content, usage, Model, stream events,
                               #   Provider trait, StreamRequest. no tokio → also used by wasm guests
  oino-agent/                  # ≈ pi-agent-core: agent loop + Agent (state, queue, steer/follow-up, abort)
                               #   (merge of oino-agent-loop + oino-agent)
  oino-session/                # pi session v3 format: entries, tree, context build, list/repository,
                               #   v1/v2 migration. pure + tokio fs; readable by clients without core
  oino-core/                   # ≈ pi-coding-agent/core: AgentSession, AgentSessionRuntime/Services,
                               #   settings (~/.oino, .oino), resources (skills, prompts, AGENTS.md, SYSTEM.md),
                               #   system prompt, tools/{read,write,edit,bash} over env::ExecutionEnv,
                               #   compaction + branch summaries + context edits, models (registry,
                               #   virtual models, resolver), auth storage, packages (install/update),
                               #   extensions/{api types, runner, ExtensionRuntime trait, ui protocol},
                               #   export (html), usage totals, modes/{print, json}
  oino-wasm-host/              # wasmtime component host implementing oino_core::ExtensionRuntime
  oino-rpc/                    # pi-compatible JSONL RPC: server (`--mode rpc`) + typed client codec
  oino-acp/                    # ACP v1 agent adapter over oino-core (agent-client-protocol crate isolated here)
  oino-client/                 # SessionBackend trait: InProcess (oino-core) | Rpc (oino-rpc) [| Acp]
  oino-tui/                    # terminal client (ratatui) over oino-client; interactive mode
  oino/                        # bin `oino`: CLI (clap), mode dispatch (interactive|print|json|rpc|acp),
                               #   package CLI, self-update, bundles provider + unipi .wasm
  oino-ui/                     # GPUI views shared by desktop/mobile (gpui-kit); no core deps beyond oino-client
apps/
  oino-desktop/                # GPUI desktop (Linux/macOS/Windows): shell, bundling, installers
  oino-mobile/                 # GPUI mobile (iOS/Android) — remote-first via oino-client::Rpc
extensions/                    # separate workspace, wasm32-wasip2
  sdk/oino-extension/          # guest SDK (single crate, optional provider exports)
  sdk/oino-provider-kit/       # shared HTTP/SSE/wire helpers for providers (uses oino-ai)
  providers/{openai-compat, openai-responses, anthropic, oino-router}/
  unipi/{ask-user, autocomplete, background-tasks, btw, compactor, dream, footer, fusion,
         info-screen, input-shortcuts, kanboard, long-horizon, memory, notify, skill-registry,
         subagents, updater, utility, watchdog, web-api, workflow, …}/   # ports of unipi 3.0.0 packages
  fixtures/{fixture, fixture-provider}/
  dist/                        # built .wasm + checksums.json (generated; bundled by crates/oino & apps)
xtask/                         # build/bundle wasm, checksums, release packaging (replaces build-wasm-extensions.sh)
docs/                          # user docs (pi-style) ; docs/dev/v1/ specs
scripts/                       # installers, pi-oracle (pi 1.4.0 parity fixtures)
```

Dependency direction (strict, enforced with `cargo deny`/a layering test):

```text
oino-ai ← oino-agent ← oino-core ← {oino-wasm-host, oino-rpc, oino-acp, oino-client}
oino-ai ← oino-session ← oino-core
oino-client ← {oino-tui, oino-ui} ← {crates/oino, apps/*}
oino-ai ← extensions/sdk/* ← extensions/{providers,unipi}/*
```

No crate below `oino-client` may depend on ratatui, gpui, wasmtime (except
`oino-wasm-host`) or `agent-client-protocol` (except `oino-acp`).

### Mapping current → target

| Current | Target |
|---|---|
| oino-types | oino-ai (rewritten to pi shapes) |
| oino-agent-loop + oino-agent | oino-agent |
| oino-session | oino-session (v3) |
| oino-harness | oino-core::agent_session |
| oino-resource, oino-env, oino-tools | oino-core::{resources, env, tools} |
| oino-auth | oino-core::auth (slim) |
| oino-extension-core (manifest/permissions) | oino-core::extensions::manifest |
| oino-extension-manager (lifecycle) | oino-core::packages |
| oino-wasm-host | oino-wasm-host |
| oino-tui | oino-tui (client) |
| oino-app main.rs | split: oino-core, oino-tui, crates/oino |
| oino-extension-builtins, -runtime, -sdk, oino-provider-catalog | deleted |
| extensions/crates/oino-extension-guest(+provider) | extensions/sdk/oino-extension |
| extensions/crates/oino-provider-kit | extensions/sdk/oino-provider-kit |
| extensions/crates/provider-* | extensions/providers/* |
| extensions/crates/ext-*, extensions/built-in/* | deleted (unipi ports) |
| crates/oino-app/assets/providers | extensions/dist (generated) |

## 9. Suggested order of v1 work packages

1. `rm-legacy`: delete dead crates/modules/packages (§5), fix clippy errors, add
   `rust-toolchain.toml`.
2. `core-ai` + `core-session`: oino-ai types and session v3 with migration (everything
   else depends on these shapes).
3. `core-agent`: merge agent-loop + agent onto oino-ai.
4. `core-session-runtime`: oino-core AgentSession + services + settings/resources +
   built-in tools + compaction; print/JSON modes; golden tests against pi fixtures.
5. `wit-v1` + `wasm-host-v1` + guest SDK; port the four providers.
6. `rpc` (pi-compatible) → `client` → `tui` re-targeting → `acp`.
7. unipi ports (parallelisable per package) and GPUI apps.
