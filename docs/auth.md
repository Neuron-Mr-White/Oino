# Auth and Models

Oino uses extensions for provider setup. The recommended path is the built-in OmniRoute extension: Oino talks to OmniRoute, and OmniRoute holds your provider keys.

## First setup

1. Install Oino. The main installer enables the OmniRoute extension automatically. From a source checkout, you can also refresh built-ins manually:

   ```bash
   bash scripts/install-all-builtins.sh
   ```

2. Start Oino:

   ```bash
   oino
   ```

3. In Oino, run:

   ```text
   /router setup
   ```

4. Open the dashboard:

   ```text
   /router dashboard
   ```

5. Log in with the local default password:

   ```text
   oino
   ```

6. Add provider keys in the OmniRoute dashboard.

7. Refresh models:

   ```text
   /router fetch-models
   ```

8. Pick a model:

   ```text
   /model
   ```

## Daily commands

```text
/auth              show auth/readiness status
/account           show current provider/runtime status
/router status    check the local OmniRoute sidecar
/router fetch-models    fetch and cache OmniRoute models
/router restart   restart the managed sidecar
```

## Model cache

Oino caches fetched model lists under `~/.oino/model-catalogs/`. `/router fetch-models` fetches fresh OmniRoute models and updates the visible list immediately. `/reload` reloads the cached list; it does not fetch from the network.

## If login fails

If the OmniRoute dashboard rejects the password after an old setup, run:

```text
/router reset-password
/router restart
```

Then log in with `oino`.
