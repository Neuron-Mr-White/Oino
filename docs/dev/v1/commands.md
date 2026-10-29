# Oino slash commands — feature command names

Status: **binding** (work id `unipi-native`, 2026-10-27). Branch `v1-alpha`.

Oino is a native product: the features ported from unipi are Oino's own, so their slash
commands carry **no `unipi:` prefix** and there are **no aliases** for the old names. The
internal extension ids (`unipi.*`), bus topics (`unipi:<module>:<event>`), session custom
types (e.g. `unipi:kanboard-next`, `unipi:plan-mode`) and settings namespaces are unchanged
(the owner may rename those later).

## 1. Rules

1. **Core built-ins always win.** The frontend built-ins (`/settings`, `/model`,
   `/thinking`, `/scoped-models`, `/login`, `/logout`, `/new`, `/resume`, `/name`,
   `/session`, `/tree`, `/fork`, `/clone`, `/compact`, `/import`, `/copy`, `/export`,
   `/trust`, `/reload`, `/hotkeys`, `/changelog`, `/extensions`, `/quit`; pi's `/share`,
   `/bug` in the completion list) are never shadowed. A ported command with a colliding
   name is either **merged** into the built-in (its surface moves there) or **renamed** to a
   short, clear, non-prefixed name.
2. **Feature commands are unique among themselves** (one owner per name).
3. Hidden runner commands (settings-hub actions, updater/long-horizon runners) follow the
   same naming; they do not appear in the `/` menu.
4. Feature commands come from the extensions' own registrations (`command:<name>` groups);
   a test asserts that no registered name collides with a built-in. On a tie, built-ins sort
   before feature commands. (Round 5 D1 removed `unipi.command-enchantment`, which used to
   add package tags to these groups.)

## 2. Collisions and decisions

| Old | Collides with | Decision | Result |
|---|---|---|---|
| `/unipi:settings [search]` (unipi.utility) | `/settings` | **Merged**: the core `/settings` hub already lists every extension namespace; the utility command and its `openSettings{query}` path are removed. | `/settings` |
| `/unipi:changelog` (unipi.updater) | `/changelog` | **Merged**: the core `/changelog` forwards to a hidden `changelog` runner in `unipi.updater` that renders the entries panel (the update overlay still renders the changelog between versions). | `/changelog` |
| `/unipi:model` (unipi.fusion) | `/model` | **Renamed**: the model / Fusion lead+sidekick picker is not the core model selector. The `/m…` completion hint now ranks `/fusion` after the core `/model` (was pinned above it). | `/fusion` |

No other ported command collides with a built-in or with another feature command.

## 3. Mapping (old → new)

Visible commands:

| Old | New | Owner | Description |
|---|---|---|---|
| `/unipi:plan` | `/plan` | unipi.workflow | Plan mode — plan with the user, then approve a plan |
| `/unipi:permission` | `/permission` | unipi.workflow | Permission mode — ask · auto (jev-judged) · full |
| `/unipi:goal` | `/goal` | unipi.long-horizon | One objective until verifiably true |
| `/unipi:ralph` | `/ralph` | unipi.long-horizon | Checklist grind over iterations |
| `/unipi:swarm` | `/swarm` | unipi.long-horizon | Parallel fan-out + synthesis |
| `/unipi:graph` | `/graph` | unipi.long-horizon | Dependent multi-step agent graph |
| `/unipi:regular` | `/regular` | unipi.long-horizon | Stop the active owner; prompts without long-horizon routing |
| `/unipi:visualize-progress` | `/visualize-progress` | unipi.long-horizon | Live view of the goal / ralph / swarm / graph run |
| `/unipi:memory` | `/memory` | unipi.memory | Memory status and session overrides (`status`, `recall on\|off`, `write on\|off`) |
| `/unipi:memory-process` | `/memory-process` | unipi.memory | Analyze text and store extracted memories |
| `/unipi:memory-consolidate` | `/memory-consolidate` | unipi.memory | Consolidate the current session into memory |
| `/unipi:memory-search` | `/memory-search` | unipi.memory | Search project memories |
| `/unipi:global-memory-search` | `/global-memory-search` | unipi.memory | Search memories across all projects |
| `/unipi:memory-forget` | `/memory-forget` | unipi.memory | Delete a memory by title |
| `/unipi:global-memory-list` | `/global-memory-list` | unipi.memory | List all memories across all projects |
| `/unipi:dream` | `/dream` | unipi.dream | Dream status, report and proposal approval |
| `/unipi:btw` | `/btw` | unipi.btw | Side question in an inline panel (read-only) |
| `/unipi:subagents` | `/subagents` | unipi.subagents | Subagents tab of the work tray |
| `/unipi:agents` | `/agents` | unipi.subagents | Manage subagent profiles |
| `/unipi:model` | `/fusion` | unipi.fusion | Pick a model or a Fusion lead+sidekick pair (renamed, §2) |
| `/unipi:fusion-stats` | `/fusion-stats` | unipi.fusion | Estimated Fusion savings |
| `/unipi:bg` | `/bg` | unipi.background-tasks | Start a shell command as a tracked background task |
| `/unipi:bg-tasks` | `/bg-tasks` | unipi.background-tasks | Background tasks tab of the work tray |
| `/unipi:bg-detach` | `/bg-detach` | unipi.watchdog | Move the running bash call to a background task |
| `/unipi:mcp-status` | `/mcp-status` | unipi.mcp | MCP server status |
| `/unipi:continue` | `/continue` | unipi.utility | Continue from where the agent stopped |
| `/unipi:retry` | `/retry` | unipi.utility | Retry the last turn (same as `/continue`) |
| `/unipi:cleanup` | `/cleanup` | unipi.utility | Remove stale Oino temp files (preview first) |
| `/unipi:doctor` | `/doctor` | unipi.utility | Check Oino's runtime: folders, config, models, Decision Model, skills |
| `/unipi:answer` | `/answer` | unipi.utility | Answer the questions in the last reply |
| `/unipi:summarize` | `/summarize` | unipi.utility | Summarize the last reply again |
| `/unipi:move` | `/move` | unipi.utility | Re-link state after moving the project folder |
| `/unipi:settings` | `/settings` (core) | unipi.utility → core | Merged (§2) |
| `/unipi:skills` | `/skills` | unipi.skill-registry | Skill settings (opens `/settings` on Skills) |
| `/unipi:info` | `/info` | unipi.info-screen | Dashboard: session, usage, tools, skills, modules |
| `/unipi:hint` | `/hint` | unipi.core | Browse and search Unicrab hints, cycle/reset |
| `/unipi:compact-vcc` | `/compact-vcc` | unipi.compactor | Lossless compaction now (no model call) |
| `/unipi:compact-by-llm` | `/compact-by-llm` | unipi.compactor | Compact now with a model-written summary |
| `/unipi:compact-then <prompt>` | `/compact-then <prompt>` | unipi.compactor | Compact, then send the prompt (input handler) |
| `/unipi:session-recall` | `/session-recall` | unipi.compactor | Search session history incl. compacted-away messages |
| `/unipi:compact-stats` | `/compact-stats` | unipi.compactor | This session's compaction savings |
| `/unipi:compact-doctor` | `/compact-doctor` | unipi.compactor | Check compaction settings |
| `/unipi:compact-help` | `/compact-help` | unipi.compactor | Compactor command help |
| `/unipi:notify-event` | `/notify-event` | unipi.notify | Toggle a notify event: `<event> <on\|off>` |
| `/unipi:kanboard` | `/kanboard` | unipi.kanboard | The board: open, close, onboard, status, show, doctor |
| `/unipi:kanboard-add` | `/kanboard-add` | unipi.kanboard | Capture a task into Backlog |
| `/unipi:kanboard-do` | `/kanboard-do` | unipi.kanboard | Grant the agent task slots + a write budget |
| `/unipi:kanboard-autowork` | `/kanboard-autowork` | unipi.kanboard | Work ready tasks one by one (`start\|stop`) |
| `/unipi:readme` | `/readme` | unipi.updater | Browse bundled README files |
| `/unipi:changelog` | `/changelog` (core) | unipi.updater → core | Merged (§2) |
| `/unipi:cron` | `/cron` | unipi.cron | Scheduled, unattended Oino runs — list/show/runs/run/stop/enable/disable/pause/resume/status/add/edit/remove ([docs/cron.md](../../../docs/cron.md)) |

`/cron` is an Oino-native feature (owner decision 2026-10-28, superseding D22's drop): the
unipi host scheduler's behaviour ported to `crates/oino-cron` + the `unipi.cron` component
(unipi-port §3.24); no `unipi:` prefix, Oino wording everywhere (commands.md §1).

Hidden runners (settings-hub actions and cross-component runners):

| Old | New | Owner |
|---|---|---|
| `unipi:plan-off`, `unipi:plan-enter`, `unipi:plan-exit` | `plan-off`, `plan-enter`, `plan-exit` | unipi.workflow |
| `unipi:permission-clear-rules` | `permission-clear-rules` | unipi.workflow |
| `unipi:goal-start`, `-status`, `-pause`, `-resume`, `-stop` | `goal-start`, `goal-status`, `goal-pause`, `goal-resume`, `goal-stop` | unipi.long-horizon |
| `unipi:ralph-start`, `unipi:lh-explicit`, `unipi:lh-mode`, `unipi:lh-graph` | `ralph-start`, `lh-explicit`, `lh-mode`, `lh-graph` | unipi.long-horizon |
| `unipi:fusion-preset` | `fusion-preset` | unipi.fusion |
| `unipi:ask-user-new-session` | `ask-user-new-session` | unipi.ask-user |
| `unipi:hint-reset` | `hint-reset` | unipi.core |
| `unipi:skills-editor` | `skills-editor` | unipi.skill-registry |
| `unipi:mcp-settings`, `-add`, `-sync`, `-reload` | `mcp-settings`, `mcp-add`, `mcp-sync`, `mcp-reload` | unipi.mcp |
| `unipi:notify-test`, `-set-gotify`, `-set-tg`, `-set-ntfy` | `notify-test`, `notify-set-gotify`, `notify-set-tg`, `notify-set-ntfy` | unipi.notify |
| `unipi:kanboard-open`, `-stop-daemon`, `-rotate-token` | `kanboard-open`, `kanboard-stop-daemon`, `kanboard-rotate-token` | unipi.kanboard |
| `unipi:rename-now` | `rename-now` | unipi.utility |
| `unipi:changelog` | `changelog` | unipi.updater (the core `/changelog` forwards here) |
| `unipi:update-check` | `update-check` | unipi.updater |
| `unipi:web-cache-clear` | `web-cache-clear` | unipi.web-api |

## 4. Not commands (unchanged identifiers)

Bus topics (`unipi:memory:stored`, `unipi:kanboard:status`, …), session custom types and
entry ids (`unipi:kanboard-reminder`, `-notice`, `-badge`, `-continue`, `-next`, `-show`,
`-help`, `-doctor`; `unipi:plan-mode`, `unipi:plan-mode-message`; `unipi:skills-revealed`,
`unipi:skills-judged`; `unipi:auto-name`; `unipi:attachments`; `unipi:lh-continue`),
extension ids `unipi.*` and settings namespaces keep their names.

## 5. Migrating user files

Oino does not translate old names at runtime. The bundled opt-in skill `unipi-migrate`
(`extensions/unipi/core/skills/unipi-migrate/`, user doc [docs/migrate-from-unipi.md](../../migrate-from-unipi.md))
rewrites `/unipi:*` in the user's prompts, skills, subagent profiles, ralph files and
`AGENTS.md`/`CLAUDE.md` with the table in §3, kept as data in
`references/commands.tsv` (regenerate it when §3 changes). Session history is not rewritten.
