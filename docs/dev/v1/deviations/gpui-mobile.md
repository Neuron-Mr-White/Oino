# Deviations: gpui-mobile

Settled rows are folded into [gpui-app.md](../gpui-app.md#appendix-settled-deviations-folded-at-100-alpha2)
(G6–G9, G11; 1.0.0-alpha.2). Screen parity: `apps/oino-mobile/DESIGN-STATUS.md`.

## Open

- iOS devices: no JIT → wasmtime Pulley needed (wit-v1 O5); device builds need `OINO_IOS_TEAM`.
- Remote attach to a desktop (`oino serve`, RPC transport, keystore tokens).
- Tablet / landscape layout; 120 Hz scroll and input-to-paint not measured.
- Move `EmbeddedHost` / `DemoHost` from `oino-desktop` into a shared crate (drop the `#[path]` includes).
- Release APK is signed with the debug key.
