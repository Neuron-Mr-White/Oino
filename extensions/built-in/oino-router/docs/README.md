# oino.oino-router

Official Oino Router provider (`oino:*` models) against `router.oino.dev`.
Picks the Anthropic Messages or OpenAI chat-completions wire format per model.

- `OINO_API_KEY` (or `OMNI_API_KEY`) — required.
- `OINO_BASE_URL` — override the base URL.
- `OINO_FORMAT` — `auto` | `openai` | `anthropic`.
- config.json keys: `base_url`, `format`, `format_overrides` (`{"<id>": "openai"|"anthropic"}`).
