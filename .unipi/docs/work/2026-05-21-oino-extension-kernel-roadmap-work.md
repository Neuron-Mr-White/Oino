# Oino Extension Kernel Roadmap — Work Notes

## 2026-05-21 — Task 1 baseline

Worktree/branch:

- Path: `/home/pi/project/oino/.unipi/worktrees/feat/extension-kernel-roadmap`
- Branch: `feat/extension-kernel-roadmap`
- Starting commit: `f81995a` (`docs: plan Oino extension kernel roadmap`)
- Starting status: clean

Baseline crate graph:

- `crates/oino-types`
- `crates/oino-agent-loop`
- `crates/oino-agent`
- `crates/oino-session`
- `crates/oino-harness`
- `crates/oino-resource`
- `crates/oino-env`
- `crates/oino-tools`
- `crates/oino-auth`
- `crates/oino-provider-openrouter`
- `crates/oino-tui`
- `crates/oino-app`

Baseline validation:

- `cargo fmt --all --check` — passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed
- `cargo test --workspace` — passed

No pre-existing validation failures were observed before extension-kernel changes.
