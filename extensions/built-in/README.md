# Built-in extension packages (legacy)

Pre-v1 provider packages kept until `providers-v1` ports them to `extensions/providers/`
(WIT `oino:extension@1.0.0`). All other pre-v1 built-ins were removed; unipi ports replace
them (`extensions/unipi/`). v1 components are built with `cargo xtask wasm` and bundled
through `crates/oino-bundle` — nothing needs installing.
