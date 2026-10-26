# Deviations: unipi-context

Settled rows #1–#23 are folded into [unipi-port.md](../unipi-port.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

| # | To | Request |
|---|---|---|
| R1 | core-runtime / wit-v1 | `models.summarize-default` with a previous summary and split-turn prefix (compactor `llm`). |
| R2 | — | Optional: rewrite legacy keys (`utility.skills` → `skills.exposure`, compactor keys) instead of translating on read. |
| R3 | unipi-core | `SkillConditionReply { hidden, reason? }` in `unipi-kit::services`. |
| — | tests | MemPalace against a real install. |
