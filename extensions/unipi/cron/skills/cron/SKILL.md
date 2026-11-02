---
name: cron
description: Schedule a one-time or repeating unattended Oino run on this PC — the cron scheduler owns it, no Oino window needed. Use when the user asks to schedule something, remind them, repeat a task daily/hourly/weekly, or run something later ("remind me in 2 hours", "every morning summarize my inbox", "check this every 5 minutes"). Covers listing/editing/removing jobs and checking what a past run did.
---

# cron — scheduled, unattended Oino runs

The cron scheduler — embedded in the Unipi host (`unipi-host`, which
runs whenever the Unipi tray app is up) — fires `oino -p "<prompt>"`
on its own: on a cron schedule or once at a given time, with nobody
there to answer it. You manage jobs with the `cron_*` tools or the
`/cron` command; jobs live as plain files under `~/.oino/cron/jobs/`,
one per job. Each job runs in its own session, so you can open what a
run did afterwards.

Firing needs the Unipi host: under a plain `oino` terminal nothing
fires, but jobs can still be listed, added, edited and removed —
`cron_run` then reports that no scheduler is running.

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
    `/cron runs <id>` — mention that to the user if nothing richer
    applies.
- Keep it short and concrete. One job, one clear outcome.

## Setting it up

1. **Pick (or create) a folder** for the job to run in (`cwd`) — it
   must exist. Reuse the project folder if this is about an existing
   project; otherwise a plain folder like `~/jobs/<name>` is fine
   (`mkdir -p` it first if it's new).
2. **Add the job** — with the `cron_add` tool, or:

   ```
   /cron add name="<short name>"
     prompt="<the full, self-contained prompt — see above>"
     schedule="<plain language, see below>"
     cwd=<the folder>
   ```

   For a one-time reminder, a `schedule` in the one-time forms
   (`in 2 hours`, `tomorrow at 9`, or an exact
   `at:2025-12-01T09:00:00Z`) does the same job.

   Plain-language schedules/times that work: `every 5 minutes`,
   `every day at 9:00`, `weekdays at 18:30`, `every monday 08:00`,
   `hourly`, `daily`; one-time forms are `in 2 hours` and
   `tomorrow at 9`. A 6-field cron (seconds first) or an exact
   `at:<RFC3339>` instant also works, for anything the phrasing above
   doesn't cover. `tz=<IANA zone>` picks the zone (default: this
   PC's).

   Optional flags worth offering:
   - **`notify=ntfy,telegram,gotify` and `notifyOn=done,failed`**: Oino
     notifies on the user's own channels when a run ends. Without
     `notify` it uses the user's enabled channels, for failed runs
     only. `notifyOn` takes `done`, `failed` and `skipped`.
   - **`timeLimitS`** (default 1800) caps a run's wall clock;
     `costCapUsd` records the run as failed once it costs more.
   - **`--overlap skip|allow`**: what a fire does while the previous
     run of the same job is still going (default `skip`, recorded);
     **`--missed run-latest|skip`**: what happens to fires missed
     while the PC was off (default `run-latest`: the single latest
     missed fire runs late, within the missed window).

## Day to day

```
/cron list                 # every job: schedule, enabled, name
/cron status               # running/paused, next fire per job
/cron show <id>            # one job + recent runs
/cron runs <id> [N]        # run history (newest first)
/cron run <id>             # fire it now (bypasses the schedule)
/cron stop <id>            # kill its running runs
/cron enable|disable <id>
/cron edit <id> schedule="every 2 hours"   # change anything
/cron remove <id>          # delete the job (run history ages out)
/cron pause                # pause everything; running ones finish
/cron resume
```

The same view lives in the app's Automations page; the `cron_*` tools
do all of this from a conversation. (`run`, `stop`, `pause`,
`resume` and live `status` need the Unipi host; the rest work
everywhere.)

## Reading a job's results

- `/cron show <id>`: the last 10 runs with their status and the
  summary the run ended with.
- `/cron runs <id>` and the `cron_list` tool: run history with each
  run's session file path — open that file with `oino --session
  <path>` (or the app's session list) to see the whole run turn by
  turn.
- Runs keep for 50 per job / 30 days by default (the scheduler
  settings in the hub's Cron section).

## Checks

- The Unipi host must be running for jobs to fire (`/cron status`
  says so). Without it, due jobs wait — the host's scheduler catches
  them up on start (the single latest missed fire, within the missed
  window) — and `run`/`stop` report that no scheduler is running.
- A job's folder must exist before the job is added; deleting a job
  removes its file, and its run history ages out with retention.
- Ask the user before creating or changing jobs (`cron_add`,
  `cron_update`, `cron_remove` ask for approval in ask/auto
  permission mode).
