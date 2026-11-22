<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-models

Model registry for Oino: `models.json`, model resolution, `auth.json` storage and virtual
models.

Providers are WASM components; this crate never streams. It stores provider registrations and
their models, merges `models.json` and config overrides in a fixed order, resolves credentials
per request and resolves `--model`/`--models`/defaults.

* `ModelRegistry` — shared catalog plus auth (`Arc<RwLock>`)
* `ModelsJson` — credential-blind `models.json` load with diagnostics
* `AuthStorage` — `auth.json` with locking and runtime keys
* `resolver` — `--model`, scopes, initial model, restore, cycling
* `virtual_models` — virtual model definitions, route wire types, validation
* `auth_check` — `oino auth check` (exit 0/1/2)
