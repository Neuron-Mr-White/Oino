# Cron — scheduled, unattended Oino runs

Oino can run prompts on a schedule with nobody at the keyboard: a **cron job** fires
`oino -p "<prompt>"` in a folder you pick, on a schedule (or once at a time you pick), and
records every **run** — status, the summary the run ended with, its cost, its output, and
its own session file you can open later.

Jobs are plain JSON files under `~/.oino/cron/jobs/` (one per job). Manage them with the
`/cron` command in any session, the `cron_*` tools (ask the agent: "summarize my inbox
every morning at 9"), or — once the Unipi app ships — its Automations page and the
`unipi-host cron` CLI.

## The scheduler

Jobs fire only while a scheduler runs — exactly one per machine (a lock file enforces
it). Under v2 the scheduler is **embedded in the Unipi host** (`unipi-host`, the tray
app): while it is up, jobs fire; when it is not, due jobs wait and the single latest
missed fire runs late once it starts again (within the missed window).

Under a plain `oino` terminal nothing fires, but everything else works: jobs can be
listed, added, edited and removed (they are just files), and `run`/`stop`/`status`
report that no scheduler is running.

## Schedules

Plain language works:

- `every 5 minutes` · `every 2 hours` · `every 30 seconds`
- `every day at 9:00` · `weekdays at 18:30` · `every monday 08:00`
- one-time: `in 2 hours` · `tomorrow at 9` · `at:2025-12-01T09:00:00Z`

Anything else is a **6-field cron** (seconds first): `0 30 9 * * MON-FRI` is 9:30 every
weekday. A job with an `at:` schedule disables itself after its one run.

## Job files

One file per job, `~/.oino/cron/jobs/<id>.json`, schema `"v": 2` (camelCase fields, the
`id` must equal the file name):

```json
{
  "v": 2,
  "id": "digest",
  "name": "Daily digest",
  "prompt": "Summarize today's inbox and write it to ~/notes/digest.md",
  "schedule": "0 30 9 * * MON-FRI",
  "tz": "Europe/Berlin",
  "cwd": "/home/me/work",
  "enabled": true,
  "timeLimitS": 1800,
  "overlap": "skip",
  "missed": "run-latest",
  "createdAt": 0,
  "updatedAt": 0
}
```

Optional fields: `model`, `thinking`, `costCapUsd`, `notify` (`{channels, events}`).
Delete a job by deleting its file; enable/disable by flipping `enabled`. Every writer
(the extension, the host CLI, the scheduler) takes `jobs/.lock` and writes atomically,
and the scheduler watches the folder, so edits land without a restart. Run records live
under `runs/<job id>/<run id>.json` with a capped `.log` beside them; runs keep for
50 per job / 30 days by default (the scheduler settings).

A pre-v2 `jobs.json` is split into per-job files automatically the first time anything
opens the store (the original is kept as `jobs.json.imported`).

## Managing jobs

```
/cron list                 # every job: schedule, enabled, name
/cron status               # running/paused, next fire per job
/cron show <id>            # one job + recent runs
/cron runs <id> [N]        # run history (newest first)
/cron run <id>             # fire it now (bypass the schedule)
/cron stop <id>            # kill its running runs
/cron enable|disable <id>
/cron edit <id> schedule="every 2 hours"
/cron remove <id>
/cron pause | /cron resume
```

`run`, `stop`, `pause`, `resume` and live `status` need the Unipi host (they talk to its
embedded scheduler); the rest read and write the files directly and work everywhere.
The `cron_list`, `cron_add`, `cron_update`, `cron_remove` and `cron_run` tools do the
same from a conversation (the write tools ask for approval first).

## Unattended prompts

A scheduled run starts a fresh session with no memory of any conversation, and nobody
answers its questions: write the prompt as a complete brief — the whole task, the
choices made for it, and where the result goes (a file, a notification). Set the job's
`notify` events to get a message on your channels when a run ends. Keep it short and
concrete: one job, one clear outcome.
