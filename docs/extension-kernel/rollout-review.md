# Extension Kernel Rollout and Review Handoff

## Current state

The extension-kernel roadmap branch implements the kernel contracts, registry composition, built-in migration, extension manager snapshots, safe mode, JSON-v1 runtime boundary, capability broker, tool/command bridge, UI surface contracts/rendering, many-extension management UX, persistence/session APIs, package layouts, package lifecycle service, community registry fixture metadata, author SDK/devkit, examples, and parity coverage gates.

## Compatibility guarantees for this branch

- Contracts are ready for review, tests, and fixture authoring, but should still be considered pre-stable until maintainers accept the branch.
- Oino will support semantic parity with selected Pi extension capabilities, not Pi API compatibility.
- Oino-owned manifests and paths are the compatibility boundary: `oino.extension.json`, `oino.package.json`, and `.oino`/`~/.oino` layouts.
- Runtime capabilities must remain permission-gated and audited even after install-time approval.

## Migration notes

- Existing built-in Oino tools and commands are still available through the same user-facing defaults. Built-ins are now represented internally as registry contributions.
- Existing `~/.oino/settings.json` support now includes extension policy settings; project settings can override global settings.
- Oino does not auto-import Pi `.pi`, Claude, or generic agent extension paths. Future migration tooling should copy or translate into Oino-owned layouts.
- External extensions must be explicitly discovered from Oino paths and enabled by policy before becoming active.

## Rollout risks

- **Contract lock-in:** manifest fields, permissions, registry snapshots, and `wasm-json-v1` become author-facing; changes after merge need migration notes.
- **Security:** community registry publication should remain fixture/local until signature authority, advisory distribution, and takedown operations are reviewed.
- **Runtime execution:** fixture runtime support is deterministic and testable; production WASM host hardening remains a follow-up before encouraging untrusted community code.
- **UX visibility:** `/extensions` now supports local package install/uninstall and enablement toggles, but hosted registry browsing/publishing and richer permission-prompt UX remain follow-ups.
- **SDK drift:** future TypeScript/JavaScript, Go, and Python SDKs must be generated from or tested against the Rust contracts.

## Follow-up issues

1. Build production WASM host execution around the JSON-v1 contract and capability broker.
2. Add a main-binary CLI (`oino extensions install/list/remove/update`) mirroring the `/extensions` panel flow.
3. Add hosted registry client/publishing workflow only after trust/signing/advisory policy review.
4. Add schema export for multi-language SDKs and keep parity tests as gates.
5. Add migration/import tooling for users who want to translate existing Pi-like assets into Oino layouts.
6. Expand permission UX for high-risk provider, network, shell/process, filesystem, and secret access.

## Review checklist

- Confirm plan statuses and work notes match commits.
- Run `mise run quality` in `.unipi/worktrees/feat/extension-kernel-roadmap`.
- Dogfood management UI with `OPENROUTER_API_KEY=sk-or-... mise run dev`, then open `/extensions`.
- Inspect example package with `cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture`.
- Review docs: `docs/extension-kernel/README.md`, `docs/extension-sdk/README.md`, `.unipi/docs/adr/*extension*`, and `.unipi/docs/research/2026-05-21-oino-pi-extension-parity-matrix.md`.
