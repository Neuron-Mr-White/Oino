/**
 * Cron scenarios (WP-A23, Z-112/G-17): the job-file world over the H8 wire —
 *
 *   cron-list    the Cron page: four jobs (next fire, last-run chip, off
 *                badge, a running one) and the pause-all switch; the sidebar
 *                carries the cron-run session with the clock badge
 *   cron-form    the add-job form with the live schedule preview resolved
 *                from "every day at 9:00" (`cron_parse`)
 *   cron-badge   the sessions list where a cron-run session (clock badge)
 *                sits among working / unread / pinned rows
 *
 * The ops come from `mock/cron.ts` (`cronOps`) over the same DTOs the host
 * adapter puts on the wire; `cron-form` and the phone captures drive the real
 * DOM after mount, the same way `git.ts` opens its layer.
 */
import type { MockScenario } from '../host';
import { cronOps, type MockCronJob, type MockCronRun, type MockCronWorld } from '../cron';
import { genCards } from '../gen';

const HOST = 'this-pc';
const CWD = '/home/you/projects/digest';
const DAY = 86_400_000;
const now = Date.now();

const jobs: MockCronJob[] = [
  {
    id: 'daily-digest',
    name: 'Daily inbox digest',
    prompt: 'Summarize today’s inbox into five bullets with links, then post the digest to the team channel.',
    schedule: '0 0 9 * * *',
    tz: 'Europe/Berlin',
    cwd: CWD,
    enabled: true,
    time_limit_s: 1800,
    created: now - 21 * DAY,
    updated: now - 2 * DAY,
    next_fire: new Date(now + 5 * 3600_000).toISOString(),
    last_run_status: 'ok',
    notify: { events: ['done', 'failed'], channels: ['ntfy'] },
  },
  {
    id: 'weekly-report',
    name: 'Weekly usage report',
    prompt: 'Roll last week’s model usage into the report sheet and draft the summary email.',
    schedule: '0 0 18 * * 5',
    tz: 'Europe/Berlin',
    cwd: '/home/you/projects/reports',
    enabled: true,
    time_limit_s: 3600,
    created: now - 60 * DAY,
    updated: now - 3 * DAY,
    next_fire: new Date(now + 2 * DAY).toISOString(),
    last_run_status: 'failed',
  },
  {
    id: 'cleanup-scratch',
    name: 'Clean the scratch dir',
    prompt: 'Delete scratch files older than 14 days and report what went.',
    schedule: '0 0 4 * * 0',
    tz: 'UTC',
    cwd: '/tmp/scratch',
    enabled: false,
    time_limit_s: 600,
    created: now - 9 * DAY,
    updated: now - DAY,
    last_run_status: 'skipped',
  },
  {
    id: 'one-time-handoff',
    name: 'Handoff notes for Monday',
    prompt: 'Write the handoff notes from this week’s sessions into HANDOFF.md.',
    schedule: new Date(now + 26 * 3600_000).toISOString(),
    tz: 'UTC',
    cwd: '/home/you/projects/oino',
    enabled: true,
    time_limit_s: 900,
    created: now - 3600_000,
    updated: now - 3600_000,
    next_fire: new Date(now + 26 * 3600_000).toISOString(),
    running: true,
  },
];

const digestRun = (id: string, status: MockCronRun['status'], ago: number, summary?: string): MockCronRun => ({
  id,
  job_id: 'daily-digest',
  status,
  fired_at: now - ago,
  started_at: now - ago + 2000,
  ended_at: now - ago + 43_000,
  exit_code: status === 'ok' ? 0 : 1,
  ...(summary ? { summary } : {}),
  ...(status === 'ok' ? { session_path: `/home/you/.oino/sessions/--home-you-projects-digest/${id}.jsonl`, cost_usd: 0.0412 } : {}),
  missed: false,
});

const runs: MockCronWorld['runs'] = {
  'daily-digest': [
    { id: 'r-now', job_id: 'daily-digest', status: 'running', fired_at: now - 60_000, started_at: now - 55_000, missed: false },
    digestRun('daily-digest-0182', 'ok', DAY, 'Digest: 12 threads, 3 replies — posted to #team.'),
    digestRun('daily-digest-0181', 'failed', 2 * DAY),
    digestRun('daily-digest-0180', 'ok', 3 * DAY, 'Digest: 9 threads, 1 reply — posted to #team.'),
  ],
  'weekly-report': [
    { id: 'weekly-report-0031', job_id: 'weekly-report', status: 'failed', fired_at: now - 3 * DAY, started_at: now - 3 * DAY, ended_at: now - 3 * DAY + 120_000, exit_code: 1, reason: 'model auth expired (401)', missed: false },
  ],
};

const world: MockCronWorld = { jobs, runs, sessions: new Set(['daily-digest-0182', 'daily-digest-0180']) };

/** The cron-run session cards (the run id is the session id; Z-112 badge). */
function cronCards() {
  return genCards(6, { seed: 5, folders: ['digest', 'oino', 'website'], changes: true, unread: 2, pinned: 1 }).map((card, i) =>
    i === 1
      ? {
          ...card,
          session_id: 'daily-digest-0182',
          name: 'Daily inbox digest · run 182',
          cwd: CWD,
          source: 'cron' as const,
          run_state: 'idle' as const,
        }
      : card,
  );
}

/** `#/cron…` once mounted; the form capture also types the schedule so the
 *  debounced `cron_parse` preview settles before the shot. */
function goCron(form: boolean, type = false): void {
  const hash = form ? '#/cron?form=new' : '#/cron';
  setTimeout(() => {
    location.hash = hash;
    if (!type) return;
    setTimeout(() => {
      const input = document.querySelector<HTMLInputElement>('[data-testid="cron-form-schedule"]');
      if (!input) return;
      input.value = 'every day at 9:00';
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }, 400);
  }, 120);
}

export const cronList: MockScenario = {
  name: 'cron-list',
  hosts: [{ id: HOST, name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: cronCards(), ops: cronOps(world) }],
  setup: () => goCron(false),
};

export const cronForm: MockScenario = {
  name: 'cron-form',
  hosts: [{ id: HOST, name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: cronCards(), ops: cronOps(world) }],
  setup: () => goCron(true, true),
};

export const cronBadge: MockScenario = {
  name: 'cron-badge',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      conn: { state: 'connected' },
      cards: [
        ...cronCards().slice(0, 2),
        {
          session_id: 'weekly-report-0030',
          name: 'Weekly usage report · run 30',
          cwd: '/home/you/projects/reports',
          run_state: 'working' as const,
          updated_at: now - 40_000,
          message_count: 8,
          source: 'cron' as const,
          changes: { add: 40, del: 6 },
        },
        ...genCards(5, { seed: 9, prefix: 'w', folders: ['oino', 'website', 'mobile'], changes: true, needsYou: 3, unread: 2 }),
      ],
      ops: cronOps(world),
    },
  ],
};
