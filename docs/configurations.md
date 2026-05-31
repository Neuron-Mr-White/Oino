# Configuration

Oino works with defaults, then saves user choices as you use it.

## Important locations

| Path | Purpose |
| --- | --- |
| `~/.oino/settings.json` | User settings. |
| `~/.oino/sessions/` | Saved sessions. |
| `~/.oino/model-catalogs/` | Cached model lists. |
| `~/.oino/extension-packages/` | Globally installed extension packages. |
| `~/.oino/SYSTEM.md` | Global instructions. |
| `.oino/AGENT.md` | Project instructions. |
| `.oino/prompts/` | Project prompt templates. |
| `.oino/skills/` | Project skills. |

## Useful environment variables

| Variable | Purpose |
| --- | --- |
| `OINO_MODEL` | Initial model, for example `router:openai/gpt-4.1`. |
| `OMNIROUTE_BASE_URL` | OmniRoute OpenAI-compatible base URL. Defaults to `http://localhost:20128/v1`. |
| `OMNIROUTE_API_KEY` | Optional API key if your OmniRoute endpoint requires one. |
| `OINO_HOME` | Override the home root used by install helper scripts. |
| `OINO_PREFIX` | Override where installers place the `oino` binary. |
| `OINO_REPO` | Override the repository cloned by installers. |
| `OINO_REF` | Ask installers to check out a specific branch, tag, or commit. |

Example:

```bash
OINO_MODEL=router:openai/gpt-4.1 oino
```

## Reloading

Use `/reload` after editing settings, prompts, skills, themes, or extension files. Use `/router fetch-models` when you want fresh model data from OmniRoute.
