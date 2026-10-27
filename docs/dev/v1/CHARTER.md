# Oino v1.alpha Charter

Branch: `v1-alpha` (all v1 work lands here; never commit to `main`).
This file is the shared contract for every contributor and agent. Read it first.

## Vision

1. **Oino is another pi core.** `oino-core` is a Rust equivalent of the pi-coding-agent
   runtime (pi 1.4.0): agent loop, AgentSession, SessionManager, extension runner and
   extension API, built-in tools (`read`, `write`, `edit`, `bash`), resource loading
   (skills, prompt templates, AGENTS.md, SYSTEM.md, settings), compaction, branch
   summaries, context edits, virtual models, print / JSON / RPC modes. Same behaviour as
   pi wherever possible. Then ACP (Agent Client Protocol) on top of RPC.
2. **Everything else is a client or an extension.**
   - Providers are WASM extensions (OpenAI-compatible, OpenAI Responses, Anthropic
     Messages, Oino router). No provider code in core.
   - The TUI is a native Rust client crate/binary over `oino-core` (like pi-coding-agent's
     interactive mode sits on the runtime). Not a WASM plugin.
   - All of unipi 3.0.0 is ported to Rust WASM component extensions with full feature
     parity. Mechanisms follow how pi + unipi achieve them (events, tools, commands,
     subprocesses, subagents, timers, UI requests).
   - Extension UI is frontend-agnostic, modelled on pi's `rpc-extension-ui`: extensions
     emit structured UI requests/widgets; each frontend (TUI, GPUI) renders natively.
3. **GPUI frontend for all devices, speed first.** A desktop + mobile distro built with
   GPUI (`github.com/itsbalamurali/gpui-mobile`, `github.com/longbridge/gpui-kit` /
   gpui-component). UX learns from `github.com/zai-org/ZCode`, not from the unipi app.
   The desktop app bundles oino-core + provider extensions + all unipi extensions, so
   installing Oino Desktop is enough. macOS/Windows builds use hosts `coffee` (Windows,
   `ssh coffee`) and `desmondgiam` (macOS); phones are plugged in for mobile.

## Settled decisions

- Pin parity target: `@earendil-works/pi-coding-agent` 1.4.0. Reference sources:
  - Installed dist + docs: `~/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent/`
    (`docs/`, `dist/*.d.ts`, `dist/*.js`, `node_modules/@earendil-works/pi-agent-core`, `pi-ai`).
  - Upstream source may be cloned to `/tmp/pi-src` from `github.com/earendil-works/pi` (tag/commit for 1.4.0).
- Out of scope from pi: pi-tui component library, pi-ai provider implementations,
  pi-mcp, pi-codemode, pi-telemetry (MCP etc. may come later as extensions). Pi npm
  packages (TypeScript) are not loadable; markdown resources are.
- Unipi reference: `/home/oi/Projects/Personal/archived/unipi` (v3.0.0, `packages/*`,
  `crates/kanboard`). Port everything except the unipi desktop app UI.
- Session format: adopt pi session format **v3 exactly** (flat entries, `type`, 8-hex
  `id`, `parentId`, ISO timestamps, camelCase, pi `AgentMessage` shapes). Oino v1/v2 files
  migrate on load. Location stays `~/.oino/sessions/--<cwd>--/<ts>_<id>.jsonl`.
- Paths: `~/.oino/` (global) and `.oino/` (project), laid out like pi's `~/.pi/agent/`.
- Remove all current Oino built-in extensions (ralph-loop, vcc, notify, budget, presets,
  search, footer-status, ask-user, craft-skill, mode-sandbox, router UI, ...); unipi
  ports replace them. Remove the revival specs and P5 / FrontierHarness entirely.
- Breaking changes are free: WIT (`oino:extension`), crate layout, settings keys. Zero users.
- Monorepo like pi: `crates/` (core libraries + clients), `extensions/` (WASM
  extensions: providers + unipi ports), `apps/` (GPUI desktop/mobile), `wit/`.

## Quality gates ("beautiful Rust", audit-ready)

Every commit on `v1-alpha` must keep these green for the crates it touches:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic` (justify any
  `#[allow]` inline with a reason)
- `cargo test --workspace`
- `#![forbid(unsafe_code)]` in every crate except where the WASM host strictly needs it
  (document why).
- No `unwrap`/`expect`/`panic!` outside tests; typed errors via `thiserror`, `anyhow` only
  at binary edges.
- Rustdoc on all public items; crate-level docs describe boundary and API map.
- Small modules, explicit ownership, enums over stringly flags, no dead code.
- Follow the `rust-best-practices` skill (`/home/oi/.agents/skills/rust-best-practices/SKILL.md`).

## Test hosts (tailnet)

The owner allows testing on these hosts. They run other services: never stop, restart,
reconfigure or starve them. Keep jobs bounded (`cargo build -j 4` remote, nice/low priority,
one build at a time per host), work only under a dedicated dir (e.g. `~/oino-test`), clean up
processes you start (kill by PID only) and large temp files afterwards.

| Host | OS | Access | Notes |
|---|---|---|---|
| `coffee` | Windows (tiny11, 16 threads, ~8 GB RAM) | `ssh coffee` (PowerShell) | unstable under heavy load; console session may be locked (no screenshots) |
| `desmondgiam` | macOS (Intel x86_64, 8 threads, 8 GB RAM, cargo + Xcode) | `ssh mac` (user desmondgiam) | disk ~93% full (~16 GB free): reuse `~/oino-release`, `cargo clean` large targets you created, never fill the disk; may be asleep, skip if unreachable |
| `ti` | Ubuntu | `ssh ti@ti` (Tailscale SSH, user `ti`) | Tailscale may demand a browser check (owner must approve); if it does, skip and report |

Credentials: only from `/home/oi/Projects/Neuron/credentials/` (same as
`/home/oi/Projects/Personal/credentials/`): `pem/white` (SSH key), `mac/sudo.pass`,
`ti/sudo.pass`, `coffee/windows.env`. Read them at use time (e.g. `sudo -S < file` piped over
SSH); never print, copy into the repo, commit, log or store them in memory/docs.

## Working rules for agents

- Work only in the paths your task assigns; other agents work concurrently in the same
  checkout. Stage with explicit paths (`git add <paths>`), never `git add -A`.
- Commit on `v1-alpha` with clear messages prefixed by your work id, e.g. `[core-session] ...`.
- If you need a change outside your paths, do not make it: record it under
  "Requests for other work" in your report.
- Specs live in `docs/dev/v1/`. Keep docs in sync with behaviour.
