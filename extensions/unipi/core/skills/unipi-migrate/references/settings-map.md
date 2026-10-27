# Settings map

Targets: global `$OINO/settings.json`, project `<project>/.oino/settings.json`, secrets
`$OINO/secrets.json`. Oino reads pi 1.4.0 settings keys unchanged and keeps unknown keys, so
the core part is mostly a merge. Extension settings live under `extensionSettings.<namespace>`
(layered defaults ⊕ global ⊕ project, project wins).

## 1. pi `settings.json` (`$PI_DIR/settings.json`, project `.pi/settings.json`)

Merge into `$OINO/settings.json` (project: `<project>/.oino/settings.json`). On a key that
already exists in the Oino file keep the Oino value and record a conflict.

| pi key | Action |
|---|---|
| every key not listed below (`defaultProvider`, `defaultModel`, `defaultThinkingLevel`, `modelThinkingLevels`, `enabledModels`, `thinkingBudgets`, `steeringMode`, `followUpMode`, `compaction.*`, `branchSummary.*`, `retry.*`, `transport`, `httpIdleTimeoutMs`, `cacheWarming`, `showCacheMissNotices`, `images.*`, `shellPath`, `shellCommandPrefix`, `enableSkillCommands`, `defaultProjectTrust`, `theme`, `hideThinkingBlock`, `quietStartup`, `doubleEscapeAction`, `treeFilterMode`, `collapseChangelog`, `autocompleteMaxVisible`, `showHardwareCursor`, `markdown.*`, `terminal.*`, …) | copy unchanged |
| `defaultProvider` + `defaultModel` | copy, then check `oino --list-models <defaultModel>` lists `<defaultProvider>/<defaultModel>`; if not, keep it and warn (the provider may come from a pi TypeScript extension, see providers.md §3) |
| `modelThinkingLevels`, `enabledModels` | copy; entries for providers that do not exist in Oino are harmless, list them as warnings |
| `packages` | **not migrated**: pi packages are TypeScript (npm/git). `npm:@pi-unipi/unipi` is replaced by Oino's bundled `unipi.*` extensions. List every other entry as "no Oino port (TypeScript package)" |
| `extensions` | **not migrated**: TypeScript extension paths (`*.ts`) cannot load in Oino; list each |
| `skills`, `prompts`, `themes` (resource arrays) | rewrite each relative entry (pi resolves them against `$PI_DIR`, project ones against `<project>/.pi`) to an absolute path; drop entries inside `$PI_DIR/npm/node_modules/@pi-unipi/` (bundled in Oino); entries inside `$PI_DIR/{skills,prompts,themes}` point to the copied `$OINO/...` dirs instead; keep `!`/`+`/`-` prefixes |
| `sessionDir` | if set: absolute-ise; ask the user whether Oino should keep using that dir (then do not copy sessions) or drop the key (sessions copied to `$OINO/sessions`) |
| `lastChangelogVersion` | drop (pi's version; Oino tracks its own) |
| `enableInstallTelemetry` | drop (pi telemetry; Oino has none) |
| `unipi.*` (flat keys written by unipi ≤ 2.x) | map with §3 "old flat keys", then drop from the core file |

## 2. unipi module config → `extensionSettings.<ns>`

Sources, in increasing precedence:

1. old flat `unipi.*` keys of `$PI_DIR/settings.json` (§3, last table);
2. old flat files `$UNIPI/config/<ns>.json` (`background-tasks.json`, `subagents.json`);
3. `$UNIPI/config/<ns>/config.json` (global);
4. project overrides: `$UNIPI/workspace/<uuid>/config/<ns>/config.json` and
   `<project>/.unipi/config/<ns>/config.json` → **project** `extensionSettings.<ns>` of
   `<project>/.oino/settings.json`.

Algorithm per namespace `ns`:

1. Skip `ns` when it is not an Oino namespace (`fields.json` from phase 1). Known: `cron`
   (dropped, no scheduler), `tips` (replaced by `hints`; tip counts do not carry over) — both
   **not migrated**. `agents` is a profiles dir (state-map §5), not settings.
2. Load JSON; expand flat dotted keys (`"events.x.enabled": true`) into nested objects.
3. Apply the §3 rewrites for `ns`.
4. Flatten to dot keys. Keep a key when it equals a schema field key, is under a field key
   whose type is an object/page/`action`-backed map (`skills.states.<name>.*`,
   `notify.events.<event>.*`, `info-screen.groups.<g>.*`, `web-api.providers.<id>.*`), or is a
   key of the namespace `defaults`. Everything else: **not migrated** ("no Oino setting"),
   listed per key.
5. Respect field scope: fields marked `scope: global` (e.g. `fusion.startup.*`, secrets) only
   go to the global file; `scope: project` fields (`permission.mode`) only to a project file
   (see the `permission` rewrite).
6. Secret fields (type `secret`) → §4. Never write them into `settings.json`.
   Values that are paths into the old install are repointed: `~/.unipi/images`,
   `$UNIPI/global/image` (e.g. `image.generate.outputDir`) → `$STATE/unipi.utility/global/images`
   (or drop the key: that is Oino's default); any other `~/.unipi/…` or `~/.pi/agent/…` path:
   point it at the migrated copy if there is one, else keep it and warn.
7. Write non-secret values with a JSON merge into `extensionSettings.<ns>` (Oino value wins
   on conflict), or one RPC call per key (§4) — both are equivalent.

## 3. Rewrites (unipi 3.0 legacy keys → Oino keys)

These are the legacy shapes Oino no longer reads. Apply before filtering.

| ns | unipi key | Oino key / value |
|---|---|---|
| `compactor` | `overrideDefaultCompaction: false` (and no `method`) | `method: "llm"`; `true` or absent → `method: "vcc"` |
| `compactor` | `method: "jev"`, `piCompact: "jev"` | `"vcc"` |
| `compactor` | `autoCompaction.enabled` (and no `trigger`) | `trigger: "percent"` when true, `"pi"` when false |
| `compactor` | `autoCompaction.thresholdPercent`, `autoCompaction.notify` | `thresholdPercent`, `notify` (top-level values win) |
| `compactor` | `sections: {<id>: bool}` (and no `summarySections`) | `summarySections`: every id of `activeWork, requests, state, decisions, files, commits, errors, lessons, transcript` whose value is not `false` |
| `compactor` | `autoCompaction.cooldownMs`, `autoCompaction.repeatMinGrowthTokens`, `cooldownMs`, `jev`, `briefTranscript.*`, `commits.*`, `filesAndChanges.*`, `fts5Index.*`, other tuning keys | not migrated (Oino's compactor has fixed internals; 10 settings) |
| `info-screen` | `showOnBoot: true` / `false` (and no `bootMode`) | `bootMode: "on"` / `"off"` |
| `footer` | `colorMode: "mono"` | `colorMode: "none"` |
| `footer` | `preset`, `separator`, `groups`, `glanceMode` (classic footer) | not migrated (new footer layout) |
| `utility` | `badge.generationModel` (not `"inherit"`) | `rename.model` (if unset) |
| `utility` | `badge.autoGen` | `rename.auto` (if unset); drop all other `badge.*` |
| `utility` | `skills.mode` (`judged\|all\|off`), `skills.discovery: false` | namespace `skills`: `exposure.mode` (`discovery: false` → `"off"`) |
| `utility` | `skills.threshold`, `skills.maxSkills`, `skills.recheck` | `skills`: `exposure.threshold`, `exposure.maxSkills`, `exposure.recheck` |
| `skills` | `states.<name>: "on"` | `states.<name>: {"enabled": true}` |
| `skills` | `states.<name>: "off"` or `"unlisted"` | `states.<name>: {"enabled": false}` |
| `skills` | `states.<name>.discoverable: false` (no `enabled: false`, no `mustShow: true`) | `states.<name>.enabled: false`; drop `discoverable` |
| `kanboard` | `doCredits` (and no `doWrites`) | `doWrites` |
| `kanboard` | `blocking`, `defaultPlan`, `defaultStrategy`, `queueMax`, `agentCommand`, `root`, `slug` | not migrated (runner removed / per-project now) |
| `long-horizon` | `judge.provider`, `judge.model`, `judge.baseUrl`, `judge.timeoutMs` | `decisionModel.provider/model/baseUrl/timeoutMs` and `decisionModel.source: "custom"` — only when they differ from the `decision-model` namespace values (else leave `source: "inherit"`) |
| `long-horizon` | `judge.apiKey` (non-empty) | secret `decisionModel.apiKey` (§4) |
| `permission` | `mode` in the **global** file | `defaultMode` (if unset); `mode` is project-only in Oino |
| `notify` | `events.memory_consolidated.*`, `events.workflow_end.*` | not migrated (no such events in Oino) |
| `notify` | `$UNIPI/config/notify/ntfy.json` `{enabled, priority, serverUrl, topic, token}` | `ntfy.enabled/priority/serverUrl/topic`, secret `ntfy.token` (config.json values win) |
| `subagents` | `types.<name>.enabled` | not migrated as a setting; tell the user to toggle the profile in `/agents` |
| `background-tasks` | `defaultTimeoutSeconds`, `delegate.*`, `fusion.*` | not migrated (no Oino equivalent) |
| `mcp` | `servers`, `sync.*` | not settings: state file, see state-map §6 |
| `web-api` | `$UNIPI/config/web-api/auth.json` `{<provider>: "<key>"}` | secret `providers.<provider>.apiKey` |

Old flat keys in pi `settings.json` (unipi ≤ 2.x): `unipi.<module>.<rest>` → namespace
`<module>` in kebab-case (`longHorizon` → `long-horizon`, `info` → `info-screen`) with key
`<rest>`, then the table above. Known ones:

| pi key | Oino |
|---|---|
| `unipi.info.bootMode`, `unipi.info.bootTimeoutMs` | `info-screen.bootMode`, `info-screen.bootTimeoutMs` |
| `unipi.info.groups.<g>.show`, `unipi.info.groups.<g>.stats.<s>` | `info-screen.groups.<g>.show`, `.stats.<s>` (only groups/stats in the schema) |
| `unipi.longHorizon.judge.*`, `.defaultMode`, `.verifierModel` | `long-horizon` (judge rewrite above) |
| `unipi.skills.discovery: false` | `skills.exposure.mode: "off"` |

## 4. Secrets

Secret fields (type `secret` in `fields.json`), e.g. `decision-model.apiKey`,
`<ns>.decisionModel.apiKey`, `notify.gotify.appToken`, `notify.telegram.botToken`,
`notify.telegram.chatId`, `notify.ntfy.token`, `web-api.providers.<id>.apiKey`,
`image.keys.openrouter`, `image.keys.fal`, `image.{generate,edit,recognize}.apiKey`.
unipi kept them inline in its `config.json`. Write them through Oino's settings API so they
land in `$OINO/secrets.json` (key `<owner-ext-id>/<ns>/<key>`, mode 0600) and never in
`settings.json`:

```bash
# one JSON line per value; build the file with python (json.dumps) so values are quoted right
cat > "$MIG/secrets.jsonl" <<'EOF'
{"type":"_oino/set_extension_setting","id":"1","namespace":"notify","key":"telegram.botToken","value":"<value>","scope":"global"}
EOF
(cat "$MIG/secrets.jsonl"; sleep 5) | timeout 60 oino --mode rpc 2>/dev/null | grep '"command":"_oino/set_extension_setting"'
shred -u "$MIG/secrets.jsonl" 2>/dev/null || rm -f "$MIG/secrets.jsonl"
```

Every response must be `"success":true`. Check with
`{"type":"_oino/get_extension_settings","id":"g","namespace":"notify","scope":"global"}`: a set
secret reads back as `••••••`. The same RPC (`scope: "global"|"project"`, run with the project
as cwd and the project trusted) works for non-secret keys; `_oino/set_setting
{key, value, scope}` writes core keys.
