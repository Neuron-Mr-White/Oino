# pi oracle: core-types-session

Fixture generators for `crates/oino-session/tests/fixtures/` (pi-coding-agent 1.4.0, Node 24).

| Script | Output |
|---|---|
| `generate.mjs <dir>` | 34 synthetic v3 sessions built with pi's own `SessionManager` |
| `scrub.mjs <dir> name=file…` | real `~/.pi/agent/sessions` files with private strings replaced (`real-sessions.txt`) |
| `scrub-legacy.mjs <dir> name=file…` | pre-v1 Oino files from `~/.oino/sessions`, scrubbed |
| `context.mjs <dir>` | `<name>.expected.json` per `<name>.jsonl`: tree, labels, name, and for every leaf the branch, context entries, projection and `buildSessionContext` |
| `regen.sh` | regenerates `fixtures/pi/*` (set `SCRUB_REAL=1` to re-scrub real sessions) |

Legacy-migration goldens are blessed from Rust (`OINO_BLESS=1 cargo test -p oino-session --test
legacy_migration`), then `node context.mjs crates/oino-session/tests/fixtures/oino-legacy/expected`
records how pi reads the migrated files. `PI_CODING_AGENT_PKG` overrides the pi package root.
