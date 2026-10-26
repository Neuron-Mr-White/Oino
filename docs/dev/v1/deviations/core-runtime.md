# Deviations: core-runtime

Owner: `core-runtime`. Format per README §5: section, change, reason, impact. pi 1.4.0 behaviour
won where runtime.md abbreviated or contradicted pi without listing the difference in §16; every
pi-parity choice is covered by a golden test against the pi oracle (`scripts/pi-oracle/core-runtime/`).

## Contract differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | runtime §1.2 | `StreamFn::stream(StreamRequest{model, context, options, api_key, cancel})`; `onPayload`/`onResponse`/`onProviderStreamEvent` are not loop config fields. | In Oino the provider hooks run between core and the provider component (provider-host), not in the loop. | wit-v1: route `provider-host` hooks to the session runner (`emit_chain` exists for `before_provider_request`/`_headers`). |
| 2 | runtime §1.1 | Blocked tool default text is pi's `Tool execution was blocked` (spec: `Tool call blocked`). | pi `prepareToolCall`. | none. |
| 3 | runtime §1.1/§7.2 | `BeforeToolCallResult.input` (Oino) replaces validated arguments; pi mutates `args` in place. | WASM handlers return values. | none. |
| 4 | runtime §1.1 | Validation messages follow TypeBox (golden-tested, 80 cases); the `format` keyword is not checked. | No TypeBox in Rust. | Tools with `format` validate less strictly. |
| 5 | runtime §2 | `AgentSession::prompt` returns `Started` once the run is prepared (`before_agent_start` done); the run continues in a task — await `wait_for_idle()` for completion. pi awaits the whole run (RPC uses `preflightResult`). | RPC/ACP need the disposition before completion. | modes: respond on return, stream events. |
| 6 | runtime §2 | A prompt or triggering `send_custom_message` made while `agent_settled` handlers run is deferred until they finish and reports `Queued`. | pi `_deferredSettledActions` returns `void`. | none. |
| 7 | runtime §7.2 | `tool_call` handler errors block with `Extension error: {message}` (spec); pi blocks with the raw message. | Spec text explicit. | none. |
| 8 | runtime §7.2 | A `message_end` replacement replaces the agent transcript's last message and the persisted entry; the loop's local copy (tool calls of that turn) is not changed. pi mutates the object in place. | No shared mutable objects across the WASM boundary. | Extensions rewriting tool calls at `message_end` do not change execution. |
| 9 | runtime §4/§9.7 | `Services::create` resolves trust from `-a/-na`, `trust.json` and `defaultProjectTrust` (`ask` ⇒ untrusted); the `project_trust` extension event is not dispatched yet. | Extensions load after trust (they include trust-gated `.oino/extensions`). | Frontends ask interactively before `Services::create` and pass `approve`. |
| 10 | runtime §4 | Extensions are loaded through `services::ExtensionLoader::load(host, project_trusted)`; core does no manifest/package discovery. | Discovery and manifests belong to the WASM host and package manager. | wit-v1/integration implement the loader. |
| 11 | runtime §7 | `oino-extension-api` and `oino-protocol` do not exist yet: the instance contract, host operations, client events and UI types live in `oino_core::{extensions::{api, host}, events, ui}` with the spec'd JSON shapes. Host calls are one `HostApi::call(ext, cx, HostOp)` enum instead of one trait method per WIT function. | Contract crates not started (core-types-session handoff). | core-types-session / wit-v1 move them; core switches to re-exports. |
| 12 | runtime §8 (tools) | Tool-result image normalisation (`normalizeToolResultImages`) and prompt image processing (`processImage`) are not done by core; `oino-tools` `read` resizes its own images. | Deferred. | Extension tools returning large images are passed through. |
| 13 | runtime §14 | `export_jsonl` resolves a relative path against the session cwd (pi: process cwd). | Libraries do not read the process cwd. | modes pass absolute paths. |
| 14 | runtime §6.1 tests | Golden session-event comparison ignores `message_update.message` (pi's shallow copy keeps mutating after the event) and the content of the first assistant partial. | pi artefacts, not behaviour; the wire form drops them (modes §3.1). | none. |

Subcrate deviations (reported by the implementers, also binding here):

- **oino-settings / oino-resources:** CLI `--append-system-prompt` values replace the discovered
  `APPEND_SYSTEM.md` (pi), not combined (§9.3). `mcp.json` is not a protected resource (§16.6).
  Invalid enum values fall back to defaults. Locks use std `File::lock` on `<file>.lock`.
- **oino-models:** credential order is pi's: `--api-key` > `auth.json` > `models.json`/override
  `apiKey` > registration `authEnv` (§13 lists env before `apiKey`). A model is available only
  when a loaded component implements its API. Router state equal to the stored state is not
  re-stored. OAuth refresh has no built-in timeout (host enforces it).
- **compaction:** summary requests set no `maxTokens` (pi 1.4.0, issue #9512); prompt-cache
  writes disabled via `cacheRetention: none`; pi's provider `env` option is dropped.

## Deferred (not in this pass)

Cache warming (§14), event bus (§7.5), turn arbiter (§7.6), nested agents (§12), services and
secrets (§7.4/§5.9), extension settings registry (§10.3), package manager (§15), provenance (§2),
command output capture (§7.4; command echo is implemented), bash detach wiring (§8, the
`oino-tools` registry exists), `before_provider_request`/`after_provider_response`/
`provider_stream_event` routing, `mcp_servers_change` (never emitted by design), virtual-model
`route` for summaries uses `reason: direct` only.
