# Deviations: unipi-agents

Settled rows #1–#24 are folded into [unipi-port.md](../unipi-port.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

| # | To | Request |
|---|---|---|
| R1 | wit-v1 | Task notifications with `deliverAs: followUp` and the service's `details`. |
| R2 | wit-v1 + core-runtime | `agents.agent.set-ui-policy` (subagents Ctrl+B, fusion non-blocking handoffs). |
| R3 | wit-v1 | Nesting depth / max depth on `call-context`. |
| R4 | core-runtime | Persist the startup model from an extension (`control.set-model(…, persist)`). |
| R5 | frontends | Render `code{language: "mermaid"}` natively. |
| — | core-runtime | Bash detach (watchdog kill, background-tasks adopt); process stats beyond Linux. |
| — | tests | Long-horizon graph runs end to end (goal continuation is verified). |
