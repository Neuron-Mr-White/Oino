# Deviations: unipi-core

Settled rows #1–#14 (kit, `unipi.core`) are folded into [unipi-port.md](../unipi-port.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2). `unipi.utility`
rows live in `extensions/unipi/utility/DEVIATIONS.md` and are folded there too.

## Open

| # | To | Request |
|---|---|---|
| R3 | wit-v1 | `host-info` local UTC offset (late-night hint, notify quiet hours, info-screen days). |
| R5 | wit-v1 | `testing::MockHost` or `export_extension!` gated on `wasm32` in the SDK. |
| U1–U2 | core-runtime + frontends | `tools.override` render-only registration; call guest `render` for built-in tools (utility render styles). |
| U3 | frontends | `editor` dialog `cursorLine` + Tab to the next `A<n>:` line. |
| U4 | frontends + host | Attachments from arbitrary folders (frontend file read or scoped host read). |
| U5–U6 | core-runtime | `control.move-project-state` (trust.json + sessions dir); tmp GC. |
