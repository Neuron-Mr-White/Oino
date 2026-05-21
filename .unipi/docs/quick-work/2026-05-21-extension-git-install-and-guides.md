---
title: "Extension Git Install and Guides"
type: quick-work
date: 2026-05-21
---

# Extension Git Install and Guides

## Task

Support installing extensions from Git/GitHub repositories in addition to local file paths, then write comprehensive user and developer documentation for creating, testing, installing, managing, and publishing extensions.

## Changes

- `crates/oino-app/src/main.rs`: added install source classification for local paths, GitHub shorthand, GitHub URLs, generic Git URLs, and `#branch-or-tag` refs; Git sources are cloned into a temporary checkout before package lifecycle install/update.
- `crates/oino-tui/src/app.rs` and `crates/oino-tui/src/render.rs`: updated `/extensions` install prompts/status text to advertise path/GitHub sources.
- `docs/extension-kernel/user-guide.md`: added end-user install/manage/update/uninstall/troubleshooting guide.
- `docs/extension-kernel/developer-guide.md`: added authoring, manifest, contribution, testing, GitHub publishing, install, update, and security guide.
- `README.md`, `docs/extension-kernel/README.md`, `docs/extension-sdk/README.md`, `docs/extension-kernel/rollout-review.md`: linked the new guides and updated capability status.

## Verification

- `cargo test -p oino-app extension_install_source_accepts_local_paths_and_git_sources --no-fail-fast`
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `mise run quality`

## Notes

Git installs require `git` on `PATH`. Repositories must contain `oino.package.json` at the repository root. A `#ref` suffix maps to `git clone --branch` and is intended for branch or tag names.
