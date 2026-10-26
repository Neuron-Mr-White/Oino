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
| command runners | `control.command-invoke` + hidden commands (`hidden: true`) for every runner name in audit-unipi §1.3.5 (`unipi:plan-enter`, …) |
| skill conditions | service fan-out `skill-condition.check{skill}` → `{hidden: bool}` (skill-registry calls) |
| move handlers | service fan-out `move.handle{oldRoot, newRoot, dryRun, backup}` (utility `/unipi:move` calls) |
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

No automatic import of `~/.unipi` data, except memory (`/unipi:memory migrate` reads
`~/.unipi/memory` through a read-only `fs.paths` grant).

### 2.4 jev

`unipi-kit::jev::ask(state, questions, settings) -> Option<Answers>`: transports TypeSafe
`/v1/systemone` (1 000 ms) and OpenRouter decisions `/api/alpha/decisions` (6 000 ms);
key order: secret `decision-model/apiKey` → env `TYPESAFE_API_KEY` / `OPENROUTER_API_KEY`
(permission `env`) → `credential-for(provider matching baseUrl)`. Fail-open (`None`). Shared
namespace `decision-model` (keys `provider`, `model` = `typesafe/jev-1.13`, `baseUrl`,
`apiKey` (secret), `timeoutMs`). Additional source `model`: uses `models.complete` with a
structured-output prompt against a configured model (opt-in, never default).

### 2.5 Cuts and changes (D22)

- Dropped: `/unipi:cron`, `cron` and `browser` bundled skills, browser-state context,
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
moment tips, show counts in `/ext-global/hints.json`), `/unipi:hint [next|reset]`, shortcuts
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
Part A: `/unipi:settings [search]` (→ `openSettings`), `/unipi:continue`, `/unipi:retry`,
`/unipi:doctor`, `/unipi:cleanup`, image tools `image_generate`/`image_edit`/`image_recognize`
(models `generate-image`/`complete` or provider HTTP with `auth: provider`), namespaces
`utility`, `image`. Part B: render styles (`render.style` → tool render hints + `ui.density`),
auto-rename (nested agent with tool `rename_session`, jev gate, `session.set-session-name`),
`/unipi:answer [reply|questions|web]` (reply panel; questions in editor via `editor-set`;
web form via loopback `http.listener`), attachments (paste detection → `insertEditorToken`),
`/unipi:summarize [q]`, `/unipi:move` (service fan-out `move.handle` + session file move via
`sessions-list`/core `import`), rename runner `unipi:rename-now`. Drops `/unipi:cron`,
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
"unipi:ask-user-new-session", prefill)` (hidden command; queued until idle per runtime §7.4)
which calls `control.new-session` then `ui.editor-set(prefill)`; `notifyOnAsk` → os-notify. Replaces Oino `ask-user`.

### 3.6 `unipi.footer` (unipi-ux, B4)
Perms: `process.spawn` (`git`), `fs.project: read`. Caps: C16 C17 C29. Frame segments via
`setFrame` (model, thinking, cwd, branch, context %, cost, tps, PLAN/mode badges from sticky
topics `unipi:workflow:status`, `unipi:long-horizon:state`, `unipi:kanboard:status`,
`unipi:fusion:status`, `unipi:bg:tasks`), stats line via `setFooter`; `/unipi:footer
[on|off]`; publishes `unipi:footer:tps`. Replaces Oino `footer-status`.

### 3.7 `unipi.info-screen` (unipi-ux, B4)
Perms: `sessions.readAll`, `process.spawn` (`git`), `fs.project: read`. Caps: C7 C16 C17.
`/unipi:info [page]` → `overlay` panel with `tabs`; pages from `info-screen.groups` fan-out +
built-ins (usage index over `sessions-list`/`session-read`, modules page from
`_oino/extensions`-equivalent `control.commands-list`/`all-tools` + service list); splash via
`setHeader` per `bootMode`. Namespace `info-screen`.

### 3.8 `unipi.notify` (unipi-integrations, B1)
Perms: `osNotify`, `network.hosts: ["*"]` (user-configured Gotify/ntfy/Telegram hosts;
allowed because the component is bundled), `secrets`, `models.complete`. Caps: C1 C15 C16 C19. Tool `notify_user`, commands
`/unipi:notify-event`, runners set-gotify/set-ntfy/set-telegram/test as hidden commands,
event matrix, re-notify (timer 2 min), recap (complete), "All done" via host run state.
Note: user-configured hosts are granted through manifest `network.hosts: ["*"]` (bundled
extensions may use `*`). Replaces Oino `notify`.

### 3.9 `unipi.background-tasks` (unipi-agents, B2)
Perms: `process.spawn` (`*`), `sessionControl`. Caps: C11 C12 C16 C17. Tools `bg_run`,
`bg_status`, `bg_logs`, `bg_kill` (schemas exact) over host `tasks`; `/unipi:bg`,
`/unipi:bg-tasks`; tray tab service `work-tray.tab`; shortcut `unipi.bg.clear` (`ctrl+alt+c`);
service `tasks.notification` renders the `<background-task-notification>` custom message text
exactly as unipi (followUp delivery, trigger turn when `triggerOnCompletion`). Publishes
`unipi:bg:tasks`.

### 3.10 `unipi.watchdog` (unipi-agents, B2)
Perms: `process.spawn` (`ps`, `getconf`), `sessionControl`, `network.hosts` (jev), `env`
(`TYPESAFE_API_KEY`, `OPENROUTER_API_KEY`), `secrets`. Caps: C11 C13 C16. Stuck-tool detection
via `control.running-bash` + `process-stats`/`process-tree`, jev stuck check,
`/unipi:bg-detach [reason]` → `control.bash-detach`. **Session caps (D22)**: settings
`sessionCaps.{maxTurns, maxCostUsd, maxWallMinutes, maxConsecutiveToolErrors, summaryTurn}`
(all off by default) and flags `--max-turns`, `--max-cost`, `--max-wall`, `--max-tool-errors`;
on reaching a cap: `control.abort()`, notify, and (if `summaryTurn`) one follow-up user message
asking for a summary. Replaces Oino `budget`.

### 3.11 `unipi.subagents` (unipi-agents, B2)
Perms: `agents.spawn` (`maxDepth: 2`), `fs.project: read`, `sessionControl`. Caps: C2 C10
C16 C17 C27 C28 (wait-source). Tools `run_subagent` (profile enum rebuilt on profile change),
`read_subagent` (registered here; fusion uses service `subagents.register-external`);
profiles: bundled + `/ext-global/profiles/*.md` + project `.oino/agents/*.md`;
`/unipi:subagents` (tray tab), `/unipi:agents` (profile manager panel); shortcut
`unipi.subagents.foreground` (`ctrl+b`); background runs deliver completion custom messages;
graph-run service; concurrency default 8 (cap 16). `UiPolicy`: foreground = Forward,
background = AutoDeny with unipi's note text.

### 3.12 `unipi.btw` (unipi-agents, B2)
Perms: `agents.spawn`. Caps: C10 C17. `/unipi:btw [question]` → `editorReplace` panel;
ephemeral agent `seed: BranchCopy`, tools `read, grep, find, ls`, `persistence: Memory`,
streaming answer into the panel (`updatePanel` coalesced ≤ 20/s); history within the panel.

### 3.13 `unipi.fusion` (unipi-agents, B2)
Perms: `agents.spawn`, `sessionControl`, `models.complete`. Caps: C8 C10 C16 C17 C28.
Sidekick = persistent nested agent (`persistence: File`), tool `sidekick {message, block?}`;
`/unipi:model` picker as `custom` dialog (effort/lead/sidekick focus, filter, thinking via
`select`/`list` nodes); `/unipi:fusion-stats`; presets + preset editor hidden command; lead
policy; wait-source when a non-blocking handoff is pending. Replaces Oino `router` UI
(model-picker UX).

### 3.14 `unipi.long-horizon` (unipi-agents, B3)
Perms: `sessionControl`, `models.complete`, `credentials: ["*"]`, `fs.project: write`,
`network.hosts` (jev), `services.call: ["unipi.subagents/*", "*/evidence.contribute"]`.
Caps: C1 C2 C3 C6 C16 C25 C26 C28 (nudge-provider 100). All modes (goal, ralph, swarm, graph,
regular), all tools (schemas exact), owner coordinator (`/ext-workspace/state.json`,
`goal.json`, `graph.json`), verifier, budgets, runaway guard, compaction-context provider,
`/unipi:visualize-progress` (mermaid → SVG → PNG in guest via `resvg`; OUS `image`, text
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
write}, {path: "~/.unipi/memory", access: read, alias: "legacy-memory"}]`, `agents.spawn`,
`network.hosts` (`pypi.org`, `files.pythonhosted.org`). Caps: C2 C10 C11 C25. Memory files at
`/ext-global/memory/<project>/<type>/<id>.md`; warm `mempalace-mcp` stdio reader (process
pipe, idle unload 10 min timer); side-save nested agent; all tools and commands per audit.

### 3.18 `unipi.skill-registry` (unipi-context, B3)
Perms: `fs.agentDir: read`, `fs.project: read`, `network.hosts` (jev). Caps: C1 C25.
`before_agent_start` → modified `systemPromptOptions.skills`; `resources_discover` →
`skillPaths` (vault); skill conditions fan-out; `/unipi:skills` → `openSettings{namespace:
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
tab, `/unipi:dream …`; bundled skill `craft-skill` (with `scaffold`/`check` scripts) replaces
Oino `craft-skill`.

### 3.21 `unipi.web-api` (unipi-integrations, B1)
Perms: `network.hosts: ["*"]`, `network.fingerprint`, `secrets`, `credentials: ["*"]`,
`models.complete`. Caps: C2 C9 C15. Tools `web_search {query, source?: 1..6}`,
`multi_web_content_read` (full schema), `web_llm_summarize`; providers in guest (wigolo,
DuckDuckGo, Jina, SerpAPI, Tavily, Perplexity); smart-fetch via `impersonate`; HTML→markdown in
Rust (`html2md`-class crate + readability extraction); cache in `/cache`; hidden command
`unipi:web-cache-clear`.

### 3.22 `unipi.mcp` (unipi-integrations, B3)
Perms: `process.spawn: ["*"]`, `network.hosts: ["*"]`, `fs.agentDir: read`, `fs.project:
read`, `secrets`. Caps: C2 C11 C15 C16 C17. MCP client (stdio via process pipes; streamable
HTTP/SSE via `http.fetch`), config `~/.oino/mcp.json` + trusted `.oino/mcp.json` (pi file
names) + ACP `session/new` servers from bus topic `oino:acp:mcp-servers`; dynamic tools
`<server>__<tool>`; `/unipi:mcp-status`; hub runners as hidden commands; browser panels.

### 3.23 `unipi.updater` (unipi-integrations, B4)
Perms: `network.hosts: ["api.github.com", "github.com", "objects.githubusercontent.com"]`.
Caps: C13 C15 C17. Periodic check (`checkIntervalMs`, default 1 h) of
`https://api.github.com/repos/Neuron-Mr-White/Oino/releases/latest`; `autoUpdate: notify|off`
(no in-extension install; notification tells the user to run `oino update` or use the desktop
updater); `/unipi:readme [extension]` (bundled `docs` of that extension as `markdown` panel),
`/unipi:changelog` (bundled Oino CHANGELOG).

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
| router UI | `unipi.fusion` `/unipi:model` |
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
