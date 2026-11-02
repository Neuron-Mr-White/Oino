# Unipi

Unipi is the oino coding agent as a desktop (and phone) app: sessions across projects, a
virtualised chat timeline with markdown and tool rows, approvals, extension panels (including
a native kanboard board), a side pane with review / files / plan / browser, real terminals,
a command palette and a full settings screen — the unipi feature set (workflow permissions,
memory, subagents, fusion second model, cron, notify, watchdog, …) on top of the oino core.

**Unipi is built on [oino](../../README.md) end to end** — treat it as the reference
application for the oino SDK:

- **Host = oino-core embedded.** The daemon `unipi-host` (`crates/unipi-host`) embeds
  `oino-core` (agent loop, sessions v3, tools, settings) plus the Unipi services (wire
  protocol, cron scheduler, kanboard, browser control). The Tauri app talks to it over the
  protocol in `crates/unipi-proto` / `packages/protocol`.
- **Extensions = oino WASM components.** Every Unipi feature is a `wasm32-wasip2` component
  under [`extensions/unipi/`](../../extensions/unipi) (`unipi.core`, `unipi.workflow`,
  `unipi.memory`, `unipi.subagents`, `unipi.kanboard`, `unipi.browser`, `unipi.notify`, …)
  written against the [`oino-extension`](../../extensions/sdk/oino-extension) guest SDK and
  loaded by `oino-wasm-host` — the same runtime the `oino` CLI uses.
- **Frontend = Tauri + SolidJS** (`src/`, `src-tauri/`), with a shared protocol package
  (`packages/protocol`) generated from the same schema as the host's wire types.

## Build and run

Prerequisites, the dev/test identity (never collide with an installed release — identifier
`dev.unipi.app.dev`), bundle targets (AppImage/deb, dmg, NSIS, Android apk) and release
layout live in [BUILDING.md](BUILDING.md). The short version from the repo root:

```bash
cargo xtask wasm                              # build the extension components into extensions/dist
mise run unipi:dev                           # = scripts/unipi-dev.sh: dev app against a local host
mise run unipi:build                         # = scripts/unipi-build.sh: local release bundle
```

or by hand in `apps/unipi`: `npm ci`, then `cargo tauri dev --config tauri.dev.conf.json`
(sidecar binaries via `src-tauri/unipi-host-sidecar.sh`).

Frontend only: `npm test` (vitest) and `npm run typecheck` (tsc); cargo gates for the
workspace: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` (all inside `apps/unipi`).

## Reading the code

Good entry points, in rough order:

| What | Where |
|---|---|
| How a chat turn reaches oino-core | `crates/unipi-host/src/` (host services; the oino-core embed) and `crates/unipi-proto/src/` (wire events) |
| The frontend state machine | `src/lib/chat/` (`store.ts` applies host events, `model.ts` builds rows) |
| Extension panels / kanboard | `src/lib/board/`, plus `extensions/unipi/kanboard/` |
| A full oino guest extension | `extensions/unipi/core/` (hints, `/answer`, app bridge) or `extensions/unipi/notify/` (small) |
| Embedding oino-core yourself | [docs/sdk.md](../../docs/sdk.md) and `crates/oino/src/startup.rs` in the SDK |

For what each bundled extension does, see [docs/unipi.md](../../docs/unipi.md).
