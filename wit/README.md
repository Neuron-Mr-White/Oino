# WIT

`oino-extension/` is the package `oino:extension@2.0.0` (extension-api §3 as amended by
[../docs/dev/v2/specs/ext-abi.md](../docs/dev/v2/specs/ext-abi.md) §8), one file per
interface plus `worlds.wit` (`extension`, `provider-extension`). `oino-extension/deps/` holds
WASI 0.2.12 (`cli`, `clocks`, `filesystem`, `io`, `random`, `sockets`; no `wasi:http`, D7).

## Compatibility statement (2.0.0)

- A 2.x host hosts **only** `oino:extension@2.x` components: there is no dual-version hosting
  and no in-place adapter. A 1.0.0 package fails at load with a diagnostic naming the required
  version (`component does not export oino:extension/guest@2.0.0 … rebuild against
  oino:extension@2.0.0`; the manifest `apiVersion` check says the same for `oino.extension/1`).
  Every component in this monorepo is rebuilt together, so no released artifact sees the
  intermediate state.
- Within 2.x the 1.x rule holds: additive record fields and new functions/interfaces are minor
  bumps; any removal or signature change is a major bump.
- Components that import a *subset* of the world keep loading — adding world imports is
  additive (relied on by `host-service`, ext-abi §3.4.2).
- WASI dependencies stay at 0.2.x.

Validate: `cargo xtask wit` (runs `wasm-tools component wit wit/oino-extension`).
