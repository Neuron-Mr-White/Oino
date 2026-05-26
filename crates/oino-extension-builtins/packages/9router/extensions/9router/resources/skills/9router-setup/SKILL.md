---
name: 9router-setup
description: Configure Oino to use a local or external 9router endpoint for model auth/routing.
---

# 9router Setup

Use this skill when the user wants to configure Oino through 9router instead of built-in provider auth.

## Steps

1. Check whether the extension is installed:
   - Ask user to open `/extensions` and install `builtin:9router` if needed.
2. Have them run `/9router setup`. It initializes Oino's 9router config and starts the managed sidecar when Docker/Podman is available. Use `/9router guide` only when they want read-only instructions.
   - External endpoint mode: use `/9router use-external` and start 9router yourself.
   - If Docker/Podman is missing, `/9router setup` prompts them to run `/9router install-podman` or install a runtime manually.
3. For external endpoint mode, guide them to start 9router:

   ```bash
   docker run -d \
     --name 9router \
     -p 20128:20128 \
     -v "$HOME/.9router:/app/data" \
     -e DATA_DIR=/app/data \
     ghcr.io/decolua/9router:0.4.59
   ```

4. Tell them to open `http://localhost:20128/dashboard` and configure providers/combos there.
5. If 9router requires API keys for `/v1/*`, have them set `NINEROUTER_API_KEY`.
6. Verify and refresh Oino's model cache with:

   ```text
   /9router status
   /9router models
   ```

7. Select a live/cached model:

   ```text
   /model 9router:kr/claude-sonnet-4.5
   ```

## Notes

- Oino should not copy or migrate provider credentials automatically without explicit user consent.
- Prefer pinned/known-good tags over `latest`.
- Known-good tag for this extension version: `0.4.59`.
- Persistent config lives at `~/.oino/extensions/9router/config.json`.
- Managed start/restart fallback order is: resolved pinned tag → last-good tag → known-good tag.
