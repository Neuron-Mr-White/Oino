# Deviations: unipi-ux

Settled rows #1–#13 are folded into [unipi-port.md](../unipi-port.md#appendix-settled-deviations-folded-at-100-alpha2); the protocol additions they use are listed in
[ui-protocol.md](../ui-protocol.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

| # | To | Request |
|---|---|---|
| Q2 | frontends | `ui.editor-history-push` (input-shortcuts "↑ restores"). |
| Q1, Q4, Q5 | TUI | Verify the TUI honours `replaces`, the `form` / `custom` hints and the footer segment fields (the desktop does). |
| Q6 | core-runtime | Model `subscription` flag for the `sub` cost label. |
| Q7 | core-runtime / wit-v1 | Guest read of `_oino/extensions` (load times, registration counts). |
| Q9 | unipi-integrations | notify: listen to `unipi:ask-user:prompt` for re-notify. |
