# WIT

`oino-extension/` is the package `oino:extension@1.0.0` (extension-api §3), one file per
interface plus `worlds.wit` (`extension`, `provider-extension`). `oino-extension/deps/` holds
WASI 0.2.12 (`cli`, `clocks`, `filesystem`, `io`, `random`, `sockets`; no `wasi:http`, D7).

Validate: `cargo xtask wit` (runs `wasm-tools component wit wit/oino-extension`).
