# Deviations: providers-v1

Settled rows #1–#14 are folded into
[extension-api.md](../extension-api.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).
providers-more differences (M1–M10) stay in `extensions/providers/README.md`.

## Open

- OpenAI / Anthropic WebSocket transports (`StreamOptions.transport` is ignored; SSE only).
- Anthropic workload-identity federation (`ANTHROPIC_FEDERATION_*`).
- OAuth flows (Anthropic, Copilot, Codex, ChatGPT) are tested with oracle fixtures and the faux
  provider only, not against live accounts.
- Commit the host e2e scripts (`scripts/pi-oracle/providers-{v1,more}/host-e2e.rs`) as
  `crates/oino-wasm-host/tests/providers.rs`.
