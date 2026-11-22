---
name: kanboard
description: "Kanboard — the project's deferred-work board. Use when the user asks to note a task for later, to see what is on the board, or to work board tasks (e.g. do UNI-5): `start` a task before working on it and `finish` it with a summary when done; while working a board task: read it with `unipi-kanboard show`, add notes, block with a question, or file follow-up work."
---

# Kanboard

The board is a per-project list of deferred work: backlog, todo, in progress, in
review, blocked, done, cancelled (and an archive). **The board files are owned by
the `unipi-kanboard` binary** — never edit `task.md` files by hand; every write
goes through the CLI so the format and the transition rules hold.

## The tool (preferred)

Chats in projects with an active board have a `kanboard` tool. Boards are set up
and unarchived from Superagent. Pass the board command as `argv`, without
the binary or the global flags — the tool adds your identity (`--actor agent
--session …`), the project and the origin for you (`--json` too):

- `kanboard {"argv":["list","--ready"]}`
- `kanboard {"argv":["add","Fix login timeout","--status","todo"]}`
- `kanboard {"argv":["start","OIN-12"]}` — claims that ticket
- `kanboard {"argv":["finish","OIN-12","--comment","Fixed; tests added","--attach","/abs/screenshot.png"]}` — attach evidence to the report
- `kanboard {"argv":["move","OIN-12","blocked","--comment","Need the API key"]}`
- `kanboard {"argv":["show","OIN-12"],"project":"other-slug"}` — another board
- `kanboard {"argv":["attach","OIN-12","/abs/screenshot.png","--note","Finished UI"]}` — attach evidence separately

Do not pass `--actor`, `--session`, `--project`, `--home`, `--gate` or
`--origin`; the tool sets them (use the `project` field for another board).
`reap`, `release`, `duplicate`, `archive-sweep`, `validate` and
`project add|archive|unarchive` are CLI-only. Writes are never capped — add as
many tasks as the work needs. Claims are per ticket, so a lead and a chat can
hold different tickets in one project.

## The CLI (fallback)

When there is no tool (a shell outside a session), always pass the actor and
the project, and call it by absolute path (it may not be on `PATH`):

```sh
<binary> --actor agent --project <slug> list [--ready] [--all] [--json]   # bare list = todo/in_progress/blocked/in_review/done only; --all adds backlog/cancelled/archived
<binary> --actor agent --project <slug> show <ID>
<binary> --actor agent --project <slug> next                     # what would be picked next + why (read-only)
<binary> --actor agent --project <slug> chain <ID>               # upstream deps + downstream dependents
<binary> --actor agent --project <slug> search "<text>" [--all]  # id/title/body, archived excluded unless --all
<binary> --actor agent --project <slug> add "<title>" [--status todo] [--after <ID>] [--body-file <f>] [--attach <file>]…
<binary> --actor agent --project <slug> note <ID> "<text>" [--attach <file>]…
<binary> --actor agent --project <slug> attach <ID> <file> --note "<what it shows>"
<binary> --actor agent --project <slug> attachments <ID>
<binary> --actor agent --project <slug> edit <ID> --title|--body|--labels …   # only tasks you created, while in backlog/todo
<binary> --actor agent --project <slug> start <ID>                          # todo → in progress, or blocked → in progress to resume a task you blocked (either way claims it for your session)
<binary> --actor agent --project <slug> finish <ID> --comment "<summary>" [--attach <file>]…   # attach screenshots for visible results when available
<binary> --actor agent --project <slug> move <ID> blocked --comment "<what you need>" [--attach <file>]…
<binary> --actor agent --project <slug> move <ID> todo [--comment "<why>"]   # unblock your own blocked task (comment optional) — then `start <ID>` to resume it, or leave it in todo
<binary> --actor agent --project <slug> move <ID> cancelled --comment "<why>"   # abandon a ticket: backlog/todo freely, in_progress/blocked only if your session holds it
<binary> --actor agent --project <slug> link <ID> --after <DEP>
<binary> --actor agent --project <slug> unlink <ID> --after <DEP>
<binary> --actor agent --project <slug> order <ID> --top|--bottom|--before <ID>
```

There is **no queue and no runner** — the session works tasks itself
(Superagent dispatches separate unattended workers).
`queue`, `unqueue`, `claim-next`, `set-run` and `edit --strategy/--plan` are
removed and refused.

`--json` gives machine-readable output for every subcommand (task objects carry
`ready`, `waitingFor`, `depsStatus` and `staleness`).

`list`/`search` show a one-line body excerpt; read a task's full description with
`show <ID>` before editing or starting it.

`settings show` reads the Oino runtime and effective limits; `settings set`
and `rotate-token` are user-only — the agent is refused.

## Lanes and who may move what

| Move | Who |
|---|---|
| backlog ↔ todo | user, agent |
| todo → in progress | the agent with `start <ID>` (claims it for its session; is idempotent for the same session) |
| blocked → in progress | the agent with `start <ID>`, to resume a task it blocked — same claim rules, same dependency rules |
| edit a task | agent — only tasks it created, and only in backlog/todo |
| in progress → in review | the agent with `finish <ID> --comment` — only a task its own session `start`ed |
| in progress → blocked | agent, system — **comment required** (what you need) |
| in progress → todo | user or system `release` — comment required (the session died or gave the task up) |
| blocked → todo | **user or agent** — comment optional (the answer, when there is one) |
| blocked → backlog | **user or agent** — comment optional (park it back in the backlog instead of unblocking to todo) |
| in review → done | user only |
| in review → todo/backlog | user only — comment optional (a rework note, when there is one) |
| backlog/todo → cancelled | user, agent — **comment required** for an agent |
| in progress/blocked → cancelled | the agent session that holds the claim — **comment required**, clears the claim |
| in review → archived | user only (one-click archive) |
| done/cancelled → archived | user (or automatically) |

`done`, `cancelled` and `archived` are final: nothing leaves them — file a new
task instead (`duplicate <ID>` does that for you). Old archived/cancelled
tasks may be moved to cold storage (`projects/<slug>/cold/`); the board never
lists them — read the files there directly if you need one.

## Rules for agents

1. **Never pass `--actor user`.** That is an honour system: pretending to be the
   user to cancel a task or mark it done breaks the board's whole contract.
2. **Work the tasks yourself, in this session** (the user says "do UNI-5"):
   **`start` a task before working on it, `finish` it with a summary when done**
   — `start <ID>` before your first edit, `finish <ID> --comment "<what you
   did>"` before your turn ends (or `move <ID> blocked --comment "<what you
   need>"` if you cannot). Reads, `start`, `note`, `move` and `finish` are never
   charged (nothing is, `add` included).
   Dispatcher workers have a preallocated claim and exact session identity supplied by
   the host's `kanboard` tool. In Review means a human reviews the completed work
   (the board monitor will nudge you back to it). **Never move a task to
   `done`.**
3. **Cancel only a ticket you hold.** `move <ID> cancelled --comment "<why>"` — from
   backlog/todo freely, or from in_progress/blocked only while your session holds
   the claim. Never cancel from in_review; hand it to the owner instead.
4. **Follow the blocking rule in your task prompt**: by default work
   autonomously and record assumptions with `note <ID> "assumed: <what/why>"`;
   block (with a comment saying exactly what you need) only when you truly
   cannot continue — `move <ID> blocked --comment "<what you need>"`. Once you
   have what you needed, you may unblock it yourself — `move <ID> todo` (a
   comment is optional, unlike blocking) — and resume it with `start <ID>`
   (blocked → in progress; the same claim rules as a fresh
   `start`, and it still is idempotent for the same session). You do not have to wait for the
   user to answer in the board if you already have the answer.
5. **Work only on the task you were given.** Follow-up work goes to the board as
   a new task in Backlog (`add "<title>"`), optionally `link <new> --after <ID>`.
   Any number of sessions may hold tasks in a project at once. If the board
   refuses a `start`, a dependency is the usual reason. You may
   block only the task your own session is running (`move <ID> blocked` checks
   `--session`/`UNIPI_KANBOARD_SESSION` against the claim).
6. **Sidekicks and subagents can write the board too** — through the `kanboard`
   tool or the CLI (they get their own session identity and claims). Brief them
   with the task, then read their reports; update the parent tickets yourself.
   (The lead session works tasks itself; Superagent dispatches distinct
   unattended worker sessions.)
7. Use `note <ID> "<text>"` for progress worth remembering (decisions, what you
   verified, what you left undone) — it is the activity log the next reader sees.
8. **Write the `finish` / `blocked` comment as the report the user reads.** It
   is shown as the task's status banner and opens in a reader, rendered as
   markdown. Lead with one plain sentence (what you did, or what stops you),
   then short paragraphs or `-` bullets separated by blank lines; put questions
   for the user as a numbered list under `**Need from you:**`. Name files and
   commands in backticks. Do not repeat a note you already wrote — the report
   replaces it. Attach evidence with `--attach <file>`.
9. Dependencies form a DAG: a task is ready only when every dep reached the chain
   gate (`in_review` by default, `done` when configured). Cancelled deps block
   forever, so unlink or re-plan instead of waiting. `chain <ID>` shows the
   whole line, `next` shows what would be picked and why others wait.

## Terminal-only execution

The board panel (`/kanboard open`) and the daemon's JSON API can create,
edit, reorder, link and move tasks, but they **never run a task** — there is no
run button. Work starts only from a terminal: the session
itself. Use Superagent to dispatch ready todo tickets through the host watcher.

## Attachments

Users attach screenshots, logs and documents to tasks; they appear in the
text as markdown with `att:<ID>/<name>` references, and `show <ID> --json` lists
them under `attachments` with an absolute `path` (or use `attachments <ID>`).
Open image paths with the `read` tool when your model supports vision; if it
does not, call `image_recognize` on the path. Never silently skip an available
mockup. If a ticket cites an owner message in Superagent chat, read that exact message with `owner_messages` too; it may hold the only copy of an image not attached to the ticket. To hand back evidence (a screenshot, a log, a report), pass
`--attach <file>` (repeatable) to `finish`, `move … blocked` or `note`, or use
`attach <ID> <file> --note "…"`: the file is stored beside the board and the
comment embeds it, so the user sees the image or file inline. A file's path
written in the comment is replaced by the embed; otherwise the embed is
appended. For visible results, attach a screenshot to the ticket with
`finish <ID> --comment "…" --attach <png>` or
`attach <ID> <png> --note "…"`; optional when there is no visual result or
screenshot. Do not only paste screenshots in chat. Keep finish comments to
high-level details in simple technical English unless the owner asks for
detail. Attaching to a task you started is free.


## Superagent workers (v3)

Superagent has no workspace or shell access. It resolves an explicit registered project, manages boards across
projects and dispatches through `superagent_start`, `superagent_stop` and
`superagent_relay`. Watcher and manual starts share one host dispatcher. Legacy
transcript cards remain readable.

Set ticket ceilings with `add --autonomy low|medium|high` or
`edit <ID> --autonomy low|medium|high` (default Medium). Workers are unattended.
Dispatcher claims persist the exact worker session id before launch and are
per ticket (a lead and a chat can hold different tickets in one project); they
never return interrupted work to todo automatically.
Interrupted work becomes blocked. Explicitly move it to todo to retry.
