# Deviations: modes

Owner: `modes` (`crates/oino`, `crates/oino-client`, `crates/oino-rpc`). Settled rows are folded
into [modes.md](../modes.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2);
row #5 into [architecture.md](../architecture.md#appendix-settled-deviations-folded-at-100-alpha2).

## Open

| # | Item | Notes |
|---|---|---|
| 11 | Shutdown | No mode exits on `AgentSessionRuntime::shutdown_requested()`; RPC does not refresh model catalogs in the background. |
| 13 | Subcommands | `--remote`, `auth`, `mcp`, `serve`, `ext`, `sessions` are unavailable. |
| 17 | `--offline` | `OINO_OFFLINE` is not exported to the process environment (edition 2024 `set_var` is `unsafe`). |
| 19 | Tool context | Built-in tools keep the first session's `SessionToolContext` after `new_session` / `fork` / `switch_session`. |
| 20 | `oino serve` | TCP+TLS transport, hub, pairing, multi-client — not implemented (blocks `--remote`, mobile remote attach). |
| R | Remote TUI | `_oino/list_sessions`, `_oino/get_keybindings`, `_oino/render`, `_oino/topics` (desktop StatusPanel), `_oino/agents` (live child transcripts). |
