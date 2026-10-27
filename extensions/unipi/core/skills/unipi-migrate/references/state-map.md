# State and resource map

`$STATE/<ext-id>/global/<rel>` is the extension's `/ext-global/<rel>`;
`$STATE/<ext-id>/workspace/<wsid>/<rel>` is `/ext-workspace/<rel>` for the project whose
Oino workspace id is `<wsid>` (SKILL.md "Variables"). unipi's per-project dir is
`$UNIPI/workspace/<uuid>/` where `<uuid>` comes from `<project>/.unipi-workspace.json`.

Copy with `cp -an` (never overwrite) unless a step says "merge". Create target dirs with
`mkdir -p`. Each step lists: source → target, transformation, check.

## 1. Sessions

- `$PI_DIR/sessions/--<cwd>--/*.jsonl` → `$OINO/sessions/--<cwd>--/` (same dir names; pi
  session format v3 is Oino's native format; older pi versions migrate on load).
  `cp -an "$PI_DIR/sessions/." "$OINO/sessions/"`. Skip files that already exist (same name
  = same session). If pi `sessionDir` was set, copy from that dir instead (or keep using it,
  settings-map §1).
- Sessions keep `/unipi:*` text and `unipi:*` custom entries as history; Oino still renders
  the custom entry types (their ids are unchanged). Do not rewrite session files.
- Check: number of `*.jsonl` in source = number copied + number skipped as existing;
  `head -c 300` of one copied file starts with `{"type":"session"`; `oino -r` (or `/resume`)
  lists them for that cwd.
- Not migrated: `$UNIPI/workspace/<uuid>/sessions/` (per-process unipi state),
  `$UNIPI/workspace/<uuid>/state/subagents/sessions/*/index.json` (subagent run indexes of
  old sessions).

## 2. Resources

| Source | Target | Notes |
|---|---|---|
| `$PI_DIR/skills/` | `$OINO/skills/` | copy dirs; a name already in Oino → conflict (keep Oino's) |
| `$PI_DIR/prompts/` | `$OINO/prompts/` | prompt templates, same format |
| `$PI_DIR/themes/*.json` | `$OINO/themes/` | pi theme JSON |
| `$PI_DIR/keybindings.json` | `$OINO/keybindings.json` | same file format; merge per action id (Oino entry wins) |
| `$PI_DIR/AGENTS.md`, `CLAUDE.md` | `$OINO/AGENTS.md` | if Oino has one: append under a `<!-- migrated from pi -->` heading after asking |
| `$PI_DIR/SYSTEM.md`, `APPEND_SYSTEM.md` | `$OINO/SYSTEM.md`, `APPEND_SYSTEM.md` | an existing Oino `SYSTEM.md` wins; show both to the user |
| `$PI_DIR/trust.json` | `$OINO/trust.json` | merge `{"<dir>": true\|false}`; Oino entries win |
| `~/.agents/skills`, `<project>/.agents/skills` | — | read by Oino directly; nothing to copy |
| `$PI_DIR/extensions/*.ts`, `$PI_DIR/npm/`, `$PI_DIR/git/`, `$PI_DIR/bin/` | — | **not migrated**: TypeScript/npm packages do not run in Oino |

## 3. Memory (`unipi.memory`)

- `$UNIPI/memory/<project>/<type>/<id>.md` → `$STATE/unipi.memory/global/memory/<project>/<type>/<id>.md`
  (`<type>` ∈ `preference`, `decision`, `pattern`, `summary`; same front matter `id, title,
  tags, project, created, updated, type`). Copy only `*.md` under a `<type>` dir:
  `cd "$UNIPI/memory" && find . -mindepth 3 -maxdepth 3 -path './*/*/*.md' -print0 | xargs -0 -I{} sh -c 'mkdir -p "$0/$(dirname "{}")" && cp -n "{}" "$0/{}"' "$STATE/unipi.memory/global/memory"`
- An existing Oino memory with the same `<project>/<type>/<id>.md` wins (conflict).
- Not migrated: `$UNIPI/memory/**/memory.db`, `*.db`, `mempalace.yaml` (indexes),
  `$UNIPI/memory-v2-backup-*`, `$UNIPI/memory.db`, `$UNIPI/db/`, `$UNIPI/agent-memory/`
  (empty in 3.0).
- MemPalace: the palace `~/.mempalace/palace` and `~/.mempalace/config.json` /
  `~/.config/mempalace` are shared with Oino as they are — nothing to copy. Optional, only if
  the user wants the copied files re-indexed: one project at a time,
  `mempalace mine "$STATE/unipi.memory/global/memory/<project>" --wing <project> --agent unipi`
  (slow and memory-hungry; skip when `mempalace` is not installed).
- Settings: namespace `memory` (settings-map §2).
- Check: file counts per project match; `/memory status` and `/memory-search <word>` find a
  migrated memory.

## 4. Skills registry and vault (`unipi.skill-registry`)

- `$UNIPI/skill-vault/` (or `$UNIPI_SKILL_VAULT`) → `$OINO/skill-vault/` (vault skills stay
  off until turned on).
- `$UNIPI/config/skills/config.json` → `extensionSettings.skills` (states rewrite in
  settings-map §3; `proxy`, `exposure.*`).
- Check: `/skills` → "Skill settings…" lists them with the migrated on/off states.

## 5. Subagent profiles (`unipi.subagents`)

- `$UNIPI/config/agents/*.md` → `$STATE/unipi.subagents/global/profiles/*.md` (same front
  matter: `display_name, description, tools, disallowed_tools, extensions, skills, model,
  thinking, prompt_mode, enabled`). Skip `*.bak`, `*.pre-*`.
- Project `<project>/.unipi/config/agents/*.md` and
  `$UNIPI/workspace/<uuid>/config/agents/*.md` → `<project>/.oino/agents/*.md`.
- Models in `model:` must exist in Oino (`oino --list-models <model>`); warn otherwise.
- Rewrite `/unipi:*` in them (SKILL.md step 8).
- Check: `/agents` lists them.

## 6. Other modules

| Module (ext id) | Source | Target | Transformation / note |
|---|---|---|---|
| kanboard (`unipi.kanboard`) | `$UNIPI/kanboard/projects/<slug>/` `project.json`, `tasks/*.md`, `attachments/` | `$STATE/unipi.kanboard/projects/<slug>/` | In every `tasks/*.md` front matter: delete `strategy:` and `plan:` lines; in the `run:` block replace `owner: system` with `owner: agent` and `mode: direct` with `mode: none`. Skip `board.lock`; `queues/` and `summaries/` belong to the removed runner (not migrated). `project.json` `root` is the project path (edit it if the project moved). Write a non-empty `run:` block in canonical form, keys in this order: `session`, `pid`, `host`, `mode`, `goal` (`null` when absent), `started`, `owner`. Edit only the front matter (between the first two `---` lines) line by line; never touch the body. Then check with the read-only `OINO_KANBOARD_HOME="$STATE/unipi.kanboard" oino-kanboard --project <slug> validate` (`validate --fix` is a board write: the kanboard write gate refuses it inside an agent session without a `/kanboard-do` grant); report remaining rule problems (e.g. a dependency on a task that does not exist) without fixing them. |
| kanboard | `$UNIPI/kanboard/settings.json` (daemon) | `$STATE/unipi.kanboard/settings.json` | keep `models`, `summaryModel`, `summaryInstruction`; set `piCommand: ["oino"]`; drop `agentCommand`, `defaultStrategy`, `defaultPlan`, `queueMax`, `blocking` (now rejected). Skip `daemon.lock`, tokens (a new token is minted). |
| kanboard | `$UNIPI/config/kanboard/config.json` | `extensionSettings.kanboard` | settings-map §3 |
| long-horizon (`unipi.long-horizon`) | `<project>/.unipi/ralph/<name>.md` (+ `<name>.state.json`), `$UNIPI/workspace/<uuid>/state/long-horizon/ralph/*` | `<project>/.oino/ralph/` | same format; rewrite `/unipi:*` |
| long-horizon | `$UNIPI/ralph/*` (old global ralph files) | ask which project; then `<project>/.oino/ralph/` | otherwise not migrated |
| long-horizon | `$UNIPI/workspace/<uuid>/state/long-horizon/goal.json` (`{version:1, goal}`) | `$STATE/unipi.long-horizon/workspace/<wsid>/goal.json` | only when `goal.status` is `active`, `waiting` or `paused`: set `status` to `"paused"`, remove `lease`, and add every field Oino requires that is missing, with these values: `generation: 0`, `tokenBudget: null`, `tokensBaselinePending: true`, `tokensAtStart: 0`, `tokensNow: 0`, `turn: 0`, `maxTurns: 50`, `noProgressStreak: 0`, `stallCap: 8`, `notMetStreak: 0`, `blockedProposalStreak: 0`, `revision: 0`, `updatedAt: <now ISO>` (unipi 3.0 goals lack `generation`; a goal that does not parse is dropped and overwritten with `{"goal": null}` on the next Oino start in that project). Check: `oino --mode rpc` started **in the project** + `{"type":"prompt","id":"q","message":"/goal-status"}`, or `/goal` interactively, shows the objective; else re-create it with `/goal <objective>` (put the objective in the report). Finished goals: not migrated. |
| long-horizon | `.../state/long-horizon/state.json`, `graph.json` | — | **not migrated**: owner leases and run history of old processes |
| fusion (`unipi.fusion`) | `$UNIPI/config/fusion/preset.json` (`schema_version: 1`) | `$STATE/unipi.fusion/global/preset.json` | same schema; check models exist |
| fusion | `<project>/.unipi/config/fusion/preset.json` | `<project>/.oino/fusion-preset.json` | same schema |
| mcp (`unipi.mcp`) | `$PI_DIR/mcp.json` and `$UNIPI/config/mcp/mcp-config.json` (`{"mcpServers": {…}}`) | `$STATE/unipi.mcp/global/mcp.json` | merge servers by name (unipi's entry wins over pi's; Oino's existing entry wins over both) |
| mcp | `$UNIPI/config/mcp/config.json` (`{servers: {name: {enabled, addedAt}}, sync}`) | `$STATE/unipi.mcp/global/config.json` | same shape; merge |
| mcp | project `.pi/mcp.json`, `.unipi/config/mcp/mcp-config.json` | `<project>/.oino/mcp.json` | merge as above |
| mcp | `$UNIPI/config/mcp/servers.json` (catalog cache) | — | not migrated (re-synced by `/mcp-sync`) |
| hints (`unipi.core`) | `$UNIPI/global/hints/hints.json` | `$STATE/unipi.core/global/hints.json` | same shape (`counts.<id>.{count,last}`); merge, keep the larger `count` / later `last` |
| input-shortcuts | `$UNIPI/config/input-shortcuts.json` `{stash}` | `$STATE/unipi.input-shortcuts/workspace/<wsid>/input-shortcuts.json` | only if `stash` is non-empty; unipi kept one global stash, Oino keeps one per project: ask which project |
| utility images | `$UNIPI/global/image/*` | `$STATE/unipi.utility/global/images/` | generated images (user files) |
| notify, web-api, watchdog, background-tasks, compactor, footer, info-screen, updater, ask-user, command-enchantment, dream, subagents, app-bridge, decision-model, permission | `$UNIPI/config/<ns>/config.json` | `extensionSettings.<ns>` + secrets | settings-map §2–§4; nothing else to copy |
| workflow (`unipi.workflow`) | permission rules inside `$UNIPI/config/permission/config.json` | `extensionSettings.permission` | same keys; global `mode` → `defaultMode` |

## 7. Projects

For each project found in phase 1 (only with the user's go-ahead per project):

| Source | Target |
|---|---|
| `<p>/.pi/settings.json` | `<p>/.oino/settings.json` (settings-map §1) |
| `<p>/.pi/{skills,prompts,themes}/` | `<p>/.oino/{skills,prompts,themes}/` |
| `<p>/.pi/SYSTEM.md`, `APPEND_SYSTEM.md` | `<p>/.oino/` |
| `<p>/.pi/extensions/` | not migrated (TypeScript) |
| `<p>/.unipi/config/<ns>/config.json`, `$UNIPI/workspace/<uuid>/config/<ns>/config.json` | `<p>/.oino/settings.json` `extensionSettings.<ns>` (project scope) |
| `<p>/.unipi/config/agents/` | `<p>/.oino/agents/` |
| `<p>/.unipi/ralph/` | `<p>/.oino/ralph/` |
| `<p>/.unipi-workspace.json` | leave it (Oino ignores it); the user may delete it |
| `AGENTS.md`, `CLAUDE.md` | stay; only the command rename (SKILL.md step 8) |

Project settings only load in a trusted project: make sure `$OINO/trust.json` has the
project (copied from pi's trust in §2) or tell the user to trust it on first start.

## 8. Settings of every module

All namespaces (`hints`, `decision-model`, `permission`, `long-horizon`, `memory`, `dream`,
`utility`, `image`, `skills`, `info-screen`, `subagents`, `background-tasks`, `web-api`,
`ask-user`, `mcp`, `notify`, `kanboard`, `command-enchantment`, `compactor`, `footer`,
`updater`, `input-shortcuts`, `fusion`, `watchdog`, `app-bridge`) follow settings-map §2.
The watchdog session caps are `watchdog.sessionCaps.{maxTurns,maxCostUsd,maxWallMinutes,
maxConsecutiveToolErrors,summaryTurn}`; background task caps are
`background-tasks.{maxFinishedTasks,maxOutputBytes}`.

## 9. Not migrated (always list these that exist, with the reason)

| Source | Reason |
|---|---|
| `$PI_DIR/extensions/*.ts`, `settings.packages`, `$PI_DIR/npm`, `$PI_DIR/git` | TypeScript/npm pi packages cannot run in Oino (WASM extensions only); unipi itself is bundled |
| `herdr` (`$PI_DIR/herdr-agent-state*`, `herdr:*` events) | herdr integration dropped (D22) |
| cron (`$UNIPI/config/cron/`, `$UNIPI/app-host/cron/`, `/unipi:cron`) | no scheduler in Oino (dropped D22) |
| UniPi desktop/phone app: `$UNIPI/app-host/`, `$UNIPI/bridge/`, `$UNIPI/gui/`, `notify/app-endpoints.json`, pairing/devices/push keys | the unipi app is not part of Oino; Oino Desktop/Mobile pair separately |
| tips (`$UNIPI/config/tips/`, `$UNIPI/global/tips/`) | replaced by hints |
| caches and logs: `$UNIPI/cache/`, `$UNIPI/logs/`, `$UNIPI/tool-results/`, `$UNIPI/config/*-cache.json`, `$UNIPI/config/web-api/cache/`, `$UNIPI/global/updater/`, `$UNIPI/global/compactor/session.db`, `$UNIPI/db/`, `$PI_DIR/*crash*` | regenerated by Oino; tied to old versions |
| `$UNIPI/workspace/<uuid>/state/{dream,subagents}`, `long-horizon/{state,graph}.json`, `$UNIPI/workspace/<uuid>/sessions/` | run state of old processes |
| `$UNIPI/config/settings-version.json`, `$UNIPI/state-version.json`, `$UNIPI/config/.backup/` | unipi's own migration ledger |
| `$UNIPI/move-backup/`, `$UNIPI/memory-v2-backup-*`, `$UNIPI/quick-work/`, `$UNIPI/worktrees/`, `$UNIPI/sumbench/`, `$UNIPI/trajectory/`, `$UNIPI/docs/` | backups, scratch and benchmark data of unipi itself; mention so the user can keep them |
| pi-extension private files (`$PI_DIR/antigravity-accounts.json`, `models-store.json`, `omniroute-bridge/`, `.zcode2pi-state.json`, `fff/`) | belong to TypeScript extensions that do not run in Oino |
