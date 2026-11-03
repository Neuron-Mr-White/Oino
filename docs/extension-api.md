# Extension API and WIT

The contract between Oino and extensions is the WIT package `oino:extension@2.0.0` in
`wit/oino-extension/` (component model async on wasmtime 48, wit-bindgen 0.62). Data-model
values (messages, models, events, tool results) cross the boundary as canonical JSON with
exact wire shapes; host capabilities are typed WIT.

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

The authoritative description, including permissions and manifest fields, is the WIT world
itself (`wit/oino/extension.wit`); validate it after changes with `cargo xtask wit`.

## Guest SDK

`extensions/sdk/oino-extension` wraps the bindings in idiomatic Rust (`Extension` trait,
`export_extension!`, typed event payloads from `oino-ai`). `unipi-kit` adds shared helpers
used by the unipi ports (settings schema, bus topics, work tray, jev decisions).

## Bounded hook payloads

An extension's linear memory never shrinks, so event payloads that grow with the session
are bounded. `session_before_compact` carries pi's `preparation` and `branchEntries`
unchanged up to 4 MiB each. Above that the host drops `details` objects larger than
16 KiB and image bytes, and clips strings in entries that are still larger than 512 KiB. If
the payload is still too big, `branchEntries` keeps only the newest entries that fit, so
the live context goes in before older history, and `branchEntriesOmitted` says how many
older entries were left out. Likewise the oldest `preparation.messagesToSummarize` are dropped and counted in
`messagesToSummarizeOmitted`. To read more of the session, page it with `session::query`
(WIT `session.query-entries`).

## Known gaps

- `control.shutdown` is recorded but no mode exits on it yet.
- `models.classify` and `models.generate-image` are unsupported.
