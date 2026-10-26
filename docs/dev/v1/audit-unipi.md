# Audit: unipi 3.0.0 inventory (porting reference)

Work id: `audit-unipi`. Branch `v1-alpha`. Status: complete inventory, no code changed.

This document lists everything in unipi 3.0.0 that Oino v1 must port as Rust WASM
component extensions (charter §Vision 2). Use it as the porting reference. Part 1 is
the suite and the shared core. Part 2 has one section for each package. Part 3 has
the cross-package matrices. Part 4 has the derived results: (a) the host capabilities,
(b) the port batches, (c) the Oino built-ins that each package supersedes.

## 0. Scope, sources and method

- Source tree: `/home/oi/Projects/Personal/archived/unipi` at commit `c90133d`
  (version 3.0.0 in every `package.json`). In scope: `packages/*` (24 directories),
  `crates/kanboard`, `packages/kanboard-bin`. Out of scope: `unipi-app/` (the desktop
  and phone UI), `scripts/`, the docs site.
- Unipi's own reference docs were read and checked against the source:
  `docs/reference/{commands,tools,shortcuts,settings,glossary}.md` and
  `docs/architecture/*.md`. Those docs come from the `register*` calls in the source.
  This audit lists what they omit: `/unipi:cron`, `/unipi:dream`,
  `/unipi:visualize-progress`, `/unipi:bg-detach`, `/unipi-app-session`,
  `rename_session`, the command-runner names, and the `pi.events` channels.
- Mechanical scans (grep or perl over all non-test `.ts`) gave the per-package lists:
  `pi.on`/`on(` events, `register*` calls, `ctx.ui.*`, `ctx.*`, `sessionManager.*`,
  `Symbol.for` holders, `UNIPI_EVENTS.*`, the core registries, process spawns, `fetch`,
  env vars and file paths. Tool schemas come from the TypeBox `parameters` blocks.
  Test counts are the `it(`/`test(` calls per package. Treat them as approximate.
- **API version gap.** Unipi pins `@earendil-works/pi-coding-agent` `^0.87.1`. Oino
  targets pi **1.4.0** (charter). Every event that unipi uses exists in pi 1.4.0
  `dist/core/extensions/types.d.ts`, including `session_info_changed`,
  `thinking_level_select`, `ui_prompt_start`/`ui_prompt_end` and
  `session_compact_failed`. `entry_appended`, which `core/command-echo.ts` mentions, is
  a TUI-internal signal and not an extension event. Port to pi 1.4 semantics.
  Port to the 0.87 shape only where the two differ and the unipi behaviour depends on
  the older shape.

### Size

| Area | TS lines (non-test + test) | Test files | Approx. test cases |
|---|---:|---:|---:|
| core | 18 591 | 29 | 315 |
| long-horizon | 14 334 | 25 | 289 |
| utility | 12 924 | 27 | 348 |
| background-tasks | 9 993 | 11 | 24+ (node:test style) |
| app-bridge | 7 918 | 13 | 169 |
| kanboard (pkg) | 7 219 | 15 | 179 |
| memory | 7 007 | 13 | 78 |
| compactor | 6 890 | 12 | 104 |
| notify | 6 560 | 13 | 124 |
| footer | 5 688 | 16 | 140 |
| fusion | 5 638 | 18 | 135 |
| web-api | 5 210 | 8 | 75 |
| info-screen | 4 356 | 2 | 13 |
| mcp | 3 608 | 2 | 10 |
| workflow | 3 381 | 6 | 119 |
| subagents | 3 166 | 5 | 45 |
| dream | 2 653 | 3 | 37 |
| updater | 2 396 | 5 | 39 |
| ask-user | 2 152 | 6 | 38 |
| watchdog | 1 968 | 8 | 43 |
| input-shortcuts | 1 785 | 7 | 35 |
| skill-registry | 1 591 | 2 | 31 |
| autocomplete (command-enchantment) | 1 363 | 2 | 40 |
| btw | 947 | 1 | 8 |
| unipi (umbrella) | 158 | 0 | 0 |
| **crates/kanboard (Rust)** | 6 511 src lines (+ tests) | 15 `.rs` + 4 `.mjs` | 204 `#[test]` |
| root `tests/` | 7 files | | prefix-cache payload/tool-registration, manifest, settings metadata, tui width |

Total: about 160 k TypeScript lines and 18 k Rust lines, including tests.

---

# Part 1 — The suite and shared core

## 1.1 Package map

| Dir | npm name | Extension entry | Bundled skills | Runtime deps | Hard unipi deps |
|---|---|---|---|---|---|
| `core` | `@pi-unipi/core` | none (library) | — | typebox | — |
| `unipi` | `@pi-unipi/unipi` | `index.ts` (umbrella) | — | all | all |
| `workflow` | `@pi-unipi/workflow` | `index.ts` | — | — | core |
| `long-horizon` | `@pi-unipi/long-horizon` | `index.ts` | — | `@resvg/resvg-wasm`, `beautiful-mermaid` | core |
| `memory` | `@pi-unipi/memory` | `index.ts` | `memory` | `js-yaml` | core, info-screen (types) |
| `dream` | `@pi-unipi/dream` | `extensions/dream.ts` | `craft-skill` | — | core |
| `utility` | `@pi-unipi/utility` | `src/index.ts` | `utility`, `summarize`, `image`, `browser`, `cron` | — | core |
| `skill-registry` | `@pi-unipi/skill-registry` | `index.ts` | — | — | core |
| `info-screen` | `@pi-unipi/info-screen` | `index.ts` | — | — | core |
| `subagents` | `@pi-unipi/subagents` | `src/index.ts` | `subagents` | `acorn`, pi-agent-core | core, utility (renderer) |
| `background-tasks` | `@pi-unipi/background-tasks` | `src/index.ts` | — | `turndown`, pi-ai | core |
| `btw` | `@pi-unipi/btw` | `extensions/btw.ts` | `btw` | — | core |
| `web-api` | `@pi-unipi/web-api` | `src/index.ts` | `web` | `defuddle`, `linkedom`, `wreq-js` (native), opt `wigolo-sdk` | core |
| `ask-user` | `@pi-unipi/ask-user` | `index.ts` | `ask-user` | — | core |
| `mcp` | `@pi-unipi/mcp` | `src/index.ts` | `mcp` | — | core |
| `notify` | `@pi-unipi/notify` | `index.ts` | `notify`, `configure-notify` | `node-notifier` | core |
| `kanboard` | `@pi-unipi/kanboard` | `index.ts` | `kanboard` | opt `@pi-unipi/kanboard-<plat>` | core |
| `autocomplete` | `@pi-unipi/command-enchantment` | `src/index.ts` | — | — | core (settings) |
| `compactor` | `@pi-unipi/compactor` | `index.ts` | `compactor` | pi-agent-core | core, info-screen (types) |
| `footer` | `@pi-unipi/footer` | `index.ts` | — | — | core, background-tasks (`getSharedTaskRegistry`) |
| `updater` | `@pi-unipi/updater` | `index.ts` | `configure-updater` | — | core |
| `input-shortcuts` | `@pi-unipi/input-shortcuts` | `src/index.ts` | — | — | core |
| `fusion` | `@pi-unipi/fusion` | `src/index.ts` | — | — | core, subagents, utility (renderer) |
| `watchdog` | `@pi-unipi/watchdog` | `index.ts` | — | — | core, background-tasks |
| `app-bridge` | `@pi-unipi/app-bridge` | `index.ts` | — | — | core |
| `kanboard-bin` | `@pi-unipi/kanboard-{linux-x64,linux-arm64,darwin-x64,darwin-arm64,win32-x64}` | — (prebuilt `unipi-kanboard`) | — | — | — |

`core` also imports `@pi-unipi/fusion/src/preset.js`. This is a type-level cycle that the
port should break.

## 1.2 Umbrella (`packages/unipi/index.ts`)

The umbrella loads 23 modules in a fixed order: workflow, long-horizon, memory, dream,
utility, skill-registry, info-screen, subagents, background-tasks, btw, web-api,
ask-user, mcp, notify, kanboard, command-enchantment, compactor, footer, updater,
input-shortcuts, fusion, watchdog and app-bridge. Before the first module loads, it
does these steps:

1. `withCommandEcho(pi)` wraps `registerCommand`, so that each slash command leaves a
   `❭ /cmd args` UI-only entry (§1.3.11).
2. `installHarnessProvenance(api)` watches `input`, `message_start` and `message_end`
   to tag harness-sent user messages (§1.3.10).
3. `installArbiter(pi)` makes core the only owner of `agent_before_settle` (§1.3.3).
4. `installPendingWorkMonitor(pi)` (§1.3.4).
5. `migrateState()` (a one-time v2→v3 move into `~/.unipi/{global,workspace}`) and
   `sweepOrphanSessions()` (deletes the session dirs of dead pids).
6. A proxy captures every `registerTool`. Each tool is wrapped with
   `withHarnessToolAnnotations` (provenance header for appended annotations). When
   `utility.render.style === "simple"`, every captured tool is registered again with
   `simpleWrapTool` (a collapsed one-line renderer; the schema and `execute` do not
   change) after all modules load.
7. A per-module tracking proxy records load time and the contributions of each module
   (tools, commands, shortcut count) on `globalThis.__unipi_load_times` and
   `__unipi_contributions`. The info screen's Modules page reads them.

**Port note.** In Oino, the host loads each package as its own component. The host
gives the umbrella behaviours (load order, contribution ledger, load timing, a
uniform tool-render wrapper, command echo) as native features or as a "core"
component. They are not a proxy over the API.

## 1.3 `packages/core` subsystems (library used by every package)

Core has no extension entry. It is a set of process-wide registries on `globalThis`
under `Symbol.for(...)` keys, so that two copies of core share one state. A WASM port
cannot share `globalThis` between components. **Each registry below must become
either a host service or one "unipi-core" component with an RPC surface.**

### 1.3.1 Typed event bus (`bus.ts`, `events.ts`)

- `bus.emit(name, payload)`, `bus.get(name)` (last sticky value) and `bus.on(pi, name,
  fn)` (returns an unsubscribe function, and auto-unsubscribes on that pi's
  `session_shutdown`). Listener errors are isolated. Shutdown clears the sticky state.
  Publishers publish again on `session_start`. Holder: `Symbol.for("unipi.bus")`.
- `UNIPI_EVENTS` (all names start with `unipi:`): `module:ready`, `permission:mode:changed`,
  `plan:mode:changed`, `ralph:loop:end`, `ralph:iteration:done`,
  `long-horizon:mode:resolved`, `long-horizon:owner:changed`, `memory:stored`,
  `memory:deleted`, `mcp:server:started`, `mcp:server:error`, `mcp:tools:registered`,
  `compactor:compacted`, `notify:sent`, `ask-user:prompt`, `update:check`,
  `update:available`, `update:applied`, `long-horizon:state`*, `long-horizon:progress`*,
  `kanboard:status`*, `fusion:status`*, `workflow:status`*, `skills:reveal`.
  (* = sticky; `STICKY_EVENTS` has 5 keys. The architecture doc says 4 and is out of
  date: it omits `LH_PROGRESS`.)
- Routing by name, used by notify and the hints registry: pi lifecycle names go to
  `pi.on`, `unipi:*` names go to the bus, and all other names go to `pi.events`.
- Foreign channels that stay on `pi.events`: `herdr:blocked` and `herdr:working`
  (refcounted `{active,label}` claims for the Herdr multiplexer), `rpiv:ask-user:prompt`,
  and the background-tasks API (`unipi-background-tasks:request:v1` /
  `:response:v1` / `:terminal:v1`).

### 1.3.2 Other `Symbol.for` holders (inter-package shared state)

| Key | Owner | Readers | What it is |
|---|---|---|---|
| `unipi.bus` | core | all | bus |
| `unipi.turn.arbiter` | core | LH, kanboard, bg, fusion, subagents | arbiter providers and wait sources |
| `unipi.turn.pending-work-monitor` | core | — | monitor singleton |
| `unipi.evidence.contributors` | core | LH verifier, kanboard | evidence contributors |
| `unipi.harnessMessages.state` | core | utility renderer | harness arm and records |
| `unipi.commandEcho.completers` / `.silent` | core | app-bridge | command echo |
| `unipi.child-usage.listeners` | core | app-bridge | child usage deltas |
| `unipi.remote-dialogs` | app-bridge | ask-user, core dialogs | phone-race dialog runner |
| `unipi.tui.shared-ticker` | core | cards, widgets | one animation interval |
| `unipi.background-tasks.shared-registry` (`-changed`) | background-tasks | footer, watchdog, notify, core work list | live task registry |
| `unipi.subagents.shared-list` / `-manager` / `-subscribe` / `.transcript` | subagents | core work list, tray, app-bridge | subagent runs |
| `unipi.subagents.graph-run-port` | subagents | long-horizon | runs graph steps as background subagents |
| `unipi.fusion.api` | fusion | app-bridge | sidekick status and set pair |
| `unipi.btw.api` | btw | app-bridge | btw from phone |
| `unipi.dream.api` | dream | app-bridge, tray | dream runs |
| `unipi.footer.shared-tps` | footer | app-bridge | live tokens per second |
| `unipi.app-bridge.ui-wrapped` | app-bridge | — | idempotence marker |
| `unipi.harnessToolAnnotations`, `unipi.replyBg.patched`, `unipi.simpleImageCollapseInstalled` | utility | — | idempotence markers |
| `globalThis.__unipi_info_registry` | info-screen | any package | info-screen page registry |
| kanboard API on `globalThis` (`captureToBacklog`) | kanboard | input-shortcuts | add a backlog task from the editor |

### 1.3.3 Turn arbiter (`src/turn/arbiter.ts`)

- Owns the only `agent_before_settle` handler and delivers at most **one** nudge per
  settle. The decision order is fixed: outcome ≠ completed → none; pending messages →
  none; `continue` already true → none; any **wait source** gives a reason → none; ask
  each **nudge provider** (2 000 ms time box each); pick the highest priority (a tie
  goes to the earliest registered); `onDelivered()`; append a `custom_message` entry
  to a *copy* of `event.entries`; return `continue: true`.
- Providers: long-horizon (100), kanboard claims (50), kanboard autowork (40). Wait
  sources: background-tasks (a running task with `triggerOnCompletion`), fusion (a
  non-blocking handoff), subagents (a background run).
- Install is idempotent and does nothing in a child process. The children are
  `UNIPI_FUSION_CHILD`, `UNIPI_SUBAGENT_CHILD` and `UNIPI_KANBOARD_CHILD`
  (`isChildProcess()`). `onSettleDecision()` listeners exist for status displays and
  tests.

### 1.3.4 Pending-work monitor (`src/turn/pending-work.ts`)

While any wait source has a reason, this monitor holds one `herdr:working` claim. It
checks again when the shared work-list changes, with a 1 s poll as a fallback.
`agent_start` clears the claim. The footer's "waiting on …" line, notify's "All done"
logic and the app-bridge `waiting` field use the same rule.

### 1.3.5 Small registries

- **Evidence contributors** (`src/evidence.ts`): named async contributors with a
  2 000 ms time box. They give blocking items to the long-horizon verifier. Kanboard
  adds one blocking item for each open claim.
- **Compaction context** (`compaction-context.ts`): `registerCompactionContext(id,
  () => string|null)`. The compactor puts these blocks at the start of each summary as
  `[Active Work]`. Registered by long-horizon and kanboard.
- **Command runners** (`command-runner.ts`): `registerCommandRunner(name, fn)`,
  `runCommandByName`, `callCommandRunner` (returns a value). The settings-hub action rows
  and the cross-module calls use them. Registered names: `unipi:mcp-add`,
  `unipi:mcp-reload`, `unipi:mcp-settings`, `unipi:mcp-sync`, kanboard
  open/stop-daemon/rotate-token, agents manager, LH mode runner, notify
  set-gotify/set-ntfy/set-telegram/test, fusion preset editor, hints reset, `unipi:async`,
  `unipi:goal-{start,stop,pause,resume,status}`, `unipi:hint`, `unipi:lh-explicit`,
  `unipi:lh-graph`, `unipi:permission-clear-rules`, `unipi:plan-{enter,exit,off}`,
  `unipi:ralph-start`, `unipi:rename-now`, `unipi:skills-editor` and
  `unipi:web-cache-clear`. Pi has no `executeCommand`, so these runners exist.
- **Skill conditions** (`src/skill-conditions.ts`): synchronous predicates that hide a
  bundled skill. Utility's `browser` skill is hidden while the PC browser is off.
- **Move handlers** (`move.ts`): each package registers a handler that rewrites its
  state on `/unipi:move`. The handlers receive `{oldRoot,newRoot,dryRun,backup,log}`.
  Registered by kanboard, memory and utility (pi state).
- **Progress renderer/entries** (`src/tui/progress.ts`, `appendProgress`): a progress
  custom entry. Kanboard uses it.

### 1.3.6 Settings engine, schema, hub and migrations (`src/settings/*`)

- `registerSettings({namespace, defaults, schema, scope?, ...})`. Reads are layered:
  defaults ⊕ global ⊕ project, and the project layer wins. Writes go to exactly one
  scope.
  - Files: `~/.unipi/config/<ns>/config.json` (global) and
    `<project>/.unipi/config/<ns>/config.json` (project).
  - Ledger: `~/.unipi/config/settings-version.json`. Backups:
    `~/.unipi/config/.backup/`.
  - API: `getSettings`, `setSettings`, `unsetSettings`, `openSettingsHub`.
- The schema has sections that hold fields. Field types: boolean, enum (labelled
  options with descriptions), multiselect, string, number (unit, validation), secret
  (masked), order list, model picker, `action` (calls a command runner), page
  (sub-page) and `advanced` (collapsed). Fields can be dynamic. A field can be global
  only or project only. Fields carry badges and hints.
- The **hub** (`src/settings/hub.ts`, 1 281 lines) is one overlay for all namespaces:
  - Search and filter, with the scope toggle `g`.
  - Edit keys: `Enter`/`Tab` edit, `Space` cycle, `u` undo, `d` set default, `R` revert
    to the value at open.
  - Order editors: `Shift+J/K`, `Alt+↑/↓`.
  - Page stack.
  - The `use default` value deletes the project key.
  Utility opens it with `/unipi:settings [search]`.
- Namespaces in 3.0.0: `ask-user`, `background-tasks`, `command-enchantment`,
  `compactor`, `cron`, `decision-model`, `dream`, `footer`, `fusion`, `hints`,
  `image`, `info-screen`, `input-shortcuts`, `kanboard`, `long-horizon`, `mcp`,
  `memory`, `notify`, `permission`, `skills`, `subagents`, `updater`, `utility`,
  `watchdog` and `web-api`. The full key tables are in unipi
  `docs/reference/settings.md`. Part 2 lists the keys for each package.

### 1.3.7 Workspace identity and state layout (`src/workspace/*`)

- Workspace id: a uuid in a `.unipi-workspace.json` marker in the project root. Core
  adds the marker to `.gitignore` and canonicalizes the path with realpath. The id is
  never bound again to a new path, so a moved repo keeps it. Session id:
  `<workspaceId>-<pid>`.
- Layout:

  ```text
  ~/.unipi/config/<module>/…                        global user defaults
  ~/.unipi/global/<module>/…                        cross-project (hints.json, images, model cache)
  ~/.unipi/workspace/<id>/config/<module>/…         project config overrides
  ~/.unipi/workspace/<id>/state/<module>/…          durable project state
  ~/.unipi/workspace/<id>/sessions/<sid>/<module>/  per-session state, GC'd by sweepOrphanSessions
  ~/.unipi/cache/…  ~/.unipi/logs/…  ~/.unipi/tool-results/…  ~/.unipi/bridge/…
  ```

- Helpers: `stateDir(module, scope, cwd)`, `statePath`, `unipiRoot`, `workspaceRoot`,
  `initUnipiDirs`.
- File helpers: `sanitize`, `ensureDir`, `readJson`/`writeJson` (atomic), `tryRead`,
  `tryDelete`, `fileExists`, `resolvePath`.
- Other helpers: `randomId`, `parseArgs`, `getPackageVersion`, `formatTokens`,
  `compareVersions`.

Oino: the charter puts state under `~/.oino/` and `.oino/`. Map `~/.unipi/` →
`~/.oino/unipi/` or straight into `~/.oino/<module>/`. This is still open (§4.4).

### 1.3.8 jev decision-model client (`src/jev/*`)

- `askJev({state, questions, settings, signal})`. Questions have the type `choice`
  (label plus confidence) or `noul` (a number from 0 to 1).
- Transports: TypeSafe native `/v1/systemone`, or the OpenRouter decisions endpoint
  `/api/alpha/decisions`. Time box: 1 000 ms native, 6 000 ms for the decisions
  endpoint.
- Fail-open: every error, timeout or missing key gives `null`.
- Key order: the stored key, then the env vars (`TYPESAFE_API_KEY`,
  `OPENROUTER_API_KEY`), then pi's model registry (`getApiKeyAndHeaders` for a
  provider that matches the base URL).
- Shared settings: namespace `decision-model`, keys `provider`, `model`
  (`typesafe/jev-1.13`), `baseUrl`, `apiKey` and `timeoutMs`. A namespace can override
  them with `decisionModel.source: inherit|custom`.
- Callers: long-horizon (mode judge), workflow (bash risk), utility (rename gate),
  skill-registry (skill relevance) and watchdog (stuck check).

### 1.3.9 Child-agent runtime (`src/child-agent/*`)

Fusion and subagents use this shared runtime for **persistent child pi processes**.

- `pi-spawn.ts` finds the pi CLI. The order is the `UNIPI_SUBAGENT_PI_BINARY` env var,
  then `process.argv[1]`, then the installed package.
- `child-args.ts` passes the lead's `-e/--extension`, `--no-extensions` and `--skill`
  flags to the child, and always adds `--no-skills`.
- `runtime.ts` speaks **pi RPC mode** (`pi --mode rpc`) over stdio JSONL:
  - It sends `prompt` (with `streamingBehavior: followUp`), `steer`, `abort`,
    `set_thinking_level` and `get_last_assistant_text`.
  - It reads the child's event stream and keeps the last 300 events and 4 000 chars of
    tool output for each step.
  - It forwards each child `extension_ui_request` to the lead UI for a blocking
    handoff. For a background handoff, it answers with an automatic deny and a note.
  - Cancel grace: 5 000 ms, then kill.
- `readers.ts` registers the **`read_subagent`** tool, which reads or waits for a
  subagent or sidekick report. It is defined in core and registered while subagents
  or a Fusion pair is active.
- `delivery.ts` delivers a completion message (`sendMessage` with a custom type) when
  a background child finishes.

### 1.3.10 Harness messages (`harness-messages.ts`)

- Tag `unipiHarness = {version:1,id,source,title,synopsis,delivery,severity?}`. The
  `delivery` field is one of `direct`, `steer`, `followUp`, `nextTurn`, `boundary` or
  `before_agent_start`.
- `sendHarnessUserMessage()` arms the exact text, calls `pi.sendUserMessage`, matches
  it in the `input` handler (source `extension`), and attaches the tag at
  `message_end`. The tag is stored in the session file.
- Custom messages carry `details.unipiHarness`. Tool-result annotations carry
  `details.unipiHarnessAnnotations`.
- Utility renders them as a violet panel. Nine packages tag harness text.

**Oino:** make provenance a first-class field on `sendUserMessage`/`sendMessage`
(origin extension id, source label, delivery). The arm/match workaround is then not
needed.

### 1.3.11 Command echo (`command-echo.ts`)

- Wraps `registerCommand`. Each run appends a `unipi-command-echo` entry (UI only).
- The `notify`/`custom` output of a command is recorded as a `COMMAND_OUTPUT_TYPE`
  entry (≤ 4 000 chars, with `screen` set when the command opened a TUI-only screen).
  The app shows this as the command's reply.
- `silenceCommandEcho(name)` turns the echo off for one command, for example
  `/unipi-app-session`.

**Oino:** make this a host feature, because every frontend needs it.

### 1.3.12 Work list and work tray (`src/work/*`)

- `index.ts`: one `WorkItem[]` (kind `bg|subagent|sidekick`, title, dot
  `running|stopped|failed|done`, detail, startedAt). It reads the producers lazily
  from the `Symbol.for` holders.
- `tray.ts`: one bottom pane that **replaces the editor**. Ways to open it:
  - `↓` when the editor is empty and items exist (through `ui.onTerminalInput`).
  - `Shift+↓`, `/unipi:bg-tasks` and `/unipi:subagents`.
  Behaviour:
  - Tabs come from `registerWorkTrayTab` (Background tasks, Subagents, Dream).
  - `←/→` switch tabs and `Esc` closes the tray.
  - A pane can capture the arrow keys.
  - While the tray is closed and has items, one dim hint line shows under the editor.
- `pane-kit.ts`: shared list and detail pane rendering.

### 1.3.13 Unicrab hints (`src/hints/*`, settings `hints`)

- 127 tips in 9 categories. The startup tip is the one with the lowest show count.
  After an upgrade, "what's new" tips show first.
- Moment tips (once each per session): late night, 50 tool calls with no error,
  `rm -rf`, force push, a hot cache, a finished goal, context ≥ 70%, bash ≥ 30 s, and
  3 tool errors in a row. At most 4 event tips per session.
- Uses `turn_end`, `tool_*`, `session_compact_failed` and others.
- UI:
  - Hint widget above the editor (`setWidget`).
  - Start screen with Unicrab pixel art through `setHeader`. It needs 40 or more
    columns; at 72 or more, the large crab shows. A Kitty-graphics image variant
    exists.
  - Browser overlay (`/unipi:hint`).
- Keys: `Alt+H` (next tip), `Alt+Shift+H` (previous tip).
- Store: `~/.unipi/global/hints/hints.json`.
- Command: `/unipi:hint [next|reset]`.
- Utility installs the engine.

### 1.3.14 TUI kits

`tui-overlay.ts` (box drawing and theme helpers), `src/tui/kit.ts`, `hub-kit.ts`
(hub widgets), `viz.ts` (`bigText`, `spark`, `columns`, `brailleArea`, `gauge`,
`shareBar`, `grid`), `progress.ts`, `shared-ticker.ts` (one interval for all spinners),
`spinner-line.ts` and `tui-width.ts`. These are frontend code. **They do not port as
extension code.** Frontends own the rendering. Extensions emit data (§4.1 C17).

### 1.3.15 Other core helpers

- `src/attach/detect.ts`: finds file paths in pasted text and turns them into
  `[Image #N]` / `[File #N]` tokens.
- `model-cache.ts`: `~/.unipi/config/models-cache.json`. Utility writes it on each
  session start from the live registry, with the image models added.
- `bounded-output.ts`: bounded tool output (head and tail, then a spill file).
- `detachable-bash.ts`: wraps pi's bash operations, so that a running foreground bash
  child can be **adopted** by the background-task registry (watchdog and
  `/unipi:bg-detach`).
- `remote-dialogs.ts`: `raceRemote(runTui)`. The TUI dialog runs against a phone
  answer, and the first answer wins.
- `src/child-usage.ts`: `unipi-child-usage` custom entries (`{v:1,source:
  subagent|fusion|bg-task,id,usage}`) in the lead session, so that the lead's cost
  includes the children. Live listeners exist.
- `src/compaction-savings.ts`: computes the tokens and dollars that compaction saved,
  from the entries.
- `utils.ts`: `herdr:blocked`/`herdr:working` emitters, `execSync("pi --version")` and
  more.
- `kanboard-label.ts` and `src/package-colors.ts`: display helpers.

### 1.3.16 Core-registered surface

- Command: `/unipi:hint`.
- Tool: `read_subagent`.
- Shortcuts: `Alt+H`, `Alt+Shift+H`.
- Entry renderer: `COMMAND_OUTPUT_TYPE`.
- Entries appended: `unipi-child-usage`, `unipi-command-echo`, command-output and
  progress entries.
- Events used: `agent_before_settle`, `agent_settled`, `agent_start`,
  `before_agent_start`, `input`, `session_start`, `session_shutdown`, `tool_call`,
  `tool_execution_start`, `tool_execution_end`, `tool_result`, `turn_end`,
  `message_start` and `message_end` (provenance).

---

# Part 2 — Per-package inventory

Each section has the same fields: **Purpose**, **Features**, **Commands**, **Tools**,
**Keys**, **Settings/config**, **State**, **Pi API**, **UI (as data)**, **External**,
**Inter-package**, **Tests**. The "Pi API" field lists the events with `on:`, the
`pi.*` methods with `pi:`, the `ctx.*` uses with `ctx:` and the `ctx.ui.*` uses with
`ui:`.

## 2.1 workflow — permission gate and plan mode

- **Purpose:** checks each tool call against a permission mode (`ask|auto|full`), and
  gives a read-only plan mode with a review screen.
- **Features:**
  - Gate order for each tool call:
    1. Saved rules. A deny rule blocks in all modes; an allow rule runs the call.
    2. Read-only tools run.
    3. A `write` or `edit` outside the project or temp folder shows a prompt in `ask`
       and `auto` modes.
    4. A dangerous `bash` pattern (for example `rm -rf`, `sudo`, `git push --force`)
       shows a prompt. In `auto` mode, jev must say `safe` with confidence ≥
       `jevConfidence`.
    5. Other tools run in `auto` and `full` modes.
  - Prompt options: Allow once, Always allow, Deny, and Deny with note. The note goes
    into the block reason.
  - With no UI: the gate blocks dangerous bash and deny rules, and runs everything
    else.
  - Plan mode:
    - The plan file is `docs/plans/<date>-<session-id>.md`.
    - The agent can write only the plan file, and `bash` must be read-only.
    - Each turn gets a reminder message. The system prompt does not change.
    - Review: Approve & implement (sends the plan as the next user message), Keep
      planning (sends feedback), or Discard.
- **Commands:** `/unipi:plan [on|off|view|approve]`,
  `/unipi:permission [ask|auto|full|status]`.
  Runners: `unipi:plan-enter`, `unipi:plan-exit`, `unipi:plan-off`,
  `unipi:permission-clear-rules`.
- **Tools:** `plan_submit {}`. It works in plan mode only and opens the review.
- **Keys:** `Alt+P` toggles plan mode. `Alt+M` cycles the permission mode.
- **Settings (`permission`):** `mode` (project only), `defaultMode` (global, `auto`),
  `jevJudge` (true), `jevConfidence` (0.7), `rules[]`, `decisionModel.*`.
  `UNIPI_DEBUG_PERMISSION=1` logs to `~/.unipi/logs/permission.log`.
- **State:**
  - `PLAN_STATE_ENTRY` custom entry, read back from `sessionManager.getEntries` on
    `session_start`.
  - Plan file in the project.
  - Rules in the settings file.
- **Pi API:**
  - on: `before_agent_start`, `session_start`, `session_shutdown`, `tool_call`
    (block/allow).
  - pi: `appendEntry`, `sendMessage` (`PLAN_MESSAGE_TYPE`), `registerShortcut`.
  - ctx: `sessionManager.getEntries`, `getSessionId`, `cwd`, `hasUI`.
  - ui: `select`, `input` (×3), `notify`, `setStatus("unipi-workflow")`, `custom`
    (plan review).
- **UI:**
  - Status slot `unipi-workflow`.
  - Approval dialog (select: 4 options; input: deny note).
  - Plan review custom overlay: markdown plan plus actions approve, keep planning
    (input) and discard.
  - Bus: `WORKFLOW_STATUS` (sticky, read by the footer), `PLAN_MODE_CHANGED`,
    `PERMISSION_MODE_CHANGED`.
- **External:** jev HTTP. FS write for the plan file.
- **Inter-package:** core (jev, harness, settings, bus). The footer shows the PLAN
  badge and the mode.
- **Tests:** 6 files, about 119 cases: bash classifier, rules, gate order, plan.

## 2.2 long-horizon — goal / ralph / swarm / graph owners

- **Purpose:** multi-turn autonomous work, with one owner for each session, a verifier
  that judges completion, budgets, and a durable agent graph.
- **Features:**
  - **Gate** at each `before_agent_start`. The order is: child → `none`; an explicit
    command; the active owner; the jev judge (off by default, threshold 0.8, one
    cached answer for 10 s); `defaultMode`.
  - The gate adds the mode prompt section. It filters the tools with
    `setActiveTools`, and also filters the provider payload in
    `before_provider_request`, so that the list stays stable for the cache.
  - **Owner coordinator:**
    - One active owner and one parked owner.
    - Leases have `{ownerId, generation}`. A tool call with an old generation is
      rejected.
    - History of 10 owners.
    - State: `~/.unipi/workspace/<id>/state/long-horizon/state.json`.
  - **Goal:**
    - Caps: turns 50/200, stall cap 8/50, token budget ≥ 1 000, objective ≤ 500
      chars.
    - Status audit every 5 turns.
    - Verifier (separate model call, 30 s) reads an evidence brief: 100 changes,
      4 000 chars and the last 5 messages.
    - Verdicts: `met`, `not_met`, `impossible`, `inconclusive`. A run pauses after 5
      `not_met` or 2 `inconclusive` verdicts in a row. 3 blocked proposals end as
      `blocked`.
    - Task gate: pending todos give a `[Task reminder]` (2 per goal).
    - Durable continuation outbox `pendingContinuation` in `goal.json`, with 2
      recovery re-deliveries.
    - Wait backoff: 5 s × 2ⁿ, up to 5 min.
    - Progress estimate (`goalProgress`, `progressModel`). The user sees it; the
      model does not.
  - **Ralph:**
    - Task file `.unipi/ralph/<name>.md` with a `- [ ]` checklist, 2 items per
      iteration, and a reflection every 5 iterations.
    - `ralph_done` checks the file.
  - **Agent graph (swarm/graph):**
    - Maka model. Graph state in `graph.json` (the current graph plus 20 past graphs,
      written atomically).
    - Each step runs as a background subagent through `unipi.subagents.graph-run-port`.
    - Handoff caps: 16 KB per input, 48 KB total. Stored result: 64 KB (16 KB when
      archived).
    - Interrupted steps resume once on the next `session_start`.
    - Wake aggregation: 400 ms. Graph and swarm have different checkpoint rules.
    - Limits per update: 32 work items, 20 stop targets, 64 inputs.
  - **Runaway guard:** 6 detectors (same action, an ABAB cycle, an unchanged
    action/result, the same error family, the same result, status polling). It
    sends at most one steer per turn.
  - Compaction brief provider: the goal or ralph state goes into `[Active Work]`.
  - Nudge provider at priority 100, with a one-slot nudge stash. A kickoff from an
    idle session sends a user message directly.
  - Mermaid progress chart → SVG (beautiful-mermaid) → PNG (resvg-wasm) → inline
    terminal image (`/unipi:visualize-progress`).
- **Commands:**
  - `/unipi:goal`, `/unipi:ralph` (`start <name> [content]`), `/unipi:swarm` and
    `/unipi:graph`. Each takes `<prompt>|status|stop|resume|clear`, and graph and
    swarm also take `history`.
  - `/unipi:regular` and `/unipi:visualize-progress`.
  - Runners: `unipi:goal-{start,stop,pause,resume,status}`, `unipi:ralph-start`,
    `unipi:lh-explicit`, `unipi:lh-graph`, LH mode runner, `unipi:async`.
- **Tools:** each tool is visible only in its mode. Key schemas:
  - `create_goal {objective: string, token_budget?: int≥1000}`.
  - `get_goal {}`.
  - `update_goal {mode: "status"|"token_budget", status?: "complete"|"blocked",
    summary?, safety_refusal?: bool, token_budget?: int|null, expected_goal_id?,
    expected_updated_at?}`.
  - `ralph_done {}`, `loop_status {}`.
  - `update_agent_graph {operation?: add_work|stop|finish, task?, add_work?:
    [{work_id?, instruction, profile?, operator_id?, input_ids?: string[],
    selected_result_inputs?: [{source_graph_id, result_id}], replaces?}] (≤32),
    stop?: [{target_id, reason}] (≤20), finish?: {result_ids[], reason}}`.
  - `view_agent_graph {graph_id?, history?: bool}`.
  - `yield_agent_graph {reason?}`. It returns `terminate`.
  - `agent_output {id, graph_id?, max_bytes?}`.
  - `agent_swarm_status {}`.
  - `todowrite {todos: [{content, status, priority}]}` (≤ 50 items, 200 chars each,
    in all 4 modes).
- **Keys:** none.
- **Settings (`long-horizon`):** `judge.enabled` (false), `judge.threshold` (0.8),
  `defaultMode` (`none`), `verifierModel`, `goalProgress` (`loop|status|off`),
  `progressModel`, `decisionModel.*`. `UNIPI_LH_ALLOW_CHILD` is a test override.
- **State:** `state.json`, `goal.json` and `graph.json` under the LH state dir;
  `.unipi/ralph/<name>.md` (and the `ralph/` state folder); the `long-horizon-decision`
  custom entry; the `unipi:lh-continue` custom message type.
- **Pi API:**
  - on: `before_agent_start`, `agent_start`, `agent_end`, `agent_settled`,
    `before_provider_request` (filters the tools in the payload), `tool_call`
    (lease/generation checks), `tool_execution_start`, `tool_execution_end`
    (runaway), `session_compact`, `session_start`, `session_shutdown` (freezes the
    graph).
  - pi: `registerTool`, `registerCommand`, `getActiveTools`, `setActiveTools`,
    `appendEntry`, `sendUserMessage`.
  - ctx: `isIdle`, `model`, `modelRegistry` (verifier credentials, judge),
    `mode`, `cwd`, `hasUI`.
  - ui: `notify`, `setStatus` (×2), `custom` (visualize overlay).
- **UI:**
  - Status slots for the mode and goal.
  - Visualize overlay: a graph or progress chart as an inline image, or a text
    fallback.
  - Bus sticky `LH_STATE` (`{mode, owner, status, turn, budget…}`, read by the footer
    and the kanboard monitor) and `LH_PROGRESS` (progress snapshot, read by the app).
  - Bus one-shot: `LONG_HORIZON_MODE_RESOLVED`, `LONG_HORIZON_OWNER_CHANGED`,
    `RALPH_ITERATION_DONE`, `RALPH_LOOP_END` (notify).
- **External:**
  - Model calls: the verifier and progress estimate through the model registry, and
    the judge through jev HTTP. `typesafe.ts` also has a chat/completions fallback.
  - Subagent spawns, through the subagents port.
  - In-guest WASM: resvg, and mermaid layout.
- **Inter-package:**
  - Requires core (arbiter, evidence, compaction-context, jev, harness).
  - Soft dependencies: subagents (graph port), kanboard (evidence, status),
    compactor (`COMPACTOR_COMPACTED`).
- **Tests:** 25 files, about 289 cases.

## 2.3 memory — MemPalace-backed cross-session memory

- **Purpose:** facts, decisions, preferences and patterns that last across sessions.
- **Features:**
  - Markdown memory files at `~/.unipi/memory/<project>/<type>/<id>.md` with YAML
    front matter. The types are preference, decision, pattern and summary.
  - Each file is indexed into a **MemPalace** palace (semantic search, shared with
    other tools).
  - Reads go through one lazy, warm `mempalace-mcp --palace <p> --read-only` stdio
    server per session. It unloads after 10 min idle (≈300 MB loaded).
  - Write path, in order:
    1. The MemPalace daemon `/jobs` queue (HTTP) when the daemon runs.
    2. Else `mempalace mine` run directly.
    3. Else a pending file that the next session replays.
  - `MALLOC_ARENA_MAX=2` is set on every MemPalace child.
  - Auto-install: `uv tool install mempalace`. Daily PyPI version check and upgrade.
  - Recall reminder on the first turn, with optional `mempalace wake-up --wing
    <project>` output.
  - End-of-task save pass:
    - `side`: an in-memory `createAgentSession` side session reads the run and stores
      facts.
    - `reminder`: an end-of-task note asks the agent to save.
    - `off`.
  - v2→v3 migration with backups.
  - Writes and deletes emit `MEMORY_STORED` and `MEMORY_DELETED`.
- **Commands:**
  - `/unipi:memory [status|migrate|recall on|off|write on|off]`
  - `/unipi:memory-process <text>`, `/unipi:memory-consolidate`
  - `/unipi:memory-search <term>`, `/unipi:global-memory-search <term>`
  - `/unipi:memory-forget <title>`, `/unipi:global-memory-list`
- **Tools:**
  - `memory_store {title, content, tags?: string[], type?: preference|decision|pattern|summary}`
  - `memory_search {query, limit?=10, scope?: all|project (default all)}`
  - `global_memory_search {query, limit?}`
  - `memory_delete {title?|id?}`
  - `memory_list {}`, `global_memory_list {}`
  - `memory_store` and `memory_delete` are hidden when `write` is off.
- **Keys:** none.
- **Settings (`memory`):** `recallAtStart` (true), `write` (true), `wakeUp` (true),
  `saveMode` (`side|reminder|off`), `autoStartDaemon` (false),
  `mempalaceAutoUpdate` (true).
  - External opt-out: `~/.config/mempalace/agent-hooks.json` `enabled:false`.
  - `UNIPI_DEBUG_MEMORY=1` logs to `~/.unipi/logs/memory.log`.
  - `MEMPALACE_DAEMON_STATE_ROOT` sets the daemon state root.
- **State:** the memory files, the palace (`~/.mempalace/…`), the pending-writes
  file, the daemon `endpoint.json`, the migration ledger, the UI card entries
  (`CARD_CUSTOM_TYPE`, `SAVE_CARD_TYPE`), and the custom messages `RECALL_CUSTOM_TYPE`
  and `RETRO_CUSTOM_TYPE`.
- **Pi API:**
  - on: `before_agent_start` (×2: recall and tool exposure), `agent_start`,
    `agent_end` (save pass), `tool_call`, `session_compact`, `session_start`,
    `session_shutdown`.
  - pi: `registerTool`, `registerCommand`, `appendEntry`, `sendMessage`,
    `sendUserMessage`, `getActiveTools`, `setActiveTools`, `getAllTools`,
    `getThinkingLevel`.
  - ctx: `getSystemPrompt`, `model`, `modelRegistry.getApiKeyAndHeaders`,
    `sessionManager.getBranch`, `getHeader`, `thinkingLevel`.
  - ui: `confirm` (×3: migrate), `notify`, `setStatus("unipi-memory")`.
  - Side session: `createAgentSession` + `SessionManager.inMemory`, with
    `createExtensionRuntime`, `ResourceLoader` and the tool definitions.
- **UI:** status slot (`🧠 …` progress), memory cards (UI-only entries), migration
  confirm dialogs, a recall reminder (a custom message that the model sees), and an
  info-screen "Memory" group.
- **External:**
  - Binaries: `mempalace`, `mempalace-mcp`, `uv`, `python`, `which`.
  - HTTP: daemon `/jobs` and PyPI.
  - FS: `~/.unipi/memory`, `~/.mempalace`.
  - A long-lived stdio JSON-RPC child.
- **Inter-package:** core. Dream drives memory's tools in its child (`-e` memory
  entry). The compactor `COMPACTOR_COMPACTED` event triggers re-recall.
- **Tests:** 13 files, about 78 cases. Root `packages/memory/tests` runs in the root
  test script.

## 2.4 dream — background reflection over past sessions

- **Purpose:**
  - Find repeated struggles in past sessions.
  - Write memory lessons from them.
  - Propose skills or checks for the user to approve.
- **Features:**
  - Off by default.
  - On each open with `enabled`, check if a dream is due: `minSessions` (5) new
    struggle sessions and `minGapHours` (12).
  - Build scrubbed digests: failed tool calls plus their recovery, and user
    corrections.
  - Spawn a **detached** `pi -p --mode text --no-extensions --no-skills
    --no-context-files --skill <craft-skill> -e <memory entry> --session-dir <staging>
    --thinking <lvl> [--model]` with a prompt file.
    - Runtime cap: `maxRuntimeMin` (40).
    - Git is used for the staging work.
  - The child applies memory edits itself. Skills and checks are only *proposed*.
  - The next open shows a "Dream report ready" card. The user approves or rejects with
    `/unipi:dream`.
  - An approved skill lands in `.agents/skills/` only if `craft-skill check` passes.
  - Work-tray **Dream** tab:
    - Rows show runs: status, start, duration, digests and pending proposals.
    - The detail shows the report, the proposals, and the child's live trajectory and
      log.
    - Keys: `o` full report, `s` stop, `r` run now, `1–9` pick a proposal, `a`/`x`
      approve/reject (press twice), `d` dismiss.
- **Commands:** `/unipi:dream [tray|report|approve <n>|reject <n>|run|stop]`.
- **Tools:** none. The bundled skill is `craft-skill` (scripts `scaffold` and `check`).
- **Keys:** the tray keys above.
- **Settings (`dream`):** `enabled` (false), `minSessions` (5), `minGapHours` (12),
  `model`, `thinking` (medium), `skillsTarget` (`.agents/skills`), `maxRuntimeMin` (40).
  `PI_DREAM_BIN` and `PI_DREAM_STAGING` are overrides.
- **State:** `stateDir("dream","state")`: `state.json`, `run.json`, `trajectory.log`,
  `DREAM_REPORT.md`, and the staging sessions. `REPORT_CARD` custom entry.
- **Pi API:**
  - on: `session_start`, `session_shutdown`.
  - pi: `registerCommand`, `appendEntry`.
  - ui: `notify`, plus the tray tab through core.
  - Timers: `setInterval` ×2 to poll the run.
- **UI:** the report card entry, the tray tab (list and detail panes), and the
  `unipi.dream.api` for the app.
- **External:**
  - A detached `pi` child process with a timeout.
  - `git`, and `bash` for the report scripts.
  - FS: read the session files of past sessions (the pi sessions dir).
- **Inter-package:** core, memory (child entry). Exposes `unipi.dream.api`.
- **Tests:** 3 files, about 37 cases.

## 2.5 utility — settings hub, rendering, naming, attachments, images, maintenance

- **Purpose:** the grab-bag module. It owns the settings hub command, transcript
  rendering, session auto-naming, `/unipi:answer`, attachments, image tools,
  doctor/cleanup/move, the cron window and the browser-state context.
- **Features:**
  - **Settings hub** command. The hub itself is in core.
  - **Rendering:**
    - `render.style`: `simple` (collapsed one-liners, the default), `regular`
      (pi's own) and `advanced` (syntax colors, diffs, test summaries).
    - Registers again the pi tools `read`, `bash`, `edit` and `write`, only for the
      renderers (`createReadToolDefinition`, `createLocalBashOperations` and so on).
    - Registers `registerToolRenderer` and `registerMarkdownTransformer`.
    - Registers renderers for 10 known harness custom types (violet harness panel)
      and patches the user-message card.
    - Delegated-step renderer (`src/render/delegated.ts`), which fusion and
      subagents import.
    - Image collapse.
  - **Auto-rename:**
    - Runs after each round that ends with no error.
    - jev answers two questions: was it a real task, and did the topic change.
    - A separate one-tool in-memory session (`createAgentSession`, tool
      `rename_session {name}`) writes the name.
    - It never overwrites a name set with `/name`.
    - Herdr pane title sync (`pane.report_metadata` over `HERDR_SOCKET_PATH`, 500 ms
      timeout).
  - **`/unipi:answer`:**
    - `reply`: a scroll view of the last reply with a fixed input.
    - `questions`: the editor is filled with `Q1…/A1:` pairs. `Ctrl+G` opens
      `$EDITOR`.
    - `web`: a local HTTP form on 127.0.0.1. Over SSH, it prints an `ssh -L` hint and
      tries port 47321 first.
    - A hint line above the editor shows after a reply that has questions.
  - **Attachments:**
    - A pasted or dropped path becomes an `[Image #N]` or `[File #N]` token.
    - `Backspace` deletes a whole token.
    - On submit, the tokens expand: images as image content, files as `[File #N:
      path]`.
    - Kitty, Ghostty, iTerm2 and WezTerm show previews.
    - The `ATTACHMENTS_ENTRY` custom entry records them.
  - **Image tools:**
    - Transports: OpenRouter, fal.run, any pi provider in the OpenAI images format,
      or a custom `baseUrl`.
    - Generated images are saved to `~/.unipi/images`.
    - `image_recognize` hides while the session model reads images.
  - **Model cache:** writes `models-cache.json` and `image-models-cache.json` on each
    session start.
  - **`/unipi:continue`, `/unipi:retry`:** start a turn with no text
    (`unipi-continue` custom message).
  - **`/unipi:summarize [question]`:** runs the bundled `summarize` skill on the last
    reply. `summarize` is must-show by default.
  - **`/unipi:doctor`:** checks the folders, config, Node, model cache, Decision Model
    and skill exposure.
  - **`/unipi:cleanup [--dry-run] [--yes]`:** deletes only the targets on an
    allowlist, for example tool outputs and `unipi-*` temp files older than 7 days.
  - **`/unipi:move [<old-path>] [--dry-run]`:**
    - Links the state again after a project move. It runs every registered move
      handler and rewrites pi's own state: the sessions dir and `trust.json`.
    - Git checks.
    - Log: `~/.unipi/logs/move-<ts>.log`. Backups: `~/.unipi/move-backup/<ts>/`.
    - Opens an orphan picker when no path is given.
  - **`/unipi:cron`:**
    - A thin window onto `unipi-host cron … --json`, which comes from the unipi-app
      host.
    - Subcommands: `list`, `show`, `runs`, `run`, `stop`, `enable`, `disable`,
      `pause`, `resume`, `status`.
    - Argument completions list the job ids.
    - Bundled `cron` skill.
  - **Browser state:**
    - Reads `~/.unipi/app-host/config.json` and `unipi-host browser profiles --json`
      (cached for 1 min).
    - Adds the "PC browser on" context line.
    - Shows or hides the `browser` skill through `registerSkillCondition`.
- **Commands:**
  - `/unipi:settings [search]`, `/unipi:continue`, `/unipi:retry`
  - `/unipi:answer [reply|questions|web]`, `/unipi:summarize [q]`
  - `/unipi:doctor`, `/unipi:cleanup`, `/unipi:move`, `/unipi:cron`
  - Runner: `unipi:rename-now`.
- **Tools:**
  - `image_generate {prompt, model?}`
  - `image_edit {prompt, image (path|data URL|base64), model?}`
  - `image_recognize {image, prompt?, model?}`
  - Internal: `rename_session {name}`, in the naming side session only.
  - Re-registered: `read`, `bash`, `edit`, `write` (render only).
- **Keys:**
  - `Backspace` token delete, observed through `onTerminalInput`.
  - Paste handling.
  - `Ctrl+G` in answer-questions.
- **Settings:**
  - `utility`: `rename.auto`, `rename.model`, `rename.herdrSync`, `answer.method`,
    `answer.hint`, `answer.port`, `render.style`, `attachments.enabled`,
    `attachments.preview`.
  - `image`:
    - Each of `generate`, `edit` and `recognize` has `enabled`, `model`, `baseUrl`
      and `apiKey`.
    - `generate` and `edit` also have `api`.
    - Other keys: `generate.outputDir`, `generate.saveToDisk`,
      `recognize.systemPrompt`, `keys.openrouter`, `keys.fal`.
  - `cron`: settings for the cron window.
  - Legacy: `.unipi/config/util-settings.json`, `.unipi/config/badge.json`.
- **State:**
  - Custom entries `AUTO_NAME_ENTRY` and `ATTACHMENTS_ENTRY`.
  - Files: the model caches, `rename.log`, `~/.unipi/images`, and the move logs and
    backups.
  - Pi state that `/unipi:move` touches: the pi sessions dir and `trust.json`.
- **Pi API:**
  - on: `session_start` (×8), `session_shutdown`, `before_agent_start` (×3),
    `agent_start`, `agent_end` (×4), `input` (×3: attachments expand, provenance,
    answer), `message_start`, `message_update`, `message_end`, `model_select`,
    `tool_call`.
  - pi: `registerTool`, `registerCommand`, `registerMessageRenderer`,
    `registerToolRenderer`, `registerMarkdownTransformer`, `appendEntry`,
    `sendMessage`, `getActiveTools`, `setActiveTools`, `getCommands`,
    `getSessionName`, `setSessionName`.
  - ctx: `isIdle`, `waitForIdle`, `model`, `modelRegistry`,
    `sessionManager.buildContextEntries`, `mode`.
  - ui: `confirm`, `select`, `notify` (×31), `setWidget` (`HINT_WIDGET`, `WIDGET`),
    `custom` (answer reply view and others), `getEditorText`, `setEditorText`,
    `onTerminalInput`.
  - In-process sessions: `createAgentSession` + `SessionManager.inMemory`.
- **UI:**
  - Answer-hint widget.
  - Answer reply overlay: a scrollable markdown view plus an input.
  - Doctor report and cleanup preview (notify text plus a confirm).
  - Move preview: a grouped diff, a confirm, and an orphan picker (select).
  - Tool renderers in 3 densities. Harness panels. Inline image previews (Kitty
    protocol). Markdown transformer.
- **External:**
  - HTTP: OpenRouter and fal.run images, chat/completions for recognize.
  - A local HTTP server (`node:http`) for the answer web form.
  - Unix socket client to Herdr.
  - Processes: `git`, `unipi-host`, `$EDITOR`, `xdg-open`/`open`.
  - Clipboard images for paste (through pi).
  - Env: `HERDR_ENV`, `HERDR_SOCKET_PATH`, `HERDR_PANE_ID`, `HERDR_TAB_ID`,
    `UNIPI_DEBUG_RENAME`, `COLORTERM`, `PI_AGENT_DIR`, `PI_CODING_AGENT_DIR`.
- **Inter-package:** core (hub, hints install, jev, move registry, skill conditions).
  Exports the delegated renderer and `withHarnessToolAnnotations` and
  `simpleWrapTool` to the umbrella, fusion and subagents.
- **Tests:** 27 files, about 348 cases.

## 2.6 skill-registry — skill exposure control

- **Purpose:** controls which skills the model sees, per scope, with a stable listed
  set for the prefix cache.
- **Features:**
  - Per-skill `enabled` and `mustShow` in global or project scope. A disabled skill
    also blocks `/skill:name`.
  - Vault `~/.unipi/skill-vault/` (`UNIPI_SKILL_VAULT`), up to 3 levels deep. Vault
    skills are off by default. They are added through `resources_discover →
    {skillPaths:[vault]}`.
  - **Judged exposure** on the first real prompt (greetings are skipped):
    1. Skills that the prompt names are kept.
    2. Must-show skills are kept.
    3. jev scores the others: ≥ threshold, best first, up to `maxSkills` (12).
    4. A system-prompt section names the hidden skills and their folders.
    The set is then frozen for the session.
  - On later prompts, a reveal message names at most 5 hidden skills that the prompt
    names or that jev finds relevant (recheck).
  - `unipi:skills:reveal {names}` bus event from other packages, for example
    kanboard.
  - Respects the core skill conditions.
- **Commands:** `/unipi:skills`, which opens the hub at the Skills group. Runner:
  `unipi:skills-editor`.
- **Tools:** none.
- **Keys:** in the skill grid: arrows, `←→/Tab` columns, `Space` toggle, `d` clear,
  `g` scope, `p` proxy, letters to filter, `Enter` save, `Esc` cancel.
- **Settings (`skills`):** `proxy` (false), `exposure.mode` (`judged|all|off`),
  `exposure.threshold` (0.8), `exposure.maxSkills` (12), `exposure.recheck` (true),
  `states{}`, `decisionModel.*`.
- **State:** `SKILLS_JUDGED_ENTRY` and `SKILLS_REVEALED_ENTRY` custom entries (the
  frozen set is rebuilt from them), and the custom message `unipi-skills-revealed`.
- **Pi API:**
  - on: `resources_discover`, `session_start`, `input`, `before_agent_start` (shapes
    `systemPromptOptions.skills` and the sections).
  - pi: `registerCommand`, `appendEntry`, `sendMessage`, `getCommands`.
  - ctx: `sessionManager.getEntries`, `getSessionId`.
  - ui: `custom` (skill grid), `notify`, `setStatus("skills")`.
- **UI:** a skill grid overlay (rows: skill and source tag; columns E/M; tri-state per
  scope; filter line) and a status slot.
- **External:** jev HTTP. FS: the vault and the skill dirs.
- **Inter-package:** core. Kanboard emits `SKILLS_REVEAL`.
- **Tests:** 2 files, about 31 cases.

## 2.7 info-screen — dashboard and startup splash

- **Purpose:** session cost and usage, the context bucket, compaction savings and
  module state.
- **Features:**
  - Startup splash (`bootMode` `on|off|auto-close`, `bootTimeoutMs` 2500). It shows
    the ready time, the resumed session, today's spend and any update. It does not
    take key input in auto-close mode, and waits under other overlays.
  - `/unipi:info [page]` dashboard pages:
    - This session, Usage (today/week/month/all, a 30-day chart, model share)
    - Tools, Skills, Modules (contributions and load times)
    - Compactor savings (g/p/s scopes)
    - MCP, Memory, Web, Updates, Keys
  - **Context bucket:** the parts are the system prompt, tool schemas, summaries,
    prompts, replies and tool results. The page counts chars ÷ 4, scaled to pi's
    count.
  - Usage parser over `~/.pi/agent/sessions/**` with a cache per file. It writes the
    cache at most every 30 s.
  - Page registry (`globalThis.__unipi_info_registry.registerGroup({id,name,icon,
    priority,config:{stats[]},dataProvider,render?})`). Data must be JSON-safe.
    Memory cache 5 s. Snapshots in `~/.unipi/cache/info-screen.json` and
    `~/.unipi/cache/info-screen/<workspace>.json`.
  - Collects `MODULE_READY` events for 150 ms, then refreshes.
- **Commands:** `/unipi:info [page]`.
- **Tools:** none.
- **Keys:** in the dashboard: `←/→ h/l Tab` page, `1–9`, `↑↓ k/j`, `g/G`, `r`, `R`,
  `q/Esc`.
- **Settings (`info-screen`):** `bootMode`, `bootTimeoutMs`, `groups.<id>.show`,
  `groups.<id>.stats.<stat>`, `groupOrder`.
- **State:** the snapshot caches, `usage-stats.json`.
- **Pi API:**
  - on: `session_start`, `session_shutdown`, `turn_end`, `model_select`.
  - pi: `registerCommand`.
  - ctx: `getContextUsage`, `getSystemPrompt`, `model`, `sessionManager`, `mode`.
  - ui: `custom` ×2 (dashboard overlay and splash overlay).
- **UI:** two overlays. Page data: tiles, sections, meter rows, rank bars, legends,
  tags, big text, sparklines, braille area charts, gauges, share bars and grids.
- **External:** FS reads of all pi session files (`PI_CODING_AGENT_DIR`) and the git
  branch.
- **Inter-package:** core. Every package can add a page. The compactor and memory
  import its types. The footer reads the cached Memory page.
- **Tests:** 2 files, about 13 cases. Root tests: `info-screen-boot-timer`.

## 2.8 subagents — child agents

- **Purpose:** hand a self-contained task to a child agent.
- **Features:**
  - Each run is a child pi process (`pi --mode rpc`, core runtime).
    - Foreground or background.
    - Resume from the child's session file.
    - Capacity: `maxConcurrent` (8, up to 16).
    - Nesting depth: `UNIPI_SUBAGENT_MAX_DEPTH` (1 = no nesting).
    - Task text kept: 20 000 chars.
  - Profiles:
    - `subagent_explore`: 7 read-only tools, default model.
    - `subagent_general`: all tools except `sidekick,read_subagent,run_subagent`,
      lead model.
    - Custom Markdown agents in `~/.unipi/config/agents/` or
      `<project>/.unipi/config/agents/`. Front matter: `name`, `description`,
      `model`, `tools`/`allowed-tools`, `thinking`, `max-nesting`.
    - Default model order: the setting, then the Fusion sidekick model, then the lead
      model.
  - A user message during a foreground run moves the run to the background.
    `Ctrl+B` moves all foreground runs. `Esc` cancels.
  - Live cards (`subagent-step` entries) show the model, tool calls, tokens and cost.
    The completion arrives as a `subagent-completion` custom message.
  - Child usage goes into the lead's ledger.
  - A foreground child sends its UI prompts to the lead. A background child gets
    automatic denies.
  - Graph run port for long-horizon. Wait source for the arbiter.
  - Settings and agent files are read again each turn.
  - Uses `acorn` to parse JS in the tool-restriction logic.
- **Commands:** `/unipi:subagents` (tray tab), `/unipi:agents` (profile manager:
  list, create, edit, copy, delete).
- **Tools:**
  - `run_subagent {title, task, profile (enum rebuilt at load), is_background?:
    bool, resume?: agent_id}`
  - `read_subagent`, which is registered by core.
- **Keys:** `Ctrl+B` (through `onTerminalInput`), `Esc`. In the panel: `Enter`
  transcript, `f` foreground, `x` cancel.
- **Settings (`subagents`):** `enabled` (true), `defaultModel`, `defaultThinking`
  (`inherit`), `maxConcurrent` (8).
- **State:** child sessions at
  `~/.unipi/workspace/<id>/state/subagents/sessions/<lead-session-id>/`, and
  `index.json` (run registry).
- **Pi API:**
  - on: `before_agent_start`, `session_start`, `session_shutdown` (cancels the
    children).
  - pi: `registerTool`, `registerCommand`, `registerEntryRenderer("subagent-step")`,
    `registerMessageRenderer("subagent-completion")`, `appendEntry`, `sendMessage`,
    `getActiveTools`, `setActiveTools`.
  - ctx: `hasPendingMessages`, `model`, `modelRegistry`, `thinkingLevel`,
    `sessionManager`, `mode`.
  - ui: `confirm`, `editor` (profile body), `input`, `select`, `notify`, `setWidget`
    (`FG_KEY`, the live tail of foreground runs), `setWorkingMessage`,
    `onTerminalInput`.
- **UI:** a live card for each run (entry renderer), a foreground live-tail widget,
  the tray Subagents tab (list, then transcript detail), the profile manager
  dialogs, and the completion message card.
- **External:** spawns the pi child process. FS: session files and agent files.
- **Inter-package:** core (runtime, work list, arbiter), utility (renderer). Reads
  `FUSION_STATUS` (sidekick model). Exposes `unipi.subagents.*` holders and the graph
  port.
- **Tests:** 5 files, about 45 cases.

## 2.9 background-tasks — long shell commands

- **Purpose:** run shell commands in the background, and wake the agent when they
  finish.
- **Features:**
  - `bg_run` starts a named shell command and returns at once with an id and an output
    path.
    - Output goes to `<tmp>/unipi-bg-tasks/<session>-<pid>-<nonce>/`
      (`UNIPI_BG_TMP_DIR`).
    - Output cap: `maxOutputBytes` (20 MiB, minimum 1 KiB). Above the cap, the task is
      killed and fails.
    - `isAgent` commands get a telemetry wrap (`UNIPI_BG_DISABLE_PI_TELEMETRY`).
  - At the end, a `<background-task-notification>` custom message (followUp) wakes
    the agent when `triggerOnCompletion` is on.
  - A wake line above the editor shows while the agent is idle and waiting.
  - Stop: SIGTERM, then SIGKILL after a grace period. Process-group kill; `taskkill /T`
    on Windows (`UNIPI_BG_SHELL`, `UNIPI_BG_SHELL_PATH`).
  - Bounded log reads (default 30 KiB, head or tail).
  - Keeps `maxFinishedTasks` (30) finished tasks.
  - Adoption of a running foreground bash child (watchdog, `/unipi:bg-detach`).
  - Versioned `pi.events` API for other extensions:
    - Requests on `unipi-background-tasks:request:v1`: `capabilities`, `run`,
      `status`, `logs`, `kill`.
    - Replies on `…:response:v1`. Task ends on `…:terminal:v1`.
    - `request_id` ≤ 200 chars and never reused. Error text ≤ 240 chars.
  - Shared registry `getSharedTaskRegistry()` (`Symbol.for`).
  - Work-tray "Background tasks" tab:
    - Keys: `↑↓`, `Enter` detail (live log), `l` full log (≤ 4 MB), `g/G`.
    - `s` stop, `x x` kill, `a a` stop all, `d/D` remove finished, `R` rerun.
    - `c` log path, `←/Esc` back.
- **Commands:** `/unipi:bg [--agent] [--name "N"] <command>` (notify only, no turn),
  `/unipi:bg-tasks [task-id]`.
- **Tools:**
  - `bg_run {name, command, isAgent: bool, description?, timeoutSeconds?,
    notifyOnCompletion?, triggerOnCompletion?}`
  - `bg_status {taskId?}`
  - `bg_logs {taskId, maxBytes?, tail?}`
  - `bg_kill {taskId}`
  - All tools and keys exist only when `enabled` is on.
- **Keys:** `Shift+↓` opens the tray. `Ctrl+Alt+C` clears the finished notices.
- **Settings (`background-tasks`):** `enabled` (true), `notifyOnCompletion` (true),
  `triggerOnCompletion` (true), `defaultTimeoutSeconds` (0), `maxFinishedTasks` (30),
  `maxOutputBytes` (20 MiB). Env: `UNIPI_BG_MAX_OUTPUT_BYTES`.
- **State:**
  - In-memory registry and output files.
  - Custom entries for the start and completion cards.
  - Custom message `background-task-notification`.
  - `background-tasks.json` (persisted registry, under core paths).
- **Pi API:**
  - on: `session_start`, `session_shutdown` (kills the tasks).
  - pi: `registerTool`, `registerCommand`, `registerShortcut`, `appendEntry`,
    `sendMessage`, `events`.
  - ctx: `modelRegistry.getAll`, `model`, `sessionId`, `cwd`, `mode`, `hasUI`.
  - ui: `notify`. The tray tab goes through core.
  - Timers: an interval for the output watch and 9 timeouts.
- **UI:** start and completion cards (entries), the wake line, the tray tab, and the
  completion message that the model sees.
- **External:**
  - Shell spawn (sh, cmd or bash) with streaming to a file, detached process groups,
    signals, and `taskkill`.
  - `turndown` converts the HTML output of some agent commands.
- **Inter-package:** core (arbiter wait source, work list). Read by footer, watchdog,
  notify and core.
- **Tests:** 11 files.

## 2.10 watchdog — stuck-tool detection

- **Purpose:** find hung tool calls with jev, then move them to the background, kill
  them or warn.
- **Features:**
  - **Heartbeat:** ≤ 15 s (minimum 1 s).
  - **Due check:**
    - The item is at least `firstCheckMin` (2) old.
    - The last check was at least 0.9 × `intervalMin` (3) ago.
  - **State text sent to jev:** the tool, the command (≤ 500 chars), the run time, the
    time since new output, and an output tail of 3 000 chars.
  - **jev questions:** `status` (choice: `progressing|waiting|stuck|looping`) and
    `persistent` (noul: is it a long-lived service).
  - **Act rule:** all of these must be true:
    - `stuck` or `looping` at or above `confidence`.
    - Not a healthy service: `persistent` < 0.5, unless the status is `looping`.
    - `agreeChecks` (2) checks in a row agree.
  - **Bash calls:** in addition, a Linux process-activity sample is taken.
    - It reads `/proc` CPU and IO, and `getconf CLK_TCK`.
    - A sleeping process counts as active.
    - An explicit timeout, `sleep` or bounded loop vetoes the trigger.
  - **Actions:**
    - `background` (default): adopt the process into background-tasks and return the
      tool early. The task is then excluded from future checks.
    - `kill`: find exactly one matching child through `ps`. SIGTERM the group, then
      SIGKILL after 3 s. Prefix the result with `⚠ Killed by unipi watchdog …` and set
      `isError`.
    - `warn`.
    - Background tasks are stopped through the registry. Other tools get `off`,
      `warn` or `abort-turn` (`ctx.abort`).
    - Without adoption, it falls back to `kill`. On Windows it only warns.
  - Warnings are delivered at the next turn start as one `unipi-watchdog` custom
    message.
  - Fast tools are never watched: read, write, edit, grep, find, ls, `ask_user`,
    `read_subagent`, `bg_kill`, `bg_tasks`.
- **Commands:** `/unipi:bg-detach [reason]` moves the most recent foreground bash call
  to the background.
- **Tools:** none.
- **Keys:** none.
- **Settings (`watchdog`):** `enabled` (true), `intervalMin` (3), `firstCheckMin` (2),
  `confidence` (0.5), `agreeChecks` (2), `action` (`background|kill|warn`),
  `watchBash` (true), `watchBgTasks` (false), `otherTools` (`off|warn|abort-turn`),
  `decisionModel.*`.
  - `UNIPI_DEBUG_WATCHDOG=1` logs to `~/.unipi/logs/watchdog.log`.
  - `UNIPI_WATCHDOG_FORCE_ACT=1` forces an act decision (for tests).
- **State:** in-memory item table only.
- **Pi API:**
  - on: `tool_execution_start`, `tool_execution_update` (output-change tracking),
    `tool_execution_end`, `tool_result` (annotates the killed result),
    `before_agent_start` (delivers warnings), `session_start`, `session_shutdown`.
    It also uses a private `__watchdog_tick` event.
  - pi: `on`, `registerCommand`.
  - ctx: `abort`, `cwd`, `hasUI`.
  - ui: `notify`, `setStatus("watchdog")` (`watchdog: N`).
- **UI:** a status slot and annotations in the tool result.
- **External:** `ps`, `getconf`, `/proc`, process-group signals, jev HTTP.
- **Inter-package:** core (jev, detachable bash), background-tasks (registry and
  adoption).
- **Tests:** 8 files, about 43 cases.

## 2.11 btw — side questions

- **Purpose:** ask a side question while the main agent works. Nothing goes back into
  the main session.
- **Features:**
  - A panel replaces the editor.
  - Each question runs in a **new in-memory session** (`createAgentSession` +
    `SessionManager.inMemory`).
    - The session is seeded with a copy of the current main branch, including the
      tool calls that are in progress.
    - It has 4 read-only tools: `read`, `grep`, `find`, `ls`.
    - It uses the main model and thinking level. The credentials come from
      `modelRegistry.getApiKeyAndHeaders`.
  - Only one answer streams at a time.
  - The history is kept in memory. It is cleared on session start, on resume and on a
    tree move.
  - In print mode, the answer is shown as a notification.
  - A `context` hook hides legacy `btw-note` messages from the model.
- **Commands:** `/unipi:btw [question]`.
- **Tools:** none. The 4 tools live in the side session only.
- **Keys:** in the panel: `Enter` ask, `↑/↓` history, `PgUp/PgDn` scroll, `Ctrl+C`
  stop (or close when idle), `Esc` close (the stream continues).
- **Settings:** none.
- **State:** memory only.
- **Pi API:**
  - on: `context`, `session_start`, `session_shutdown`, `session_tree`.
  - pi: `registerCommand`, `registerMessageRenderer(BTW_MESSAGE_TYPE)`,
    `getThinkingLevel`.
  - ctx: `getSystemPrompt`, `model`, `modelRegistry`, `sessionManager.getBranch`,
    `getHeader`, `mode`.
  - ui: `custom` (panel), `notify`.
- **UI:** the editor-replacement panel: a question list, a tool-call line for each
  call, a streaming markdown answer, and a scroll area. `unipi.btw.api` for the app.
- **External:** model streaming through the session's provider.
- **Inter-package:** none at import time (core only through the app API).
- **Tests:** 1 file, about 8 cases.

## 2.12 web-api — web search and read

- **Purpose:** web search and page reading, with free providers first.
- **Features:**
  - Providers ranked by cost.
    - `web_search` sources: 1 wigolo, 2 DuckDuckGo, 3 Jina Search, 4 SerpAPI,
      5 Tavily, 6 Perplexity.
    - `multi_web_content_read` sources: 0 smart-fetch (default), 1 wigolo, 2 Jina
      Reader, 3 Firecrawl, 4 Perplexity.
    - `web_llm_summarize` source: Perplexity.
  - With no `source`, the tool falls through to the next provider on failure. With an
    explicit `source`, only that provider runs.
  - **smart-fetch:** `wreq-js` gives a browser TLS fingerprint. `linkedom` parses the
    HTML and `defuddle` extracts the main content.
  - Result cache in `~/.unipi/config/web-api/cache/` (1 h; the key is url, browser,
    format and maxChars). Expired entries are pruned at start and end.
  - wigolo (AGPL) is loaded at run time only if it is installed.
  - Tools that have no provider are hidden with `setActiveTools`.
- **Commands:** none. Hub action: `unipi:web-cache-clear`.
- **Tools:**
  - `web_search {query, source?: 1..6}`
  - `multi_web_content_read {url: string|string[], source?: 0..4, browser?, os?,
    format?: markdown|html|text|json, maxChars?, timeoutMs?, removeImages?,
    includeReplies?: bool|"extractors", proxy?, batchConcurrency?, verbose?}`
  - `web_llm_summarize {url, prompt?}`
- **Keys:** none.
- **Settings (`web-api`):** `providers.<id>.enabled`, `providers.<id>.apiKey`,
  `smartFetch.{browser,os,includeReplies,maxChars,timeoutMs,batchConcurrency,removeImages}`.
  Env keys: `FIRECRAWL_API_KEY`, `JINA_API_KEY`, `PERPLEXITY_API_KEY`,
  `SERPAPI_KEY`, `TAVILY_API_KEY`.
- **State:** the cache dir, and an `auth.json` read for keys.
- **Pi API:**
  - on: `before_agent_start` (tool exposure), `session_start`, `session_shutdown`.
  - pi: `registerTool`, `getActiveTools`, `setActiveTools`.
  - ui: `confirm` (cache clear), `notify`.
- **UI:** none besides the hub. Info-screen "Web API" group.
- **External:**
  - About 81 `fetch` call sites: DuckDuckGo HTML, Jina, SerpAPI, Tavily, Perplexity,
    Firecrawl.
  - **TLS-fingerprinted HTTP** (`wreq-js` is a native addon).
  - Optional local wigolo SDK.
- **Inter-package:** core.
- **Tests:** 8 files, about 75 cases.

## 2.13 ask-user — multiple-choice questions

- **Purpose:** the agent asks 1 to 4 multiple-choice questions in one dialog.
- **Features:**
  - Off by default. When it is off, the tool refuses and tells the agent to ask in its
    reply.
  - Each question has 2 to 4 options and an optional "Other" free text. Pasted images
    or files become tokens; images are sent with the answers.
  - Skipped questions are allowed.
  - Option actions:
    - `end_turn`.
    - `new_session` with `prefill`: Compact & run (`ctx.compact`, then the prefill)
      or Run directly.
  - One dialog at a time. Accepts the legacy single-question schema.
  - Fails in a subagent, with a "tell your lead" message. With no UI, it tells the
    agent to ask in its reply.
  - Phone race through `raceRemote`.
  - Emits `ASK_USER_PROMPT` (notify re-notifies).
  - Shows the answers in the transcript as a short tree.
- **Commands:** none.
- **Tools:** `ask_user {questions: [{question, header (≤16), options: [{label,
  description?, value?, action?: end_turn|new_session, prefill?}] (2–4),
  multi_select?, other?}]}`. Max 4 questions. The result is JSON
  `{selected[], custom_text, skipped}` for each question, plus images.
- **Keys:** in the dialog: `↑↓`, `1–9`, `Space`, `Enter`, `←→`/`Tab`/`Shift+Tab`,
  `Ctrl+V`/`Alt+V` image paste, `?` (not ready), `Esc` (stop or send).
- **Settings (`ask-user`):** `enabled` (false), `notifyOnAsk` (true),
  `maxQuestions` (4), `escape` (`stop|send`), `digitAdvance` (true),
  `other` (`agent|always|never`), `helpLine` (true).
- **State:** none.
- **Pi API:**
  - on: `before_agent_start` (exposure), `session_start`.
  - pi: `registerTool`, `getActiveTools`, `setActiveTools`.
  - ctx: `abort`, `compact`, `hasUI`, `cwd`.
  - ui: `custom` ×2 (dialog, launcher), `notify`, `setEditorText` (prefill).
- **UI:** the question dialog overlay:
  - A tab strip with one header per question and a ✓ for answered ones.
  - The question text, option rows with descriptions, and an Other input with tokens.
  - A help line.
  - A session-launcher dialog (Compact & run / Run directly).
- **External:** clipboard image read (`wl-paste`, `xclip`, macOS/Windows equivalents)
  through `spawnSync`.
- **Inter-package:** core (remote dialogs, harness, bus). Notify listens.
- **Tests:** 6 files, about 38 cases.

## 2.14 mcp — MCP client

- **Purpose:** connect MCP servers and register their tools.
- **Features:**
  - All enabled servers connect in parallel at `session_start` (10 s timeout per
    request).
  - The tools register as `<server>__<tool>` in one sorted list, with the schema keys
    sorted (cache stability). At most 20 servers.
  - Result cap: 64 KiB head and tail. The full result (≤ 16 MiB) spills to
    `~/.unipi/tool-results/` (mode 0600).
  - Config merge: global `~/.unipi/config/mcp/` and project `.unipi/config/mcp/`. Each
    folder has `mcp-config.json` (Claude Desktop format, 0600) and `config.json`
    (enabled states). The project definition wins.
  - Catalog: `servers.json`, with a 49-server seed. Sync from
    `punkpeye/awesome-mcp-servers` (raw GitHub).
  - Changes need a restart: unipi cannot unregister tools.
- **Commands:** `/unipi:mcp-status`. Runners for the hub: `unipi:mcp-add`, `-settings`,
  `-sync`, `-reload`.
- **Tools:** dynamic `<server>__<tool>`.
- **Keys:** in the overlays: `j/k`, `gg/G`, `Ctrl+d/u`, `/` search, `Enter` edit, `q`
  quit.
- **Settings (`mcp`):** action rows only. The server list is not a setting.
- **State:** the config files, the catalog and the tool-result spill files.
- **Pi API:**
  - on: `session_start`, `session_shutdown` (kills the children).
  - pi: `registerTool` (dynamic, after the connect), `registerCommand`, `events`.
  - ui: `custom` ×2 (server list, catalog browser with a JSON editor), `notify`.
- **UI:**
  - Server list overlay: enable/disable and edit.
  - Catalog browser: search, a list, and a JSON config editor pane with validation
    status lines.
  - Status report text. Info-screen "MCP Servers" group.
  - Bus: `MCP_SERVER_STARTED`, `MCP_SERVER_ERROR`, `MCP_TOOLS_REGISTERED`.
- **External:** stdio JSON-RPC MCP server processes, HTTP MCP servers (fetch), GitHub
  raw download.
- **Inter-package:** core. Notify listens to `MCP_SERVER_ERROR`.
- **Oino note:** pi 1.4 has built-in MCP (`pi.registerMcpServer`, `mcp.json`). The
  charter puts pi-mcp out of scope, "may come later as extensions". This package is
  the extension that brings it.
- **Tests:** 2 files, about 10 cases.

## 2.15 notify — notifications

- **Purpose:** desktop and phone notifications for events. Also the `notify_user`
  tool.
- **Features:**
  - Platforms:
    - native (node-notifier: notify-send, terminal-notifier, SnoreToast)
    - Gotify (priorities 2/5/8)
    - Telegram bot API
    - ntfy (priorities 2/3/5, token)
  - Per-event enable and platform routing. Events:
    - `workflow_end`, `ralph_loop_end`, `mcp_server_error`: on by default.
    - `agent_end` and `agent_settled`: off. They dedupe into one "finished"
      notification after settle plus 1.5 s of quiet (`UNIPI_NOTIFY_FINISH_GRACE_MS`).
      While work is pending, one "All done" is sent instead.
    - `memory_consolidated`, `session_shutdown`, `ask_user_prompt`,
      `permission_request` (from `@gotgenes/pi-permission-system`): off.
    - `input_needed`: a catch-all on `ui_prompt_start`/`ui_prompt_end`, sent only
      while the agent runs, and only once per prompt.
  - Re-notify for blocking events: every 2 min, 3 times at most, with the title
    "(still waiting)" and high priority. It stops on a key press, on herdr
    not-blocked, on a new turn, when the prompt closes, or at session end. Only one
    re-notify runs at a time.
  - Silence after input (window 10 s, per platform).
  - Recap: the last message (≤ 2 000 chars) goes to `recap.model`
    (chat/completions, max 100 tokens). `recap.disableThinking` is available.
  - UniPi app push: AES-256-GCM encryption with the key `~/.unipi/app-host/notify-key`
    (wire format `unipi1:` + b64(iv‖ct‖tag)). The title is pinned to "UniPi" and the
    message carries a deep link. UnifiedPush fan-out to
    `~/.unipi/config/notify/app-endpoints.json`.
  - Native `suppressWhenFocused` (Windows only).
  - Emits `NOTIFICATION_SENT`.
- **Commands:** `/unipi:notify-event <event> on|off`. Runners: set-gotify, set-ntfy,
  set-telegram, test.
- **Tools:** `notify_user {message, title?, priority?: low|normal|high, platforms?:
  ("native"|"gotify"|"telegram"|"ntfy")[]}`.
- **Keys:** none.
- **Settings (`notify`):**
  - `defaultPlatforms`, `events.<e>.{enabled,platforms}`
  - `native.{enabled,suppressWhenFocused}`
  - `gotify.{serverUrl,appToken,priority}`, `telegram.{botToken,chatId}`
  - `ntfy.{serverUrl,topic,token,priority,appDetail}`
  - `silenceAfterInput.{enabled,windowMs,platforms}`
  - `renotify.{enabled,intervalMs,maxRepeats}`
  - `recap.{enabled,model,disableThinking}`
  - Files: `ntfy.json`.
- **State:** the settings, `app-endpoints.json`, and the key file (read only).
- **Pi API:**
  - on: `agent_start`, `agent_end`, `agent_settled`, `session_start`,
    `session_shutdown`, `ui_prompt_start`, `ui_prompt_end`.
  - pi.events: `herdr:blocked`, `rpiv:ask-user:prompt`, and any configured hook
    names, routed by name.
  - pi: `registerTool`, `registerCommand`, `getSessionName`.
  - ctx: `modelRegistry.getApiKeyAndHeaders` (recap), `sessionManager.getEntries`.
  - ui: `custom` ×3 (ntfy setup wizard, platform pages), `notify`.
- **UI:** the platform setup overlays (ntfy setup with a QR/topic) and the test
  action.
- **External:**
  - HTTP: Gotify, Telegram, ntfy and the recap model.
  - The native notifier binaries.
  - Crypto (in-guest).
  - Terminal focus detection (Windows).
- **Inter-package:** core. Reads the background-tasks registry (pending-work rule),
  `ASK_USER_PROMPT`, `RALPH_LOOP_END`, `MCP_SERVER_ERROR`.
- **Tests:** 13 files, about 124 cases.

## 2.16 kanboard — per-project task board (TS extension + Rust crate)

- **Purpose:**
  - A persistent board for each project.
  - A web board served by a daemon.
  - Budgeted agent work and autowork, driven by the turn arbiter.
- **Features (extension):**
  - Finds the binary: `UNIPI_KANBOARD_BIN`, then the npm platform package, then
    `crates/kanboard/target/*`.
  - All board writes go through the `unipi-kanboard` CLI (JSON). The extension never
    edits task files.
  - **Guard** on `tool_call` (bash commands that invoke the CLI):
    - Reads always pass.
    - Children (`UNIPI_KANBOARD_CHILD`, subagents, sidekick) never write.
    - Writes cost the `-do` budget: `doTasks` slots (5) and `doWrites` (10).
    - A close of the session's own claim is free.
    - At most `turnAddLimit` (20) adds per turn.
  - **Monitor** (arbiter providers):
    - Claims provider at priority 50, autowork provider at 40.
    - Defers while a long-horizon owner is active.
    - Disarms on abort or error.
    - Runaway limits: 5 nudges per task, stall after 2 idle runs, 3 offers per task.
    - Gives a "question" notice when the last paragraph ends with `?`.
  - Evidence contributor: an open claim blocks goal completion.
  - Compaction-context block: the board contract.
  - Reminders: before file edits, it reminds the agent to `start` a Todo task. Each
    reminder has an anti-poisoning suffix.
  - Badges on tool results.
  - Session start: reap the stale claims of dead pids, `archive-sweep`, and a notice
    about the released tasks. Session end: release this session's claims to Todo.
  - Env passed to the bash children: `UNIPI_KANBOARD_SESSION`, `PID`, `PROJECT`,
    `HOME`, `MAX_SESSIONS`.
  - Notices are buffered as UI-only entries and filtered from the context.
  - Shares `captureToBacklog` with input-shortcuts.
  - Daemon open: `--host 127.0.0.1|0.0.0.0|tailscale` (`tailscale ip` exec) and
    `--port`. Prints the link (with the token for a non-loopback bind). Optional
    browser open.
- **Commands:**
  - `/unipi:kanboard [open|close|onboard|status|show [--all]|doctor]`
  - `/unipi:kanboard-add [-p 1-5] [--after ID] [--status backlog|todo] <title>`
    (lines after the title become the body; file paths become attachments)
  - `/unipi:kanboard-do <request>|off`
  - `/unipi:kanboard-autowork start|stop`
  - Runners: open, stop-daemon, rotate-token.
- **Tools:** none. The agent uses the CLI through `bash` and the `kanboard` skill.
- **Keys:** none.
- **Settings (`kanboard`):** `chainGate` (`in_review|done`), `idleMin` (10), `host`,
  `port`, `archiveAfterDays` (0), `retentionDays` (90), `openBrowser`, `requireAuth`,
  `keepToken`, `maxSessions` (2), `turnAddLimit` (20), `reminders` (true),
  `doTasks` (5), `doWrites` (10). They are mirrored to the daemon `settings.json`.
- **State:** the board files (see the crate). Custom entries: badge, notice and
  progress. Custom message types: claims nudge, autowork nudge, show, doctor and help.
  Debug log in `~/.unipi/logs`.
- **Pi API:**
  - on: `session_start`, `session_shutdown`, `before_agent_start` (×2: reminders,
    budget prompt), `agent_start` (×2), `agent_end` (disarm), `agent_settled`,
    `context` (drop notices), `tool_call` (guard), `tool_result` (×2: badges and R1
    annotations).
  - pi: `registerCommand`, `registerMessageRenderer(SHOW_CUSTOM_TYPE)`,
    `appendEntry`, `sendMessage`.
  - ctx: `sessionManager.getSessionId`, `cwd`.
  - ui: `confirm`, `notify` (×40).
- **UI:**
  - In-chat board render (message renderer: lanes and cards).
  - Notices, badges and progress entries.
  - Bus sticky `KANBOARD_STATUS` (claims label, read by the footer).
  - The web board is served by the daemon. **It is the unipi-app web build, so it is
    out of scope** (§4.4).
- **External:**
  - Spawns: the `unipi-kanboard` CLI (sync JSON) and the detached daemon
    (`serve`).
  - `tailscale`, `xdg-open`/`open`, `powershell` (Windows).
  - HTTP to the daemon (`/api/health`).
- **Inter-package:** core (arbiter, evidence, compaction-context, skills reveal,
  progress). Reads `LH_STATE` and `LONG_HORIZON_OWNER_CHANGED`.
- **Tests:** 15 files, about 179 cases.

### 2.16.1 `crates/kanboard` (Rust binary `unipi-kanboard`, edition 2024)

- Dependencies: clap, serde/serde_json/serde_norway, chrono, axum 0.8, tower-http,
  rust-embed, tokio, notify 8 (fs watch), getrandom, anyhow, thiserror.
- **Storage** under `~/.unipi/kanboard/` (`UNIPI_KANBOARD_HOME`):

  ```text
  daemon.json daemon.lock token settings.json
  projects/<slug>/project.json board.lock tasks/<PREFIX>-<n>.md attachments/<ID>/ cold/
  ```

  - Slug: `<basename>-<sha256(root)[..6]>`.
  - Each write is atomic: tmp file, fsync, rename, all under `flock(board.lock)`.
  - Task file: YAML front matter (`id`, `title`, `status`, `priority`, `order`,
    `deps`, `labels`, `created`, `updated`, and `run` while claimed), a markdown body,
    and an append-only `## Activity` section.
- **CLI:**
  - Projects: `project add|list|show|archive|unarchive`.
  - Tasks:
    - `add`, `list [--ready]`, `show`, `chain`, `search`
    - `move`, `note`, `attach`, `attachments`, `edit`
    - `link`/`unlink`, `order`, `duplicate`
    - `next`, `start [--pid]`, `finish`, `release`
  - Maintenance: `reap`, `archive-sweep`, `validate [--fix]`, `settings show|set`,
    `rotate-token`.
  - Daemon: `serve`, `status`, `stop`.
  - Global flags: `--project`, `--actor user|agent|system`, `--session`, `--gate`,
    `--json`.
  - Exit codes: 0 ok, 1 rule error, 2 usage error. JSON errors: `{ok:false,
    kind: rule|usage|not_found|io, error}`.
- **Transitions:**
  - The table is in the crate README (§Transitions).
  - Final states: `done`, `cancelled`, `archived`.
  - A task is ready when it is Todo, has no claim, and every dependency reached the
    gate.
  - `next` sorts by priority, then order, then id.
- **Daemon:**
  - Single instance through `flock(daemon.lock)`.
  - `daemon.json` holds `{pid,port,version,startedAt,host,token?}`.
  - Stops after `--idle-min` with no HTTP request and no SSE client.
  - A file watch over `projects/` bumps the project revision. SSE:
    `/events?project=`.
  - Auth: a token is needed for a non-loopback bind or with `--require-auth`. It is
    accepted as `?t=`, a cookie or a bearer header. The daemon checks that `Origin`
    matches `Host` on POST.
  - JSON API: health, projects, dashboard, running, tasks CRUD, move (409
    `needsComment`), undo, note/edit/link/unlink/order/duplicate, attachments
    (≤ 25 MiB), files, the summarize and lane actions, settings, models and rules.
- **Web UI:** an embedded `ui-dist` from the unipi-app web build (pinned in
  `ui.lock.json`), with a deprecated SolidJS `web/` fallback.
- **Tests:** 204 `#[test]` (cli, transitions, deps, format, sessions, start_finish,
  cold, labels, daemon, attachments, summarize, title_optional) and `.mjs` UI tests.
- **Port note:** this is already Rust. Move it into the Oino workspace as a crate
  (library plus binary) and bring it to the charter's quality gates (pedantic clippy,
  no unwrap, rustdoc). The extension can call the **library** in-guest only if the
  WASI fs and locking are enough. Otherwise it calls the binary through host exec.
  Recommended: the host runs the binary. Keep the daemon a native process.

### 2.16.2 `packages/kanboard-bin`

Five npm packages (`linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`,
`win32-x64`) that each ship a prebuilt `unipi-kanboard` in `bin/`. In Oino, the
installer or desktop bundle replaces this distribution.

## 2.17 command-enchantment (`packages/autocomplete`)

- **Purpose:** better `/unipi:*` autocomplete.
- **Features:**
  - Wraps pi's autocomplete provider at `session_start`.
  - Adds a colored package tag (for example `[workflow]`).
  - Tiers: a unipi override of a pi name (`settings`), then an exact full match, then
    an exact short-name match, then a prefix match, then a fuzzy match (shorter
    first). Pi commands come first within a tier, then a fixed package order.
  - `/skill:*` suggestions are hidden until the user types `/skill:`.
  - Argument suggestions are kept, also on `Tab` after a space.
  - An audit test checks the command list in `src/constants.ts`.
- **Commands, tools and keys:** none.
- **Settings (`command-enchantment`):** `autocompleteEnhanced` (true).
- **Pi API:** on `session_start`. ui: `addAutocompleteProvider` (a wrapping
  provider).
- **UI:** autocomplete item decoration (a tag label and a color class) and order.
- **Inter-package:** core (settings). Fusion and the compactor add their own provider
  layers (`/model` → `/unipi:model`, `/unipi:compact-then`).
- **Tests:** 2 files, about 40 cases.

## 2.18 compactor — lossless compaction and recall

- **Purpose:**
  - Context compaction that does not drift.
  - Search over the full history.
- **Features:**
  - Every compaction route goes through `session_before_compact`: pi auto, pi
    `/compact`, unipi commands, and the percent trigger.
  - **vcc method** (0 LLM calls):
    1. Rebuild from the **full raw branch** each time, never from the previous
       summary.
    2. Drop the user text that extensions sent. It is marked at `input` with a
       `compactor-origin` entry; 8 known text shapes are the fallback.
    3. Redact secrets, then clip.
    4. Sections with fixed budget shares:

       | Section | Share |
       |---|---|
       | Active Work (from `registerCompactionContext`) | 16% |
       | Your Requests | 14% |
       | Latest State | 12% |
       | Decisions & Constraints | 16% |
       | Files | 7% |
       | Lessons | 10% |
       | Project Knowledge | 7% |
       | Commits | 5% |
       | Open Errors | 6% |
       | Recent Transcript | rest (≥ 18%) |

    5. End with the `session_recall` note.
    - Auto budget: 1.5 to 4 k tokens. Cap: 8% of the window.
    - Smart kept tail: 5 to 25 k tokens. Recut above 40% of the window.
    - Chars per token are calibrated from 2 to 6.
  - **llm method:** pi's summarizer (`generateSummaryWithUsage`, `compact`) with the
    active-work block at the start and `llmInstructions`. It falls back to pi's own
    summarizer.
  - **Percent trigger:**
    - Compacts at `turn_end` when usage passes the threshold.
    - Returns a **compaction draft** at the boundary, so nothing is aborted.
    - Cooldown 60 s. Minimum growth 4 000 tokens.
  - `/unipi:compact-then <prompt>` is not a registered command. The `input` hook
    detects it and adds its own autocomplete entry. Idle, it compacts and then sends a
    follow-up. Streaming, it queues the compaction and sends a steer or follow-up.
  - A compaction card (UI-only entry). Emits `COMPACTOR_COMPACTED` (the turn-boundary
    compaction does not fire pi's `session_compact`).
  - Stats (`compact-stats`) and doctor.
  - Debug: `/tmp/compactor-debug.json` (`UNIPI_COMPACTOR_DEBUG`).
- **Commands:**
  - `/unipi:compact-vcc [keep:N]`, `/unipi:compact-by-llm [focus]`
  - `/unipi:session-recall <q> [scope:all] [page:N]`
  - `/unipi:compact-stats`, `/unipi:compact-doctor`, `/unipi:compact-help`
  - The pseudo-command `/unipi:compact-then`.
- **Tools:**
  - `session_recall {query?, expand?: number[], page?≥1, scope?: lineage|all, mode?:
    hybrid|touched}`
    - 5 hits per page (at most 50).
    - An expanded hit is at most 16 KiB.
    - Search budget: 3 s.
    - `#N:path` drill-in.
  - `context_budget {}`.
- **Keys:** none.
- **Settings (`compactor`):** `method` (`vcc|llm`), `piCompact` (`follow|vcc|llm`),
  `trigger` (`pi|percent`), `thresholdPercent` (80), `notify`, `smartKeepTail`,
  `summaryBudgetTokens` (0 = auto), `sections.{activeWork,requests,state,decisions,
  files,commits,errors,lessons,transcript}`, `cooldownMs`, `repeatMinGrowthTokens`,
  `llmInstructions`, `debug`.
- **State:** the `compactor-origin` entries and the card entries in the session.
  `settings.json`, the legacy db (cleanup target) and `compactor.json`.
- **Pi API:**
  - on: `session_before_compact` (returns the compaction), `session_compact`,
    `turn_end` (compaction draft), `input` (origin marks and compact-then),
    `context` (drops legacy hidden types), `tool_result`, `agent_end`,
    `agent_settled`, `session_start` (×2).
  - pi: `registerTool`, `registerCommand`, `appendEntry`, `sendMessage`,
    `getThinkingLevel`.
  - ctx: `compact`, `getContextUsage`, `isIdle`, `getCommands`, `model`,
    `modelRegistry.getApiKeyAndHeaders`, `sessionManager.getBranch`.
  - ui: `addAutocompleteProvider`, `notify`.
- **UI:** the compaction card (`Compacted 7.9k → 2.2k tokens …`), the recall results
  (`compactor-recall` custom type), and the stats/doctor text. Footer and info-screen
  counters.
- **External:** model calls for the llm method.
- **Inter-package:** core (compaction-context, harness), info-screen (types). Reads
  the long-horizon and kanboard blocks.
- **Tests:** 12 files, about 104 cases. `scripts/compactor-eval.mts` exists outside
  the package.
- **Oino note:** Oino core already has compaction and branch summaries (charter
  §Vision 1). The compactor needs (a) a `session_before_compact` hook that can return a
  full replacement compaction, (b) the turn-boundary compaction draft, and (c) a host
  call into the default summarizer.

## 2.19 footer — framed editor and stats line

- **Purpose:** a "glance frame" around the input, plus live session stats.
- **Features:**
  - **Top border:** the UNIPI brand with an animated rainbow, the long-horizon mode,
    the git branch, the PLAN badge and permission mode, and the kanboard claims.
  - **Bottom border:** the workspace, context use and window, the model and thinking
    level, and the Fusion lead and sidekick.
  - **Stats line:** turns and steps, model and tool time, average TTFT and tokens per
    second, input/output tokens, cost (`sub` for subscriptions), cache hit %, and
    compactions.
  - **Process line:** counts of background tasks by status.
  - **Responsive:**
    - Narrow terminals drop parts by priority: compactions, cache, time, turns,
      speed, cost. Tokens always stay.
    - Badges leave the frame before the branch or model truncates.
    - The last column is never written.
    - Below 20 rows, the stats and process lines hide.
  - Color modes: `auto` (truecolor, 256 or `NO_COLOR`), `truecolor`, `256`, `none`.
    Icon styles: `nerd`, `emoji`, `text`. Rainbow at `xhigh`/`max` thinking.
  - **Incremental usage scan:**
    - Each second, only new entries are processed. A compaction or branch change
      triggers a full rescan.
    - The live TPS tracker is fed from the stream events.
    - Child usage entries are folded in.
- **Commands:** `/unipi:footer [on|off]`.
- **Tools and keys:** none.
- **Settings (`footer`):** `enabled`, `colorMode`, `iconStyle`, `rainbow`
  (`always|brand-only|off`), `processLine`,
  `strip.{turns,time,speed,tokens,cost,compactions,cache}`,
  `badges.{mode,planPermission,fusion,kanboard}`.
- **State:** none. `unipi.footer.shared-tps` is shared for the app.
- **Pi API:**
  - on: `session_start`, `session_shutdown`, `turn_start`, `message_start`,
    `message_update` (TPS), `message_end`, `tool_execution_start`,
    `tool_execution_end`, `agent_settled`.
  - pi: `registerCommand`.
  - ctx: `isIdle`, `model`, `modelRegistry`, `mode`.
  - ui: `setEditorComponent` (GlanceEditor replaces the editor), `setFooter`,
    `setWidget("footer-top")`, `setWidget("footer-secondary")`, `setFocus`,
    `notify`.
  - Timers: a 1 s interval (scan and rainbow) and 2 other intervals.
- **UI:**
  - Editor chrome: top and bottom borders with titled segments.
  - Footer line: stats segments with priorities.
  - A process widget.
  - All of these are rendered from bus sticky state plus the usage snapshot.
- **External:** the git branch (`git` exec or a `.git/HEAD` read). Env:
  `TERM_PROGRAM`, `GHOSTTY_RESOURCES_DIR`, `POWERLINE_NERD_FONTS`, `NO_COLOR`.
- **Inter-package:** core (bus), background-tasks (registry). Reads `LH_STATE`,
  `KANBOARD_STATUS`, `FUSION_STATUS` and `WORKFLOW_STATUS`.
- **Tests:** 16 files, about 140 cases.
- **Oino note:** this is mostly frontend work. Port it as a "status model" extension
  that emits structured footer and frame segments with priorities, and let the
  TUI/GPUI clients do the layout.

## 2.20 updater — self-update and docs browser

- **Purpose:** find a new unipi release on npm and install it. Browse the READMEs and
  the changelog.
- **Features:**
  - At session start, reads `~/.unipi/cache/updater/last-check.json`.
  - If the last check is older than `checkIntervalMs`, gets the npm `latest` dist-tag
    (10 s timeout). It never offers a downgrade, and remembers skipped versions.
  - The update overlay shows the changelog between the two versions (from GitHub raw,
    or the local `CHANGELOG.md`).
  - `Y` runs `pi install npm:@pi-unipi/unipi` (60 s timeout).
  - `autoUpdate`: `disabled`, `notify`, or `auto` (5 s countdown).
  - Bus: `UPDATE_CHECK`, `UPDATE_AVAILABLE`, `UPDATE_APPLIED`.
  - Info-screen group.
- **Commands:** `/unipi:readme [package]`, `/unipi:changelog`.
- **Tools:** none.
- **Keys:** in the overlay: `Y/y`, `n/q/Esc`, `j/k`, `g/G`. In the browsers: `Enter`
  open, `q/Esc` back.
- **Settings (`updater`):** `checkIntervalMs` (1 h), `autoUpdate` (`notify`).
- **State:** the `last-check.json` cache, skipped versions.
- **Pi API:** on `session_start`. pi: `registerCommand`. ui: `custom` ×4 (update
  overlay, README browser, changelog list/detail), `notify`. Timers: one interval.
- **External:** npm registry HTTP, GitHub raw HTTP, `pi install` exec, and reads of
  the package files.
- **Inter-package:** core.
- **Tests:** 5 files, about 39 cases.
- **Oino note:** this must change. Oino ships its extensions inside the binary or the
  desktop bundle, so the update target is Oino itself or its extension packages.

## 2.21 input-shortcuts — editor chords

- **Purpose:** editor keys for a stash, undo/redo, copying the last reply and
  capturing a kanboard task.
- **Features:**
  - `Alt+S` opens a chord overlay. The keys are:
    - `S` stash or restore.
    - `A` append the stash.
    - `U` undo, `R` redo.
    - `Y` copy the last reply (pi `copyToClipboard`).
    - `K` add the editor text to the kanboard backlog.
    - `Esc` or any other key closes the overlay.
  - `Alt+I` inserts a tab.
  - The stash is persisted in `.unipi/config/input-shortcuts.json` (atomic write).
  - Undo history:
    - Two snapshots per typing burst: before the first key and after the last key.
    - A burst ends after 600 ms idle or after 2 s.
    - Limits: 100 states or 2 M chars, for the session only.
  - **`Ctrl+C` observation:** after pi clears the input, the cleared text is kept as a
    struck-through UI entry (`CLEARED_ENTRY`). `↑` restores it.
  - Status messages: 2 s for success, 3 s for an error.
- **Commands and tools:** none.
- **Keys:** `Alt+S` (chord), `Alt+I`. Raw `onTerminalInput` watches the typing bursts
  and `Ctrl+C`.
- **Settings (`input-shortcuts`):** `chordKey` and `tabInsertKey`. They are shown but
  not applied; the keys stay hard-coded.
- **State:** the stash file and the `CLEARED_ENTRY` entries.
- **Pi API:**
  - on: `session_start` (×2), `session_shutdown` (×2).
  - pi: `registerShortcut` ×2, `appendEntry`.
  - ctx: `sessionManager.getBranch` (last reply).
  - ui: `custom` ×2 (chord overlay), `getEditorText` (×9), `setEditorText` (×7),
    `onTerminalInput` ×2, `setStatus`, `setWidget(PROBE_WIDGET)`.
- **UI:** the chord overlay (a key legend) and a transient status slot.
- **External:** clipboard write.
- **Inter-package:** core; kanboard `captureToBacklog` (soft).
- **Tests:** 7 files, about 35 cases.

## 2.22 fusion — lead plus persistent sidekick, and the model picker

- **Purpose:** pair a strong lead model with a cheaper sidekick in a persistent child.
  Also a better model picker.
- **Features:**
  - **`/unipi:model` picker:**
    - Lists all models with their prices and a thinking level per row.
    - Single-model rows and a Fusion row (lead, sidekick and effort for each).
    - `Alt+Enter` saves the selection as the startup model.
    - Autocomplete puts `/unipi:model` first when the user types `/model`.
    - Pi's own `/model` or `Ctrl+P` ends Fusion mode (`model_select`).
  - **Sidekick:**
    - One persistent child for each lead session. It is a `pi --mode rpc` child that
      loads the lead's extensions and no skills (env `UNIPI_FUSION_CHILD=1`,
      `UNIPI_SUBAGENT_CHILD=1`).
    - Its session file:
      `~/.unipi/workspace/<id>/sessions/<sid>/fusion/sidekick/<lead-session-id>.jsonl`.
    - It compacts its own context. Its shells and servers stay alive.
  - **`sidekick` tool:**
    - By default it blocks until the report arrives. With `block:false` it returns at
      once and a completion message comes later; a wait source holds the nudges.
    - A second call during a run sends an interrupt (steer).
    - Approvals in a blocking handoff go to the lead's UI. In a non-blocking handoff
      they are auto-denied.
    - Each step streams into the lead transcript as a UI-only `sidekick-step` entry.
  - **Lead policy:**
    - A system-prompt section tells the lead what to delegate.
    - The first direct `edit`/`write` of each turn gets a reminder annotation.
    - Every 4 non-trivial `bash` calls since the last handoff, a reminder is added
      (`nudge.ts`, with a list of read-only commands such as `sed` without `-i`).
  - `/unipi:fusion-stats`: the sidekick's tokens, costs, the estimated savings,
    handoffs and runtime state.
  - **Presets:**
    - `~/.unipi/config/fusion/preset.json` and `<project>/.unipi/fusion-preset.json`.
    - They hold the model lists, levels, recents, badges and a `prices` override.
    - Edited with "Edit fusion presets…".
  - Sets the startup model and thinking from settings (`setModel`,
    `setThinkingLevel`).
  - Bus: `FUSION_STATUS` (sticky).
- **Commands:** `/unipi:model`, `/unipi:fusion-stats`. Runner: preset editor.
- **Tools:** `sidekick {message: string, block?: bool (default true)}`; `read_subagent
  {agent_id?, block?, timeout? (s, default 30, max 600)}` (from core).
- **Keys:** in the picker: `↑↓`, typing to filter, `←→` thinking level,
  `Tab`/`Shift+Tab` focus (effort, lead, sidekick), `Space`, `Enter`, `Alt+Enter`,
  `Esc`.
- **Settings (`fusion`):** `startup.model`, `startup.thinking` (global only),
  `default.lead`, `default.sidekick`.
- **State:** the preset files, the sidekick session file and the fusion session state
  dir. Custom entries `sidekick-step` and the `sidekick-completion` custom message.
- **Pi API:**
  - on: `session_start`, `session_shutdown` (stops the child), `before_agent_start`
    (policy section, tool exposure), `turn_start`, `turn_end`, `tool_result`
    (reminders), `model_select`, `agent_settled`.
  - pi: `registerTool`, `registerCommand`, `registerEntryRenderer("sidekick-step")`,
    `registerMessageRenderer("sidekick-completion")`, `appendEntry`, `sendMessage`,
    `getActiveTools`, `setActiveTools`, `setModel`, `setThinkingLevel`,
    `getThinkingLevel`.
  - ctx: `hasPendingMessages`, `isIdle`, `model`, `sessionManager`, `mode`.
  - ui: `custom` ×2 (picker, preset editor), `addAutocompleteProvider`, `setWidget`
    (sidekick wake widget), `notify` (×12).
- **UI:** the model picker overlay, the preset editor, the sidekick step cards, the
  wake widget, the completion card, and the stats text.
- **External:** spawns the child pi. Model prices come from the registry or the
  preset.
- **Inter-package:** core (child runtime, arbiter, work list), subagents (manager and
  shared types), utility (delegated renderer). Exposes `unipi.fusion.api`.
- **Tests:** 18 files, about 135 cases.

## 2.23 app-bridge — live mirror for the UniPi phone app

- **Purpose:** let the UniPi phone app (through `unipi-host`) mirror a pi session that
  runs in a Herdr pane, and drive it.
- **Features:**
  - TUI mode only. Off with `UNIPI_APP_BRIDGE=0`. Never opens in a child.
  - Listens on the unix socket `~/.unipi/bridge/<pid>.sock` (owner-only). Writes
    `<pid>.json` (session file, cwd, herdr pane and socket, run state) and
    `<pid>.files.json`. `sweepDead` deletes the files of dead pids.
  - JSONL wire protocol, version 1. Message types (`case`):
    - `prompt`, `abort`, `answer` (dialogs), `queue_edit`, `queue_move`,
      `queue_promote`, `queue_remove`
    - `history`, `resync`, `watch`, `tree`, `tree_go`, `sessions`, `session_new`,
      `session_resume`, `session_fork`, `session_rename`, `reload`, `compact`
    - `set_model`, `set_thinking`, `set_fusion`, `set_lh_mode`, `lh_graph`,
      `lh_progress_get`
    - `btw`, `btw_list`, `dream_get`, `dream_detail`, `dream_action`
    - `work_background`, `work_log`, `work_rerun`, `work_stop`, `work_transcript`
    - `files`, `file_share`, `media`, `paths_stat`, `search`, `arg_completions`
  - Dialog hub: wraps `ctx.ui.select`, `confirm`, `input` and `editor`, and `ask_user`,
    so that the phone sees the dialogs. The first answer wins. Publishes the
    `unipi.remote-dialogs` racer.
  - `/model`, `/thinking` and `/compact` are mapped to bridge calls. A phone send
    while pi works becomes a steer; queueing a follow-up is also possible.
  - Hidden command `/unipi-app-session` (silenced echo). It runs session operations
    inside a command context, because only command contexts have `newSession`,
    `fork`, `switchSession`, `navigateTree` and `reload`.
  - Env: `UNIPI_BRIDGE_DIR`, `UNIPI_BRIDGE_HOLD_MS`, `UNIPI_BRIDGE_INFLIGHT_MS`,
    `UNIPI_BRIDGE_PENDING_GRACE_MS`, `UNIPI_HOST_DIR`, `HERDR_*`, `PI_VERSION`.
- **Pi API:**
  - on: `session_start`, `session_shutdown`, `session_tree`, `session_compact`,
    `session_info_changed`, `input`, `agent_start`, `agent_end`, `agent_settled`,
    `turn_end`, `message_start`, `message_update`, `message_end`,
    `tool_execution_start`, `tool_execution_update`, `tool_execution_end`,
    `model_select`, `thinking_level_select`.
  - pi: `registerCommand`, `sendUserMessage`, `appendEntry`, `getCommands`,
    `getSessionName`, `setSessionName`, `getThinkingLevel`, `setThinkingLevel`,
    `setModel`.
  - ctx: `abort`, `isIdle`, `reload`, `newSession`, `fork`, `switchSession`,
    `navigateTree`, `sessionManager.{getBranch,getEntries,getEntry,getLeafEntry,
    getLeafId,getSessionFile}`.
  - ui: wraps all dialogs, `getEditorText`, `setEditorText`.
- **UI:** none of its own. It mirrors the transcript, dialogs, work and run state.
- **External:** a unix socket server (`node:net`), a `fork`/`spawn` helper for media
  and paths, and file sharing.
- **Inter-package:** reads every shared holder (fusion, btw, dream, subagents, bg,
  footer TPS, LH progress, child usage, command echo).
- **Tests:** 13 files, about 169 cases.
- **Oino note:** this is **superseded by the architecture**. Oino's frontend-agnostic
  RPC/ACP surface is the mirror protocol, and any number of frontends (TUI, GPUI
  desktop, GPUI mobile) attach to one oino-core session. Do not port app-bridge as a
  WASM extension. Its *behaviours* become host requirements:
  - multi-client attach;
  - first-answer-wins dialogs;
  - a session operations API;
  - a work list;
  - an `arg_completions` RPC;
  - a file and media share.
  See §4.1 C17 and C24.

## 2.24 unipi (umbrella)

See §1.2. In Oino, the bundle is the default extension set that the desktop app and
the installer enable.

---

# Part 3 — Cross-package matrices

## 3.1 Pi events used (extension handlers)

Legend: ● = handles the event. A handler that returns a result is marked with its
effect.

| Event | Used by |
|---|---|
| `input` | core (provenance), compactor (origin marks, compact-then), skill-registry, utility ×3 (attachments expand, answer, provenance), app-bridge |
| `before_agent_start` | core (provenance/arbiter), workflow (plan reminder), long-horizon (gate: sections, tools), memory ×2 (recall, exposure), skill-registry (skills shaping), subagents (exposure), web-api (exposure), ask-user (exposure), kanboard ×2 (reminders, budget), fusion (policy, exposure), utility ×3, watchdog (warnings) |
| `agent_start` | core, long-horizon, memory, kanboard ×2, notify, utility, app-bridge |
| `agent_end` | compactor, long-horizon, memory (save pass), kanboard (disarm), notify, utility ×4, app-bridge |
| `turn_start` | footer, fusion |
| `turn_end` | core (hints), compactor (**returns compaction draft**), fusion, info-screen, app-bridge |
| `context` | btw (filter), compactor (filter), kanboard (filter notices) |
| `before_provider_request` | long-horizon (**filters payload tools**) |
| `message_start` / `message_update` / `message_end` | core (provenance tag at end), footer (TPS), utility, app-bridge |
| `tool_execution_start` / `_update` / `_end` | core, footer, long-horizon (runaway), watchdog (update for output change), app-bridge |
| `tool_call` | core, workflow (**gate: block**), long-horizon (lease), memory, kanboard (**guard: block**), utility |
| `tool_result` | core, compactor, fusion (**annotate**), kanboard ×2 (**badge/annotate**), watchdog (**prefix + isError**) |
| `agent_before_settle` | core arbiter **only** (returns entries + `continue`) |
| `agent_settled` | core, compactor, footer, fusion, kanboard, long-horizon, notify, app-bridge |
| `session_start` | every extension package (23/23) |
| `session_shutdown` | all except command-enchantment, compactor, skill-registry, updater (cleanup of processes, timers, sockets, children) |
| `session_before_compact` | compactor (**returns custom compaction**) |
| `session_compact` | compactor, long-horizon, memory, app-bridge |
| `session_compact_failed` | core hints |
| `session_tree` | btw, app-bridge |
| `session_info_changed` | app-bridge |
| `resources_discover` | skill-registry (**returns skillPaths**) |
| `model_select` | fusion, info-screen, utility, app-bridge |
| `thinking_level_select` | app-bridge |
| `ui_prompt_start` / `ui_prompt_end` | notify |
| Not used | `project_trust`, `session_before_switch`, `session_before_fork`, `session_before_tree`, `context_with_system`, `before_provider_headers`, `after_provider_response`, `provider_stream_event`, `user_bash`, `cache_warming_decision`, `mcp_servers_change` |

## 3.2 ExtensionAPI (`pi.*`) methods used

| Method | Packages |
|---|---|
| `on` | all |
| `registerTool` | core, workflow, long-horizon, memory, utility, subagents, background-tasks, web-api, ask-user, mcp (dynamic), notify, compactor, fusion, umbrella (re-register) |
| `registerCommand` | core, workflow, long-horizon, memory, dream, utility, skill-registry, info-screen, subagents, background-tasks, btw, mcp, notify, kanboard, compactor, footer, updater, fusion, watchdog, app-bridge |
| `registerShortcut` | core, workflow, background-tasks, input-shortcuts |
| `registerMessageRenderer` | btw, kanboard, subagents, fusion, utility |
| `registerEntryRenderer` | core, subagents, fusion |
| `registerToolRenderer` | utility |
| `registerMarkdownTransformer` | utility |
| `sendUserMessage` | core (harness), long-horizon, memory, app-bridge |
| `sendMessage` (custom) | workflow, memory, skill-registry, subagents, background-tasks, kanboard, compactor, fusion, utility |
| `appendEntry` | core, workflow, long-horizon, memory, dream, utility, skill-registry, subagents, background-tasks, input-shortcuts, kanboard, compactor, fusion, app-bridge |
| `getActiveTools` / `setActiveTools` | core, long-horizon, memory, utility, subagents, web-api, ask-user, fusion |
| `getAllTools` | memory |
| `getCommands` | core, utility, skill-registry, app-bridge |
| `setModel` | fusion, app-bridge |
| `getThinkingLevel` / `setThinkingLevel` | memory, btw, compactor, fusion, app-bridge (set: fusion, app-bridge) |
| `getSessionName` / `setSessionName` | utility, notify, app-bridge |
| `events` (on/emit) | core, background-tasks, notify, mcp |
| `registerProvider` | none (long-horizon only reads providers that others register) |
| Not used | `registerFlag`, `getFlag`, `registerVirtualModel`, `registerMcpServer`, `exec`, `setLabel`, `getMcpServers` |

## 3.3 `ctx` and `ctx.ui` surface used

- `ctx`:
  - `cwd`, `hasUI`, `mode`, `model`, `thinkingLevel`, `sessionId`
  - `isIdle`, `hasPendingMessages`, `waitForIdle`
  - `abort`, `compact`
  - `getContextUsage`, `getSystemPrompt`, `getCommands`
  - `modelRegistry.{getAll,getApiKeyAndHeaders}`
  - Command-only: `reload`, `newSession`, `fork`, `switchSession`, `navigateTree`
- `sessionManager`: `getBranch`, `getEntries`, `getEntry`, `getLeafEntry`,
  `getLeafId`, `getHeader`, `getSessionFile`, `getSessionId`,
  `buildContextEntries`.
- `ctx.ui` (count of packages):

  | Method | Packages |
  |---|---:|
  | `notify` | 20 |
  | `custom` (overlays/panels) | 13 |
  | `setWidget` | 6 |
  | `setStatus` | 7 |
  | `confirm` | 6 |
  | `select` | 4 |
  | `input` | 3 |
  | `editor` | 1 |
  | `onTerminalInput` | 4 (core tray, subagents, input-shortcuts, utility) |
  | `getEditorText` / `setEditorText` | 4 |
  | `addAutocompleteProvider` | 3 |
  | `setEditorComponent` | 1 (footer) |
  | `setFooter` | 1 (footer) |
  | `setHeader` | 1 (core hints) |
  | `setWorkingMessage` | 1 (subagents) |
  | `setFocus` | 1 (footer) |

- Not used: `setTitle`, `setWorkingIndicator`, `setHiddenThinkingLabel`, `setTheme`,
  `setToolsExpanded`, `pasteToEditor`.
- Pi internals that unipi imports and would need an Oino equivalent:
  - `createAgentSession`, `SessionManager.inMemory`, `createExtensionRuntime`,
    `ResourceLoader`.
  - Tool definitions: `create{Read,Write,Edit,Bash,Grep,Find,Ls}ToolDefinition`,
    `createLocalBashOperations`.
  - Compaction: `compact`, `generateSummaryWithUsage`, `DEFAULT_COMPACTION_SETTINGS`.
  - Text and markdown: `parseFrontmatter`, `parseSkillBlock`, `truncateTail`,
    `formatSize`, `highlightCode`, `getLanguageFromPath`, `getMarkdownTheme`.
  - Other: `copyToClipboard`, `getAgentDir`, `KeybindingsManager`, `CustomEditor`.
  - pi-tui widgets.

---

# Part 4 — Derived results

## 4.1 (a) Host capabilities for a Rust WASM component extension system

Today's Oino WIT (`wit/extension.wit`, world `extension`) gives:
- host functions: `log`, `secret`, `kv-get`/`kv-set`, `session-append`,
  `session-entries`, `complete`, `notify`, `confirm`, `tool-progress`, `exec`
  (synchronous, output captured);
- guest exports: `init` (registration), `handle-event(kind,payload) -> json`,
  `run-tool`, `run-command`;
- WASI: http outgoing, monotonic clock, filesystem (preopens).

That covers about a fifth of unipi's needs. The full set follows. For each capability,
the entry gives the IDs (C1…), **how pi+unipi do it today**, **who needs it**, and the
**WIT direction**.

**C1 — Lifecycle and event hooks with result semantics.**
- *Today:* `pi.on(event, handler)`. Pi composes the handlers in load order with
  defined semantics (§3.1): notification; chained transform (`input`, `context`,
  `message_end`, `tool_result`, `before_provider_request`); gate (`tool_call` block,
  `input` handled); actionable boundary (`turn_end`/`agent_before_settle` entries and
  `continue`); replacement (`session_before_compact` compaction,
  `resources_discover` paths). Handlers are async. Errors fail open, except
  `tool_call`, which fails closed.
- *Needed by:* all packages.
- *WIT:*
  - Subscribe by name at `init`, so the host dispatches only the subscribed events.
  - The dispatch has a typed payload and a typed result union for each event family.
  - A per-handler deadline (a 2 s default; configurable).
  - Async handlers (a component-model async call, or a poll handle).
  - Deterministic order by load order.
  - The full pi 1.4 event set, including `ui_prompt_start`/`end`,
    `session_info_changed`, `thinking_level_select` and `session_compact_failed`.

**C2 — Tools.**
- *Today:* `registerTool` with TypeBox schemas.
  - The model gets `content`. The tool also returns `details` (state reconstruction
    and rendering), `isError`, `terminate`, `usage` and streaming `onUpdate`.
  - `ctx.signal` handles cancellation.
  - Tools that block for a long time: `ask_user` waits on the UI; `read_subagent` waits
    up to 600 s; `sidekick` blocks until the report arrives.
  - **Dynamic registration after `session_start`** (MCP). Tools can be registered
    again (the umbrella's render wrapper; utility's re-registration of
    read/bash/edit/write).
  - Exposure and annotations exist in pi 1.4.
- *Needed by:* 14 packages.
- *WIT:*
  - `register-tool` at any time, with a JSON Schema, sequential or parallel, exposure,
    annotations and a namespace.
  - An async `run-tool` with a cancel token and progress updates
    (`tool-progress(details)`).
  - A result with `details`, `is-error`, `terminate`, `usage` and image content.
  - A tool may suspend on host calls (UI, child agent, timers) without blocking the
    host.

**C3 — Active tool set and payload control.**
- *Today:* `getActiveTools`, `setActiveTools`, `getAllTools`. long-horizon also
  rewrites the provider payload's tool list in `before_provider_request`, so that the
  prefix stays stable.
- *Needed by:* long-horizon, memory, web-api, ask-user, subagents, fusion, utility.
- *WIT:* `tools-active-get`/`-set`, `tools-all`, and a `before-provider-request` hook
  that can mutate the payload. Oino can also give a first-class "per-turn tool filter"
  that keeps the cache stable, instead of payload surgery.

**C4 — Slash commands.**
- *Today:* `registerCommand` with `getArgumentCompletions`.
  - The handler receives an `ExtensionCommandContext` with `waitForIdle`,
    `newSession`, `fork`, `switchSession`, `navigateTree`, `reload`, `compact` and
    `abort`.
  - `getCommands()` lists everything.
  - Command echo and output capture are host-level in Oino (§1.3.11).
  - Command runners (§1.3.5) cover the missing `executeCommand`.
- *Needed by:* 20 packages.
- *WIT:*
  - `register-command` with a completion export.
  - A command context with session operations.
  - `commands-list`.
  - **`command-invoke(name,args)`** across extensions, which replaces the runner
    registry.

**C5 — Keybindings and input observation.**
- *Today:*
  - `registerShortcut(KeyId)`.
  - `ui.onTerminalInput(data) → {consume}` (the raw-key tap for the tray `↓`,
    `Ctrl+B`, `Ctrl+C` capture, typing bursts and `Backspace` token delete).
  - Editor text get/set.
- *Needed by:* core, workflow, background-tasks, input-shortcuts, subagents, utility.
- *WIT:*
  - Register **intent shortcuts** (key id plus action id), which a frontend maps.
  - An **editor model** API: `editor-get`, `editor-set`, `editor-insert`, and editor
    change events (debounced) with a conditional consume of special keys
    (`↓`-when-empty, `Backspace`-on-token).
  - Raw terminal bytes must not reach extensions (GPUI has none). Turn each unipi
    raw-key use into an intent: the `editor.empty-arrow-down` trigger, the
    `editor.cleared` event, and the attachment-token model.

**C6 — Messages and entries into the session.**
- *Today:*
  - `sendUserMessage(content,{deliverAs: steer|followUp|nextTurn})`.
  - `sendMessage({customType,content,display,details},{triggerTurn,deliverAs})`.
  - `appendEntry(customType,data)` (UI-only, persisted, branch-scoped).
  - Boundary entries returned from `turn_end`/`agent_before_settle`.
  - Harness provenance (§1.3.10) and child-usage entries.
- *Needed by:* 14 packages.
- *WIT:* `send-user-message(content, delivery, provenance)`,
  `send-custom-message(type, content, display, details, trigger, delivery,
  provenance)`, `append-entry(type, data)` (the current `session-append`). Provenance
  is native.

**C7 — Session read access.**
- *Today:* `sessionManager.getBranch`, `getEntries`, `getEntry`, `getLeafId`,
  `getLeafEntry`, `getHeader`, `getSessionFile`, `getSessionId`,
  `buildContextEntries`; session name get/set. Some packages also read **other**
  session files on disk: info-screen usage over `~/.pi/agent/sessions`, dream digests,
  and `/unipi:move`.
- *WIT:*
  - `session-branch`, `session-entries(all-branches)`, `session-entry(id)`,
    `session-header`, `session-id`, `session-file`.
  - `session-name-get`/`-set`.
  - A **sessions index** query (list or read the session files of a cwd or of all
    cwds) with pi v3 entry shapes. It replaces direct fs reads of `~/.oino/sessions`.

**C8 — Agent and session control.**
- *Today:*
  - Model control: `setModel`, `get/setThinkingLevel`, `ctx.model`,
    `modelRegistry.getAll`, `getApiKeyAndHeaders`.
  - Run state: `isIdle`, `hasPendingMessages`, `getContextUsage`, `getSystemPrompt`.
  - Actions: `abort`, `compact({customInstructions})`, `shutdown`.
- *Needed by:* fusion, compactor, info-screen, ask-user, watchdog, long-horizon,
  utility, app-bridge.
- *WIT:*
  - `model-get`/`-set`, `thinking-get`/`-set`, `models-list` (with prices,
    capabilities, image support, reasoning).
  - `run-state` (idle, pending, streaming), `context-usage`, `system-prompt`.
  - `abort`, `compact(instructions)`.
  - **No raw API keys.** Use credentialed calls instead (C9).

**C9 — Model calls (non-session completions).**
- *Today:*
  - Through pi's model registry with the session credentials
    (`getApiKeyAndHeaders`, then the extension's own HTTP or pi's `compact` and
    summarizer). Users: compactor llm, notify recap, the long-horizon verifier and
    progress estimate, utility image recognize, and the pi default summarizer.
  - Plus direct third-party HTTP with stored or env keys: jev (TypeSafe or
    OpenRouter decisions), image generation (OpenRouter images, fal.run), Perplexity.
- *WIT:*
  - `complete(request)` and a streaming `stream(request)` against any registered
    model (provider extension), with a returned usage. The current `complete` is a
    start.
  - `summarize-default(entries, instructions)`, which exposes core's compaction
    summarizer.
  - **`credential-for(provider|base-url)`** gives scoped auth headers for outgoing
    HTTP, so jev and the image transports can use the keys of pi/Oino providers.
    Alternative: a host-side "authenticated fetch".

**C10 — Nested agent sessions (subagents).** This is the largest capability. It has
two modes today:
- **(i) In-process ephemeral sessions.**
  - `createAgentSession` + `SessionManager.inMemory`, with a custom tool list, a
    seeded message history, a custom system prompt and a streamed answer. No
    extensions, or a minimal runtime.
  - Users: btw (seeded from the branch; read/grep/find/ls), memory side-save, utility
    rename (one tool).
- **(ii) Persistent child agents.**
  - `pi --mode rpc` children with their own session files. The parent sends
    `prompt`/`steer`/`abort`/`set_thinking_level`/`get_last_assistant_text`, and reads
    the event stream.
  - The child's `extension_ui_request` is forwarded to the lead UI or auto-denied.
  - Usage is reported to the lead ledger. Children resume from the session file.
  - Child markers (env) switch off nudges, owners and board writes. Extension
    inheritance comes from the lead argv.
  - Tool allow and exclude lists. Nesting depth. Concurrency of 8 (up to 16).
    Cancel grace of 5 s.
  - Users: subagents, the fusion sidekick, long-horizon graph steps (through
    subagents), dream (one-shot `-p` with `--session-dir`), and `bg_run` agent
    commands.
- *WIT:*
  - A host `agent` resource: `agent-spawn(spec{profile, model, thinking,
    system-prompt?, tools allow/deny, extensions policy, seed: none|branch-copy|
    session-file, persistence: memory|file(path), cwd, child-role marker,
    ui-policy: forward|auto-deny(note)|none, max-depth})`.
  - Handle methods: `prompt(msg, delivery)`, `steer`, `abort`, `set-thinking`,
    `last-assistant-text`, `events() -> stream<agent-event>`, `usage()`,
    `close(grace)`.
  - Runs inside oino-core as nested `AgentSession`s, not OS processes. A process mode
    is optional, for isolation or the dream detach.
  - The host enforces depth and concurrency limits, and feeds the child usage into the
    parent ledger (C29).
  - `is-child`/`child-role` is exposed to extensions, which replaces the
    `UNIPI_*_CHILD` env sniffing.

**C11 — OS process spawn and streaming.** Four patterns today:
- (a) Short synchronous exec with JSON stdout: git, ps, getconf, `unipi-kanboard
  --json`, tailscale, `uv`, `which`, `pi install`, `unipi-host cron|browser`,
  clipboard tools, `$EDITOR`.
- (b) Long-running background commands: a detached process group, stdout and stderr
  streamed to a file, an output byte cap, a timeout, SIGTERM then SIGKILL of the
  group, `taskkill /T` on Windows, exit status. Users: background-tasks and
  `isAgent` commands.
- (c) Long-lived stdio JSON-RPC children: MCP servers and `mempalace-mcp`, with
  respawn on crash and idle unload.
- (d) Detached daemons that outlive the session: the kanboard daemon, the MemPalace
  daemon, the dream child.
- Also: **adopting** the running foreground `bash` tool child into the background
  registry (detachable bash), and process introspection (the child tree of the agent
  process, `/proc/<pid>/stat` and `io` for activity).
- *WIT:* the current `exec` covers (a) only, and synchronously. Add:
  - a `process` resource: `spawn(cmd,args,cwd,env,stdin: pipe|null, stdout/stderr:
    pipe|file(path)|null, detached, new-process-group)` with `write-stdin`,
    `read-stdout` (stream), `wait`, `signal(term|kill, group)`, `pid`;
  - exec with a timeout and an env;
  - `process-tree(pid)` and `process-stats(pid)` (cpu, io, state) for watchdog;
  - a host-owned **bash tool hook** so that an extension can request "detach the
    running bash call `<id>` into a background task". Oino's built-in bash tool should
    support detaching natively.
  - Policy: process spawn is a capability that the user grants, with a per-extension
    allowlist.

**C12 — Background-task service.**
- *Today:* the background-tasks package owns the registry. Other packages read it
  through `Symbol.for` or the `pi.events` v1 protocol.
- *Recommendation:* make background tasks **a host service**:
  - a process registry plus output files plus the terminal-notification wake;
  - the `bg_*` tools stay in the extension, which drives the host service;
  - watchdog, footer and notify subscribe through the bus (C16).
- It must support: tasks that wake the agent (the arbiter wait-source rule),
  `isAgent` usage accounting, persistence of finished tasks (30), and a rerun.

**C13 — Timers and scheduling.**
- *Today:* `setInterval` and `setTimeout` in the guest. Examples:
  - watchdog heartbeat ≤ 15 s; footer 1 s; renotify 2 min; memory reader idle
    10 min;
  - LH wait backoff; graph wake aggregation 400 ms; notify settle grace 1.5 s;
  - updater interval; dream run polling; the bridge hold timers; MODULE_READY
    batching 150 ms; status-message expiry 2 to 3 s.
- *WIT:* `timer-set(id, delay-ms, repeat?) / timer-cancel(id)`. Each firing comes as
  an event into `handle-event`, which re-enters the guest. Spinner and animation
  ticks belong to the frontends, not the extensions.

**C14 — Filesystem (scoped).**
- *Today:* direct Node fs. The areas:
  - `~/.unipi/**` config, state, session, cache, logs and tool-results; project
    `.unipi/**`.
  - Project files: the plan file, ralph task files, skill targets, `docs/plans`,
    attachment reads, image saves.
  - Temp dirs; the pi agent dir (sessions, trust, auth, models); `~/.mempalace`;
    `~/.config/mempalace`.
  - Atomic writes (tmp + rename), mode 0600, `flock` locks; fs watch (kanboard
    daemon).
- *WIT:* WASI fs with **named preopens**: `ext-global` (`~/.oino/<ext>/`),
  `ext-workspace-state`, `ext-session` (GC'd), `project` (read; write by grant),
  `tmp`, `cache`, `logs`. Add host helpers: `atomic-write`, `lock(path)` (advisory),
  `watch(path) -> events`, `chmod-0600`. The workspace identity and state layout
  (§1.3.7) become host-defined directories.

**C15 — Networking.**
- *Today:*
  - HTTP client: fetch in web-api (about 81 sites), notify, updater, mcp, the image
    transports, jev, the memory daemon and PyPI, the kanboard daemon health check,
    and Perplexity.
  - **TLS fingerprint impersonation** (`wreq-js` native addon) for smart-fetch.
  - HTTP **server**: the `/unipi:answer web` form on 127.0.0.1. The kanboard daemon
    is native.
  - Unix-socket **client** (Herdr `pane.report_metadata`) and **server**
    (app-bridge).
  - SSE (the kanboard daemon; MCP streamable HTTP).
  - WebSocket: not used.
- *WIT:*
  - `wasi:http/outgoing-handler` with streaming bodies, timeouts and a proxy.
  - A host-side **fingerprinted fetch** option (rquest or wreq in the host; a WASM
    guest cannot do TLS fingerprinting itself).
  - `http-listen(loopback, port) -> incoming requests` for the answer web form, or
    let the host serve extension-provided pages.
  - A unix-socket client by grant (Herdr), or turn Herdr integration into a host
    feature.
  - Network egress allowlist per extension.

**C16 — Inter-extension messaging.** Several channels exist today:
- Typed bus with sticky topics (§1.3.1).
- Function registries:
  - turn arbiter: nudge providers and wait sources;
  - long-horizon inputs: evidence contributors and compaction context;
  - registries for skill conditions, move handlers, info-screen groups, work-tray
    tabs, hints and settings namespaces.
- Request/response services:
  - command runners;
  - the bg-task API v1;
  - the subagents graph run port;
  - kanboard `captureToBacklog`;
  - `fusion.api`, `dream.api`, `btw.api`.
- Foreign `pi.events` channels (`herdr:*`, `rpiv:*`).

*WIT:*
- `bus-publish(topic, json, sticky)`, `bus-subscribe(topic)` (delivered as events,
  sticky values replayed), `bus-get(topic)`.
- **Services:** `service-register(name)` and `service-call(target-ext?, name, json,
  timeout) -> result<json>`, with an export `handle-service-call`. These replace every
  `Symbol.for` holder.
- Host-owned **registries** where the host must decide:
  - the arbiter becomes a host feature: `nudge-propose` is a guest export polled at
    settle, and `wait-reason` is a guest export;
  - evidence and compaction-context also become guest exports that the host collects
    with a time box.

**C17 — UI request and widget protocol (frontend-agnostic).**
- *Today:* the pi TUI API plus the RPC subset (`select`, `confirm`, `input`, `editor`,
  `notify`, `setStatus`, `setWidget`, `setTitle`, `set_editor_text`). Unipi uses far
  more:
  - custom overlays: settings hub, model picker, dashboards, trays, panels, review
    screens, browsers, a JSON editor, a skill grid, chord legends;
  - editor replacement (footer glance frame, btw panel, work tray);
  - footer and header replacement; widgets above and below the editor;
  - autocomplete providers; message, entry and tool renderers; markdown transforms;
    inline images (Kitty);
  - the phone race (first answer wins).

*WIT:* a declarative UI protocol:
- **Dialogs (await an answer):** `select`, `multi-select`, `confirm`, `input`,
  `editor`, and `form` (ask_user: tabbed questions with options, other text and
  attachments). Each has a deadline and a cancel. First-answer-wins across attached
  frontends.
- **Notifications:** `notify(level, text)`, transient `status(key, text, ttl)`.
- **Slots:** `widget(key, placement: above-editor|below-editor|footer|header,
  content: rich-lines|component-spec)`, `working-message`, `title`.
- **Editor chrome:** `frame-segments(top[], bottom[])` with priorities (footer), an
  editor overlay or replacement request (`panel(key, spec)` for btw and the tray),
  and an autocomplete contributor (an export `complete-input(prefix) -> items[] with
  tag, color class, sort tier`).
- **Views (component specs, pushed and patched):** a small widget vocabulary that
  both TUI and GPUI render:
  - layout: list, table/grid, tabs, split pane;
  - text: markdown, code/diff, log tail;
  - data: key-value, meter, gauge, sparkline, bar chart, braille area, image;
  - controls: text input, toggle, key hints.
  - Input comes back as actions (`on-key`/`on-select` → guest event).
  - This covers the settings hub (unless host-rendered, see C18), the dashboards,
    the picker, the plan review, the tray panes, the dream and bg detail views, the MCP
    browser and the updater.
- **Transcript renderers:** declarative render specs for custom message and entry
  types and for tool calls and results (`collapsed-line`, `panel{rail-color,
  source-label}`, `card{fields}`, `diff`, `image`) in densities
  `simple|regular|advanced`. Utility's tool renderers, the harness panels, the
  sidekick and subagent step cards, and the kanboard board render use them. TUI
  components cannot cross the WASM boundary.

**C18 — Settings registry and hub.**
- *Today:* the core engine with a schema-driven TUI hub. Layered files, scopes and
  migrations.
- *WIT:*
  - `settings-register(namespace, schema, defaults, scope rules)`,
    `settings-get(ns)` (layered), `settings-set(ns, key, value, scope)`,
    `settings-unset`, and a `settings-changed` event.
  - Each frontend renders the hub from the schema. Action rows call `command-invoke`
    (C4).
  - Secret fields are kept in the host secret store (extends `secret`).

**C19 — Notifications to the OS and devices.**
- *Today:* node-notifier (notify-send, terminal-notifier, SnoreToast), terminal focus
  detection, and HTTP push (Gotify, Telegram, ntfy, UnifiedPush) from the guest.
- *WIT:* `os-notify(title, body, urgency, action-url?)` through the frontend or host,
  plus `frontend-focused() -> bool`. The HTTP platforms stay in-guest (C15).

**C20 — Clipboard.**
- *Today:* read an image (wl-paste, xclip, pbpaste, powershell) for ask-user and
  utility paste; write text (`copyToClipboard`) for input-shortcuts `Y`.
- *WIT:* `clipboard-read-image() -> option<image>`, `clipboard-write-text(text)`.
  Both route to the active frontend: a GPUI or mobile clipboard, or a terminal OSC52.

**C21 — Open URL / browser and external editor.**
- *Today:* `xdg-open`/`open` (the kanboard board, the answer form, dream), `$EDITOR`
  (answer questions `Ctrl+G`).
- *WIT:* `open-url(url)` and `edit-text-externally(text) -> text`, both routed
  through the frontend.

**C22 — Secrets and credentials.**
- *Today:* settings `secret` fields, env vars (`*_API_KEY`), and pi auth through the
  model registry. web-api reads `auth.json`.
- *WIT:* `secret-get(name)` (exists), `secret-set`, env read by grant, and
  `credential-for` (C9).

**C23 — Environment and identity.**
- *Today:* env (`HERDR_*`, `UNIPI_*`, `TERM_PROGRAM`, `NO_COLOR`), pid, platform,
  cwd, home, git root and branch, the workspace id marker, the pi version, the package
  version, the frontend mode (`ctx.mode`, `hasUI`).
- *WIT:* `host-info{oino-version, platform, frontend-kinds, has-ui, mode}`,
  `workspace{id, root, git-root, branch}`, `env-get(name)` by grant, and `child-role`.

**C24 — Session operations from extensions.**
- *Today:* command-context only: `newSession`, `fork`, `switchSession`,
  `navigateTree(summarize)`, `reload`. app-bridge reaches them through a hidden
  command.
- *WIT:* expose them through a command context, and as RPC methods for frontends
  (ACP covers most of them).

**C25 — Resource and prompt shaping.**
- *Today:*
  - `resources_discover` returns `skillPaths`.
  - `before_agent_start` edits `systemPromptOptions` (skills list, sections,
    guidelines) and returns `message`.
  - `context` filters messages.
- *Needed by:* skill-registry, long-horizon, fusion, memory, workflow, kanboard.
- *WIT:* typed results for these hooks (C1). Prompt sections are keyed and stable
  (prefix-cache rule: no clocks or counters in the system prompt).

**C26 — Compaction integration.**
- *Today:* `session_before_compact` returns a custom compaction. The `turn_end`
  boundary returns a compaction draft. `ctx.compact()`. Pi's summarizer is called by
  the extension.
- *WIT:* typed hook results (C1), `summarize-default` (C9), and a compaction draft
  entry type in the boundary result.

**C27 — Built-in tool access for nested work.**
- *Today:* `create*ToolDefinition` gives read, grep, find, ls, bash, edit and write to
  nested sessions (btw, subagents explore), and wraps the bash operations.
- *WIT:* the agent-spawn spec (C10) names the host tools. `ctx.executeTool(name,
  args)` (pi 1.4) lets an extension run a host tool inside its own tool (for example a
  bounded read).

**C28 — Turn arbiter as a host feature.**
- *Today:* `agent_before_settle`, with one core handler and registered providers.
- *Recommendation:* the host polls each extension's optional `propose-nudge` and
  `wait-reason` exports at settle, with time boxes. It applies the unipi decision
  order (§1.3.3) and delivers one `custom_message`. This also removes the need for a
  shared singleton across components. The pending-work signal (§1.3.4) becomes host
  run state ("idle but waiting: …") that every frontend shows.

**C29 — Usage accounting.**
- *Today:* `unipi-child-usage` entries, tool result `usage`, and the footer's
  incremental scans.
- *WIT:* the host accounts child-agent usage into the parent ledger automatically
  (C10). It exposes `usage-summary(session|scope)`, with prices from the model
  registry, for the footer and info-screen.

Capabilities that need **no host support** (they run in-guest in Rust):
- AES-GCM, sha256 and random ids;
- YAML and front-matter parsing;
- HTML-to-markdown (defuddle/linkedom/turndown → Rust html crates);
- mermaid layout to SVG to PNG (resvg in Rust);
- fuzzy matching, secret redaction, the compaction planner, the runaway detectors and
  the graph engine.

**WIT gap summary vs today:**

| Status | Capabilities |
|---|---|
| Missing entirely | C3 (active-set control), C5, C10, C11 (async/streaming), C12, C13, C15 (server, fingerprint), C16, C17 (all but `notify` and `confirm`), C18, C19 to C21, C24, C26, C28 |
| Partial | C1 (one untyped `handle-event`, no result semantics, no subscriptions), C2 (no dynamic registration, cancel or streaming results), C6 (`session-append` only), C7 (`session-entries` only), C9 (`complete` only), C14 (preopens unspecified), C22 |

## 4.2 (b) Port batches and dependency order

The batches follow the hard dependencies (core → producers → consumers) and the soft
(bus) dependencies. Each batch ships with the unipi tests ported to Rust (or to a
WASM-host harness).

**Batch 0 — Platform (host and WIT; no unipi feature yet).** Blocks everything.
- WIT v1 `oino:extension` with:
  - C1 typed hooks, C2 async tools, C4 commands with completions and
    `command-invoke`;
  - C6 messages and entries with provenance, C7 session reads, C8 control, C9 model
    calls and credentials;
  - C13 timers, C14 scoped fs, C15 HTTP client, C16 bus and services;
  - C17 dialogs, notify, status and widgets, plus the render-spec vocabulary v0;
  - C18 settings registry, C23 host info, C28 arbiter.
- A guest SDK crate (`oino-extension-sdk`, already exists) grows typed bindings plus
  a **`unipi-kit`** library for the shared non-UI logic that every port reuses:
  - state paths, the settings schema builder and atomic JSON;
  - the jev client, harness metadata helpers and bounded output;
  - attachment detection and compaction-savings math.
- Frontend work in parallel: the TUI renders the dialog, status, widget, render-spec
  and settings-hub schemas.

**Batch 1 — Core component and leaf utilities.**
1. `core` behaviours that stay extension-level: hints (Unicrab), the work-tray
   **model** (tabs from services; the frontend renders), command-echo (host),
   model-cache (host: `models-list`), and `/unipi:hint`.
2. `utility` part A: `/unipi:settings` (opens the hub), `/unipi:continue|retry`,
   `/unipi:doctor`, `/unipi:cleanup`, model cache, image tools (`image_*`).
3. `command-enchantment`: an autocomplete contributor (C17).
4. `notify`: the HTTP platforms in-guest, C19 native, `notify_user`, events through
   the bus. Supersedes Oino `notify`.
5. `web-api`: providers in-guest. Smart-fetch needs the host fingerprinted fetch
   (C15).
6. `ask-user`: C17 form dialog and C20 clipboard. Supersedes Oino `ask-user`.
7. `input-shortcuts`: C5 editor model and C20.

**Batch 2 — Process and delegation.** Needs C10, C11 and C12.
1. `background-tasks`: the host bg service plus the `bg_*` tools, the tray tab and the
   wake rule.
2. `subagents`: `run_subagent`, `read_subagent`, profiles, the agent manager and the
   graph port as a service.
3. `btw`: an ephemeral nested session seeded from the branch.
4. `fusion`: the sidekick (a persistent nested session), the `/unipi:model` picker
   (view spec), presets and the lead policy.
5. `watchdog`: needs bg, C11 `process-stats`, and host bash detach.

**Batch 3 — Context, memory and long-running work.**
1. `compactor`: C26. Supersedes Oino `vcc`.
2. `memory`: C11c (the stdio MCP-style reader), the daemon client and the side
   session (C10i).
3. `skill-registry`: C25.
4. `workflow`: the `tool_call` gate, plan mode, the permission dialogs and jev.
   Supersedes Oino `mode-sandbox` (partly).
5. `long-horizon`: needs subagents (graph port), the arbiter (C28), compaction
   context, evidence, jev, C3 and the provider-payload tool filter. Supersedes Oino
   `ralph-loop` and `budget` (partly).
6. `kanboard`: move `crates/kanboard` into the workspace (`crates/oino-kanboard` lib
   + bin) and bring it to the quality gates. Then the extension: the CLI through host
   exec, the guard, the monitor (arbiter), and the daemon as a host-spawned detached
   process. The board UI becomes a GPUI/TUI view; see §4.4.
7. `dream`: needs memory and C10 (a detached or one-shot child). The `craft-skill`
   skill supersedes Oino's `craft-skill`.
8. `mcp`: an MCP client extension (stdio and HTTP), with dynamic tool registration.
   Coordinate with any core MCP decision.

**Batch 4 — Presentation and status.** These are mostly frontend contracts.
1. `footer`: a status model (frame segments plus stats line) on bus sticky state and
   `usage-summary`. Supersedes Oino `footer-status`.
2. `info-screen`: dashboard pages as view specs, the page registry service, the
   splash, and the usage index (C7 sessions index).
3. `utility` part B:
   - transcript render styles (render specs for tools and harness panels);
   - auto-rename (C10i plus jev, `session-name-set`), with Herdr sync as an optional
     host integration;
   - `/unipi:answer` (reply view, questions in the editor, the web form through C15
     listen or a frontend form);
   - attachments (the editor token model);
   - `/unipi:summarize` (skill plus a command), `/unipi:move` (move handlers through
     services, sessions and trust remap);
   - browser state and `/unipi:cron` (depend on `unipi-host`, §4.4).
4. `updater`: redesigned for Oino's distribution (an update check of the Oino
   release, or of extension packages). The README and changelog browser becomes a
   markdown view.

**Not ported as extensions:**
- `app-bridge` → oino RPC/ACP multi-client (host); see §2.23.
- `unipi` umbrella → the desktop or installer default bundle.
- `kanboard-bin` → the Oino installer or bundle.
- The core TUI kits → the frontends.

Dependency graph (→ = depends on):

```text
B0 platform ─┬─► B1 core/hints/tray-model, utility-A, command-enchantment, notify, web-api, ask-user, input-shortcuts
             ├─► B2 background-tasks ─► watchdog
             │   subagents ─► fusion ;  btw
             ├─► B3 compactor ; memory ─► dream ; skill-registry ; workflow ; mcp
             │   long-horizon (─► subagents, arbiter, compaction-ctx, evidence)
             │   kanboard (─► crate port, arbiter, long-horizon soft)
             └─► B4 footer (─► bus topics from workflow/LH/kanboard/fusion/bg)
                 info-screen (─► all MODULE_READY, sessions index)
                 utility-B (─► render specs) ; updater
```

## 4.3 (c) Current Oino built-ins and the packages that supersede them

The charter removes every current built-in. These unipi packages replace them:

| Oino built-in (`extensions/built-in/*`, `extensions/crates/ext-*`) | What it does today | Superseded by | Notes |
|---|---|---|---|
| `ralph-loop` (`ext-ralph`) | Iterative loop, auto-continue, steering, history, state files, cleanup/nuke, promise tags | **long-horizon** (ralph mode, plus goal, swarm and graph) | Unipi ralph has task-file checklists, a verifier, budgets and arbiter delivery. The Oino "promise tag" and "nuke" semantics are dropped. |
| `vcc` (`ext-vcc`) | Deterministic compaction and recall | **compactor** (`vcc` method, `session_recall`, `context_budget`) | Same lineage (vcc = "lossless"). The compactor adds the llm method, the percent trigger, active-work blocks and redaction. |
| `notify` (`ext-notify`) | ntfy for selected lifecycle events | **notify** | Adds native, Gotify and Telegram, the event matrix, re-notify, recap and `notify_user`. |
| `ask-user` | Structured question tool with a TUI modal | **ask-user** | — |
| `footer-status` | Footer lines: model, thinking, cwd, context | **footer** (+ info-screen) | — |
| `budget` (`ext-budget`) | Run caps (turns, cost, wall time, consecutive tool errors), an optional summary turn | **long-horizon** goal budgets (turns, stall, tokens) + **watchdog** (hung tools) + the long-horizon runaway guard | **Gap:** unipi has no plain-session cost or wall-time cap and no consecutive-error cap. Decide whether to drop these or add them to long-horizon or a small extension (§4.4). |
| `presets` (`ext-presets`) | `/preset minimal\|standard\|thorough`: tool set plus a prompt section | No direct counterpart. Nearest: **workflow** (plan and permission modes), **long-horizon** gate (tool sets per mode), **skill-registry** (exposure), **fusion** presets (model presets) | The charter removes it. Tool-set presets are not a unipi feature. |
| `search` (`ext-search`) | Deferred grep/find over the project, loaded with `tool_search` | **Core tools**, not unipi. Pi has `grep`, `find` and `ls` tool definitions; unipi's btw and explore use them | Recommend that oino-core ship pi's optional grep, find and ls tools. No unipi port is needed. |
| `mode-sandbox` | `/mode <profile>`: prompts, tool allow-lists, agent guidance | **workflow** (plan mode, permission modes) + **subagents** custom profiles (tool allow-lists) | — |
| `craft-skill` | Skill to design and validate skills | **dream**'s bundled `craft-skill` skill (scaffold and check scripts) | — |
| `router` (OmniRoute auth/router UI) | Routes models through OmniRoute | None in unipi. **fusion** `/unipi:model` replaces the model-picker UX | Providers stay WASM provider extensions (charter). |
| `oino-router`, `provider-anthropic`, `provider-openai` (+ `provider-openai-compat`, `-responses`, `provider-oino` crates) | Providers | **Not superseded**: they stay, as charter providers | — |
| `fixture`, `fixture-provider`, `oino-extension-guest*`, `oino-provider-kit` | Test fixtures and the SDK | Not superseded: the platform | The guest SDK grows for batch 0. |

## 4.4 Open questions and risks (for the planning owner)

1. **State root mapping.** The choices are `~/.unipi/**` → `~/.oino/unipi/**`, or
   folding each module into `~/.oino/<module>/` and `.oino/`. Also: should the
   workspace-id marker (`.unipi-workspace.json`) become `.oino-workspace.json`, or
   should Oino use the cwd-keyed session dir layout of the charter? Do we migrate
   existing unipi data? There are zero Oino users, but the author has unipi data.
2. **Subagent execution model.** Nested `AgentSession` in-process (recommended:
   speed, shared provider extensions, no pi CLI), or a child `oino --mode rpc` process
   (isolation, parity with unipi, and the sidekick's shells live in a separate
   process)? The sidekick "keeps shells alive" either way through the process
   service.
3. **Turn arbiter location.** As a host feature (recommended, C28), or as a privileged
   "unipi-core" component? A host feature changes pi parity a little: pi has no
   arbiter, and pi composes `agent_before_settle` handlers.
4. **jev / TypeSafe dependency.** Five packages need it, and it is a third-party
   decision API (`typesafe/jev-1.13`). Keep it as is, or allow any small model
   through `complete` with a structured-output prompt? It fails open, so a missing
   key only degrades the features.
5. **Kanboard web board.** Today the daemon serves the **unipi-app** web build, which
   is out of scope. Options: (a) keep the deprecated SolidJS fallback UI; (b) serve no
   UI and render the board as a GPUI/TUI view over the JSON API (recommended); (c)
   ship a new web UI.
6. **`unipi-host` features** (cron jobs, PC browser profiles, app push key, app
   endpoints) belong to the unipi app host, which is out of scope. `/unipi:cron`, the
   `browser` and `cron` skills, the notify app-push encryption and the browser-state
   context have no backend in Oino. Drop them, or re-home them as Oino host features?
7. **TLS fingerprinting** for web-api smart-fetch needs a host-side client (C15). Is a
   native dependency (for example `rquest`) acceptable in oino-core or an
   `oino-net` crate?
8. **MCP.** The charter excludes pi-mcp "for now", but unipi's `mcp` package *is* an
   MCP client. Port it as an extension in batch 3, or wait for a core MCP decision?
9. **Updater semantics.** Unipi updates itself from npm. Oino bundles its extensions,
   so the updater must target Oino releases or extension packages. This needs a design.
10. **Budget gap** (§4.3): drop, re-home or keep plain-session cost and wall caps.
11. **Herdr integration** (pane titles, `herdr:*` claims) is specific to one
    multiplexer. Keep it as an optional host integration, or drop it.
12. **The render-spec vocabulary** (C17) is the largest design decision for frontend
    agnosticism. About 13 packages draw custom overlays today. Agree on a small
    component set before batch 1 UI work.
13. **pi 0.87 → 1.4 behaviour diffs.** Unipi was written against 0.87.1. Check the
    1.4 semantics of `agent_before_settle` (entries copy), `turn_end` compaction
    drafts, `message_end` replacement, and `before_agent_start` `systemPromptOptions`
    against the unipi assumptions while porting long-horizon, compactor and
    skill-registry.
