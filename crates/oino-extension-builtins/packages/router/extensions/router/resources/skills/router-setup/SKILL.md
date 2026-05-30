---
name: router-setup
description: Configure Oino to use a local or external OmniRoute endpoint for model auth/routing.
---

# OmniRoute Setup

Use this skill when the user wants to configure Oino through OmniRoute instead of built-in provider auth.

## Steps

1. Check whether the extension is installed:
   - Ask user to open `/extensions` and install `builtin:router` if needed.
2. Have them run `/router setup`. It initializes Oino's OmniRoute config and starts the managed sidecar when Docker/Podman is available. Use `/router guide` only when they want read-only instructions.
   - External endpoint mode: use `/router use-external` and start OmniRoute yourself.
   - If Docker/Podman is missing, `/router setup` prompts them to run `/router install-podman` or install a runtime manually.
3. For external endpoint mode, guide them to start OmniRoute:

   ```bash
   docker run -d \
     --name oino-router \
     -p 20128:20128 \
     -v "$HOME/.oino/extensions/router/data:/app/data" \
     -e DATA_DIR=/app/data \
     diegosouzapw/omniroute:3.8.7
   ```

4. Tell them to open `http://localhost:20128/dashboard` and configure providers/combos there.
5. If OmniRoute requires API keys for `/v1/*`, have them set `OMNIROUTE_API_KEY`.
6. Verify and refresh Oino's model cache with:

   ```text
   /router status
   /router models
   ```

7. Select a live/cached model:

   ```text
   /model router:kr/claude-sonnet-4.5
   ```

## Notes

- Oino should not copy or migrate provider credentials automatically without explicit user consent.
- Prefer pinned/known-good tags over `latest`.
- Known-good tag for this extension version: `3.8.7`.
- Persistent config lives at `~/.oino/extensions/router/config.json`.
- Managed start/restart fallback order is: resolved pinned tag → last-good tag → known-good tag.
