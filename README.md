<div align="center">

# Oino

### A Rust coding agent: pi 1.4.0 core, WASM extensions, terminal + desktop + ACP clients.

[Install](#install) · [Quick start](#quick-start) · [What's inside](#whats-inside) · [Docs](#documentation) · [Development](#development)

**Status: 1.0.0-alpha.1** — see [docs/dev/v1/STATUS.md](docs/dev/v1/STATUS.md) for parity and known issues.

</div>

---

Oino is another [pi](https://github.com/earendil-works/pi) core written in Rust. `oino-core`
behaves like `@earendil-works/pi-coding-agent` 1.4.0: the same agent loop, sessions (pi
session format v3), tools (`read`, `write`, `edit`, `bash`, plus `grep`, `find`, `ls`),
skills, prompt templates, `AGENTS.md` / `SYSTEM.md`, compaction, branch summaries, settings,
print / JSON / RPC modes and the same command line. Everything else is a client or an
extension:

- **Providers are WASM components** (Anthropic Messages, OpenAI Responses, OpenAI-compatible
  chat completions, the Oino Router). No provider code lives in core.
- **All of unipi 3.0.0** (workflow/permissions, plan mode, memory, compactor, subagents,
  background tasks, long-horizon goals, kanboard, MCP, web search, notifications, …) is ported
  to Rust WASM components and **bundled into the binary**. Installing `oino` is enough.
- **Clients**: the terminal UI (`oino`), [ACP](docs/acp.md) for editors such as Zed
  (`oino --mode acp`), [RPC](docs/rpc.md) on stdio, and the GPUI **Oino Desktop** app.

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

The installers put `oino` and `oino-kanboard` into `~/.local/bin` (override with
`OINO_PREFIX`). They prefer a tagged release binary and fall back to building from source
(`cargo xtask wasm` + `cargo build --release -p oino -p oino-kanboard`). Details:
[docs/install.md](docs/install.md). Oino Desktop: [docs/desktop.md](docs/desktop.md).

Uninstall: delete `~/.local/bin/oino` and `~/.local/bin/oino-kanboard`; `~/.oino` holds your
settings, credentials and sessions.

## Quick start

```bash
export ANTHROPIC_API_KEY=sk-ant-...     # or OPENAI_API_KEY, OPENROUTER_API_KEY, … (docs/providers.md)
oino                                    # interactive terminal UI
oino -p "Summarize this repository"     # print mode
oino --mode json "List the crates"      # JSON event stream
oino --mode acp                         # Agent Client Protocol on stdio (Zed, …)
```

Inside the TUI: `/model` picks a model, `/settings` opens settings, `/unipi:info` shows the
bundled extensions, `/hotkeys` lists key bindings, `ctrl+d` exits.

## What's inside

| Part | Where | Docs |
|---|---|---|
| Core runtime (pi 1.4.0 parity) | `crates/oino-core`, `oino-agent`, `oino-session`, `oino-tools`, … | [sessions](docs/sessions.md), [settings](docs/settings.md) |
| `oino` CLI: TUI, print, JSON, RPC, ACP | `crates/oino`, `oino-tui`, `oino-rpc`, `oino-acp` | [tui](docs/tui.md), [rpc](docs/rpc.md), [acp](docs/acp.md) |
| WASM extension host + WIT `oino:extension@1.0.0` | `wit/`, `crates/oino-wasm-host` | [extensions](docs/extensions.md), [extension API](docs/extension-api.md) |
| Provider components | `extensions/providers/*` | [providers](docs/providers.md), [models](docs/models.md) |
| unipi 3.0.0 ports | `extensions/unipi/*` | [unipi](docs/unipi.md) |
| Oino Desktop (GPUI) | `apps/` | [desktop](docs/desktop.md) |

## Documentation

- [Install and update](docs/install.md)
- [Providers and credentials](docs/providers.md) · [Models and `models.json`](docs/models.md)
- [Terminal UI](docs/tui.md) · [Key bindings](docs/keybindings.md) · [Themes](docs/themes.md)
- [Settings](docs/settings.md) · [Environment variables](docs/environment-variables.md)
- [Sessions](docs/sessions.md) · [Skills](docs/skills.md) · [Prompt templates](docs/prompt-templates.md)
- [Extensions](docs/extensions.md) · [Extension API and WIT](docs/extension-api.md) · [Packages](docs/packages.md)
- [Bundled unipi extensions](docs/unipi.md)
- [RPC mode](docs/rpc.md) · [ACP mode](docs/acp.md) · [Embedding (SDK)](docs/sdk.md)
- [Oino Desktop](docs/desktop.md)

Developer specifications live in [docs/dev/v1/](docs/dev/v1/README.md) (charter, specs,
deviations, [status](docs/dev/v1/STATUS.md)).

## Development

Three cargo workspaces: the root (`crates/*`, `xtask`), `extensions/` (`wasm32-wasip2`
components) and `apps/` (GPUI).

```bash
cargo xtask wasm                      # build every component into extensions/dist (bundled by oino-bundle)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
cargo test --workspace
(cd extensions && cargo clippy --target wasm32-wasip2 --workspace --all-targets -- -D warnings && cargo test --workspace)
(cd apps && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace)
sh scripts/install-smoke.sh           # install into a temp prefix and run one faux prompt
```

Quality gates and test layers: [docs/dev/v1/quality.md](docs/dev/v1/quality.md).

## License

MIT OR Apache-2.0.
