# Extension Development

Extensions are packages with one `oino.package.json` and one or more `oino.extension.json` manifests.

## Validate an extension package

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
```

## Package shape

```text
my-package/
  oino.package.json
  extensions/
    my-extension/
      oino.extension.json
      ...assets or wasm files...
```

## What extensions can contribute

- Slash commands
- Tools
- Settings pages
- Themes
- Keymap entries
- Prompts and skills
- Provider/runtime metadata
- Hooks and diagnostics

## Hook events

`HookEventKind` is the stable event vocabulary (snake_case in manifests):
`startup`, `resource_discovery`, `session`, `input`, `command`,
`before_agent_turn`, `after_agent_turn`, `context`, `provider_request`,
`provider_response`, `message_stream`, `tool_call`, `tool_result`,
`tool_update`, `model_selection`, `thinking_selection`, `compaction`, `tree`,
`reload`, `install`, `update`, `remove`, `package_lifecycle`, plus the pi-style
set: `before_agent_start`, `context_with_system`, `before_provider_headers`,
`provider_stream_event`, `turn_end`, `agent_before_settle`, `agent_settled`,
`session_before_fork`, `session_before_switch`, `session_before_tree`,
`session_compact_failed`, `session_info_changed`, `resources_discover`,
`project_trust`, `ui_prompt_start`, `ui_prompt_end`, `cache_warming_decision`,
`agent_start`, `agent_end`, `turn_start`, `message_start`, `message_end`,
`tool_execution_start`, `tool_execution_end`.

Payloads are typed (`oino_extension_runtime::HookEventPayload`) — transcripts
are `oino_types::Message` values, not strings. Actionable events have typed
results: `before_agent_start` returns `BeforeAgentStartResult { section_edits }`
(never a whole prompt) and `turn_end` returns
`TurnEndResult { entries, continue_run }`.

## Local testing loop

1. Edit the package.
2. Run the devkit validator.
3. Install or copy it into `~/.oino/extension-packages/`.
4. Run `/reload`.
5. Open `/extensions` and check diagnostics.

Keep extension IDs stable and prefer explicit permissions. Oino does not implicitly load unrelated agent extension folders.

## WASM components (`oino:extension@0.1.0`)

WASM runtimes are real Wasmtime components, not the retired `wasm-json-v1`
JSON fixtures. The WIT package lives at `wit/` (`oino:extension@0.1.0`) with
two worlds: `extension` (tools/commands/hooks) and `provider-extension` (adds a
pull-based `provider.stream`/`response.next` streaming path). Host imports in
`oino:extension/host` (`log`, `secret`, `kv-get`/`kv-set`, `session-append`,
`notify`, `confirm`, `exec`, `tool-progress`) are each gated by a manifest
permission; `wasi:http/outgoing-handler` enforces `network.hosts`.

Guest crates live in the separate `extensions/` workspace (not the host
workspace). A component implements `Extension` (plus `Provider` for providers)
and exports it with `export_extension!` / `export_provider!` from
`oino-extension-guest` / `oino-extension-guest-provider`. Payloads cross the
boundary as JSON; `host::tool_progress`, `host::kv_*`, `host::secret`, and a
blocking `http` helper with `read_chunk()` for SSE are provided by the SDK.

### Build workflow

```bash
rustup target add wasm32-wasip2
bash scripts/build-wasm-extensions.sh   # builds + copies fixture components
cargo build --release --target wasm32-wasip2 --manifest-path extensions/Cargo.toml
```

wasm32-wasip2 emits a ready-to-load component directly — no separate
`cargo-component`/`wasm-tools` step. A wasm manifest must declare
`runtime: { kind: "wasm", entry: "plugin.wasm", abi: "oino:extension@0.1.0" }`;
other ABIs are rejected as `UnsupportedWasmAbi` and the extension is skipped
with a diagnostic. Host-side limits: 256 MiB memory per store, epoch deadlines
(hooks 2 s, commands 30 s, tools 10 min, provider `next` 120 s), traps mark an
extension unhealthy after three failed re-instantiations.

### `handle_event` result contract

`guest.handle_event(kind, payload)` returns a JSON value:

- `null` → no-op.
- `input`: request `{"text": <user text>}` → `{"payload": <replacement text>}`.
- `context`: request `{"messages": [Message]}` → `{"payload": [Message]}`.
- `tool_call`: request `{"call": ToolCall}` → `{"payload": ToolCall}` to
  rewrite, or `{"block": {"reason": str, "terminate": bool}}` to block.
- `tool_result`: request `{"result": ToolResult}` → `{"result": ToolResult}`.
- `before_agent_start` → `{"section_edits": [PromptSection]}`.
- `turn_end`: request `{"message", "tool_results"}` →
  `{"entries": [Message], "continue_run": bool}`.
- observe-only kinds (`agent_start`, `agent_end`, `agent_settled`,
  `message_start`, `message_end`, `turn_start`, `tool_execution_start`,
  `tool_execution_end`) receive `{"event": <AgentEvent JSON>}`; results are
  ignored. Hook errors/timeouts mark the extension unhealthy and fall back to
  no-op.

### Host capability policy

- `host.secret(name)` resolves only names declared in `permissions.secrets`,
  from `~/.oino/extensions/<id>/config.json` then the environment.
- `host.kv-*` persists under `~/.oino/extension-data/<id>/kv/` and requires a
  non-empty `session_persistence` declaration.
- `host.exec` requires `shell_process.allowed` and an exact program match in
  `shell_process.commands`; the child environment is cleared and only carries
  `PATH` and `HOME`; an optional `cwd` must lie inside a declared filesystem
  preopen.
- `host.notify` is surfaced in the TUI; `host.confirm` currently returns
  `false` after emitting a notice (interactive confirm is not wired yet).
- `network.hosts` gates `wasi:http` outgoing requests by exact host match.
  **`raw_network` is currently not granted to anything**: raw `wasi:sockets`
  (TCP/UDP) and DNS lookups are disabled in every component regardless of the
  manifest flag.
