<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/oino.svg">
  <img src="docs/assets/brand/oino-invert.svg" alt="oino" width="160">
</picture>

# Oino

### A Rust coding-agent SDK: agent core, WASM extensions, terminal + ACP clients.

UniPi — the desktop/mobile distribution built on oino — lives in [apps/unipi](apps/unipi).

[Install](#install) · [Quick start](#quick-start) · [What's inside](#whats-inside) · [Crates](#crates) · [Docs](#documentation) · [Development](#development)

**Status: 1.0.2** — the SDK crates are stable and published; see [Releases](https://github.com/Neuron-Mr-White/Oino/releases)
for changes and [docs/](docs/) for known gaps.

</div>

---

Oino is a coding-agent core written in Rust. `oino-core` provides the agent loop, sessions
(session format v3), tools (`read`, `write`, `edit`, `bash`, plus `grep`, `find`, `ls`),
skills, prompt templates, `AGENTS.md` / `SYSTEM.md`, compaction, branch summaries, settings,
print / JSON / RPC modes and a fixed command line. Everything else is a client or an
extension:

- **Providers are WASM components** (Anthropic Messages, OpenAI Responses, OpenAI-compatible
  chat completions). No provider code lives in core.
- **Extensions are WASM components too** — the SDK ships provider components plus example
  extensions; the full unipi 3.0.0 feature set (workflow, memory, subagents, kanboard, …)
  lives in the [UniPi distribution](#unipi--built-with-oino) below.
- **Clients**: the terminal UI (`oino`), [ACP](docs/acp.md) for editors such as Zed
  (`oino --mode acp`), and [RPC](docs/rpc.md) on stdio.

## Install

macOS / Linux:

```bash
curl -fsSL https://oino.dev/install.sh | sh          # the oino TUI
curl -fsSL https://oino.dev/install.sh | sh -s -- --app   # … + the UniPi desktop app
```

Windows (PowerShell):

```powershell
irm https://oino.dev/install.ps1 | iex               # the oino TUI
# the UniPi desktop app (the pipe cannot carry args — download once):
irm https://oino.dev/install.ps1 -OutFile install.ps1; .\install.ps1 -App
```

Until oino.dev serves the scripts, use the raw GitHub URLs, e.g.
`curl -fsSL https://raw.githubusercontent.com/Neuron-Mr-White/Oino/v2/scripts/install.sh | sh`.
From a source checkout (needs Rust; `rust-toolchain.toml` pins 1.99.0 + `wasm32-wasip2`):

```bash
sh scripts/install.sh            # macOS / Linux (--app: the desktop app)
scripts/install.ps1             # Windows PowerShell (-App: the desktop app)
```

The installers download the latest GitHub release, verify it against the
release's `SHA256SUMS`, and put `oino` into `~/.local/bin` (Windows:
`%LOCALAPPDATA%\oino\bin`, added to your PATH) together with the extension
packages under `<prefix>/share/oino/extensions` (Windows: next to the exe).
`--app` / `-App` installs the UniPi desktop app (Linux AppImage or `.deb`,
macOS `UniPi.app`, Windows NSIS installer). Without a matching release binary
they build from source (`cargo xtask wasm` + `cargo build --release -p
oino-cli`). Details: [docs/install.md](docs/install.md).

Uninstall: delete `~/.local/bin/oino` (and `~/.local/share/oino`); `~/.oino`
holds your settings, credentials and sessions.

## Quick start

```bash
export ANTHROPIC_API_KEY=sk-ant-...     # or OPENAI_API_KEY, OPENROUTER_API_KEY, … (docs/providers.md)
oino                                    # interactive terminal UI
oino -p "Summarize this repository"     # print mode
oino --mode json "List the crates"      # JSON event stream
oino --mode acp                         # Agent Client Protocol on stdio (Zed, …)
```

Inside the TUI: `/model` picks a model, `/settings` opens settings, `/info` shows the
bundled extensions, `/hotkeys` lists key bindings, `ctrl+d` exits.

## What's inside

| Part | Where | Docs |
|---|---|---|
| Core runtime | `crates/oino-core`, `oino-agent`, `oino-session`, `oino-tools`, … | [sessions](docs/sessions.md), [settings](docs/settings.md) |
| `oino` CLI: TUI, print, JSON, RPC, ACP | `crates/oino`, `oino-tui`, `oino-rpc`, `oino-acp` | [tui](docs/tui.md), [rpc](docs/rpc.md), [acp](docs/acp.md) |
| WASM extension host + WIT `oino:extension@2.0.0` | `wit/`, `crates/oino-wasm-host` | [extensions](docs/extensions.md), [extension API](docs/extension-api.md) |
| Provider components | `extensions/providers/*` | [providers](docs/providers.md), [models](docs/models.md) |
| UniPi distribution (app + native crates + extensions) | `apps/unipi`, `extensions/unipi/*` | [unipi](docs/unipi.md) |

## Crates

All crates are on crates.io at 1.0.2 with rustdoc on docs.rs. Consumers want `oino-core`
(embedding), `oino` (the CLI binary), or `oino-extension` (writing a component) — the rest
are layers those pull in.

| Crate | What it is | |
|---|---|---|
| `oino-core` | Coding-agent runtime: `AgentSession`, extension runner, compaction, retry | [![crates.io](https://img.shields.io/crates/v/oino-core.svg)](https://crates.io/crates/oino-core) [![docs.rs](https://docs.rs/oino-core/badge.svg)](https://docs.rs/oino-core) |
| `oino-agent` | Agent loop, tool execution, stateful agent with queues | [![crates.io](https://img.shields.io/crates/v/oino-agent.svg)](https://crates.io/crates/oino-agent) [![docs.rs](https://docs.rs/oino-agent/badge.svg)](https://docs.rs/oino-agent) |
| `oino-ai` | Message, content, model and streaming types (fixed JSON shapes) | [![crates.io](https://img.shields.io/crates/v/oino-ai.svg)](https://crates.io/crates/oino-ai) [![docs.rs](https://docs.rs/oino-ai/badge.svg)](https://docs.rs/oino-ai) |
| `oino-models` | Model registry: `models.json`, resolution, auth storage, virtual models | [![crates.io](https://img.shields.io/crates/v/oino-models.svg)](https://crates.io/crates/oino-models) [![docs.rs](https://docs.rs/oino-models/badge.svg)](https://docs.rs/oino-models) |
| `oino-session` | Session format v3: entries, migration, context projection, `SessionManager` | [![crates.io](https://img.shields.io/crates/v/oino-session.svg)](https://crates.io/crates/oino-session) [![docs.rs](https://docs.rs/oino-session/badge.svg)](https://docs.rs/oino-session) |
| `oino-tools` | Built-in tools: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls` | [![crates.io](https://img.shields.io/crates/v/oino-tools.svg)](https://crates.io/crates/oino-tools) [![docs.rs](https://docs.rs/oino-tools/badge.svg)](https://docs.rs/oino-tools) |
| `oino-settings` | Settings manager: global/project merge, typed getters, locking | [![crates.io](https://img.shields.io/crates/v/oino-settings.svg)](https://crates.io/crates/oino-settings) [![docs.rs](https://docs.rs/oino-settings/badge.svg)](https://docs.rs/oino-settings) |
| `oino-resources` | Context files, skills, prompt templates, system prompt, project trust | [![crates.io](https://img.shields.io/crates/v/oino-resources.svg)](https://crates.io/crates/oino-resources) [![docs.rs](https://docs.rs/oino-resources/badge.svg)](https://docs.rs/oino-resources) |
| `oino-ui` | Wire types + builder for the extension UI protocol v2 (wasm-safe, serde-only) | [![crates.io](https://img.shields.io/crates/v/oino-ui.svg)](https://crates.io/crates/oino-ui) [![docs.rs](https://docs.rs/oino-ui/badge.svg)](https://docs.rs/oino-ui) |
| `oino-view` | Frontend-agnostic view models: transcript, extension UI state, sessions, composer | [![crates.io](https://img.shields.io/crates/v/oino-view.svg)](https://crates.io/crates/oino-view) [![docs.rs](https://docs.rs/oino-view/badge.svg)](https://docs.rs/oino-view) |
| `oino-wasm-host` | Wasmtime component host for `oino:extension@2.0.0` components | [![crates.io](https://img.shields.io/crates/v/oino-wasm-host.svg)](https://crates.io/crates/oino-wasm-host) [![docs.rs](https://docs.rs/oino-wasm-host/badge.svg)](https://docs.rs/oino-wasm-host) |
| `oino-extension-api` | Contracts between core and extension runtimes: instances, services, manifests | [![crates.io](https://img.shields.io/crates/v/oino-extension-api.svg)](https://crates.io/crates/oino-extension-api) [![docs.rs](https://docs.rs/oino-extension-api/badge.svg)](https://docs.rs/oino-extension-api) |
| `oino-client` | Client SDK: `SessionBackend` contract, RPC records, in-process / fake backends | [![crates.io](https://img.shields.io/crates/v/oino-client.svg)](https://crates.io/crates/oino-client) [![docs.rs](https://docs.rs/oino-client/badge.svg)](https://docs.rs/oino-client) |
| `oino-rpc` | JSONL RPC codec, RPC server over a `SessionBackend`, typed client | [![crates.io](https://img.shields.io/crates/v/oino-rpc.svg)](https://crates.io/crates/oino-rpc) [![docs.rs](https://docs.rs/oino-rpc/badge.svg)](https://docs.rs/oino-rpc) |
| `oino-acp` | ACP v1 (Agent Client Protocol) agent adapter over oino-core sessions | [![crates.io](https://img.shields.io/crates/v/oino-acp.svg)](https://crates.io/crates/oino-acp) [![docs.rs](https://docs.rs/oino-acp/badge.svg)](https://docs.rs/oino-acp) |
| `oino-tui` | Interactive terminal client over oino-client | [![crates.io](https://img.shields.io/crates/v/oino-tui.svg)](https://crates.io/crates/oino-tui) [![docs.rs](https://docs.rs/oino-tui/badge.svg)](https://docs.rs/oino-tui) |
| `oino` (bin: `oino`) | The CLI binary: interactive (TUI), print, JSON, RPC and ACP modes | [![crates.io](https://img.shields.io/crates/v/oino.svg)](https://crates.io/crates/oino) [![docs.rs](https://docs.rs/oino/badge.svg)](https://docs.rs/oino) |
| `oino-extension` | Guest SDK for writing WASM component extensions | [![crates.io](https://img.shields.io/crates/v/oino-extension.svg)](https://crates.io/crates/oino-extension) [![docs.rs](https://docs.rs/oino-extension/badge.svg)](https://docs.rs/oino-extension) |
| `oino-extension-macros` | Proc macros for the guest SDK | [![crates.io](https://img.shields.io/crates/v/oino-extension-macros.svg)](https://crates.io/crates/oino-extension-macros) [![docs.rs](https://docs.rs/oino-extension-macros/badge.svg)](https://docs.rs/oino-extension-macros) |

## Documentation

User guides live in [docs/](docs/) and on **[oino.dev](https://oino.dev)**:

- [Install and update](docs/install.md)
- [Providers and credentials](docs/providers.md) · [Models and `models.json`](docs/models.md)
- [Terminal UI](docs/tui.md) · [Key bindings](docs/keybindings.md) · [Themes](docs/themes.md)
- [Settings](docs/settings.md) · [Environment variables](docs/environment-variables.md)
- [Sessions](docs/sessions.md) · [Skills](docs/skills.md) · [Prompt templates](docs/prompt-templates.md)
- [Extensions](docs/extensions.md) · [Extension API and WIT](docs/extension-api.md) · [Packages](docs/packages.md)
- [UniPi's bundled extensions](docs/unipi.md)
- [RPC mode](docs/rpc.md) · [ACP mode](docs/acp.md) · [Embedding (SDK)](docs/sdk.md)
- [Desktop and mobile](docs/desktop.md) · [Migrating from unipi](docs/migrate-from-unipi.md)

## UniPi — built with oino

[apps/unipi](apps/unipi) is the reference application, built **on** oino end to end — read its
code as the flagship example of the SDK. Its host is `oino-core` embedded in a native daemon
(`apps/unipi/crates/unipi-host`), its features are oino WASM components
(`extensions/unipi/*`, built into `extensions/dist` by `cargo xtask wasm`), and its
desktop/mobile frontend is a Tauri app (`apps/unipi/src-tauri`). It is a distribution, not
part of the SDK: it carries its own cargo workspace. Start with
[apps/unipi/README.md](apps/unipi/README.md) and [docs/unipi.md](docs/unipi.md).

## Development

Three cargo workspaces: the root (`crates/*`, `xtask` — the SDK), `extensions/`
(`wasm32-wasip2` components) and `apps/unipi` (the distribution crates).

```bash
cargo xtask wasm                      # build every component into extensions/dist (staged into the distribution share dir)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
cargo test --workspace
(cd extensions && cargo clippy --target wasm32-wasip2 --workspace --all-targets -- -D warnings && cargo test --workspace)
(cd apps/unipi && cargo test --workspace)
sh scripts/install-smoke.sh           # install into a temp prefix and run one faux prompt
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow, and `mise tasks` (`.mise.toml`) for
the common tasks (`mise run quality` runs the standard gate).

## Acknowledgements

oino's agent core is a Rust port of [pi](https://github.com/earendil-works/pi) by Mario
Zechner (MIT). Unlike pi, whose extensions are TypeScript modules, oino runs extensions as
WASM components — any language that targets `wasm32-wasip2` can build one.

## License

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
