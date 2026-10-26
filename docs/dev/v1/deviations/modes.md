# Deviations: modes

Owner: `modes` (`crates/oino`, `crates/oino-client`, `crates/oino-rpc`). Format per README §5:
section, change, reason, impact. pi 1.4.0 (`dist/main.js`, `cli/args.js`, `modes/print-mode.js`,
`modes/json-event.js`, `modes/rpc/*.js`) is the reference where modes.md is silent.

## Contract differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | modes §1 ("clap derive") | The CLI parser is a rule-for-rule port of pi `parseArgs` (hand written), not clap. | pi semantics clap cannot express: `-p` takes an optional message unless it starts with `-`/`@` (`---x` allowed), multi-letter short flags (`-nbt`, `-xt`, `-ne`), unknown long flags kept for extensions with "value unless next starts with `-`/`@`", `--name` without value is an error but other value flags without value are silently ignored. | none (same flags). |
| 2 | modes §1.1 | `--mode` errors list `acp` (`Invalid mode "x". Valid values: text, json, rpc, acp`, `--mode requires text, json, rpc, or acp`). | Oino adds `acp`. | Golden texts differ from pi there. |
| 3 | D13 / modes §1.1 | Help text: product substitutions plus pi-ai provider env vars, `mcp` and codemode lines removed; `serve`, `--remote`, `--agent-dir`, `OINO_*` variables added. | Providers are extensions; no core MCP. | Help is not byte-identical to pi's. |
| 4 | architecture §2, §4.2 | RPC records (`RpcCommand` = `PiCommand` ∪ `OinoCommand`, `RpcRequest`, `RpcResponse`, `RpcSessionState`) live in `oino_client::protocol`; client events and UI records come from `oino_core::{events, ui}`. | `oino-protocol` does not exist yet. | core-types-session: move them into `oino-protocol::rpc`; `oino-client` re-exports. |
| 5 | architecture §2 | `RpcBackend` (and its `EventDecoder`) live in `oino-rpc`, not `oino-client` behind feature `rpc`. | `oino-rpc` depends on `oino-client` (trait); the reverse optional edge would be a dependency cycle, which cargo rejects even for optional deps. | Frontends depend on `oino-rpc` for remote sessions. |
| 6 | architecture §4.2 | `SessionBackend` adds `send_with_id(request_id, command)` (default: `send`) and `shutdown()`. `send` returns the reply `data` as JSON (`RpcResponseData = Option<JsonValue>`); typed results are on `RpcClient` methods. | pi passes the RPC id to `executeBash` (`bash_execution_update.id`); RPC stdin EOF must dispose the runtime; one typed enum per command reply would duplicate core types. | tui-client/acp/gpui use `send` + `serde_json::from_value`, or `RpcClient`. |
| 7 | architecture §4.2 | `InProcessBackend::events()` subscribes to the **session** hub and follows replacements made through the backend (`new_session`, `switch_session`, `fork`, `clone`, `_oino/import_jsonl`), announcing them with a synthesized `session_replaced`. | The runtime hub is one forwarding task behind the session hub, so a reply could overtake the events its command caused; session hubs are filled synchronously. | Events the new session publishes between the swap and the rebind (e.g. UI requests from `session_start` handlers) are not seen. Replacements started elsewhere are not followed (core exposes none yet). See requests. |
| 8 | modes §3.1 | Ordering: the RPC writer drains every queued event before writing a reply, so events a command caused precede its reply (pi writes them synchronously); a `prompt` reply precedes that run's `agent_start` (held at most 1 s). | pi parity on a multi-threaded runtime. | none. |
| 9 | modes §3.1 | Malformed JSON: `Failed to parse command: <serde_json message>`; bad fields: serde messages (`missing field \`modelId\``, `unknown variant …`). | No JavaScript `JSON.parse` / TypeError texts in Rust. | Golden tests compare the prefix only. |
| 10 | ui-protocol §10 | Clients without `_oino/hello`: Oino-only dialogs resolve as cancelled (no degradation to `select`/`input`), Oino fire-and-forget methods are dropped, `extensionId`/`sessionId` and `extension_error.stack` are stripped. | Degradation renderers (`OusNode::to_lines`, form → sequential selects) need `oino-protocol::ui`. | pi clients never see Oino dialogs. |
| 11 | modes §3.1 | `ctx.shutdown()` is not wired (core has no shutdown hook); RPC does not refresh model catalogs in the background (pi `modelRuntime.refresh`). | Core API missing. | Extensions cannot end an RPC session. |
| 12 | modes §2.1 | Print/JSON modes wait (≤ 5 s) for a started run's `agent_settled` to reach the output before disposing. | `AgentSessionRuntime::dispose` closes the runtime hub without draining the session forwarder (events in flight are lost). | none once core drains on dispose. |
| 13 | modes §1.1 | Not available yet (exit 1 with `Error: … is not available in this build`): interactive mode (tui-client), `--mode acp` (acp), `--remote`, subcommands `install/remove/uninstall/update/list/config/auth/mcp/serve/ext/sessions`. `--resume` (picker) errors. `--session <id>` found in another project errors with a hint to use `--fork` (pi asks interactively). `--session-id` of a missing session errors and `--fork` + `--session-id` errors (core `SessionSource` has no explicit id). Missing-session-cwd prompt is not implemented. | Owned by other items / core API missing. | see requests. |
| 14 | modes §1.1 | `-e <path>` component packages fail with `Failed to load extension "<path>": component files need the WASM extension host, which this build does not include yet`; package discovery (`~/.oino/extensions`, `.oino/extensions`, bundled) is not wired. Only `builtin:faux` and `builtin:ui-probe` exist, behind cargo feature `faux` (tests). | `oino-wasm-host` (`core_bridge::CoreLoader`) landed after this work was verified; wiring it is the next modes step. | Providers cannot be loaded from `extensions/dist` by the binary yet. |
| 15 | modes §2.1 | `@file` images are attached as read (base64), without pi `processImage`. | core deviation 12 (no image processing in core). | Large images are not resized. |
| 16 | modes §1.1 | `--list-models <search>` uses a case-insensitive subsequence match per token (pi-tui `fuzzyFilter` approximation); sorting is case-insensitive then byte order (pi `localeCompare`). | No pi-tui port. | Ordering of non-ASCII ids may differ. |
| 17 | modes §1.1 | `--offline` is accepted but `OINO_OFFLINE` is not exported to the process (pi sets `PI_OFFLINE=1`). | `std::env::set_var` is `unsafe` in edition 2024 and crates forbid unsafe. | Extensions read the CLI flag only. |
| 18 | modes §1.1 | pi re-applies `setThinkingLevel` after creation when `--thinking`/`:level` was given (persisting it); not done. | Core already clamps at creation. | `defaultThinkingLevel` is not saved by `--thinking`. |
| 19 | runtime §8 | Built-in tools keep the `SessionToolContext` of the first session (core binds it once), so after `new_session`/`fork`/… their session env vars (`OINO_SESSION_ID`, …) describe the first session. | Core API. | see requests. |
| 20 | modes §5 | `oino serve` (TCP+TLS, hub, pairing) and `Transport::Tcp` are not implemented. | Scope of this pass. | Mobile/remote frontends wait. |

## Requests for other work

- **core-runtime**: (a) `AgentSessionRuntime::dispose` should drain the session→runtime
  forwarders before closing the hub; (b) a runtime-level event stream (or a flush barrier) that
  is ordered with command completion, so `InProcessBackend` need not follow session hubs; (c)
  `SessionSource::New{id}` / fork with id for `--session-id`; (d) `SessionToolContext` rebinding
  on replacement; (e) host ops for command contexts `newSession/fork/switchSession/reload` and
  `shutdown` (pi `commandContextActions`, `shutdownHandler`) with a replacement notification the
  backend can follow; (f) RPC-visible `refresh` of dynamic model catalogs.
- **core-types-session**: `oino-protocol::{rpc, ui}` with the records in `oino_client::protocol`
  and the UI degradation helpers (ui-protocol §10).
- **modes (next step)**: compose `CliLoader` with `oino_wasm_host::core_bridge::CoreLoader`
  (`-e <package dir>` via `packages::load_dir`, discovery of `~/.oino/extensions` and trust-gated
  `.oino/extensions`, `oino-bundle` packages) and add an e2e test with `extensions/fixtures/provider-faux`.
- **integration**: root `Cargo.toml` members `crates/oino-client`, `crates/oino-rpc`, `crates/oino`
  were appended by this item (switch to globs when convenient).
- **tui-client / acp**: `SessionBackend` additions above; `FollowStream` semantics (#7).
