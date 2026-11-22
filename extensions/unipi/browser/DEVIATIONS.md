# unipi.browser — deviations from browser-control.md

| # | Spec | Implemented | Why |
|---|---|---|---|
| 1 | §11.1 ambient browser context (hidden message when the user's visible tab changed) | not yet | the host does not report viewer visibility per tab to guests yet |
| 2 | §8.3 `oino_browser` guest events (dynamic tool (un)registration on status change) | tools register at `init` and re-check on `session_start` | no `browser` event kind in the registry yet |
| 3 | §5.1 token-budget test with a cl100k estimate | `chars / 4` estimate in the unit test | no tokenizer in the wasm guest workspace |
| 4 | v2 ext-abi §7: `browser.wit` transport | `host-service.call("browser", op, command)` via `crate::execute`/`crate::status`; image rides base64 inside the reply JSON; `no_host` reason under a plain oino | generic host-service ABI; scope resolution is host-side (unipi-host) |
