<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/oino.svg">
  <img src="docs/assets/brand/oino-invert.svg" alt="oino" width="160">
</picture>

# Oino

### A Rust coding-agent SDK: agent core, WASM extensions, terminal + ACP clients.

UniPi — the desktop/mobile distribution built on oino — lives in [apps/unipi](apps/unipi).

[Install](#install) · [Quick start](#quick-start) · [What's inside](#whats-inside) · [Docs](#documentation) · [Development](#development)

**Status: 1.0.0-alpha.3** — see [docs/dev/v1/STATUS.md](docs/dev/v1/STATUS.md) for parity and known issues.

</div>

---

Oino is a coding-agent core written in Rust. `oino-core` provides the agent loop, sessions
(session format v3), tools (`read`, `write`, `edit`, `bash`, plus `grep`, `find`, `ls`),
skills, prompt templates, `AGENTS.md` / `SYSTEM.md`, compaction, branch summaries, settings,
print / JSON / RPC modes and a fixed command line. Everything else is a client or an
extension:

- **Providers are WASM components** (Anthropic Messages, OpenAI Responses, OpenAI-compatible
  chat completions, the Oino Router). No provider code lives in core.
- **Extensions are WASM components too** — the SDK ships provider components plus example
  extensions; the full unipi 3.0.0 feature set (workflow, memory, subagents, kanboard, …)
  lives in the [UniPi distribution](#unipi-the-distribution) below.
- **Clients**: the terminal UI (`oino`), [ACP](docs/acp.md) for editors such as Zed
  (`oino --mode acp`), and [RPC](docs/rpc.md) on stdio.

## Install

macOS / Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/Neuron-Mr-White/Oino/v1-alpha/scripts/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/Neuron-Mr-White/Oino/v1-alpha/scripts/install.ps1 | iex
```

From a source checkout (needs Rust; `rust-toolchain.toml` pins 1.99.0 + `wasm32-wasip2`):

```bash
sh scripts/install.sh          # macOS / Linux
.\scripts\install.ps1          # Windows
```

The installers put `oino` into `~/.local/bin` (override with
`OINO_PREFIX`). They prefer a tagged release binary and fall back to building from source
(`cargo xtask wasm` + `cargo build --release -p oino-cli`). Details:
[docs/install.md](docs/install.md).

Uninstall: delete `~/.local/bin/oino`; `~/.oino` holds your settings, credentials and
sessions.

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

## Documentation

- [Install and update](docs/install.md)
- [Providers and credentials](docs/providers.md) · [Models and `models.json`](docs/models.md)
- [Terminal UI](docs/tui.md) · [Key bindings](docs/keybindings.md) · [Themes](docs/themes.md)
- [Settings](docs/settings.md) · [Environment variables](docs/environment-variables.md)
- [Sessions](docs/sessions.md) · [Skills](docs/skills.md) · [Prompt templates](docs/prompt-templates.md)
- [Extensions](docs/extensions.md) · [Extension API and WIT](docs/extension-api.md) · [Packages](docs/packages.md)
- [UniPi's bundled extensions](docs/unipi.md)
- [RPC mode](docs/rpc.md) · [ACP mode](docs/acp.md) · [Embedding (SDK)](docs/sdk.md)

Developer specifications live in [docs/dev/v1/](docs/dev/v1/README.md) (charter, specs,
deviations, [status](docs/dev/v1/STATUS.md)).

## UniPi, the distribution

[apps/unipi](apps/unipi) is the UniPi distribution built **on** oino — it is not part of the
SDK. It carries its own cargo workspace (`apps/unipi/crates/*`: the `unipi-host` daemon,
wire protocol, cron scheduler, kanboard CLI, browser service), the Tauri desktop/mobile app
(`apps/unipi/src-tauri`, its own workspace) and the UniPi WASM extensions
(`extensions/unipi/*`, built into `extensions/dist` by `cargo xtask wasm`). Build and test
it with `cd apps/unipi && cargo test --workspace` (see [docs/dev/v2/specs/app.md](docs/dev/v2/specs/app.md)).

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

Quality gates and test layers: [docs/dev/v1/quality.md](docs/dev/v1/quality.md).

## Acknowledgements

oino's agent core is a Rust port of [pi](https://github.com/earendil-works/pi) by Mario
Zechner (MIT). Unlike pi, whose extensions are TypeScript modules, oino runs extensions as
WASM components — any language that targets `wasm32-wasip2` can build one.

## License

MIT OR Apache-2.0.
