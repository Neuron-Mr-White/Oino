<div align="center">

# Oino

### A fast Rust terminal agent with extension-first model routing and a calm keyboard-driven TUI.

[Install](#install) · [Uninstall](#uninstall) · [Quick Start](#quick-start) · [Docs](#documentation) · [Glossary](#glossary)

</div>

---

## Index

- [Install](#install)
- [Uninstall](#uninstall)
- [Quick Start](#quick-start)
- [What Oino Does](#what-oino-does)
- [Everyday Commands](#everyday-commands)
- [Documentation](#documentation)
- [Glossary](#glossary)
- [Development](#development)
- [Troubleshooting](#troubleshooting)

## Install

macOS/Linux/Unix shell:

```bash
sh -c 'u=https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/scripts/install.sh; if command -v curl >/dev/null 2>&1; then curl -fsSL "$u"; elif command -v wget >/dev/null 2>&1; then wget -qO- "$u"; else echo "Install curl or wget, or run from a source checkout: sh scripts/install.sh" >&2; exit 1; fi' | sh
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/scripts/install.ps1 | iex
```

From a source checkout:

```bash
sh scripts/install.sh        # macOS/Linux/Unix
.\scripts\install.ps1       # Windows PowerShell
```

The installer builds Oino, places the binary in `~/.local/bin` (`$HOME\.local\bin` on Windows), and enables the built-in extension pack. Set `OINO_PREFIX` to install somewhere else.

## Uninstall

Remove only the binary:

```bash
rm -f ~/.local/bin/oino                         # macOS/Linux/Unix
Remove-Item "$HOME\.local\bin\oino.exe"        # Windows PowerShell
```

Optional cleanup:

```bash
rm -rf ~/.oino/extension-packages               # remove installed extension packages
rm -rf ~/.oino                                  # remove all Oino settings, sessions, caches, and extensions
```

Use the full `~/.oino` removal only when you are sure you no longer need saved sessions or local settings.

## Quick Start

```bash
oino
```

Recommended first-run flow inside Oino:

```text
/9router setup
/9router dashboard
/9router models
/model
```

The local 9router dashboard password is:

```text
oino
```

## What Oino Does

| Feature | Use it for |
| --- | --- |
| Terminal chat | Work with models from a keyboard-first TUI. |
| 9router-first auth | Configure provider keys in the local 9router dashboard. |
| Built-in coding tools | Let the agent read, run shell commands, edit, and write files. |
| Sessions | Resume previous work with `/sessions`. |
| Prompts and skills | Reuse workflows with `/prompt:` and `/skill:`. |
| Extensions | Add commands, tools, themes, providers, and UI surfaces. |
| Themes/keymaps | Customize the look and controls from `/settings`. |

## Everyday Commands

```text
/help        show keyboard and command help
/model       choose a model
/settings    open settings
/theme       choose a theme
/auth        show auth/readiness status
/account     show provider/runtime status
/usage       show usage totals
/extensions  manage extensions
/reload      rescan resources, extensions, themes, and cached model lists
```

## Documentation

User guides:

- [Auth and models](docs/auth.md)
- [Commands](docs/command.md)
- [Extensions](docs/extension.md)
- [Built-in extensions](docs/built-in-extensions.md)
- [Keymap](docs/keymap.md)
- [Configuration](docs/configurations.md)

Developer guides:

- [Architecture](docs/architecture.md)
- [Extension development](docs/dev/extension-dev.md)
- [TUI development](docs/dev/tui.md)
- [Theme development](docs/dev/theme.md)

## Glossary

| Term | Meaning |
| --- | --- |
| Oino | The terminal agent app and Rust workspace. |
| TUI | Terminal user interface: the transcript, composer, overlays, settings, and key handling. |
| 9router | The recommended local model router and provider-key dashboard. |
| Model | A selectable `provider:model-id`, such as a `9router:...` entry. |
| Extension | An installable package that can add commands, tools, themes, providers, settings pages, or resources. |
| Built-in extension | An optional extension shipped in this repository and enabled by the installer. |
| Prompt | A reusable text template included with `/prompt:<name>`. |
| Skill | A reusable workflow folder included with `/skill:<name>`. |
| Session | Saved conversation state under `~/.oino/sessions/`. |
| Model catalog | Cached model list under `~/.oino/model-catalogs/`. |

## Development

```bash
cargo fmt --all
cargo check --workspace
cargo test --workspace
```

Useful source-checkout commands:

```bash
cargo run -p oino-app --bin oino
bash scripts/install-all-builtins.sh
scripts/install-smoke.sh
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
```

## Troubleshooting

- `oino: command not found`: add the install directory, usually `~/.local/bin`, to `PATH`.
- No models: run `/9router setup`, add provider keys in the dashboard, then run `/9router models`.
- Dashboard password does not work: run `/9router reset-password` and `/9router restart`, then use `oino`.
- Terminal state looks broken after a crash: run `reset` in your shell.
