# Deviations: core-types-session

Settled rows #1–#15 and the additive API are folded into
[session-v3.md](../session-v3.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

- `crates/oino-protocol` was never created (client events, RPC records, UI protocol/OUS,
  extension event payloads). Wire types live in `oino-core` and `oino-client::protocol`
  (architecture appendix N1). Moving them is a mechanical follow-up.
- Criterion benches (quality §5: `open` 10k < 60 ms, `build_session_context` 10k < 15 ms).
- W3: the pi oracle (`scripts/pi-oracle/core-types-session/`) imports pi from the global mise
  install (`PI_CODING_AGENT_PKG` overrides), not from a version-locked `scripts/pi-oracle/node_modules`.
