# Extension API and WIT

The contract between Oino and extensions is the WIT package `oino:extension@1.0.0` in
`wit/oino-extension/` (component model async on wasmtime 48, wit-bindgen 0.62). Pi data-model
values (messages, models, events, tool results) cross the boundary as canonical JSON with
pi's exact shapes; host capabilities are typed WIT.

| Interface | What it offers |
|---|---|
| `guest` (export) | `init`, `on-event`, `run-tool`, `run-command`, shortcuts, renderers, `route-model`, `propose-nudge`, services |
| `provider` (export) | `stream`, `list-models`, OAuth login/refresh (provider components) |
| `registry` | tools, commands, shortcuts, flags, renderers, settings namespaces, providers, virtual models, services |
| `session` | read entries / branch / tree, append entries, send messages, names, labels |
| `control` | model and thinking level, run state, context usage, compaction, tool control, command invocation, nested tool calls, settings snapshot |
| `ui` | dialogs (`select`, `confirm`, `input`, `editor`, `form`, `multiselect`, `approval`, `custom`), notify, status, widgets, frame, panels, editor text |
| `models` | `complete`, `stream`, `summarize-default`, `classify`, credentials |
| `agents` | nested in-process agent sessions (subagents, btw, dream, fusion) |
| `process`, `tasks` | subprocesses and background tasks |
| `http` | outbound HTTP through the host (allowlists, proxy, credential injection, optional TLS fingerprint) |
| `fs`, `kv`, `secrets`, `timers`, `bus`, `services`, `log`, `host-info` | state, storage, timers, cross-extension messaging |

The authoritative description, including permissions and manifest fields, is
[dev/v1/extension-api.md](dev/v1/extension-api.md); differences in the current
implementation are in [dev/v1/deviations/wit-v1.md](dev/v1/deviations/wit-v1.md).
Validate the WIT with `cargo xtask wit`.

## Guest SDK

`extensions/sdk/oino-extension` wraps the bindings in idiomatic Rust (`Extension` trait,
`export_extension!`, typed event payloads from `oino-ai`). `unipi-kit` adds shared helpers
used by the unipi ports (settings schema, bus topics, work tray, jev decisions).

## Known gaps (alpha)

- Extensions cannot write settings yet (no `set-extension-setting`); unipi ports keep such
  state in their state directory.
- Several `control` functions are not served by the core bridge yet: `run-state`, `shutdown`,
  `system-prompt-options`, `new-session`, `fork`, `switch-session`, `reload`,
  `running-bash`, `bash-detach`. They return `unsupported`.
