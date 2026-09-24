# Built-in Extensions

Oino ships optional built-in extensions. The normal installer enables them automatically.

From a source checkout, install or refresh them manually with:

```bash
bash scripts/install-all-builtins.sh      # macOS/Linux/Unix
.\scripts\install.ps1                    # Windows PowerShell, also builds/installs Oino
```

Then run `/reload` or restart Oino.

## Included packages

| Package | What it adds |
| --- | --- |
| `oino.router` | `/router` setup, dashboard, status, model refresh, restart, and password reset commands. |
| `oino.footer_status` | Extra footer status lines for model, thinking, working directory, and context. |
| `oino.ralph_loop` | Iterative development loops controlled from inside Oino (WASM component; owns `.oino/ralph` state and drives iterations via `turn_end`). |
| `oino.mode_sandbox` | `/mode <profile>` sandbox/profile controls. |
| `oino.notify` | ntfy.sh notifications for selected lifecycle events (WASM component; optional LLM summary via `host.complete`). |
| `oino.craft_skill` | Help creating Oino skills. |
| `oino.vcc` | Session compaction and recall (WASM component; `/compact` calls `session_before_compact`, recall reads `host.session_entries`). |
| `oino.ask_user` | Structured question tool with an Oino modal. |
| `oino.provider_anthropic` | Declarative `anthropic` provider (Anthropic Messages via bundled WASM component). |
| `oino.provider_openai` | Declarative `openai` provider (OpenAI Responses via bundled WASM component). |
| `oino.oino_router` | Official Oino Router provider — `oino:*` models on `router.oino.dev`. Picks Anthropic Messages or OpenAI chat-completions per model (`format`/`format_overrides` config; `OINO_API_KEY`/`OMNI_API_KEY`, `OINO_BASE_URL`, `OINO_FORMAT` env overrides). |
| `oino.budget` | Run caps (turns, cost, wall time, consecutive tool errors) with an optional summary turn before ending the run. Enabled but inert until configured (or via `--max-cost`/`--max-wall`/`--max-tool-errors`). |
| `oino.presets` | `/preset` work presets (`minimal`, `standard`, `thorough`) restricting the visible tool set and/or adding a prompt section. `--preset` for headless runs. |
| `oino.search` | Deferred `grep`/`find` tools over the project tree (gitignore-aware), loaded on demand through `tool_search`. |

## `tool_search` (host tool)

When any active extension contributes **deferred** tools (`"deferred": true`
in the manifest), the host registers a `tool_search` tool automatically.
Deferred tools are registered for execution but hidden from the model until
loaded:

- `tool_search {query}` — fuzzy-match up to 10 deferred tools (name +
  description).
- `tool_search {load: ["name", ...]}` — marks the named tools available; the
  next provider request carries a `System { tools_added }` delta mid-run.

## Managing them

Open:

```text
/extensions
```

Use the overlay to inspect health, toggle packages, and update installed packages.
