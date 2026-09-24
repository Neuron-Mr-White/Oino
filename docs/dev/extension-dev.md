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
