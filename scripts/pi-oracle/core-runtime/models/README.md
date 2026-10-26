# pi oracle: oino-models

`generate.mjs` imports pi-coding-agent 1.4.0 modules (`model-resolver`, `resolve-config-value`,
`model-config`, `provider-composer`, `virtual-models`, pi-ai `auth/helpers`) and writes the golden
fixtures compared by `crates/oino-models/tests/golden.rs`:

| Fixture | Content |
|---|---|
| `resolver.json` | fixed catalog; `resolveModelReference` (all suffix/fuzzy/usable combinations), `resolveModelScopeFromModels`, `resolveCliModel`, `localeCompare` order |
| `config-value.json` | `resolveConfigValue`, env var name helpers, `resolveConfigValueOrThrow`, header resolution |
| `models-json.json` | `ModelConfig.load` diagnostics (schema error texts) |
| `compose.json` | `composeModelProvider` model layering and API-key check/resolve per credential source |
| `virtual-models.json` | `createVirtualModel` catalog entries |

```sh
OINO_TEST_TMP=$PWD/target/tmp node scripts/pi-oracle/core-runtime/models/generate.mjs
```

`PI_CODING_AGENT_PKG` overrides the pi package root (default: the mise global install).
