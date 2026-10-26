# pi oracle: settings and resources (core-runtime)

Golden fixtures for `crates/oino-settings` and `crates/oino-resources`, produced by importing
pi-coding-agent 1.4.0 modules directly (`PI_CODING_AGENT_PKG` overrides the package root;
default is the mise global install). Scratch trees go to `target/tmp/pi-oracle/`.

| Script | pi API | Output |
|---|---|---|
| `settings.mjs` | `SettingsManager` (merge, migrations, getters, writes) | `crates/oino-settings/tests/fixtures/pi/settings-expected.json` (inputs: `settings-cases.json`) |
| `system-prompt.mjs` | `buildSystemPromptSections/State`, `buildSystemPrompt`, `normalizeBuildSystemPromptOptions` | `crates/oino-resources/tests/fixtures/pi/system-prompt-expected.json` (inputs: `system-prompt-cases.json`) |
| `misc.mjs` | `parseCommandArgs`, `substituteArgs`, `expandPromptTemplate`, `formatSkillsForPrompt`, `parseSkillBlock`, `KeybindingValueSchema`, `migrateKeybindingsConfig`, `parseFrontmatter` | `crates/oino-resources/tests/fixtures/pi/misc-expected.json` |
| `resources.mjs` | `DefaultResourceLoader` on fixture trees | `crates/oino-resources/tests/fixtures/pi/resources-expected.json` (inputs: `resources-cases.json`) |

Normalisation applied on the pi side: D13 product substitutions (preamble, docs section with
`docs_root = /DOCS`, `PI_*` env guideline), `$HOME`, `$ROOT` and `.pi` → `.oino`.

Regenerate everything with `./regen.sh` (needs Node 24).
