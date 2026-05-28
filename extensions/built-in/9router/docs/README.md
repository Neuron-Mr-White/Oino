# Oino 9router

Optional built-in extension for routing Oino through [9router](https://github.com/decolua/9router). The `/9router ...` slash commands are contributed by this extension; install and enable `builtin:9router` before using them.

## Current mode

This first implementation supports **external endpoint mode**:

1. Start 9router yourself, usually with Docker:

   ```bash
   docker run -d \
     --name 9router \
     -p 20128:20128 \
     -v "$HOME/.9router:/app/data" \
     -e DATA_DIR=/app/data \
     ghcr.io/decolua/9router:0.4.59
   ```

2. Open the dashboard: <http://localhost:20128/dashboard>
3. Configure providers and combos in 9router.
4. If your 9router requires API keys for `/v1/*`, set:

   ```bash
   export NINEROUTER_API_KEY=<api-key-from-dashboard>
   ```

5. In Oino, select a model such as:

   ```text
   /model 9router:kr/claude-sonnet-4.5
   ```

## Commands

```text
/9router setup
/9router guide
/9router status
/9router models
/9router dashboard
/9router use-external
/9router use-managed
/9router install-podman
/9router stop
/9router restart
/9router version list
/9router version pin <tag>
/9router rollback [tag]
```

## Model catalog cache

`/9router models` fetches `GET /v1/models`, writes Oino's provider cache for `9router`, and updates `/model` search with live/cached 9router models. The background model refresh also refreshes this cache when 9router is reachable.

## Fallback policy

The bundled known-good image tag is `ghcr.io/decolua/9router:0.4.59`. Managed sidecar start/restart tries the resolved tag first, then the last-good tag, then the known-good tag. Persistent config lives at `~/.oino/extensions/9router/config.json`.

The same config file also follows Oino's generic extension runtime conventions: `base_url` overrides the manifest runtime endpoint, `health_url` can override the `/auth 9router` readiness health endpoint, and dotted secret keys such as `secrets.api_key` can be used by extension runtime metadata. 9router-specific env aliases include `NINEROUTER_BASE_URL`, `NINEROUTER_HEALTH_URL`, `NINEROUTER_DASHBOARD_URL`, `NINEROUTER_IMAGE_TAG`, and `NINEROUTER_API_KEY`.
