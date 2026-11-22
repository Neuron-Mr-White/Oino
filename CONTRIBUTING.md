# Contributing to oino

Thanks for considering a contribution! oino is a Rust coding-agent SDK: the agent core,
WASM extension runtime, provider components, and the Unipi distribution app on top.

## Getting started

- Rust per [`rust-toolchain.toml`](rust-toolchain.toml) (1.99.0), plus
  `rustup target add wasm32-wasip2` for the extension components.
- Node ≥ 22 + npm for the Unipi frontend (`apps/unipi`).
- [mise](https://mise.jdx.dev/) users: `mise tasks` lists the common tasks
  (`mise run dev` runs the TUI, `mise run quality` is the standard gate).

## Layout

| Path | What |
|---|---|
| `crates/` | The SDK crates (root cargo workspace, plus `xtask`) |
| `extensions/` | WASM component workspace: `sdk/` (guest SDKs), `providers/`, `unipi/`, `fixtures/` |
| `apps/unipi/` | The Unipi distribution: host crates, Tauri app, protocol packages (own workspaces) |
| `wit/` | The `oino:extension@2.0.0` WIT world |
| `docs/` | User guides (rendered on oino.dev) |
| `scripts/` | CI / release / install helpers |

## Workflow

1. Fork, branch from `main`, keep commits focused.
2. Build and test what you touched — at minimum:

   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
   cargo test --workspace
   ```

   For extension code, also (inside `extensions/`):
   `cargo clippy --target wasm32-wasip2 --workspace --all-targets -- -D warnings` and
   `cargo test --workspace`, then `cargo xtask wasm` from the root and commit the refreshed
   `extensions/dist/checksums.json`. For the app, the same three cargo gates inside
   `apps/unipi` plus `npm test && npm run typecheck`.
3. Docs: user-facing changes update `docs/*.md` (`scripts/docs-check.sh` validates the set
   and links). New public API items need rustdoc (the workspace lints enforce
   `missing_docs`).
4. No `unwrap`/`expect`/`panic!` outside tests (clippy denies them); typed errors via
   `thiserror`; keep the TUI a deterministic state machine.
5. Open a PR describing what changed and how you verified it.

## Notes

- Tests must not need network or real credentials; providers are covered by replay
  fixtures, the agent loop by faux providers.
- Committing `extensions/dist/checksums.json` keeps release tarballs reproducible —
  CI fails if it drifts from a rebuild.
- Releases are cut by maintainers via the `v*` tag workflow (`.github/workflows/release.yml`).

License: MIT OR Apache-2.0 — contributions under the same terms.
