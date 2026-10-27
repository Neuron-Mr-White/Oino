---
name: cron
description: Schedule a one-time or repeating unattended Oino run on this PC — the cron scheduler owns it, no Oino window needed. Use when the user asks to schedule something, remind them, repeat a task daily/hourly/weekly, or run something later ("remind me in 2 hours", "every morning summarize my inbox", "check this every 5 minutes"). Covers listing/editing/removing jobs and checking what a past run did.
---

# cron — scheduled, unattended Oino runs

The cron scheduler (`oino cron serve`, usually installed as a user
service with `oino cron install`) fires `oino -p "<prompt>"` on its
own — on a cron schedule or once at a given time — with nobody there
to answer it. You manage jobs with the `cron_*` tools, the `/cron`
command, or the `oino cron` CLI. Each job runs in its own session, so
you can open what a run did afterwards.

## Before anything: write a self-contained, one-shot prompt

The job runs **unattended**: no one will be there to answer a
question, approve a choice, or clarify anything. Write the prompt as
if you were briefing someone who will never hear back from you:

- State the whole task in the prompt text — don't rely on "like we
  discussed" or anything from this conversation; the scheduled run
  starts a fresh Oino session with no memory of it.
- Decide for it, don't ask it to decide. If there's a choice to make,
  make it in the prompt ("use the top 5", "pick the most recent
  file").
- **Say where the result goes** — a job that only prints to a log
  nobody reads isn't useful. Options:
  - **A file**: "write the summary to ~/notes/digest.md (overwrite
    each time)" or "append a line to ~/logs/check.log".
  - **A notification from the job itself**: set the job's notify
    events (below). Oino sends a message on the user's configured
    channels when the run finishes, so the prompt doesn't have to.
  - At minimum, the job's own summary (the last thing the run says)
    is kept and can be read back with `/cron show <id>` or
    `oino cron log <run id>` — mention that to the user if nothing
    richer applies.
- Keep it short and concrete. One job, one clear outcome.

## Setting it up

1. **Pick (or create) a folder** for the job to run in (`cwd`) — it
   must exist. Reuse the project folder if this is about an existing
   project; otherwise a plain folder like `~/jobs/<name>` is fine
   (`mkdir -p` it first if it's new).
2. **Add the job** — with the `cron_add` tool, or:

   ```bash
   oino cron add --name "<short name>" \
     --prompt "<the full, self-contained prompt — see above>" \
     --schedule "<plain language, see below>" \
     --cwd <the folder>
   ```

   For a one-time reminder, use `--at` instead of `--schedule`:

   ```bash
   oino cron add --name "<short name>" --prompt "<prompt>" \
     --at "in 2 hours" --cwd <the folder>
   ```

   Plain-language schedules/times that work: `every 5 minutes`,
   `every day at 9:00`, `weekdays at 18:30`, `every monday 08:00`,
   `hourly`, `daily` for `--schedule`; `in 2 hours`, `tomorrow at 9`
   for `--at`. A 6-field cron (seconds first) or an exact
   `at:<RFC3339>` instant also works, for anything the phrasing above
   doesn't cover. `--tz <IANA zone>` picks the zone (default: this
   PC's).

   Optional flags worth offering:
   - **`--notify ntfy,telegram,gotify --notify-on done,failed`**: Oino
     notifies on the user's own channels when a run ends. Without
     `--notify` it uses the user's enabled channels, for failed runs
     only. `--notify-on` takes `done`, `failed` and `skipped`.
     `oino cron test-notify <id>` sends one test message.
   - **`--time-limit-s`** (default 1800) caps a run's wall clock;
     `--cost-cap` records the run as failed once it costs more.
   - **`--overlap skip|allow`**: what a fire does while the previous
     run of the same job is still going (default `skip`, recorded);
     **`--missed run-latest|skip`**: what happens to fires missed
     while the PC was off (default `run-latest`: the single latest
     missed fire runs late, within the missed window).

## Day to day

```bash
oino cron list                 # every job: schedule, enabled, name
oino cron status               # running/paused, next fire per job
oino cron show <id>            # one job + recent runs
oino cron runs [<id>] [-n N]   # run history (newest first)
oino cron log <run id>         # one run's output + session file
oino cron run <id>             # fire it now (bypasses the schedule)
oino cron stop <id>            # kill its running runs
oino cron enable|disable <id>
oino cron edit <id> --schedule "every 2 hours"   # change anything
oino cron remove <id>          # delete the job and its history
oino cron pause                # pause everything; running ones finish
oino cron resume
```

The same view lives in the desktop app's Automations page and as
`/cron …` in any Oino session.

## Reading a job's results

- `oino cron show <id>`: the last 10 runs with their status and the
  summary the run ended with.
- `oino cron log <run id>`: the run's captured output (last 64 KiB),
  its cost, and the path of its session file — open that file with
  `oino --session <path>` (or the desktop's session list) to see the
  whole run turn by turn.
- Runs keep for 50 per job / 30 days by default
  (`oino cron settings`).

## Checks

- The scheduler must be running for jobs to fire:
  `oino cron status` says so; `oino cron install` sets it up to start
  at login (Linux: systemd user service; macOS: launchd agent;
  Windows: Task Scheduler task). Without it, `run`/`stop` won't work
  and due jobs wait.
- A job's folder must exist before the job is added.
- Ask the user before creating or changing jobs (`cron_add`,
  `cron_update`, `cron_remove` ask for approval in ask/auto
  permission mode).
