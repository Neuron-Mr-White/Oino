# pi oracle: core (WP-C2)

`session-load.mjs` — pi 1.4.0 parity gate for the tool-dispatch record and the resume
repair (core spec §3): loads `fixtures/resume-repair.jsonl` (produced by oino's
`repair_interrupted_turn`, blessed from `crates/oino-session/tests/repair.rs` with
`OINO_BLESS=1 cargo test -p oino-session --test repair pi_oracle`) with pi's own
`SessionManager` and asserts entry counts, an unchanged `buildSessionContext` vs the
custom-entry-free twin, and torn-tail tolerance.

```bash
node scripts/pi-oracle/core/session-load.mjs   # PI_CODING_AGENT_PKG overrides pi
```
