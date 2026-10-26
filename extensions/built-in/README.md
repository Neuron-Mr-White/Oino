# Built-in extension packages (legacy location)

Empty. The pre-v1 provider packages were ported to `extensions/providers/` (WIT
`oino:extension@1.0.0`, work item `providers-v1`); all other pre-v1 built-ins were removed and
unipi ports replace them (`extensions/unipi/`). v1 components are built with `cargo xtask wasm`
and bundled through `crates/oino-bundle` — nothing needs installing. This directory remains
only because CI and `scripts/install.sh` still reference it (integration follow-up).
