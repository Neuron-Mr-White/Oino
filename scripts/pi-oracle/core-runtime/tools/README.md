# oino-tools pi oracle

Runs pi 1.4.0's real built-in tool definitions (`createToolDefinition(name, cwd, options)`
from `$PI/dist/core/tools/index.js`) and records goldens for `crates/oino-tools`.

```bash
cd scripts/pi-oracle/core-runtime/tools
node meta.mjs > ../../../../crates/oino-tools/tests/fixtures/meta.json   # names, schemas, prompt text
node run.mjs  > ../../../../crates/oino-tools/tests/fixtures/tools.json  # cases.json results
```

- `cases.json` is shared with `crates/oino-tools/tests/golden_tools.rs`: each case lists the
  tool, arguments, fixture files (text, base64, generated lines/repeats, images), context
  (cwd, model image capabilities, session) and options.
- Case directories live under `target/oino-tools/oracle/` (inside the repository, so
  `.gitignore` handling matches the Rust test directories). `rg`/`fd` must be available where
  pi finds them (`PATH` or `~/.pi/agent/bin`).
- Normalisation (both sides): case dir → `$CWD`, temp logs → `$TMPLOG`, wall time → `0`,
  image bytes → `$SOURCE` (the file's own bytes) / `$ENCODED` plus decoded `imageDims`,
  strings over 4096 UTF-16 units → `{ $long, head, tail, fnv }`.
- Small generated images (< 16KB) are stored in `materialized` so Rust reads pi's exact bytes.
