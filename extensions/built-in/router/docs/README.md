# Oino OmniRoute

Optional built-in extension for routing Oino through [OmniRoute](https://github.com/diegosouzapw/OmniRoute). The `/router ...` slash commands are contributed by this extension; install and enable `builtin:router` before using them.

## Current mode

Oino supports managed sidecar mode (`/router setup`) and external endpoint mode (`/router use-external`). For an external hosted endpoint, run `/router use-external` in the TUI to open the URL/API-key onboarding form; Enter verifies `/v1/models`, saves the endpoint config, saves the API key as provider `router`, and refreshes the model cache.

Manual external setup:

1. Start OmniRoute yourself, usually with Docker:

   ```bash
   docker run -d \
     --name oino-router \
     -p 20128:20128 \
     -v "$HOME/.oino/extensions/router/data:/app/data" \
     -e DATA_DIR=/app/data \
     diegosouzapw/omniroute:3.8.7
   ```

2. Open the dashboard: <http://localhost:20128/dashboard>
3. Configure providers and combos in OmniRoute.
4. If your OmniRoute requires API keys for `/v1/*`, either run `/router use-external` in the TUI and paste the key, or set:

   ```bash
   export OMNIROUTE_API_KEY=<api-key-from-dashboard>
   ```

5. In Oino, select a model such as:

   ```text
   /model router:kr/claude-sonnet-4.5
   ```

## Commands

```text
/router setup
/router guide
/router status
/router fetch-models
/router dashboard
/router use-external
/router use-managed
/router install-podman
/router stop
/router restart
/router version list
/router version pin <tag>
/router rollback [tag]
```

## Model catalog cache

`/router fetch-models` fetches `GET /v1/models`, writes Oino's provider cache for `OmniRoute`, and updates `/model` search with live/cached OmniRoute models. The background model refresh also refreshes this cache when OmniRoute is reachable.

## Fallback policy

The bundled known-good image tag is `diegosouzapw/omniroute:3.8.7`. Managed sidecar start/restart tries the resolved tag first, then the last-good tag, then the known-good tag. Persistent config lives at `~/.oino/extensions/router/config.json`.

The same config file also follows Oino's generic extension runtime conventions: `base_url` overrides the manifest runtime endpoint, `health_url` can override the `/auth router` readiness health endpoint, and dotted secret keys such as `secrets.api_key` can be used by extension runtime metadata. OmniRoute-specific env aliases include `OMNIROUTE_BASE_URL`, `OMNIROUTE_HEALTH_URL`, `OMNIROUTE_DASHBOARD_URL`, `OMNIROUTE_IMAGE_TAG`, and `OMNIROUTE_API_KEY`.
