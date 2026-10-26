# Deviations: unipi-kanboard

Settled rows K1–K12 and E1–E16 are folded into [unipi-port.md](../unipi-port.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

| # | To | Request |
|---|---|---|
| RK1 | core-runtime | Put the Oino install directory on the program search path for extension `process.exec` / spawn (installers already ship `oino-kanboard` next to `oino`). |
| RK2 | tests | Monitor nudges through the turn arbiter against a live `oino-kanboard` board (unit-level arbiter tests exist). |
| RK5 | TUI | Render the board `sidePane` (horizontal `split` of lane lists, `select` actions); the desktop has a native board. |
