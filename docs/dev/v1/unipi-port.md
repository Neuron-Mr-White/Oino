# unipi 3.0.0 port plan

Status: binding. Owners: `unipi-core`, `unipi-ux`, `unipi-agents`, `unipi-context`,
`unipi-integrations`, `unipi-kanboard` (paths in README §4). Reference:
[audit-unipi.md](audit-unipi.md) (feature inventory per package — the porting checklist) and
`/home/oi/Projects/Personal/archived/unipi` @ `c90133d`.

**Parity rule.** Every command, tool (name + schema + result text), key, setting key/default,
persisted state shape, bus topic and behaviour listed for a package in audit-unipi Part 2 is
ported unless this document cuts or changes it. Port to pi 1.4 semantics (audit-unipi §0).
Unipi unit tests are ported to Rust (native `MockHost` tests or wasm-host harness tests);
the approximate counts in audit-unipi §0 are the floor.

## 1. Component list, ids, ownership, batches

Bundled load order (= umbrella order, with `unipi.core` first): core, workflow, long-horizon,
memory, dream, utility, skill-registry, info-screen, subagents, background-tasks, btw,
web-api, ask-user, mcp, notify, kanboard, command-enchantment, compactor, footer, updater,
input-shortcuts, fusion, watchdog.

| Package | Extension id | Dir (`extensions/unipi/…`) | Owner | Batch |
|---|---|---|---|---|
| core (subset) | `unipi.core` | `core` | unipi-core | B1 |
| utility | `unipi.utility` | `utility` | unipi-core | B1 (part A), B4 (part B) |
| command-enchantment | `unipi.command-enchantment` | `command-enchantment` | unipi-ux | B1 |
| input-shortcuts | `unipi.input-shortcuts` | `input-shortcuts` | unipi-ux | B1 |
| ask-user | `unipi.ask-user` | `ask-user` | unipi-ux | B1 |
| footer | `unipi.footer` | `footer` | unipi-ux | B4 |
| info-screen | `unipi.info-screen` | `info-screen` | unipi-ux | B4 |
| notify | `unipi.notify` | `notify` | unipi-integrations | B1 |
| web-api | `unipi.web-api` | `web-api` | unipi-integrations | B1 |
| mcp | `unipi.mcp` | `mcp` | unipi-integrations | B3 |
| updater | `unipi.updater` | `updater` | unipi-integrations | B4 |
| background-tasks | `unipi.background-tasks` | `background-tasks` | unipi-agents | B2 |
| watchdog | `unipi.watchdog` | `watchdog` | unipi-agents | B2 |
| subagents | `unipi.subagents` | `subagents` | unipi-agents | B2 |
| btw | `unipi.btw` | `btw` | unipi-agents | B2 |
| fusion | `unipi.fusion` | `fusion` | unipi-agents | B2 |
| long-horizon | `unipi.long-horizon` | `long-horizon` | unipi-agents | B3 |
| compactor | `unipi.compactor` | `compactor` | unipi-context | B3 |
| memory | `unipi.memory` | `memory` | unipi-context | B3 |
| skill-registry | `unipi.skill-registry` | `skill-registry` | unipi-context | B3 |
| workflow | `unipi.workflow` | `workflow` | unipi-context | B3 |
| dream | `unipi.dream` | `dream` | unipi-context | B3 |
| kanboard | `unipi.kanboard` (+ crate `oino-kanboard`) | `kanboard` | unipi-kanboard | B3 |

Not ported (D22): `app-bridge` (replaced by `oino serve` multi-client + mobile app),
`unipi` umbrella (replaced by bundling + load order above), `kanboard-bin` (replaced by the
Oino installer shipping `oino-kanboard`).

Batch readiness: B1 needs wit-v1 SDK + core-runtime host services for registry/session/ui/
settings/bus/http. B2 needs `agents`, `process`, `tasks`. B3 needs B2 subagents (for
long-horizon) and `summarize-default`. B4 needs render specs and the settings hub in frontends.
Within a work item, follow batch order.

## 2. Cross-cutting decisions

### 2.1 Mapping of unipi core subsystems (audit-unipi §1.3)

| unipi core | Oino |
|---|---|
| bus (`unipi.bus`), `UNIPI_EVENTS`, sticky topics | host `bus`; topic names unchanged (`unipi:…`); the 5 sticky topics published with `sticky: true` |
| `pi.events` foreign channels | host bus (same topic names); `herdr:*` dropped; `rpiv:ask-user:prompt` kept as a bus topic |
| `Symbol.for` holders | services / sticky topics, §2.2 |
| turn arbiter | host arbiter (runtime §7.6): long-horizon `nudge-provider` (priority 100), kanboard claims (50) + autowork (40) as nudge providers; wait sources: fusion (non-blocking handoff), subagents (background run); background tasks are host wait sources automatically |
| pending-work monitor | host `run_state: waiting` (frontends show "waiting on …") |
| evidence contributors | service fan-out: long-horizon calls every service named `evidence.contribute` (`services.list()`), 2 s box each |
| compaction context | service fan-out `compaction-context.provide` (compactor calls each; long-horizon, kanboard provide) |
| command runners | `control.command-invoke` + hidden commands (`hidden: true`) for every runner name in audit-unipi §1.3.5 (`plan-enter`, …) |
| skill conditions | service fan-out `skill-condition.check{skill}` → `{hidden: bool}` (skill-registry calls) |
| move handlers | service fan-out `move.handle{oldRoot, newRoot, dryRun, backup}` (utility `/move` calls) |
| progress entries | `custom` entries `unipi-progress` + renderer (kanboard) |
| settings engine + hub | host settings registry (runtime §10.3), namespaces unchanged, hub rendered by frontends (ui-protocol §6.6) |
| workspace identity & layout | preopens + `wsid` (D10), §2.3 |
| jev client | `unipi-kit::jev` (fail-open), §2.4 |
| child-agent runtime | host `agents` (D8) |
| harness messages | host provenance (`send-message`/`send-user-message` `provenance`); `unipiHarness` tag fields map to `provenance{source, title, synopsis, severity}` and `delivery` |
| command echo | host feature (runtime §7.4) |
| work list + tray | `unipi.core` builds the tray (service fan-out `work-tray.tab` → tab specs) and renders it as an `editorReplace` panel |
| hints | `unipi.core` |
| TUI kits, shared ticker | frontends (OUS, ui-protocol §6.3) |
| remote dialogs (phone race) | host first-answer-wins |
| child usage | host automatic child `usage` entries (runtime §12) |
| detachable bash | host `control.bash-detach` / `tasks.adopt-bash` |
| model cache | `control.models-list` |
| compaction savings | `unipi-kit::savings` |
| bounded output, attachment detection | `unipi-kit` |

### 2.2 Shared state → services and topics

| unipi holder | Replacement |
|---|---|
| `unipi.background-tasks.shared-registry` | host `tasks.list` + sticky topic `unipi:bg:tasks` (published by background-tasks on change) |
| `unipi.subagents.shared-list/-manager/-subscribe/.transcript` | services `subagents.list`, `subagents.transcript{id}`, `subagents.cancel{id}`, `subagents.foreground{id}` + sticky topic `unipi:subagents:list` |
| `unipi.subagents.graph-run-port` | service `subagents.graph-run{step}` → `{runId}`; completion via bus `unipi:subagents:run-done{runId, result}` |
| `unipi.fusion.api` | services `fusion.status`, `fusion.set-pair` |
| `unipi.btw.api` | service `btw.ask{question}` |
| `unipi.dream.api` | services `dream.status`, `dream.run`, `dream.stop` + tray tab |
| `unipi.footer.shared-tps` | sticky topic `unipi:footer:tps` |
| `__unipi_info_registry` | service fan-out `info-screen.groups` → page/group specs |
| kanboard `captureToBacklog` | service `kanboard.capture{title, body?, priority?}` |
| `__unipi_load_times`, contributions | `_oino/extensions` (load time + registration counts recorded by the runner) |

### 2.3 State paths (D10, D11)

| unipi path | Oino |
|---|---|
| `~/.unipi/config/<ns>/config.json`, project `.unipi/config/<ns>/config.json` | `settings.json` → `extensionSettings.<ns>` (global / project) |
| `~/.unipi/global/<module>/…` | `/ext-global/…` |
| `~/.unipi/workspace/<id>/state/<module>/…` | `/ext-workspace/…` |
| `~/.unipi/workspace/<id>/sessions/<sid>/<module>/` | `/ext-session/…` |
| `~/.unipi/cache/…`, `logs/…`, `tool-results/…` | `/cache/…`, `/logs/…`, `/tmp/…` |
| project `.unipi/ralph/<name>.md` | `.oino/ralph/<name>.md` (`fs.project: write`) |
| `docs/plans/<date>-<sid>.md` (workflow) | unchanged (project write) |
| `.unipi-workspace.json` marker | not written (D10) |
| env `UNIPI_*_CHILD` | `call-context.is-child` / `child-role` (`subagent`, `fusion`, `kanboard`) |
| env `UNIPI_DEBUG_*` | extension log level (`logs/ext/<id>.log`, setting `extensionSettings.<ns>.debug: bool`) |

Oino has **no legacy unipi support** (owner decision, work `unipi-native`): nothing reads,
aliases or imports `~/.unipi` / `.unipi` data, there are no migrate commands or start-up
prompts, and legacy argument shapes, renderers, settings keys and state-file imports that only
existed for unipi installs are removed. Oino's own data lives under `~/.oino` and `.oino`.
Feature commands carry no `unipi:` prefix ([commands.md](commands.md)).

### 2.4 jev

`unipi-kit::jev::ask(state, questions, settings) -> Option<Answers>`: transports TypeSafe
`/v1/systemone` (1 000 ms) and OpenRouter decisions `/api/alpha/decisions` (6 000 ms);
key order: secret `decision-model/apiKey` → env `TYPESAFE_API_KEY` / `OPENROUTER_API_KEY`
(permission `env`) → `credential-for(provider matching baseUrl)`. Fail-open (`None`). Shared
namespace `decision-model` (keys `provider`, `model` = `typesafe/jev-1.13`, `baseUrl`,
`apiKey` (secret), `timeoutMs`). Additional source `model`: uses `models.complete` with a
structured-output prompt against a configured model (opt-in, never default).

### 2.5 Cuts and changes (D22)

- Dropped: `/cron`, `cron` and `browser` bundled skills, browser-state context,
  notify app-push encryption, Herdr integration (`herdrSync`, `herdr:*`), app-bridge,
  `/unipi-app-session`.
- Budget caps (Oino `budget`) move into watchdog (§3.10).
- Kanboard: no web UI; board panel (§3.16).
- Updater: Oino releases (§3.20).
- Dream: in-process agent; interrupted runs resume next start (D8).
- Payload surgery (long-horizon `before_provider_request` tool filtering) is replaced by
  `set-active-tools`: Oino's system-message tool deltas keep the cache prefix stable.
- Inline terminal images (Kitty) → OUS `image` nodes.
- `onTerminalInput` uses → editor intents/shortcuts: tray `↓` → `emptyArrowDown`; `Ctrl+B`
  (subagents foreground) → shortcut `unipi.subagents.foreground` default `ctrl+b`;
  Backspace-on-token → `tokenBackspace`; typing bursts → `changed`.

## 3. Per-package plan

Format: **Perms** = manifest permissions (extension-api §7.3; `ui` implied). **Caps** =
capability ids. **Notes** = port deltas. Settings namespaces, commands, tools, keys = as in
audit-unipi unless noted.

### 3.1 `unipi.core` (unipi-core, B1)
Perms: `osNotify`. Caps: C1 C4 C5 C13 C16 C17. Content: Unicrab hints (127 tips data file,
moment tips, show counts in `/ext-global/hints.json`), `/hint [next|reset]`, shortcuts
`unipi.hint.next` (`alt+h`), `unipi.hint.prev` (`alt+shift+h`), hint widget (`setWidget`
aboveEditor), start header (`setHeader` with OUS; large crab ≥72 cols as a `text` art node,
`image` variant when frontend supports images), work tray (shortcut `unipi.tray.open`
`shift+down`, `emptyArrowDown` intent consumption when items exist, tabs from
`work-tray.tab` services, `editorReplace` panel, dim hint line via `setWidget` belowEditor),
service `core.open-tray{tab?}`. Replaces: nothing in Oino.

### 3.2 `unipi.utility` (unipi-core, B1 part A, B4 part B)
Perms: `sessionControl`, `models.complete`, `credentials: ["*"]`, `network.hosts`
(`openrouter.ai`, `fal.run`, `queue.fal.run`, image hosts per settings), `fs.project: read`,
`tools.override: [read, bash, edit, write]`, `agents.spawn`, `clipboard.read`,
`services.call: ["*"]`, `network.listenLoopback`, `editExternally`.
Part A: `/settings [search]` (→ `openSettings`), `/continue`, `/retry`,
`/doctor`, `/cleanup`, image tools `image_generate`/`image_edit`/`image_recognize`
(models `generate-image`/`complete` or provider HTTP with `auth: provider`), namespaces
`utility`, `image`. Part B: render styles (`render.style` → tool render hints + `ui.density`),
auto-rename (nested agent with tool `rename_session`, jev gate, `session.set-session-name`),
`/answer [reply|questions|web]` (reply panel; questions in editor via `editor-set`;
web form via loopback `http.listener`), attachments (paste detection → `insertEditorToken`),
`/summarize [q]`, `/move` (service fan-out `move.handle` + session file move via
`sessions-list`/core `import`), rename runner `rename-now`. Drops `/cron`,
browser state.

### 3.3 `unipi.command-enchantment` (unipi-ux, B1)
Perms: none. Caps: C17 (guest `complete-input`). Namespace `command-enchantment`
(`autocompleteEnhanced`). Provides tagged/colored completion groups.

### 3.4 `unipi.input-shortcuts` (unipi-ux, B1)
Perms: `clipboard.write`, `services.call: ["unipi.kanboard/kanboard.capture"]`. Caps: C5 C17
C20. Shortcuts `unipi.chord` (`alt+s`), `unipi.tab-insert` (`alt+i`); chord legend as
`custom` dialog; `Y` yank → `clipboard-write-text`; capture-to-backlog via kanboard service.
Namespace `input-shortcuts`.

### 3.5 `unipi.ask-user` (unipi-ux, B1)
Perms: `osNotify`, `clipboard.read`. Caps: C2 C17 C20. Tool `ask_user` (schema exact) →
`form` dialog (ui-protocol §4.3); result JSON text identical to unipi; `action: end_turn` →
tool `terminate`; `action: new_session` → tool `terminate` + `control.command-invoke(
"ask-user-new-session", prefill)` (hidden command; queued until idle per runtime §7.4)
which calls `control.new-session` then `ui.editor-set(prefill)`; `notifyOnAsk` → os-notify. Replaces Oino `ask-user`.

### 3.6 `unipi.footer` (unipi-ux, B4)
Perms: `process.spawn` (`git`), `fs.project: read`. Caps: C16 C17 C29. Frame segments via
`setFrame` (model, thinking, cwd, branch, context %, cost, tps, PLAN/mode badges from sticky
topics `unipi:workflow:status`, `unipi:long-horizon:state`, `unipi:kanboard:status`,
`unipi:fusion:status`, `unipi:bg:tasks`), stats line via `setFooter`; `/footer
[on|off]`; publishes `unipi:footer:tps`. Replaces Oino `footer-status`.

### 3.7 `unipi.info-screen` (unipi-ux, B4)
Perms: `sessions.readAll`, `process.spawn` (`git`), `fs.project: read`. Caps: C7 C16 C17.
`/info [page]` → `overlay` panel with `tabs`; pages from `info-screen.groups` fan-out +
built-ins (usage index over `sessions-list`/`session-read`, modules page from
`_oino/extensions`-equivalent `control.commands-list`/`all-tools` + service list); splash via
`setHeader` per `bootMode`. Namespace `info-screen`.

### 3.8 `unipi.notify` (unipi-integrations, B1)
Perms: `osNotify`, `network.hosts: ["*"]` (user-configured Gotify/ntfy/Telegram hosts;
allowed because the component is bundled), `secrets`, `models.complete`. Caps: C1 C15 C16 C19. Tool `notify_user`, commands
`/notify-event`, runners set-gotify/set-ntfy/set-telegram/test as hidden commands,
event matrix, re-notify (timer 2 min), recap (complete), "All done" via host run state.
Note: user-configured hosts are granted through manifest `network.hosts: ["*"]` (bundled
extensions may use `*`). Replaces Oino `notify`.

### 3.9 `unipi.background-tasks` (unipi-agents, B2)
Perms: `process.spawn` (`*`), `sessionControl`. Caps: C11 C12 C16 C17. Tools `bg_run`,
`bg_status`, `bg_logs`, `bg_kill` (schemas exact) over host `tasks`; `/bg`,
`/bg-tasks`; tray tab service `work-tray.tab`; shortcut `unipi.bg.clear` (`ctrl+alt+c`);
service `tasks.notification` renders the `<background-task-notification>` custom message text
exactly as unipi (followUp delivery, trigger turn when `triggerOnCompletion`). Publishes
`unipi:bg:tasks`.

### 3.10 `unipi.watchdog` (unipi-agents, B2)
Perms: `process.spawn` (`ps`, `getconf`), `sessionControl`, `network.hosts` (jev), `env`
(`TYPESAFE_API_KEY`, `OPENROUTER_API_KEY`), `secrets`. Caps: C11 C13 C16. Stuck-tool detection
via `control.running-bash` + `process-stats`/`process-tree`, jev stuck check,
`/bg-detach [reason]` → `control.bash-detach`. **Session caps (D22)**: settings
`sessionCaps.{maxTurns, maxCostUsd, maxWallMinutes, maxConsecutiveToolErrors, summaryTurn}`
(all off by default) and flags `--max-turns`, `--max-cost`, `--max-wall`, `--max-tool-errors`;
on reaching a cap: `control.abort()`, notify, and (if `summaryTurn`) one follow-up user message
asking for a summary. Replaces Oino `budget`.

### 3.11 `unipi.subagents` (unipi-agents, B2)
Perms: `agents.spawn` (`maxDepth: 2`), `fs.project: read`, `sessionControl`. Caps: C2 C10
C16 C17 C27 C28 (wait-source). Tools `run_subagent` (profile enum rebuilt on profile change),
`read_subagent` (registered here; fusion uses service `subagents.register-external`);
profiles: bundled + `/ext-global/profiles/*.md` + project `.oino/agents/*.md`;
`/subagents` (tray tab), `/agents` (profile manager panel); shortcut
`unipi.subagents.foreground` (`ctrl+b`); background runs deliver completion custom messages;
graph-run service; concurrency default 8 (cap 16). `UiPolicy`: foreground = Forward,
background = AutoDeny with unipi's note text.

### 3.12 `unipi.btw` (unipi-agents, B2)
Perms: `agents.spawn`. Caps: C10 C17. `/btw [question]` → `editorReplace` panel;
ephemeral agent `seed: BranchCopy`, tools `read, grep, find, ls`, `persistence: Memory`,
streaming answer into the panel (`updatePanel` coalesced ≤ 20/s); history within the panel.

### 3.13 `unipi.fusion` (unipi-agents, B2)
Perms: `agents.spawn`, `sessionControl`, `models.complete`. Caps: C8 C10 C16 C17 C28.
Sidekick = persistent nested agent (`persistence: File`), tool `sidekick {message, block?}`;
`/fusion` picker as `custom` dialog (effort/lead/sidekick focus, filter, thinking via
`select`/`list` nodes); `/fusion-stats`; presets + preset editor hidden command; lead
policy; wait-source when a non-blocking handoff is pending. Replaces Oino `router` UI
(model-picker UX).

### 3.14 `unipi.long-horizon` (unipi-agents, B3)
Perms: `sessionControl`, `models.complete`, `credentials: ["*"]`, `fs.project: write`,
`network.hosts` (jev), `services.call: ["unipi.subagents/*", "*/evidence.contribute"]`.
Caps: C1 C2 C3 C6 C16 C25 C26 C28 (nudge-provider 100). All modes (goal, ralph, swarm, graph,
regular), all tools (schemas exact), owner coordinator (`/ext-workspace/state.json`,
`goal.json`, `graph.json`), verifier, budgets, runaway guard, compaction-context provider,
`/visualize-progress` (mermaid → SVG → PNG in guest via `resvg`; OUS `image`, text
fallback). Ralph task files at `.oino/ralph/<name>.md`. Replaces Oino `ralph-loop` (promise
tags and nuke dropped).

### 3.15 `unipi.compactor` (unipi-context, B3)
Perms: `sessions.readAll`, `models.complete`, `sessionControl`. Caps: C1 C7 C9 C26. Methods
`vcc` (deterministic, in guest) and `llm` (`summarize-default` or `complete`); hooks
`session_before_compact` (supplies `compaction`), percent trigger (`control.compact`),
active-work blocks via `compaction-context.provide` fan-out, redaction; tools
`session_recall`, `context_budget`; commands per audit. Replaces Oino `vcc`.

### 3.16 `unipi.kanboard` + `oino-kanboard` (unipi-kanboard, B3)
Crate: move `crates/kanboard` → `crates/oino-kanboard` (lib + bin `oino-kanboard`), edition
2024, quality gates; remove the web UI serving; keep JSON API + SSE; config/data paths under
`~/.oino/state/unipi.kanboard/`. Extension perms: `process.spawn` (`oino-kanboard`, `git`),
`process.detached`, `network.hosts` (`127.0.0.1`, `localhost`), `fs.project: read`,
`sessionControl`. Caps: C11 C15 C16 C17 C28. Commands per audit; guard (`tool_call`),
monitor (nudge providers 50/40), evidence contributor, compaction context, daemon as detached
process; board = `sidePane` panel (`split` of `list` columns, actions move/open), refreshed
from SSE (`http.fetch` stream). The `oino-kanboard` binary is installed next to `oino`
(installer/bundle) and found via the program allowlist (the host prepends the Oino install dir
to the search path for extensions).

### 3.17 `unipi.memory` (unipi-context, B3)
Perms: `process.spawn` (`mempalace`, `mempalace-mcp`, `uv`), `process.detached`,
`fs.paths: [{path: "~/.mempalace", access: write}, {path: "~/.config/mempalace", access:
write}]`, `agents.spawn`,
`network.hosts` (`pypi.org`, `files.pythonhosted.org`). Caps: C2 C10 C11 C25. Memory files at
`/ext-global/memory/<project>/<type>/<id>.md`; warm `mempalace-mcp` stdio reader (process
pipe, idle unload 10 min timer); side-save nested agent; all tools and commands per audit.

### 3.18 `unipi.skill-registry` (unipi-context, B3)
Perms: `fs.agentDir: read`, `fs.project: read`, `network.hosts` (jev). Caps: C1 C25.
`before_agent_start` → modified `systemPromptOptions.skills`; `resources_discover` →
`skillPaths` (vault); skill conditions fan-out; `/skills` → `openSettings{namespace:
"skills"}`; skill grid as settings page (`multiselect`).

### 3.19 `unipi.workflow` (unipi-context, B3)
Perms: `fs.project: write`, `sessionControl`, `network.hosts` (jev), `secrets`. Caps: C1 C2
C5 C6 C17. `tool_call` gate (order per audit) raising `approval` dialogs (options Allow once
/ Always allow / Deny / Deny with note → `allow_once`, `allow_always`, `reject_once`,
`reject_once` + feedback); plan mode (plan file, reminder custom message, `plan_submit` tool,
review `custom` dialog with markdown + 3 actions); shortcuts `unipi.plan.toggle` (`alt+p`),
`unipi.permission.cycle` (`alt+m`); publishes `unipi:workflow:status`. Replaces Oino
`mode-sandbox` (with subagents profiles for tool allow-lists).

### 3.20 `unipi.dream` (unipi-context, B3)
Perms: `sessions.readAll`, `agents.spawn`, `fs.agentDir: read`. Caps: C7 C10 C13 C16 C17.
Due check on `session_start` (`minSessions`, `minGapHours`), scrubbed digests from other
sessions, background nested agent run (`persistence: File` under `/ext-global/runs/`), tray
tab, `/dream …`; bundled skill `craft-skill` (with `scaffold`/`check` scripts) replaces
Oino `craft-skill`.

### 3.21 `unipi.web-api` (unipi-integrations, B1)
Perms: `network.hosts: ["*"]`, `network.fingerprint`, `secrets`, `credentials: ["*"]`,
`models.complete`. Caps: C2 C9 C15. Tools `web_search {query, source?: 1..6}`,
`multi_web_content_read` (full schema), `web_llm_summarize`; providers in guest (wigolo,
DuckDuckGo, Jina, SerpAPI, Tavily, Perplexity); smart-fetch via `impersonate`; HTML→markdown in
Rust (`html2md`-class crate + readability extraction); cache in `/cache`; hidden command
`web-cache-clear`.

### 3.22 `unipi.mcp` (unipi-integrations, B3)
Perms: `process.spawn: ["*"]`, `network.hosts: ["*"]`, `fs.agentDir: read`, `fs.project:
read`, `secrets`. Caps: C2 C11 C15 C16 C17. MCP client (stdio via process pipes; streamable
HTTP/SSE via `http.fetch`), config `~/.oino/mcp.json` + trusted `.oino/mcp.json` (pi file
names) + ACP `session/new` servers from bus topic `oino:acp:mcp-servers`; dynamic tools
`<server>__<tool>`; `/mcp-status`; hub runners as hidden commands; browser panels.

### 3.23 `unipi.updater` (unipi-integrations, B4)
Perms: `network.hosts: ["api.github.com", "github.com", "objects.githubusercontent.com"]`.
Caps: C13 C15 C17. Periodic check (`checkIntervalMs`, default 1 h) of
`https://api.github.com/repos/Neuron-Mr-White/Oino/releases/latest`; `autoUpdate: notify|off`
(no in-extension install; notification tells the user to run `oino update` or use the desktop
updater); `/readme [extension]` (bundled `docs` of that extension as `markdown` panel),
`/changelog` (bundled Oino CHANGELOG).

## 4. Oino built-ins replaced (charter: all removed)

| Removed Oino built-in | Replacement |
|---|---|
| ralph-loop | `unipi.long-horizon` |
| vcc | `unipi.compactor` |
| notify | `unipi.notify` |
| ask-user | `unipi.ask-user` |
| footer-status | `unipi.footer` (+ `unipi.info-screen`) |
| craft-skill | `unipi.dream` bundled skill |
| mode-sandbox | `unipi.workflow` + `unipi.subagents` profiles |
| budget | `unipi.watchdog` session caps |
| presets | none (dropped) |
| search | core `grep`/`find`/`ls` (D19) |
| router UI | `unipi.fusion` `/fusion` |
| providers | `extensions/providers/*` (providers-v1) |

## 5. Shared kit (`extensions/sdk/unipi-kit`, unipi-core, B0/B1 first deliverable)

Modules: `settings` (schema builder matching ui-protocol §6.6 + typed access), `jev`,
`provenance` (helpers building `provenance` records with unipi `delivery`/`severity`),
`bounded` (head/tail bounded output + spill to `/tmp`), `attach` (path detection →
`[Image #N]`/`[File #N]`), `savings`, `paths` (state helpers over preopens), `fmt`
(`format_tokens`, version compare), `fanout` (call every service of a name with a time box).
Must land before any other unipi package (unipi-core publishes its API first; other unipi items
may start with the SDK alone and adopt the kit when available).

## 6. Acceptance per package

1. All commands/tools/keys/settings present with exact names, schemas and texts.
2. Ported unit tests green (native `MockHost`), plus at least one wasm-host harness test per
   package exercising registration and one end-to-end behaviour.
3. Works in TUI and GPUI (panels/dialogs render; degraded RPC rendering acceptable).
4. Manifest permissions are minimal (reviewed in the item report).

---

## Appendix: settled deviations (folded at 1.0.0-alpha.2)

Sources: `deviations/unipi-{core,ux,agents,context,integrations,kanboard}.md` and `extensions/unipi/utility/DEVIATIONS.md`. Row numbers are kept so existing references stay valid.

### kit and `unipi.core` (unipi-core)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | unipi-port §2.4 | jev key order is: resolved settings `apiKey` → secret `decision-model/apiKey` in the caller's own secret store → the shared `decision-model.apiKey` secret field read through `registry.extension-settings` (the namespace declares `secretReaders: ["unipi.*"]`) → env → `credential-for`. | Secrets are namespaced per extension (extension-api §5.9); the host resolves the shared field for declared readers (R2, core-settings). | Components pick the shared-key step up when rebuilt (core, workflow, skill-registry, utility already are). |
| 2 | unipi-port §2.2 / §2.1 | New bus topic `unipi:work-tray:changed` (one-shot) and the Oino holder topics `unipi:bg:tasks`, `unipi:subagents:list`, `unipi:footer:tps` (sticky) and `unipi:subagents:run-done` are named in `unipi_kit::topics`; `topics::STICKY` = unipi's 5 + those 3. | The tray needs a change signal (unipi used `subscribeWorkChanges`). | Producers publish via `unipi_kit::bus::emit_raw`, which applies stickiness automatically. |
| 3 | unipi-port §2.1 "work list + tray" | Work-tray contract: fan-out `work-tray.tab(TabRequest{open, initialId?})` → `TrayTab` or `[TrayTab]` (`{id, label, shortLabel?, order, counts{total,running}, visible, body?: OusNode}`); interactive `actionId`s inside `body` are `<tabId>:<action>`; `unipi.core` forwards them to the tab owner as point service `work-tray.action{tab, actionId, value?}` → `{close, refresh}`. Tab switches use the `tabs` node `onChange: "$tab"`. | The spec names the service but not its payloads. | background-tasks, subagents and dream implement these (`unipi_kit::tray`). |
| 4 | unipi-port §3.1 (start header) | The header is `setHeader{key: "unipi.core/header", spec}` with `spec.t = "responsive"`: `variants: [{minWidth: 72, node: large}, {minWidth: 40, node: compact}]`, `fallback: large`. | Extensions do not know the frontend width; OUS v1 has no width-dependent node. Frontends that do not know `responsive` render `fallback` (ui-protocol §6.1 unknown-node rule). | Request R4. |
| 5 | unipi-port §3.1 (art) | Unicrab art is OUS half-block text with one tone per glyph (palette → `error`/`warning`/`default`; black outline and shell shadow transparent); `hints.crab = image` uses an OUS `image` node with the unipi PNG. | OUS spans carry no colours, only tones (ui-protocol §6.5). | Visual only. |
| 6 | audit-unipi §1.3.13 | Catalog is unipi's `lines.ts` converted to `assets/hints.json`: **132** hints (the audit's 127 is approximate). `match` closures became declarative predicates (`{field, eq}`, `{field, in}`, `{field, wordPrefix}`, `{all}`). Two texts changed: `/kanboard open` "launches the task board in your browser" → "shows the task board panel" (D22: no web UI), "how fast Pi got ready" → "how fast Oino got ready" (D13). | WASM has no closures in data; parity with D13/D22. | — |
| 7 | audit-unipi §1.3.13 (moment tips) | The late-night tip never fires. | `host-info` exposes no local time zone; UTC would fire at the wrong hours. | Request R3. |
| 8 | audit-unipi §1.3.13 (browser) | `/hint` opens an overlay panel `unipi.core/hints` (filter `input` + one `section`/`list` per category, status badge seen/unseen/learned). Filtering applies on submit, not per keystroke. | Panels exchange actions, not keystrokes. | — |
| 9 | unipi-port §3.1 (perms) | Manifest permissions: `services.call: ["*/work-tray.tab", "*/work-tray.action"]`; `osNotify` dropped. | The tray calls producers; nothing in `unipi.core` raises OS notifications. | Minimal grant set. |
| 10 | unipi-port §3.1 (tray ↓) | `emptyArrowDown` is answered `{consume: true}` after the tray panel opened (one fan-out round trip). | No safe way to spawn the open after replying (imports need the call context). | If the round trip exceeds the frontend's 50 ms window the frontend also runs its default (a no-op on an empty composer). |
| 11 | extension-api §9.3 | Tests: pure logic is unit-tested natively (`cargo test -p unipi-kit -p unipi-core` in `extensions/`); the component is tested in the wasm host by the standalone crate `extensions/unipi/core/harness` (own `[workspace]`, run with `harness/run.sh`), with a test-only producer component `harness/probe`. `export_extension!` is gated on `target_arch = "wasm32"` in `unipi.core`. | No `MockHost` (wit-v1 deviation 13); native `cdylib` links fail on the `:`-qualified export symbols; the extensions workspace must not depend on wasmtime. | Request R5, R6. |
| 12 | runtime §10.3 | `unipi_kit::settings::Store` values come from `InitInput.settings` and, on `refresh` / `oino_settings_changed`, from `registry.extension-settings(namespace)` (secret fields resolved by the host; fallback `control.get-settings()["extensionSettings"]`), layered over the registered defaults. `Store::write` / `unset` persist through `registry.set-extension-setting` and re-read the namespace. | The changed event carries keys, not values. | — |
| 13 | audit-unipi §1.2 | Umbrella behaviours not in the kit: contribution ledger/load times (`_oino/extensions`), `simple` render-style tool wrapper (utility B, render hints), command echo (host), `migrateState`/`sweepOrphanSessions` (host state GC, D10). The kit's `suite` module carries the load order, ids and namespace owners. | Host features per unipi-port §2.1. | — |
| 14 | unipi-port §5 (`fmt`) | `unipi_kit::fmt::visible_width` counts chars (no East Asian width / combining marks). | No unicode-width dependency in guests; frontends do exact layout. | Hint-line fitting is approximate for CJK labels. |

### `unipi.utility` (unipi-utility, `extensions/unipi/utility/DEVIATIONS.md`)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | unipi-port §3.2 (drops) | `/cron`, browser state + `browser`/`cron` skills, Herdr pane sync (`rename.herdrSync` stays in the defaults, no schema field) are not ported. | D22. | — |
| 2 | audit §2.5 render | Built-ins (`read bash edit write grep find ls`) are rendered through **tool renderers** (`register-renderer {target: tool}`) returning OUS for `simple` (mcode row; expanded = advanced view) and `advanced` (command/heredoc `code` nodes, output `log` tail, test summary, exit line, `diff` from pi's display diff, numbered `code` for write/read); `regular` returns `None` (frontend-native). The style comes from `utility.render.style` (it is not written into core `ui.density`, no guest settings write). Read-group collapsing ("Read 3 files"), the user-card patch, transcript spacing, the reply background, thinking hiding, image collapse and the markdown transformer are frontend concerns and are not ported; other extensions' tools keep their own renderers. | Re-registering built-ins is rejected by core (`Tool "read" is already registered`; `tools.override` render-only registration is not implemented), and render specs are per call. | Requests U1, U2. |
| 3 | audit §2.5 harness panels | The 10 known harness custom types render as OUS `card`s (rail `harness`/`warning`) in the three densities; meta from `details.unipiHarness` or `details.oinoProvenance`, else the per-type fallback. The delegated-work panel is the service `utility.render-delegated {steps, label?, style?, expanded?}` → OUS card; `utility.render-style` reports the style. | No cross-component imports. | fusion/subagents may call the service. |
| 4 | audit §2.5 auto-rename | The one-tool naming session is a nested agent (`extensions: only [unipi.utility]`, `tools.extensionTools: [rename_session]`, `childRole: rename`, memory persistence, system prompt replaced, 45 s abort); `rename_session` is registered inactive and refuses calls outside that child role. When `agents.spawn` fails (hosts without D8) it falls back to one `models.complete` call offering the same tool (`toolChoice: required`). The debug trail is `extensionSettings.utility.debug` → `/logs/rename.log` (was `UNIPI_DEBUG_RENAME`). | Equivalent for a one-turn, one-tool session; core-agents (8d98cba) provides `agents.spawn`. | — |
| 5 | audit §2.5 answer | `reply` = `custom` dialog (markdown reply, multiline `input` `onSubmit: send`, "Answer one by one" button = Tab); `questions` = `editor` dialog prefilled with the Q/A template (Oino field `cursorLine` = first `A1:` line; Tab-between-answers is a frontend feature); `web` = host `http.listener` on 127.0.0.1 (47321 first over SSH, then any port) with the waiting overlay as an `openPanel` (Esc / `$close` cancels), browser via `ui.open-url`, the `ssh -L` hint uses env `HOSTNAME`. Page branding says Oino. | No TUI components in guests; `editor-set` is unsupported in the core bridge, the editor dialog is unipi's own mechanism. | Request U3 (`cursorLine`). |
| 6 | audit §2.5 attachments | Paste detection uses `oino_editor` `changed`/`cleared` intents (scan + `ui.editor-set`), atomic delete uses `tokenBackspace` on a trailing text token, submit expansion uses the `input` transform (images appended to `images`), the chip row is `setWidget unipi-attachments` (OUS `image` previews by host `path`), the transcript line is the `unipi:attachments` entry renderer. Files are read only through granted folders: the project, `~/Downloads`, `~/Desktop`, `~/Pictures`, `~/Documents`, `/tmp`; other paths stay plain text. | No `onTerminalInput`; no arbitrary host reads (never `~/.oino/auth.json`). | Request U4 (frontend token insertion / host read of a pasted path). |
| 7 | audit §2.5 image | Provider routing for `image_recognize` goes through `models.complete` (core builds the provider image part and holds keys) instead of per-API HTTP; custom `recognize.baseUrl` stays direct OpenAI-compatible HTTP. Generate/edit keys: setting → secret `image/<key>` → `credential-for` → env. Saved images default to `/ext-global/images` (`~/.oino/state/unipi.utility/global/images`); `generate.outputDir` is honoured only when it maps to a writable preopen. The OpenRouter image list caches in `/cache/image-models-cache.json` (24 h); the shared `models-cache.json` is not written (pickers use `control.models-list` + service `image.models`). | `fs.project` is read-only; D10/D11 paths; core owns credentials. | `credential-for` is served by core (frontend-cmds #9); the env fallback stays. |
| 8 | audit §2.5 move | Sessions are moved through the `~/.oino/sessions` grant (`/oino-sessions`; read-only fallback `/agent/sessions`) with the header `cwd` rewritten, body bytes kept, conflicts skipped, empty dirs removed; temp-file + rename instead of copy + hard link. `trust.json` is host-owned: shown as a manual step. Registered handlers run through fan-out `move.handle {oldRoot,newRoot,dryRun,backup}` (dry run for the plan, then apply); their log lines are the plan. Existence checks use `process.exec test -e`; git checks use `git worktree list/repair`. Log: `/logs/move-<ts>.log`; no backup tree (nothing the utility rewrites needs one). Orphans come from session directories whose recorded cwd is gone. A custom `sessionDir` / non-default agent dir is not migrated. | No host API for trust or the sessions root. | Request U5. |
| 9 | audit §2.5 doctor/cleanup | Doctor checks the extension's preopens, project `.oino/settings.json` / `.oino/mcp.json`, Oino version/platform/frontends, model registry + image catalog, Decision Model key, skill exposure; the Node/heap and wreq-js/VC++ checks are dropped (no native deps). Cleanup's allowlist covers the utility's own `/tmp` (`tool-results/*`, `unipi-*` older than 7 days); other extensions' and core temp trees are host-GC'd (D10). | Guests only see their own folders. | Request U6. |
| 10 | audit §2.5 summarize | `/summarize` sends `/skill:summarize [focus]` with `expandPromptTemplates: true` and provenance through the raw `session.send-user-message` (the SDK helper has no provenance/expand options); in non-UI modes it waits with `control.wait-for-idle`. | SDK surface. | — |
| 11 | extension-api §7.3 | Manifest grants: `sessionControl`, `sessions.readAll`, `models.complete`, `credentials: ["*"]`, `tools.override` (declared for U1), `agents.spawn` (depth 1), `process` `git`/`test`, `network.hosts: ["*"]` (custom image endpoints and CDN downloads; bundled only) + `listenLoopback`, `fs.project: read`, `fs.agentDir: read`, the folder grants of #6/#8, `clipboard.read`, `openUrl`, `editExternally`, `services.call: ["*"]`, `secrets`, env `HOME USERPROFILE HOSTNAME SSH_* OPENROUTER_API_KEY TYPESAFE_API_KEY FAL_KEY FAL_API_KEY`. | What the code calls. | — |
| 12 | extension-api §9.3 | Tests: 100 native unit tests (`cargo test -p unipi-utility`) + standalone wasm-host harness `harness/` (`run.sh`; a `TestHost` wraps `RecordingHostServices` with a branch, session name, models list and scripted `models.complete`). | No `MockHost` in the SDK. | — |

### ask-user, command-enchantment, input-shortcuts, footer, info-screen (unipi-ux)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | ui-protocol §7.3 | `complete-input` returns one `CompletionGroup` with Oino fields `replaces: "commands"` (the frontend drops its own slash group) and `prefix`; items carry `tag` (`[workflow]`), `tone` (`c1…c12`) and `sortTier` (display rank). Built-in pi slash commands are rebuilt in the guest (pi 1.4.0 `BUILTIN_SLASH_COMMANDS`, `quit` text → Oino). | unipi wraps pi's base provider; Oino merges groups, so the enchanted group must supersede the base one to re-rank across sources. | tui-client/gpui honour `replaces`; without it they show both groups. Request Q1. |
| 2 | audit-unipi §2.17 | unipi's argument re-trigger fix (`force: false` on Tab after a space) is not ported; argument completions stay with `complete-command`. ANSI package colours map to categorical tones (packages sharing a colour share a tone). `/cron` dropped from the registry (D22). | Frontend concern in Oino. | — |
| 3 | unipi-port §3.4 | Typing bursts are driven by debounced `changed` editor intents instead of raw terminal input; text we set ourselves is ignored once (echo). Ctrl+C capture uses the `cleared` intent with an optional Oino field `reason: "interrupt"\|"submit"\|"edit"`; without `reason`, a clear of non-blank text not preceded by a submit counts. "↑ restores" relies on the frontend history; the extension cannot push into it. | No `onTerminalInput` (ui-protocol §9). | Request Q2 (`reason` on `cleared`, history push). |
| 4 | unipi-port §3.4 | Chord overlay is a `custom` dialog with buttons plus Oino fields `keys: [{key, actionId, label}]` and `closeOnUnknownKey: true` (single-letter selection, any other key closes). The stash lives in workspace state `/ext-workspace/<wsid>/input-shortcuts.json` (same `{stash}` shape), not `.unipi/config/` in the project. Kanboard capture calls `unipi.kanboard/kanboard.capture` with `{cwd, text, body}` and expects unipi's `{ok, id, attachments}` / `{ok: false, reason}`. | D10 (no implicit project writes); unipi-port §2.2 payload `{title, body?, priority?}` lacks unipi's no-title/attachments semantics. | unipi-kanboard implements the reply shape. Request Q3. |
| 5 | unipi-port §3.5 | The question dialog is a `form` with Oino presentation hints `escape`, `digitAdvance`, `helpLine`, `clarify: true` (frontend shows "? Not ready to answer, help me out!" which answers `{clarify: true}`); "Other" attachments come back as `answers[].attachments` (images) and `answers[].files` (`[{id, path}]`). The launcher offers Compact & run / Run directly / **New session** (hidden command `ask-user-new-session`) / Cancel. Instead of `ctx.abort()` the handoff result sets `terminate`. Subagent detection uses `call-context.is-child` (role `subagent` or none). OS notification on ask only when `unipi.notify` is not loaded (else notify re-notifies from the bus event). | ui-protocol §4.3 `form` lacks these; unipi-port §3.5 mandates the new-session path. | Frontends render the hints; request Q4. |
| 6 | unipi-port §3.6 | Footer is a status model: `setFrame` segments carry `id`, `priority` (drop order kanboard → fusion → permission → PLAN → mode; thinking before model) and Oino fields `bold`, `style: "badge"`, `effect: "rainbow"` (brand; frame-wide at xhigh/max with `rainbow: always`), `spans` + `fallback` (Fusion pair degrading to model│thinking); request-level `iconStyle`, `colorMode`. The strip is `setFooter{key: "strip", spec, segments, separator, minRows: 20}`. The process line is `setWidget footer-top` (aboveEditor) from sticky `unipi:bg:tasks`. The "Working…" line is the host `run_state: waiting`. Icons: `nerd` → OUS icon names, `emoji` → glyphs in text, `text` → `branch:`/`workspace:`/`mode:`/`kanboard:` labels. | Frontends own layout and animation (ui-protocol §1.5, §6.3). | Request Q5 (segment fields). |
| 7 | unipi-port §3.6 | `/footer on\|off` writes `footer.enabled` to the global settings file (the old kv override is imported once and is only used when the write is refused); the git branch is read from `/project/.git/HEAD` (falls back to `host-info.workspace().git-branch`); `process.spawn(git)` is not requested. Subscription billing (`sub`) is read from model fields `subscription`/`usingOAuth`. `unipi:footer:tps` publishes `{liveTps, sessionAvgTps, avgTtftMs, streaming, totalOutput, turns, steps, llmMs, toolMs}`. Child cost also counts Oino host `usage` entries (runtime §12). | `fs.project: read` suffices; no host API for OAuth-ness. | Requests Q6. |
| 8 | unipi-port §3.7 | Dashboard = overlay panel with a `tabs` node (`onChange: "page"`) and panel keys `r`/`R`/`q`/`1–9`; page bodies are OUS (`card` tiles, `meter`, `barChart` incl. Oino `columns: true` for the 30-day chart, `area`, `list` with Oino `columns: "auto"`, `kv`, `table`). Module pages arrive through fan-out `info-screen.groups` → `{id, name, short?, priority, config: {stats}, data, render?}` (a request payload `{id}` asks one page); unipi's renderers for mcp/compactor/memory/web-api/updater/input-shortcuts stay in info-screen. | No shared `globalThis` registry. | Producers (compactor, memory, mcp, web-api, updater) serve the group with their data. |
| 9 | unipi-port §3.7 | Usage index: `sessions-list(all)` + `session-read` per changed file, per-file cache keyed by `(modified, size \| messageCount)` in `/cache/usage-stats.json`, written after each dirty refresh (not 30 s debounced). Periods and the 30-day chart use UTC days; the splash greeting uses the UTC hour. | No local time in `host-info` (unipi-core R3). | Off by the UTC offset near midnight. |
| 10 | unipi-port §3.7 | Modules page counts tools/commands/services per extension from `control.all-tools`/`commands-list` `sourceInfo.source = "extension:<id>"` and `services.list()`; per-module load times and settings field counts are not available (boot time = init → first `session_start`). | `_oino/extensions` is not exposed to guests. | Request Q7. |
| 11 | unipi-port §3.7 | Splash is `setHeader{key: "splash"}` with a `responsive` spec (≥76 large crab, ≥44 compact, else empty; unipi-core R4) and Oino `effect` hints (`shimmer`, meter `drain` with `durationMs`). `auto-close` clears it after `bootTimeoutMs` (min 500); `on` clears it at the first input/editor change (a header cannot capture keys). Crab grids are a copy of `unipi.core`'s asset. | Non-capturing by design; no cross-extension asset sharing. | — |
| 12 | unipi-port §3.7 (perms) | Manifest: `sessions.readAll`, `fs.project: read`, `services.call: ["*/info-screen.groups"]`; `process.spawn(git)` not requested (branch from `.git/HEAD`). input-shortcuts: `clipboard.write`, `services.call: ["unipi.kanboard/kanboard.capture"]`. ask-user: `sessionControl` (set-active-tools, send-user-message), `osNotify`, `clipboard.read`. footer: `fs.project: read`. command-enchantment: none. | Minimal grants for what the code calls. | — |
| 13 | extension-api §9.3 | Tests: native unit tests per crate (113) plus one standalone wasm-host harness crate for the whole batch, `extensions/unipi/ask-user/harness` (`run.sh`; env `UNIPI_UX_WORKSPACE` builds from another workspace root when sibling work in progress breaks `extensions/`). | Same constraints as unipi-core #11. | Request Q8. |

### background-tasks, watchdog, subagents, btw, fusion, long-horizon (unipi-agents)

| # | Spec section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | §3.9 | Agent-task telemetry wrapping (`isAgent` → wrapped pi child reporting context/tokens/tools, `turndown`) is not ported; `isAgent` is kept in the schema/snapshot and `telemetryUnavailableReason` explains it. | Host tasks run plain shell commands; nested agents replace pi children (D8). | `bg_status` lacks `ctx=… tokens=… tools=…` for agent tasks. |
| 2 | §3.9 | Task ids, output paths and the output cap come from the host task service (`t…` ids, `~/.oino/tmp/tasks/…/output.log`); "dismiss" hides finished tasks in this extension only; `maxFinishedTasks` caps what is shown (host keeps 30). Stop = host kill (no SIGTERM→SIGKILL grace; the host kills the process group). | Host owns processes (extension-api §5.5). | Kill texts are unipi's; the 5 s stop wait polls host state. |
| 3 | §3.9 | The host renders the `<background-task-notification>` through `tasks.notification`, but sends it without `deliverAs: followUp` and with `details: {taskId}` (renderer re-reads the task). | Host `task_finished` builds the message (crates/oino-wasm-host/src/hub.rs). | Request R1. |
| 4 | §3.9 | Tray pane keys become OUS buttons/list actions (`bg:open`, `bg:stop:<id>`, `bg:kill:<id>` armed twice, `bg:stopall` armed twice, `bg:dismiss…`, `bg:rerun:<id>`, `bg:full:<id>`, `bg:path:<id>`); live log refresh = 1 s `work-tray:changed` ticks while the tray is open. | No raw key input for extensions. | Frontends bind keys to the actions. |
| 5 | §3.10 | Process sample uses host `process-stats`/`process-tree` (cpu ms, read/write bytes, state) for the pid from `control.running-bash`; `comm`, `wchan`, sockets, stdin are unavailable, so the sleeper veto also looks at the command text. `kill` action = `control.bash-detach` + `tasks.kill` (no `ps` scan); without detach support the item downgrades to warn. | Guests cannot read `/proc`. | `process.spawn` allow list is empty (no `ps`/`getconf`). |
| 6 | §3.10 | Session caps (D22) implemented as `sessionCaps.{maxTurns,maxCostUsd,maxWallMinutes,maxConsecutiveToolErrors,summaryTurn}` + flags; cap = `control.abort` + notify + `watchdog.session-cap` entry + one follow-up summary message; the summary run is aborted if it keeps calling tools. | New Oino feature (no unipi source). | — |
| 7 | §3.10 | `UNIPI_DEBUG_WATCHDOG` → setting `watchdog.debug` (extension log); `UNIPI_WATCHDOG_FORCE_ACT` dropped. | unipi-port §2.3. | — |
| 8 | §3.11 | `UiPolicy` is fixed at spawn: foreground runs forward dialogs, background runs auto-deny with unipi's note. Moving a run between foreground and background (Ctrl+B, tray `f`) cannot switch the policy. | `agents` has no policy setter. | Request R2. |
| 9 | §3.11 | Live foreground card = `subagents-foreground` widget (OUS cards) refreshed on child events (≤ 4/s); "Esc cancels" = tool cancellation; a pending user message moves the run to the background (unipi behaviour). | No TUI components. | — |
| 10 | §3.11 | Child usage is not re-recorded (host appends `child:<label>` usage entries, runtime §12). Nesting: `run_subagent` is not registered in child sessions; profiles with `max-nesting` raise the spawn `maxDepth` but the child cannot see its own allowance. | `call-context` has no depth. | Request R3. |
| 11 | §3.11 | Global profiles live in `/ext-global/profiles/` (= `~/.oino/state/unipi.subagents/global/profiles/`), project profiles in `.oino/agents/` (manifest `fs.project: write` for `/agents` create/copy/edit). The model picker in `/agents` uses `control.models-list` + unipi's fuzzy match. | unipi-port §2.3. | — |
| 12 | §3.11 | `read_subagent` dispatch to other owners: an owner calls `subagents.register-external {owner}` and serves `subagent-reader.read` (`{probe:true, agent_id?}` → `{owns, latest}`; otherwise `{agent_id?, block?, timeout?}` → `AgentToolResult`). | Replaces unipi's in-process reader registry. | Fusion implements it. |
| 13 | §3.11 | Graph runs: `subagents.graph-run {title, task, profile, resume?}` → `{runId}` or `{error, retryable}`; completion on bus `unipi:subagents:run-done {runId, result:{status,text,error,toolCalls,usage}}`; `subagents.profiles` lists runnable profile ids (empty when disabled/child). | Spec named only the service. | long-horizon uses it. |
| 14 | §3.12 | btw's side session uses the default system prompt + appended btw prompt (`systemPrompt: append`) instead of the stripped main system prompt; (unipi's legacy `btw-note` renderer and context filter are not ported). Answers stream into the panel (≤ 20 updates/s); keys = panel keys (`ctrl+c`, `up`, `down`) + input submit. | `AgentSpec` cannot replace the prompt with the parent's. | Minor wording difference in the aside's system prompt. |
| 15 | §3.13 | `/fusion` is a dialog sequence (row `select` with `optionDetails` pricing → lead/sidekick/effort selects → "Confirm" / "Confirm and use at startup"), not the single interactive picker; the preset editor is two `multiSelect` dialogs + target. | Custom dialogs resolve on the first action and cannot carry several field values. | Same capabilities, more steps. |
| 16 | §3.13 | "Alt+Enter startup default" writes `startup {model, thinking, active}` into the global preset and applies it at the next fresh session start (plus `fusion.startup.*` settings); pi's `settings.json` `defaultModel` is not written. | Guests cannot write core settings. | Request R4. |
| 17 | §3.13 | The sidekick spawns with `ui: forward` (blocking default); non-blocking handoffs therefore also forward approvals instead of auto-denying. | Same as #8. | Request R2. |
| 18 | §3.14 | Payload surgery replaced by `set-active-tools` (spec §2.5); hidden mode tools are also blocked in `tool_call`. Tools register inactive and the gate activates the mode's set each turn. | Spec. | — |
| 19 | §3.14 | Continuation runs in `agent_before_settle` (manifest `limits.hookTimeoutMs: 90000` for the verifier) and is delivered by the nudge provider; when the session is idle (commands, timers, graph checkpoints) messages go out as harness user messages (provenance `Long-horizon`). | Host arbiter (runtime §7.6). | — |
| 20 | §3.14 | `/visualize-progress` shows mermaid source (`code` node, `language: mermaid`) plus a text summary in an overlay; no in-guest mermaid layout / resvg PNG. | Size/time; frontends may render mermaid. | Request R5 (OUS `mermaid`/image rendering). |
| 21 | §3.14 | The goal progress estimate uses an Oino prompt (`{percent, summary}` JSON) and shows a `lh-progress` status + `unipi-progress` entry; unipi's progress-chart/snapshot views are reduced to the `unipi:long-horizon:progress` sticky snapshot. | Not ported line-for-line. | Footer/app consumers read the snapshot. |
| 22 | §3.14 | Lease/generation checks on goal tool calls, the judge's chat-model fallback (`typesafe.ts`) and the graph's per-operator usage accounting are not ported; the judge uses `unipi-kit::jev` (fail-open). `swarm/graph` checkpoint wake aggregation = 400 ms timer; capacity refusals retry every 5 s. | Simplification; jev decisions transport covers the default. | — |
| 23 | §3.14 | Owner/goal/graph state is workspace-scoped (`/ext-workspace/<wsid>/{state,goal,graph}.json`) like unipi; ralph state sits next to the task file (`.oino/ralph/<name>.state.json`). `/continue` (mentioned in the mode section) belongs to utility. | Spec paths. | — |
| 24 | all | Native tests are the ported unipi unit tests for pure logic; host-behaviour coverage is in the wasm-host harness `extensions/unipi/subagents/harness` (standalone crate, `run.sh`). | `MockHost` does not exist (unipi-core R5). | Integration: add the harness to CI. |

### workflow, skill-registry, dream, compactor, memory (unipi-context)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | unipi-port §3.19, audit §2.1 (settings) | `permission.mode` (project file) and saved rules written by the gate (`Always allow`, Alt+M, `/permission <mode>`) go to `extensionSettings.permission` of the settings file of their scope through `registry.set-extension-setting`, like unipi. Rules are read per file and concatenated (global ++ project) instead of the merged value (arrays replace). The old stores `/ext-workspace/<wsid>/permission.json` / `/ext-global/permission.json` are imported once at `session_start` and removed; they stay the fallback layer when a write is refused (untrusted project); a hub edit of `mode` clears a fallback project mode. | Arrays do not merge across layers. | — |
| 2 | audit §2.1 (approval prompt) | The prompt is the `approval` dialog: `allow_once` / `allow_always` / `deny` (reject_once) / `deny_note` (reject_once + `feedback`); a `deny_note` answer without feedback asks with `input` as unipi did. Labels and title text are unipi's. | ui-protocol §4.3. | — |
| 3 | audit §2.1 (plan review) | The review overlay is a `custom` dialog: markdown plan (side by side `split` when both `## Summary` and `## Implementation` exist), three buttons `approve` / `keep` / `discard`; a frontend that cannot host it degrades to the unipi `select`. Scroll/focus keys are frontend-owned. | OUS replaces the TUI component. | — |
| 4 | audit §2.1 (kanboard CLI) | The board-CLI allowance also accepts `oino-kanboard` (`--actor agent`). | unipi-port §3.16 renames the binary. | — |
| 5 | audit §2.1 (`UNIPI_DEBUG_PERMISSION`) | Debug logging is the setting `permission.debug` → `logs/permission.log` (same for `memory.debug` → `logs/memory.log`, `compactor.debug` → `logs/compactor-debug.json`). | unipi-port §2.3. | — |
| 6 | audit §2.6 (vault) | The vault is `~/.oino/skill-vault` (manifest `fs.paths` alias `skill-vault`, read). `UNIPI_SKILL_VAULT` is not read. The per-skill states written by the skill editor go to `extensionSettings.skills.states` of the global or project file, the editor's proxy toggle to the global `proxy`. The old stores (`/ext-global/states.json`, `/ext-workspace/<wsid>/states.json`, `/ext-global/proxy.json`) are imported once and removed; they remain the fallback layer (merged per option) when a write is refused. | D10/D11. | — |
| 7 | audit §2.6 (skill grid) | The grid is an overlay panel (`unipi.skill-registry/editor`): per row `E`/`M` toggles showing effective values (inherited ones carry `dim`), an `inherit` button (unipi `d`), filter input, scope select (`g`), proxy toggle (`p`), Save/Cancel. | Panels exchange actions, not keystrokes. | — |
| 8 | audit §2.6 (skill conditions) | Conditions come from the `skill-condition.check{skill}` fan-out; a provider may add `reason` to `{hidden: true}` for the `/skill:name` refusal (`SkillConditionReply` has no `reason` field yet). Bundled-skill detection also matches `~/.oino/bundled/unipi.*`. | unipi-port §2.1. | Kit request R3. |
| 9 | audit §2.6 (legacy migrations) | Dropped: no `utility.skills` → `skills.exposure` translation, no legacy skill states (`on`/`unlisted`/`off`, `discoverable`), no pi `settings.json` `unipi.skills.discovery` import (no legacy unipi support). | Owner decision `unipi-native`. | — |
| 10 | audit §2.4, D8 (dream runner) | A dream is a nested agent (`extensions: {only: [unipi.memory]}`, built-in tools + memory tools, `persistence: File` under the run's staging dir `/ext-global/runs/<wsid>/staging-<ms>/sessions`, `cwd` = staging, `ui: none`), aborted at `maxRuntimeMin` (exit 124 as unipi's `timeout`). Status `running` = the agent lives in this process; a run left unfinished by a previous process is resumed once (seeded with its own session file) on the next start, else marked failed. `run.json` keeps unipi's fields (`pid: -1`) plus `resumes`, `error`. The schedule lock stores `run` instead of a live pid. Git staging (`git init/add/commit`) is dropped. `PI_DREAM_BIN` / `PI_DREAM_STAGING` are dropped. | D8; no child `oino` process. | The `craft-skill` skill is bundled as a resource; its `check` is also ported to Rust for approval. |
| 11 | audit §2.4 (digests) | Session digests read `/agent/sessions/--<cwd>--/*.jsonl` (permission `fs.agentDir: read`); `sessions.readAll` is declared for parity but not needed. | Direct file reads keep digests identical to unipi's. | — |
| 12 | audit §2.4 (tray) | The Dream tab is OUS (list → `dream:open`, detail with `dream:pick`, `approve`/`reject` arm-then-confirm, `report`, `stop`, `run`, `dismiss`, `back`; digit actions `1`–`9`). A 2 s timer emits `unipi:work-tray:changed` while a run is alive. Times are UTC (no local zone, unipi-core R3). | Work-tray contract (unipi-core deviation 3). | — |
| 13 | audit §2.18 (llm method) | `llm` summaries use `models.summarize-default(messagesToSummarize + turnPrefixMessages, instructions)`; the boundary (percent / compact-then) llm draft passes the previous summary as an instruction ("Update this previous summary …") because `summarize-default` has no previous-summary parameter. Failure falls back to core's own summarizer (no result). | Host API shape. | Request R1. |
| 14 | audit §2.18 (regexes, word segmentation) | unipi's JS regexes run through `fancy-regex` (lookaround/backrefs); `Intl.Segmenter` word counting is approximated by alphanumeric/apostrophe runs; lengths count chars, not UTF-16 units. | No ICU in guests. | Truncation points can differ for CJK text. |
| 15 | audit §2.18 (tool_result diff clamp) | The edit/write diff width clamp is not ported. | Frontends render diffs natively (ui-protocol §6.4) and know their width. | — |
| 16 | audit §2.18 (`session_recall` scope) | `scope: all` searches every session entry (all branches) instead of the active branch only. | Matches the tool description; Oino exposes `session.entries()`. | — |
| 17 | audit §2.18 (autocomplete) | `/compact-then` is offered through `complete-input` (one group from `unipi.compactor`). | ui-protocol §7.3. | — |
| 18 | audit §2.18 (doctor) | The doctor reads `control.get-settings().compaction.enabled`; the legacy `~/.unipi/db/compactor` check is dropped (Oino never had it). | — | — |
| 19 | audit §2.18 (settings) | `cooldownMs` / `repeatMinGrowthTokens` are fixed (60 s / 4 000) as in unipi 3.0.0 code; unipi's legacy keys (`overrideDefaultCompaction`, `autoCompaction.*`, `jev`, boolean `sections`) are not read (no legacy unipi support). | unipi source wins over the audit list. | — |
| 20 | audit §2.3 (paths, binaries) | Memory files live in `/ext-global/memory/<project>/<type>/<id>.md`; MemPalace is driven by program name (`mempalace`, `mempalace-mcp`, `uv`, process allowlist; `MALLOC_ARENA_MAX=2`), the palace is `~/.mempalace/palace` (alias `mempalace`), the daemon endpoint is read from `~/.mempalace/daemon/<sha256(palace)[:24]>/` (no realpath canonicalisation, `MEMPALACE_DAEMON_STATE_ROOT` unsupported), daemon HTTP needs `network.hosts` `127.0.0.1`/`localhost` (added to the manifest). Version detection uses `uv tool list` / `mempalace --version` instead of the venv python. | No venv paths or arbitrary binaries from guests. | — |
| 21 | audit §2.3 (v2→v3 conversion) | Dropped: no `/memory migrate`, no `~/.unipi/memory` alias, no start-up import prompt, no `unipi://memory/…` drawer mapping (no legacy unipi support). | Owner decision `unipi-native`. | — |
| 22 | audit §2.3 (side save) | The save pass is a nested agent (`seed: branchCopy`, `persistence: Memory`, `ui: none`, tools = memory tools only, `memory_delete` denied, `childRole: memory-save`), aborted after 8 tool calls. unipi re-exposed every active tool as a stub for prefix-cache parity; the tool policy here lists only the memory tools. | `AgentSpec` tool policy. | Lower cache reuse for the save pass. |
| 23 | audit §2.3 (cards, renderers) | Memory cards, reminder badges and the session/save cards are OUS `card`s (rail `harness`/`success`/`warning`/`error`); search hits use `meter` nodes. | ui-protocol §6. | — |

### notify, web-api, mcp, updater, app-bridge (unipi-integrations)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | unipi-port §1 "Not ported (D22): app-bridge" | `unipi.app-bridge` exists, re-targeted at the Oino GPUI apps (gpui-app.md): a `sidePane` "control centre" panel (Session · Work · Modes · BTW · Info tabs) plus services `app-bridge.snapshot` / `app-bridge.action`. The socket mirror, dialog hub, queue/session/tree/file/media/search/arg-completion wire cases stay host features (RPC/serve, ui-protocol §3, modes §4). | The work order asks for app-bridge targeting the future GPUI app; the UNI-160 control-centre state (fusion, long-horizon mode/progress, dream, btw, work list, info groups) is unipi-specific and has no host equivalent. | Auto-opens only when a `desktop`/`mobile` frontend is attached (`app-bridge.autoOpen`); TUI users get `/app`. See R7. |
| 2 | unipi-port §3.23 | Updater checks `GET https://api.github.com/repos/Neuron-Mr-White/Oino/releases/latest` for stable installs and `/releases?per_page=50` for prerelease channels (channel = prerelease id of the installed Oino version; a prerelease channel also sees a newer stable = graduation). Manifest adds `raw.githubusercontent.com` (remote changelog, unipi parity) and `fs.agentDir: read` (bundled READMEs/CHANGELOG under `/agent/bundled`). | unipi's npm dist-tags and package-spec channel do not exist for Oino. | `autoUpdate: auto` is accepted and behaves like `notify` (no in-extension install). |
| 3 | audit-unipi §2.20 | The update overlay's `Y` (install) becomes "How to update" (`oino update` / desktop updater / release page); `n` = skip version, `o` = open release page (needs `openUrl`; falls back to a notify with the URL), `q` = later. The auto-countdown is dropped. Periodic checks run on a repeating `oino_timer` every `checkIntervalMs`; the overlay opens once per version per process. New hidden command `update-check` (forced check). `UPDATE_APPLIED` is emitted at `session_start` when the installed version grew since the last run (`/ext-global/state.json`). | No self-install from a guest; the desktop app owns its updater (gpui-app §6). | — |
| 4 | unipi-port §3.22 | MCP config: user `/ext-global/mcp.json` (+ unipi-style enabled states in `/ext-global/config.json`), project `.oino/mcp.json` through the `project` preopen when `control.is-project-trusted()`. `~/.oino/mcp.json` itself is not readable. | The `agent` preopen exposes only `sessions, skills, prompts, themes, extensions, bundled` (wasm-host `AGENT_SUBDIRS`). | Users put servers in `~/.oino/state/unipi.mcp/global/mcp.json` or add them via `mcp-add`. R1. |
| 5 | audit-unipi §2.14 | MCP is extended to pi `mcp.json` semantics where cheap: streamable HTTP servers (`url`, `headers`; SSE rejected), `${VAR}` (secret store → env) and whole-value `!command` expansion in `env`/`headers`, `~/` in `command`/`args`/`cwd`, `timeout` (s), `enabled`, `exposure`/`toolExposure` (`codemode` = `deferred`), project partial overrides, pagination of `tools/list`, replies to server `ping`/`roots/list`, reconnect of a dropped connection on the next call, annotations → tool annotations (`readOnlyHint` → kind `read`). Image results are passed through as image blocks *and* keep unipi's `[Image: mime]` text; `structuredContent` goes to `details`. | pi 1.4 parity (audit-unipi §2.14 Oino note: this package is the MCP extension). | Tool names stay unipi's `<server>__<tool>` (not pi's `mcp__…`), sanitised to `[A-Za-z0-9_-]`. |
| 6 | audit-unipi §2.14 | `mcp-reload` reconnects live (stop all → unregister tools → start), and add/edit/enable/disable/remove in the server panel apply immediately. Texts changed accordingly ("Reloaded MCP servers: N active, M failed, T tools.", "MCP server \"x\" saved. Connecting…"; settings row description "Reconnect every server and re-register its tools."). | Oino can unregister tools (unipi could not). | — |
| 7 | audit-unipi §2.14 | Disabled servers are listed as `○ name — stopped` (status text, panel, info group) after the started ones (unipi listed only started servers). Server processes are per extension instance (shared by sessions) and stop when the last non-child session shuts down. | One process tree per server, not per session. | — |
| 8 | audit-unipi §2.14 (UI) | Server list and catalog browser are overlay panels; the JSON editor is the `editor` dialog (validation errors re-open it with the text and an error notify). Vim keys (`j/k`, `gg/G`, …) are frontend list navigation. | Panels exchange actions, not keystrokes. | — |
| 9 | unipi-port §3.21 | Smart fetch: host `http.fetch` with `impersonate: <browser profile>` (fingerprint when the host has it, else plain request — shown in the info group); extraction is `dom_smoothie` (Readability) + `htmd` (turndown rules) instead of `defuddle`; on short pages the larger of the Readability article and the DOM fallback (`article, main, [role=main], .content, #content` → body) wins. The `os` parameter is accepted but not sent. `includeReplies: true` keeps the whole main region; `false`/`extractors` = Readability article (no per-site extractors). | No JS DOM in the guest; the host profile string has no OS part. | Output differs in detail from defuddle's markdown. R2. |
| 10 | audit-unipi §2.12 | wigolo is reached over its REST daemon (`providers.wigolo.url`, default `http://127.0.0.1:3333`, `POST /v1/search`, `/v1/fetch`) instead of the AGPL SDK. | No Node SDK in a WASM guest. | The daemon must be running (`npx wigolo init`); failures fall through as before. |
| 11 | audit-unipi §2.12 | Firecrawl uses `POST /v1/scrape {url, formats:["markdown"], onlyMainContent}`; Perplexity uses model `providers.perplexity.model` (default `sonar`). | unipi's v0 endpoint and `llama-3.1-sonar-small-128k-online` model are retired upstream. | — |
| 12 | unipi-port §3.21 (perms) | Manifest adds `sessionControl` (tool exposure via `set-active-tools`) and `env` (the five provider key names); `credentials: ["*"]` is not requested (keys come from settings/secret store/env). An opt-in "Session model" summariser (`providers.model`, off by default) uses `models.complete`. | unipi hides unusable tools with `setActiveTools`; the port grants `models.complete` without saying what for. | With defaults, `web_llm_summarize` is hidden exactly as in unipi. |
| 13 | audit-unipi §2.12 | `text` format: images are removed before links (unipi's regex order turned `![a](b)` into `!a`). | Bug fix. | — |
| 14 | unipi-port §3.8 | notify: dispatches never block hooks: jobs are queued and sent from a 0 ms `oino_timer`; the finished grace (1.5 s), "All done" grace (2.5 s) and renotify loop are `oino_timer`s keyed per session. Pending work = a running `unipi:bg:tasks` task with `triggerOnCompletion`, or host run state `waiting`; the watch listens to `unipi:bg:tasks`, `unipi:subagents:list`, `unipi:fusion:status`, `unipi:work-tray:changed`. | No `setTimeout` / shared registries in guests (unipi-port §2.2). | `UNIPI_NOTIFY_FINISH_GRACE_MS` is not read (fixed 1.5 s). |
| 15 | unipi-port §3.8 | "Silence after input" and renotify disarm use editor intents (`oino_editor`) and `input` events instead of terminal keypresses. | ui-protocol §9 (`onTerminalInput` not available). | — |
| 16 | unipi-port §3.8 | Native notifications go through `ui.os-notify` (urgency from priority); `suppressWhenFocused` uses `host-info.frontend-focused()` on every platform (unipi: Windows only). | Frontend-owned OS integration. | — |
| 17 | unipi-port §2.5 | ntfy: no E2E envelope, no `unipi://` deep link (`click`), no UnifiedPush fan-out; `appDetail: minimal` (explicit) still replaces the text with "Tap to open in Oino"; unset = full text. `tags: [eventType]` kept. | D22 drops app push; without the phone app's routing, minimal text loses information. | Default behaviour sends the full message (unipi's default was minimal routing for its app). |
| 18 | audit-unipi §2.15 | Recap uses `models.complete` with `recap.model` (`provider/id`; empty = session model), `maxTokens: 100`; `recap.disableThinking` maps to `options.reasoning = "off"`. | No direct provider HTTP with keys in guests. | — |
| 19 | audit-unipi §2.15 | Setup wizards (`notify-set-gotify`, `-set-tg`, `-set-ntfy`) are sequences of `input` dialogs (secret inputs masked) with a test send before saving; the ntfy wizard has no project/global scope choice and no QR. Telegram polls `getUpdates` every 2 s up to 5 min with notify progress. Non-secret wizard results and `/notify-event` are written to the global settings file (`registry.set-extension-setting`); secrets are written as the namespace's secret fields (host secret store, same `notify/<key>` entry as before). The old kv `settings-overlay` is imported once (leaf keys) and is only used when a write is refused. `/notify-event` no longer says "Run /reload". | Listeners read config live. | — |
| 20 | audit-unipi §2.15 | Event `permission_request` listens on bus topic `permissions:ui_prompt`; `herdr:blocked` is dropped (D22). Test notification times are UTC (`HH:MM:SS UTC`). | No foreign `pi.events`; no local time zone (unipi-core R3). | — |
| 21 | settings (all five) | Values are read as `defaults ⊕ host values` with flat dotted keys (`"events.x.enabled"`) expanded over nested objects; secret fields come resolved from the host (`registry.extension-settings`) and still fall back to the secret store (`<namespace>/<key>`). | The registry writes nested keys; older files may hold flat keys. | — |
| 22 | extension-api §9.3 | Tests: native unit tests per crate (`cargo test -p unipi-notify -p unipi-web-api -p unipi-mcp -p unipi-updater -p unipi-app-bridge` in `extensions/`, 120+ cases) plus one standalone wasm-host harness for the batch (`extensions/unipi/notify/harness`, `run.sh`), with a Python stdio MCP fixture and local HTTP servers. | Same constraints as unipi-core #11. | Harness needs `python3` on PATH. |

### `oino-kanboard` crate (unipi-kanboard)

| # | unipi | Oino | Why |
|---|---|---|---|
| K1 | binary `unipi-kanboard`, lib `kanboard` | binary `oino-kanboard`, lib `oino_kanboard`, version `1.0.0-alpha.0` | spec |
| K2 | env `UNIPI_KANBOARD_*` (HOME, PROJECT, ACTOR, SESSION, PID, MAX_SESSIONS, CHAIN_GATE, CHILD, UNDO_TTL_SECS, NO_WATCH, DEBUG_WATCH, ARCHIVE_AFTER_DAYS, RETENTION_DAYS) | the same names with the `OINO_` prefix | Oino naming; no reading of the unipi names |
| K3 | home `~/.unipi/kanboard` | `<agent dir>/state/unipi.kanboard` (agent dir is `$OINO_AGENT_DIR`, else `~/.oino`) | spec. There is no import of old boards, because charter/D10 import only memory |
| K4 | the daemon serves the embedded web UI (`ui-dist`, rust-embed, `build.rs`, `web/`) | removed. `/` and board/asset paths return 404, and `/api/health` no longer has `ui` | D22. The board is the panel in the extension |
| K5 | pi-side settings `~/.unipi/config/kanboard/config.json` | `<agent dir>/settings.json` → `extensionSettings.kanboard` (merge-write, other keys kept). `load/patch_pi_settings_at` are also exposed for tests | Oino settings layout (D11) |
| K6 | summaries run `piCommand` (pi argv) | the extension writes `piCommand = ["oino"]`. When the head is a bare `oino`, the binary uses the `oino` installed next to it (`settings::resolve_program`). Messages say "Oino" | the guest cannot know the Oino executable path |
| K7 | model list: unipi `~/.unipi/config/models-cache.json`, then `piCommand --list-models` | only `oino --list-models` (the cache file does not exist in Oino) | — |
| K8 | `fs2` locks, `libc kill` via `unsafe extern` | std `File::lock/try_lock`; `nix` signals | charter: `unsafe_code = forbid`, fewer dependencies |
| K9 | `serde_norway` | `serde_yaml_ng` (workspace dependency, same API for the one scalar unquote) | workspace dependency policy |
| K10 | — | `status` also reports `binary` (absolute path, from `current_exe`) and `hostname` | the guest needs the absolute path for the agent's CLI prefix, and has no OS host name or interface API |
| K11 | — | `add --attach-existing <F>` (hidden, repeatable): attaches only files that exist, and expands `~/` | unipi checked host paths with `existsSync`, which a guest cannot do |
| K12 | integration tests that clear ambient env with `unsafe remove_var`, the HOME `set_var` settings test, the embedded-UI test, and the models-cache unit test | env clearing dropped (the variables are renamed, so the ambient shell does not set them); settings test uses an explicit path; UI test replaced by `the_daemon_serves_no_web_ui`; auth tests target `/api/projects` | `unsafe` forbidden; web UI removed |

### `unipi.kanboard` extension (unipi-kanboard)

| # | unipi | Oino | Why |
|---|---|---|---|
| E1 | `process.env.UNIPI_KANBOARD_{SESSION,PID,PROJECT,MAX_SESSIONS}` is inherited by the agent's bash | the `tool_call` handler puts `export OINO_KANBOARD_SESSION=… OINO_KANBOARD_PID=… OINO_KANBOARD_MAX_SESSIONS=… OINO_KANBOARD_PROJECT=…` (PowerShell: `$env:…`) in front of every bash/powershell command that calls the binary, via the returned `input` | guests cannot change the host environment. Only kanboard commands are touched |
| E2 | session id `pi-<sessionId>`, claim pid = pi pid | `oino-<sessionId>`, pid = `process.self-pid()` (the Oino process) | — |
| E3 | binary search: `UNIPI_KANBOARD_BIN` → npm platform package → dev build | `$OINO_KANBOARD_BIN` (permission `env`) → `oino-kanboard` on the host search path. After the first `status`, the reported absolute path is cached and handed to the agent | D22 (`kanboard-bin` replaced by the installer) |
| E4 | slug cached with `setSettings("kanboard", {slug, root}, "project")` on onboard | onboarding writes `slug` to the project settings file (`registry.set-extension-setting`; ignored when the project is untrusted); reads: settings `slug`, else an in-memory cache, else the binary resolves the project for the cwd/git root (`project show`). `root` is not written | the binary's own resolution covers the root |
| E5 | `SettleInfo` from the unipi arbiter | built in the guest: tool calls are counted between `agent_start` and settle, and the last assistant text and tool calls come from `agent_end.messages` | the host `propose-nudge` has no settle info |
| E6 | nudge `details.unipiHarness` | `details.oinoProvenance {source, title, synopsis, severity?, delivery}`; delivery is counted on `nudge-delivered(id)` | runtime §2 provenance |
| E7 | web board + `openBrowser` | `/kanboard open` starts the daemon, prints the API URLs and opens the board **panel** (`sidePane`: lane lists in a `split`, a detail section with allowed-move buttons, a comment prompt when a move needs one, keys r/a/s), live from SSE `/events`. `openBrowser` is kept as a stored setting (label "Open board panel") | D22 |
| E8 | wildcard bind lists every reachable IPv4 (`os.networkInterfaces`) | lists the hostname (from `status`) and the tailnet address | no interface API in guests |
| E9 | the in-chat board renderer budgets widths in cells | OUS `text` rows with `wrap: false`; the frontend truncates to the width | ui-protocol: frontends own layout |
| E10 | `/kanboard status`, `doctor` and `open` run in the background (`void work()`) | awaited inside the command | host calls need a live call context |
| E11 | doctor checks that `piCommand[0]` exists on disk | reports the argv; the daemon resolves it (K6) | the guest cannot stat host paths |
| E12 | R1 annotation `details.unipiHarnessAnnotations[].meta` from core `harnessMetadata` | same shape, with ids generated in the guest | — |
| E13 | debug log `~/.unipi/logs/kanboard.log` behind `UNIPI_DEBUG_KANBOARD=1` | dropped (failures stay silent or go to `notify`, as in unipi) | unipi-port §2.3 extension log |
| E14 | `captureToBacklog({cwd, text})` on `globalThis` | service `kanboard.capture{text\|body, title?, priority?}` → `{ok, id, attachments}` / `{ok: false, reason}` | unipi-port §2.2 |
| E15 | move handler also rewrote the project settings cache | `project rebind` through the binary, then the in-memory slug cache is updated; the result goes back as `MoveReport.log` lines | the binary owns the project binding (E4) |
| E16 | permissions in the spec: `process.spawn (oino-kanboard, git)`, `fs.project: read` | `process.allow: [oino-kanboard, tailscale]` and `detached`, `network.hosts: [127.0.0.1, localhost]`, `env: [OINO_KANBOARD_BIN, OINO_KANBOARD_CHILD, OINO_KANBOARD_HOME]`, `sessionControl`. No `git` and no `fs.project` | minimal: neither is used; `tailscale` is needed for `--host tailscale` |
