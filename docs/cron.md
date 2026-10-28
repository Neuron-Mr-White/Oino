# Cron — scheduled, unattended Oino runs

Oino can run prompts on a schedule with nobody at the keyboard: a **cron job** fires
`oino -p "<prompt>"` in a folder you pick, on a schedule (or once at a time you pick), and
records every **run** — status, the summary the run ended with, its cost, its output, and
its own session file you can open later.

Jobs live in `~/.oino/cron/` as plain JSON. Manage them with the `oino cron` CLI, the
`/cron` command in any session, the Automations page in the desktop app, or the `cron_*`
tools (ask the agent: "summarize my inbox every morning at 9").

## The scheduler

Jobs fire only while a scheduler runs — exactly one per machine (a lock file enforces it):

- **`oino cron serve`** runs it in the foreground (Ctrl-C stops it; running jobs finish).
- **`oino cron install`** registers a user service so it starts at login and stays up:
  systemd user service (Linux), launchd agent (macOS), Task Scheduler task (Windows).
  `oino cron uninstall` removes it.
- **The desktop app** embeds the scheduler while it is open (it takes over the single
  owner lock when no daemon holds it, and attaches to the daemon otherwise).

`oino cron status` says whether it is running, what's paused, and each job's next fire.

## Schedules

Plain language works:

- `every 5 minutes` · `every 2 hours` · `every 30 seconds`
- `every day at 9:00` · `weekdays at 18:30` · `every monday 08:00`
- `hourly` · `daily` · `weekly`
- one-time: `in 2 hours` · `tomorrow at 9` · `2030-01-01T09:00:00Z`

Anything else: a **6-field cron** (seconds first) — `0 30 9 * * MON-FRI` is 9:30 every
weekday — or `at:<RFC3339>` for one-time runs. `--tz Europe/Berlin` picks the zone
(default: the machine's).

## Writing the prompt

A scheduled run is unattended: nobody answers questions, approves choices or clarifies.
Write the prompt as a briefing for someone who will never write back:

- state the whole task in the prompt (the run starts a fresh session with no memory of
  your conversation);
- decide for it ("use the top 5", "pick the most recent file");
- say where the result goes — a file ("write the summary to `~/notes/digest.md`"), a
  notification (see below), or at least the run's own summary, which is kept;
- keep it short. One job, one outcome.

## Managing jobs

```bash
oino cron add --name "daily digest" \
  --prompt "Summarize ~/mail/inbox; write the digest to ~/notes/digest.md" \
  --schedule "every day at 9:00" --cwd ~/jobs/digest

oino cron list                    # every job
oino cron show digest             # one job + recent runs
oino cron runs digest             # run history
oino cron log <run id>            # one run's output + its session file
oino cron run digest              # fire it now
oino cron stop digest             # kill its running runs
oino cron edit digest --schedule "every 2 hours"
oino cron enable|disable digest
oino cron remove digest
oino cron pause                   # pause everything (running ones finish)
oino cron resume
oino cron settings                # concurrency, missed window, retention, …
```

Every command takes `--json`. The same view is `/cron …` in any session (`/cron add
name="…" schedule="…" prompt="…"`), the desktop's Automations page, and the agent's
`cron_list` / `cron_add` / `cron_update` / `cron_remove` / `cron_run` tools (these ask for
approval like other write tools).

## Limits, overlaps, missed runs

- `--time-limit-s` (default 1800) caps a run's wall clock; `--cost-cap 1.00` records the
  run as failed once it costs more.
- `--overlap skip|allow` — what a fire does while the previous run of the same job is
  still going (`skip` records it as skipped, the default; `allow` starts it anyway,
  subject to the concurrency limit).
- `--missed run-latest|skip` — what happens to fires missed while the machine was off
  (`run-latest` runs the single latest missed one late, within the missed window — 6 h by
  default; never a burst of catch-up runs).
- History is kept per job: 50 runs / 30 days by default (`oino cron settings`).

## Notifications

A job can notify when a run ends, on the channels configured in Oino's notify settings
(ntfy, Gotify, Telegram): `--notify ntfy,telegram --notify-on done,failed` (default:
failed runs only, on your enabled channels). `oino cron test-notify <id>` sends a test
message.

## Where things live

| Path | Contents |
|---|---|
| `~/.oino/cron/jobs.json` | every job |
| `~/.oino/cron/runs/<job>/<run>.json` | one run record (status, summary, cost, session file) |
| `~/.oino/cron/runs/<job>/<run>.log` | the run's captured output (last 64 KiB) |
| `~/.oino/cron/settings.json` | scheduler settings |
| `~/.oino/cron/scheduler.lock` | the single-owner lock |
| `~/.oino/cron/serve.json` | the running daemon's pid and control port |

A run's session is a normal Oino session whose **header id is the run id**: `oino --session-id
<run id>` (or `--session <path>`, or the desktop's session list) shows it turn by turn.
