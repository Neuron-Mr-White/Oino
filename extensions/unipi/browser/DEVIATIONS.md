# oino.browser — deviations from browser-control.md

| # | Spec | Implemented | Why |
|---|---|---|---|
| 1 | §11.1 ambient browser context (hidden message when the user's visible tab changed) | not yet | the host does not report viewer visibility per tab to guests yet |
| 2 | §8.3 `oino_browser` guest events (dynamic tool (un)registration on status change) | tools register at `init` and re-check on `session_start` | no `browser` event kind in the registry yet |
| 3 | §5.1 token-budget test with a cl100k estimate | `chars / 4` estimate in the unit test | no tokenizer in the wasm guest workspace |
