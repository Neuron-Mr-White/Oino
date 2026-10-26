# Oino agent conventions

Read these first:

- `docs/dev/v1/CHARTER.md` — vision, settled decisions, quality gates, working rules.
- `docs/dev/v1/README.md` — spec index, decision register, ownership map.
- `docs/dev/v1/quality.md` — gates, test layers, golden tests against pi 1.4.0.
- `docs/dev/v1/STATUS.md` — what is done, partial or missing, and next steps.

## Workflow

- Search project memory before deciding anything that past work may have settled.
- Work on branch `v1-alpha`; stage explicit paths (`git add <paths>`), never `git add -A`;
  prefix commits with the work id (`[core-runtime] …`).
- Spec changes go to `docs/dev/v1/deviations/<work-id>.md`, not into the frozen specs.

## Gates (run for the workspaces you touch)

```sh
cargo xtask wasm                       # components into extensions/dist (bundled by oino-bundle)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
(cd extensions && cargo fmt --all --check && cargo clippy --target wasm32-wasip2 --workspace --all-targets -- -D warnings && cargo test --workspace)
(cd apps && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)
```

## Machine notes

- `/tmp` hits its quota on the dev machine: export `TMPDIR=$PWD/target/tmp` for builds and
  tests; tests create temp dirs under `CARGO_TARGET_TMPDIR` / `target/tmp`.
- Memory is tight: use `-j 2`–`-j 4` for cargo, `-j 1`/`-j 2` in `apps/`.
- Tests that spawn `oino` set `OINO_BUNDLED=0` so the bundled unipi components do not change
  their output.

## Code

- Follow `/home/oi/.agents/skills/rust-best-practices/SKILL.md`: typed errors (`thiserror`),
  no `unwrap`/`expect`/`panic!` outside tests, rustdoc on public items, crate docs with an API
  map, `#[allow(..., reason = "…")]` only with a reason.
- Interactive fuzzy matching uses `nucleo` (see `crates/oino-view/src/fuzzy.rs`).
- Keep the TUI a deterministic state machine; no blocking work in render.
