# oino-acp

ACP v1 (Agent Client Protocol) agent adapter over `oino-core` sessions.

Bridges ACP clients (editors, IDEs) to Oino: `session/new`, `session/prompt`, tool events,
permissions and modes, plus the Oino RPC passthrough (`_oino/rpc`, `_oino/<command>`).
