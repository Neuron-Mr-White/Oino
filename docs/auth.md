# Auth and Models

Oino uses extensions for provider setup. The recommended path is the built-in 9router extension: Oino talks to 9router, and 9router holds your provider keys.

## First setup

1. Install Oino. The main installer enables the 9router extension automatically. From a source checkout, you can also refresh built-ins manually:

   ```bash
   bash scripts/install-all-builtins.sh
   ```

2. Start Oino:

   ```bash
   oino
   ```

3. In Oino, run:

   ```text
   /9router setup
   ```

4. Open the dashboard:

   ```text
   /9router dashboard
   ```

5. Log in with the local default password:

   ```text
   oino
   ```

6. Add provider keys in the 9router dashboard.

7. Refresh models:

   ```text
   /9router models
   ```

8. Pick a model:

   ```text
   /model
   ```

## Daily commands

```text
/auth              show auth/readiness status
/account           show current provider/runtime status
/9router status    check the local 9router sidecar
/9router models    fetch and cache 9router models
/9router restart   restart the managed sidecar
```

## Model cache

Oino caches fetched model lists under `~/.oino/model-catalogs/`. `/9router models` fetches fresh 9router models and updates the visible list immediately. `/reload` reloads the cached list; it does not fetch from the network.

## If login fails

If the 9router dashboard rejects the password after an old setup, run:

```text
/9router reset-password
/9router restart
```

Then log in with `oino`.
