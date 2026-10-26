# Deviations: acp

Owner: `acp` (`crates/oino-acp`). Settled rows #1–#16 are folded into
[modes.md](../modes.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

| # | Item | Notes |
|---|---|---|
| 2 | `mcpServers` | Not published on bus topic `oino:acp:mcp-servers` (core has no frontend bus API); unipi.mcp already consumes the topic (unipi-integrations R3). |
| 3 | Refusals | `stopReason: "refusal"` is never produced. |
| 7 | Tool kinds | Extension tools are `other` (no `ToolInfo.kind`). |
| 11 | Forms | `form`, `custom`, `clipboardReadImage`, `editExternally` resolve cancelled over ACP. |
| 16 | Interop | Zed has not been tried by hand. |
