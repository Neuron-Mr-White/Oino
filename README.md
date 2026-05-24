# Oino

Oino is a Rust-native agent runtime. The workspace now has a headless core plus the first rebuilt interactive shell: API-key auth, an OpenRouter provider adapter, built-in coding tools, and a modular Ratatui chat interface with a transcript and composer.

## Quickstart: OpenRouter TUI

Set an OpenRouter API key and run the binary:

```bash
OPENROUTER_API_KEY=sk-or-... mise run dev
```

Equivalent Cargo command:

```bash
OPENROUTER_API_KEY=sk-or-... cargo run -p oino-app --bin oino
```

Standalone markdown rendering proof-of-concept:

```bash
mise run tui:render-test
```

Extension authoring/devkit smoke check:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
```

Optional configuration:

```bash
OINO_MODEL=openrouter:openai/gpt-4o-mini \
OINO_OPENROUTER_REFERER=https://example.invalid \
OINO_OPENROUTER_TITLE=Oino \
OPENROUTER_API_KEY=sk-or-... \
mise run dev
```

Default model: `openrouter:openai/gpt-4o-mini`. For auth, model, and TUI controls, see [Auth, OpenRouter, and TUI shell](docs/auth-openrouter-tui.md).

## Disposable local Podman sandbox

For intensive local testing without touching your host home/config, start a disposable Podman sandbox with tmux:

```bash
mise run podman:up
```

The task builds the current Oino binary on the host, builds a small Debian runtime image with `tmux`, starts or reuses a named `oino-test` container, creates a fresh empty git workspace at `/workspace`, and attaches to a UTF-8 tmux session. Inside tmux, use `Ctrl-b c` for more windows and run `oino`, `oino --help`, or shell commands against the fresh sandbox workspace. The Oino source checkout is not copied or mounted into the container; only the built binary directory is mounted read-only at `/opt/oino-bin`. The writable workspace and sandbox home live in Podman volumes.

Useful tasks:

```bash
mise run podman:start   # create/start without attaching
mise run podman:attach  # reconnect to tmux later
mise run podman:shell   # open bash outside tmux
mise run podman:reset   # clear and reinitialize /workspace
mise run podman:status  # inspect container/image/volumes
mise run podman:clean   # remove container, volumes, and image
```

Optional environment:

```bash
# .env.podman is git-ignored and injected into podman run/exec/tmux.
cat > .env.podman <<'EOF'
OPENROUTER_API_KEY=sk-or-...
OINO_MODEL=openrouter:openai/gpt-4o-mini
EOF

mise run podman:up
OINO_PODMAN_PROFILE=release mise run podman:up
OINO_PODMAN_LOCALE=C.UTF-8 mise run podman:up
```

Changing `.env.podman` is picked up by future script-managed `podman exec` calls and new tmux panes/windows; restart any already-running `oino` process to use changed values.

`podman:reset` clears only `/workspace` and keeps the sandbox home/config. `podman:clean` clears the container, volumes, and image so the next `podman:up` starts fully fresh. `podman:attach`/`podman:start` continue the existing container and tmux session if you have not cleaned it.

## Using the TUI

The TUI is a transcript plus a bottom composer. The complete control reference lives in focused guides so this README stays quick to scan:

- [Auth, OpenRouter, and TUI shell](docs/auth-openrouter-tui.md) covers startup, keys, models, settings, steering while a response runs, and the default tool set.
- [Commands, suggestions, and non-interactive use](docs/commands.md) covers `/help`, `/settings`, `/model`, `/thinking`, `/theme`, `/prompts`, `/skills`, `/reload`, and command-line command paths.
- [Transcript rendering, inspect, and export](docs/transcript-rendering.md) covers chat styles, Markdown rendering, transcript focus, `/inspect`, Ctrl-click links/images, and chat HTML export.
- [Sessions and history](docs/sessions.md) covers `/new`, `/sessions`, `/title`, `oino --session <uuid>`, what is saved, and follow-up migration/import/delete work.
- [Resources, prompts, skills, and project instructions](docs/resources.md) covers `~/.oino/SYSTEM.md`, project `.oino/AGENT.md`, prompt templates, skills, `/prompt:`, `/skill:`, `/P:`, and `/S:`.

Everyday defaults: Enter submits, Ctrl-J/Alt-Enter/Shift-Enter insert newlines, Esc closes transient UI or stops a running response without quitting, and Ctrl-C twice quits. `/help` shows the active shortcut labels, including keymap changes. User settings persist to `~/.oino/settings.json`, OpenRouter model names are cached at `~/.oino/openrouter-models.json`, and sessions persist under `~/.oino/sessions` after they contain messages.

## Extension kernel

Oino includes a registry-first extension kernel for Oino-native manifests and packages. Start here:

- [Extension kernel overview](docs/extension-kernel/README.md)
- [Install and manage extensions](docs/extension-kernel/user-guide.md)
- [Build and publish extensions](docs/extension-kernel/developer-guide.md)
- [Extension SDK and devkit](docs/extension-sdk/README.md)
- [Playable author fixture](examples/extensions/rust-wasm-fixture/README.md)

Key user/developer surfaces:

- `oino.extension.json` and `oino.package.json` contracts for extensions and packages.
- Oino-owned discovery roots under `.oino` and `~/.oino`; unrelated agent extension paths are not loaded implicitly.
- Registry-backed tools, commands, keymaps, hooks, UI surfaces, settings pages, themes, providers, resources, autosuggest providers, renderers, diagnostics, health, and persistence contributions.
- Runtime capability broker and `wasm-json-v1` boundary for tool/command execution, host capabilities, progress, cancellation, and diagnostics.
- `/extensions` management overlay for discovered extensions, packages, contributions, health, diagnostics, conflicts, provenance, local/GitHub install (`i` project / `I` global), uninstall (`u`/`x` package row), and project/global enablement toggles (`p`/Enter and `g`).
- Package lifecycle services plus `/extensions` local/Git/GitHub package install/uninstall/reload flow; hosted registry browsing remains a future flow.

External contributions are pending review by default unless enabled through extension policy settings. Safe mode disables non-built-in contributions while preserving diagnostics. The current implementation includes deterministic fixture runtime/testing support, kernel APIs, and local/GitHub package install/uninstall from `/extensions`; hosted community registry browsing/publishing and production untrusted WASM host hardening remain follow-ups.

The command palette labels resource types explicitly: `[SYS]` for built-in commands, `[PROMPT]` for prompt templates, and `[SKILL]` for skills. Bare `/` suggestions only open at the start of the input and list system commands. Use `/prompts` and `/skills` to browse resources with fuzzy search, `/reload` to rescan `SYSTEM.md`, `AGENT.md`, prompts, and skills, `/P:<query>` or `/prompt:<query>` anywhere to search prompt templates, and `/S:<query>` or `/skill:<query>` anywhere to search skills. See the [commands guide](docs/commands.md) for command paths and shell usage, and the [resources guide](docs/resources.md) for resource file formats.

## Auth file

Oino can also read an API key from `~/.oino/auth.json`:

```json
{
  "openrouter": { "type": "api_key", "key": "sk-or-REDACTED" }
}
```

Resolution order is:

1. runtime/test override
2. `~/.oino/auth.json`
3. `OPENROUTER_API_KEY`

The auth crate writes the file with user-only permissions on Unix where feasible and avoids logging secret values.

## Layer boundaries

- [`oino-types`](crates/oino-types): model-visible/runtime-visible data types. No async runtime, provider, session, or filesystem dependencies.
- [`oino-agent-loop`](crates/oino-agent-loop): pure async loop, stream consumption, event sink, tool protocol, and faux test utilities. Provider serialization stays outside this crate.
- [`oino-agent`](crates/oino-agent): stateful wrapper around the loop with queues, subscribers, cancellation, and idle settlement.
- [`oino-session`](crates/oino-session): append-only session trees plus JSONL persistence. It reconstructs model context without owning providers/tools.
- [`oino-resource`](crates/oino-resource): Oino-owned system prompt, project instruction, prompt template, and skill discovery. It intentionally ignores unrelated agent resource paths unless a future importer copies them into Oino paths.
- [`oino-env`](crates/oino-env): execution-environment abstraction and local filesystem/process adapter for tools.
- [`oino-tools`](crates/oino-tools): built-in local coding tools (`read`, `bash`, `edit`, `write`) implemented on `ExecutionEnv`.
- [`oino-extension-core`](crates/oino-extension-core): data-only extension contracts, manifests, package metadata, permissions, registries, diagnostics, UI surfaces, persistence, and community registry schemas.
- [`oino-extension-builtins`](crates/oino-extension-builtins): registry representation of Oino's built-in tools, commands, settings, resources, themes, provider metadata, and hooks.
- [`oino-extension-manager`](crates/oino-extension-manager): Oino-owned discovery, validation, safe mode, reload diffs, management snapshots, package lifecycle, and fixture registry metadata.
- [`oino-extension-runtime`](crates/oino-extension-runtime): JSON-v1 runtime boundary, hook runner, capability broker, and extension tool/command adapters.
- [`oino-extension-sdk`](crates/oino-extension-sdk): author templates, validators, Rust SDK helpers, local test harness, devkit CLI, examples, and coverage gates.
- [`oino-harness`](crates/oino-harness): high-level binding of agent, sessions, env, providers, resources, and typed hooks.
- [`oino-auth`](crates/oino-auth): generic credential storage/resolution. It knows provider ids/env-var mappings, not HTTP protocols.
- [`oino-provider-openrouter`](crates/oino-provider-openrouter): OpenRouter model listing, request serialization, HTTP streaming, SSE parsing, and conversion into `AssistantStreamEvent`.
- [`oino-tui`](crates/oino-tui): modular Ratatui state, slash-command suggestions, reusable overlay/settings state, composer input handling, theming, and chat transcript rendering. No provider/auth logic.
- [`oino-app`](crates/oino-app): binary/runtime wiring for auth + provider + harness + session + TUI, including non-blocking model-cache refresh.

Provider code is intentionally separate from auth: auth answers “what credential should provider `openrouter` use?”, while the provider knows OpenRouter's base URL, endpoint, headers, request JSON, SSE chunks, finish reasons, and tool-call shape. Neither concern leaks into `oino-agent-loop`.

## Troubleshooting

- Missing key: set `OPENROUTER_API_KEY` or create `~/.oino/auth.json` as shown above.
- 401/403: verify the OpenRouter key and account access.
- 429: wait for rate-limit reset or choose another model/account.
- 5xx/network errors: retry later or check connectivity.
- Terminal looks broken after a crash: run `reset` in the shell.

## Current limitations

The first shell supports token-by-token transcript updates for provider text/thinking deltas, [Markdown-rendered assistant output and chat HTML export](docs/transcript-rendering.md), local coding tool calls, [persisted JSONL sessions](docs/sessions.md), non-interactive `--session <uuid>` continuation, Oino-owned resource files, prompt templates, skills, registry-backed extension kernel contracts, `/extensions` local/GitHub package install/uninstall and enablement, `/login claude`/`/login chatgpt` delegation to official OAuth login CLIs, and documented [command paths](docs/commands.md) such as `/new`, `/sessions`, `/settings`, `/prompts`, `/skills`, `/reload`, `/model`, `/thinking`, and `/theme`. It does not yet include native in-process OAuth, MCP, a hosted extension registry, production untrusted WASM host hardening, memory DB, session migration/import/delete commands, or a full high-risk permission approval UI.
