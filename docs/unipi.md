# Bundled unipi extensions

All of unipi 3.0.0 except its desktop app is ported to
Rust WASM components and bundled into `oino` and Oino Desktop. They load in unipi's order,
`unipi.core` first. Settings live under `extensionSettings.<namespace>` in `settings.json`
([settings.md](settings.md)); state under `~/.oino/state/<id>/`.

| Component | What it does |
|---|---|
| `unipi.core` | Unicrab hints, start header, work tray (`/unipi:hint`) |
| `unipi.workflow` | permission gate (ask · auto · full) with approval dialogs and saved rules; plan mode with `plan_submit` and review (`/unipi:permission`, `/unipi:plan`, `alt+p`, `alt+m`) |
| `unipi.long-horizon` | goal mode with verifier, ralph task-file loops, swarms and durable agent graphs (`/unipi:goal`, `/unipi:ralph`, `/unipi:swarm`, `/unipi:graph`) |
| `unipi.memory` | Markdown memories plus MemPalace integration; recall reminder, `memory_*` tools |
| `unipi.dream` | periodic "dream" runs that review past struggles and propose skills (`/unipi:dream`) |
| `unipi.skill-registry` | per-skill switches, vault, judged skill exposure (`/unipi:skills`) |
| `unipi.info-screen` | dashboard of session, context, usage, tools, skills, modules (`/unipi:info`) |
| `unipi.subagents` | `run_subagent` / `read_subagent` over nested in-process agents, profiles (`/unipi:subagents`) |
| `unipi.background-tasks` | `bg_run`, `bg_status`, `bg_logs`, `bg_kill`; wakes the agent when tasks finish (`/unipi:bg`) |
| `unipi.btw` | side questions answered by a read-only nested agent (`/unipi:btw`) |
| `unipi.web-api` | `web_search` (wigolo, DuckDuckGo, Jina, SerpAPI, Tavily, Perplexity) and `multi_web_content_read` |
| `unipi.ask-user` | `ask_user` structured questions |
| `unipi.mcp` | MCP client (stdio + streamable HTTP), server catalog (`/unipi:mcp-status`) |
| `unipi.notify` | `notify_user`; native, Gotify, Telegram, ntfy notifications on run end / input needed |
| `unipi.kanboard` | kanboard board as a side panel over the `oino-kanboard` daemon (`/unipi:kanboard-*`) |
| `unipi.command-enchantment` | tagged, sorted `/unipi:*` completions |
| `unipi.compactor` | lossless "vcc" compaction, `/unipi:compact-then`, `session_recall`, `context_budget` |
| `unipi.footer` | footer segments and stats strip |
| `unipi.updater` | Oino release checks, README / changelog browser (`/unipi:readme`, `/unipi:changelog`) |
| `unipi.input-shortcuts` | `alt+s` chord: stash, undo/redo, copy reply, kanboard capture |
| `unipi.fusion` | persistent "sidekick" second model (`/unipi:model`, `/unipi:fusion-stats`) |
| `unipi.watchdog` | stuck-run detection, opt-in session caps (cost, time, tool errors) |
| `unipi.app-bridge` | control-centre side pane for the desktop / mobile apps (`/unipi:app`) |

Not ported: `unipi.utility` (render styles, auto-rename, `/unipi:answer`, `/unipi:move`,
image tools — planned), `/unipi:cron`, browser skills, app push encryption and Herdr
integration (dropped, see [dev/v1/unipi-port.md](dev/v1/unipi-port.md)). Per-component
differences from unipi are listed in `docs/dev/v1/deviations/unipi-*.md`.

## Commands

| Command | Component | Description |
|---|---|---|
| `/unipi:hint` | unipi.core | Browse and search Unicrab hints, or cycle/reset |
| `/unipi:permission` | unipi.workflow | Permission mode — ask · auto (jev-judged) · full |
| `/unipi:plan` | unipi.workflow | Plan mode — plan with the user, then approve a plan |
| `/unipi:goal` | unipi.long-horizon | One objective until verifiably true. Use: medium-complex single deliverables. Cost/success: pareto per success |
| `/unipi:ralph` | unipi.long-horizon | Checklist grind over iterations. Use: enumerable chores, repo-scale plans. Cost/success: low cost, solid succe |
| `/unipi:swarm` | unipi.long-horizon | Parallel fan-out + one synthesis on the agent graph. Use: complex decomposable work. Cost/success: higher cost |
| `/unipi:graph` | unipi.long-horizon | Dependent multi-step work on the durable agent graph. Use: later steps need earlier results. Cost/success: hig |
| `/unipi:regular` | unipi.long-horizon | Regular mode — stop any active owner and run prompts without long-horizon routing |
| `/unipi:visualize-progress` | unipi.long-horizon | Show long-horizon progress (goal, ralph checklist, agent graph) |
| `/unipi:memory` | unipi.memory | Memory status and session overrides: /unipi:memory status/recall on/off/write on/off/migrate |
| `/unipi:memory-process` | unipi.memory | Analyze text and store extracted memories |
| `/unipi:memory-consolidate` | unipi.memory | Consolidate current session into memory |
| `/unipi:memory-search` | unipi.memory | Search project memories |
| `/unipi:global-memory-search` | unipi.memory | Search memories across all projects |
| `/unipi:memory-forget` | unipi.memory | Delete a memory by title |
| `/unipi:global-memory-list` | unipi.memory | List all memories across all projects |
| `/unipi:dream` | unipi.dream | Dream (background memory consolidation): /unipi:dream status/tray/report/run/stop/approve <n>/reject <n> |
| `/unipi:skills` | unipi.skill-registry | Skill settings — proxy, exposure, and per-skill Enabled / Must show (opens /unipi:settings on Skills) |
| `/unipi:info` | unipi.info-screen | Dashboard: this session, usage, tools, skills, modules |
| `/unipi:subagents` | unipi.subagents | Open the Subagents tab of the work tray (also ↓ from an empty input) |
| `/unipi:agents` | unipi.subagents | Manage subagent profiles: list, create, edit, copy, delete custom agents |
| `/unipi:bg` | unipi.background-tasks | Start a shell command as a tracked background task: /unipi:bg [--agent] [--name "Task name"] <command> |
| `/unipi:bg-tasks` | unipi.background-tasks | Open the work tray on the Background tasks tab |
| `/unipi:btw` | unipi.btw | Ask a side question in an inline panel — read-only, never reaches the main agent. |
| `/unipi:mcp-status` | unipi.mcp | Show status of all configured MCP servers |
| `/unipi:notify-event` | unipi.notify | Toggle a notify event without the TUI: <event> <on/off> |
| `/unipi:kanboard` | unipi.kanboard | Kanboard — the board: open, onboard, status, doctor (bare = help) |
| `/unipi:kanboard-add` | unipi.kanboard | Add a board task — [-p 1-5] [--after ID] [--status backlog/todo] <title>; lines below are the description (pas |
| `/unipi:kanboard-do` | unipi.kanboard | Grant the agent kanboard task slots and a write budget (/unipi:kanboard-do <request>; off revokes) |
| `/unipi:kanboard-autowork` | unipi.kanboard | Work ready board tasks one by one — start / stop (stop finishes the current task first) |
| `/unipi:compact-vcc` | unipi.compactor | Lossless compaction now — instant structured summary, no model call (keep:N keeps N recent turns) |
| `/unipi:compact-by-llm` | unipi.compactor | Compact now with a model-written summary (optional text focuses the summary) |
| `/unipi:session-recall` | unipi.compactor | Recall earlier parts of this session. Plain keywords work best; add scope:all to reach edited or retried turns |
| `/unipi:compact-stats` | unipi.compactor | Show this session's compaction savings |
| `/unipi:compact-doctor` | unipi.compactor | Check compaction settings and leftovers |
| `/unipi:compact-help` | unipi.compactor | Show compactor help |
| `/unipi:footer` | unipi.footer | Toggle footer on/off |
| `/unipi:readme` | unipi.updater | Browse package README files |
| `/unipi:changelog` | unipi.updater | Browse changelog (Keep a Changelog format) |
| `/unipi:model` | unipi.fusion | Pick a model or a Fusion lead+sidekick pair (Devin-style picker over your preset) |
| `/unipi:fusion-stats` | unipi.fusion | Estimated Fusion savings (sidekick tokens priced at lead rates) |
| `/unipi:bg-detach` | unipi.watchdog | Move the currently running bash call to a background task |
| `/unipi:app` | unipi.app-bridge | Open the session control centre (Oino Desktop/Mobile side pane) |
