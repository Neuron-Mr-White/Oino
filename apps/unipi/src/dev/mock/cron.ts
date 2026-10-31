/**
 * The mock cron world (WP-A23): job files + run history for the capture
 * harness and component tests, answering every `cron_*` op the Cron screen
 * and its form use — over the same shapes the H8 adapter
 * (`apps/unipi/crates/unipi-host/src/cron_serve.rs`) puts on the wire (jobs live in
 * `<agent dir>/cron`, runs carry the fired session, the run id *is* the
 * session id).
 *
 * A scenario mounts it with `ops: cronOps(...)` (spec `ops` beat the mock
 * host's built-ins), so `src/dev/mock/host.ts` stays untouched.
 */
import type { MockOp } from './host';

export type MockCronJob = {
  id: string;
  name: string;
  prompt: string;
  schedule: string;
  tz: string;
  cwd: string;
  model?: string;
  enabled: boolean;
  time_limit_s: number;
  created: number;
  updated: number;
  next_fire?: string;
  running?: boolean;
  last_run_status?: 'running' | 'ok' | 'failed' | 'timeout' | 'skipped' | 'queued';
  notify?: { channels?: string[]; events?: string[] };
  wake?: { session_file?: string; session_id?: string };
};

export type MockCronRun = {
  id: string;
  job_id: string;
  status: 'running' | 'ok' | 'failed' | 'timeout' | 'skipped' | 'queued';
  fired_at: number;
  started_at?: number;
  ended_at?: number;
  exit_code?: number;
  summary?: string;
  reason?: string;
  session_path?: string;
  cost_usd?: number;
  missed?: boolean;
};

/** The mutable world behind the ops: the job list, the runs, the flags. */
export type MockCronWorld = {
  jobs: MockCronJob[];
  /** Run history, newest first per job (like the runs store). */
  runs: Record<string, MockCronRun[]>;
  paused?: boolean;
  /** What `cron_log` answers per run id (defaults to a short faux log). */
  logs?: Record<string, string>;
  /** Fired run ids the scheduler "wrote a session for" (the run id is the session id). */
  sessions?: Set<string>;
};

type Err = { code: string; message: string; hint?: string; retryable: boolean };
const err = (code: string, message: string): Err => ({ code, message, retryable: code === 'E_TRANSPORT' || code === 'E_TIMEOUT' });

const DEFAULT_SETTINGS = {
  concurrency: 2,
  missed_window_s: 3600,
  retention_runs: 50,
  retention_days: 30,
  default_time_limit_s: 1800,
  paused: false,
  enabled: true,
  devices: [],
  notify_default_events: ['failed'],
  notify_channels: ['ntfy', 'telegram'],
};

/** A plausible `cron_parse` answer: plain-language and 6-field inputs resolve;
 *  anything unknown passes through with generic next fires (the preview only
 *  needs expression + times). */
export function mockParse(input: string, at: boolean, tz?: string): { expression: string; one_time: boolean; tz: string; next_fires: string[] } {
  const zone = tz ?? 'UTC';
  const iso = (offsetMs: number) => new Date(Date.now() + offsetMs).toISOString();
  // "every day at 9:30" (or the equivalent 6-field `0 30 9 * * *`) → the
  // canonical form plus the next three fires at that wall-clock time.
  const plain = /^every day at (\d{1,2}):(\d{2})$/.exec(input);
  const machine = /^0 (\d{1,2}) (\d{1,2}) \* \* \*$/.exec(input);
  if (plain || machine) {
    const minute = Number(plain ? plain[2] : machine![1]);
    const hour = Number(plain ? plain[1] : machine![2]);
    const next = [...Array(3)].map((_, i) => {
      const d = new Date();
      d.setUTCHours(hour, minute, 0, 0);
      if (d.getTime() <= Date.now()) d.setUTCDate(d.getUTCDate() + 1);
      d.setUTCDate(d.getUTCDate() + i);
      return d.toISOString();
    });
    return { expression: `0 ${minute} ${hour} * * *`, one_time: false, tz: zone, next_fires: next };
  }
  if (at || /^in |^tomorrow/.test(input)) {
    return { expression: iso(2 * 3600_000), one_time: true, tz: zone, next_fires: [iso(2 * 3600_000)] };
  }
  return { expression: input, one_time: false, tz: zone, next_fires: [iso(3600_000), iso(25 * 3600_000), iso(49 * 3600_000)] };
}

/** Every `cron_*` op over `world` (mutable: add/edit/run mutate it like the store). */
export function cronOps(world: MockCronWorld): Record<string, MockOp> {
  const runsOf = (jobId: string) => world.runs[jobId] ?? [];
  const findJob = (id: unknown) => {
    const job = world.jobs.find((j) => j.id === id);
    if (!job) throw err('E_NOT_FOUND', `no job ${JSON.stringify(String(id))}`);
    return job;
  };
  const allRuns = () => Object.values(world.runs).flat();
  return {
    cron_list: () => ({ jobs: world.jobs.map((j) => ({ ...j, running: j.running ?? false })), paused: world.paused ?? false }),
    cron_show: (a) => ({ job: { ...findJob(a.id), running: findJob(a.id).running ?? false }, runs: runsOf(String(a.id)) }),
    cron_runs: (a) => ({ runs: a.id ? runsOf(String(a.id)) : allRuns() }),
    cron_log: (a) => {
      const run = allRuns().find((r) => r.id === a.runId);
      if (!run) throw err('E_NOT_FOUND', `no run ${JSON.stringify(String(a.runId))}`);
      return { job_id: run.job_id, run, log: world.logs?.[run.id] ?? 'Digest: 12 threads, 3 replies.\nDone in 41 s.' };
    },
    cron_add: (a) => {
      const job = a.job as MockCronJob;
      if (world.jobs.some((j) => j.id === job.id)) throw err('E_CONFLICT', `job ${job.id} already exists`);
      const now = Date.now();
      const made: MockCronJob = { ...job, created: job.created ?? now, updated: job.updated ?? now, enabled: job.enabled ?? true, time_limit_s: job.time_limit_s ?? 1800, tz: job.tz ?? 'UTC' };
      world.jobs = [...world.jobs, made];
      return { ...made, running: false };
    },
    cron_edit: (a) => {
      const job = findJob(a.id);
      Object.assign(job, (a.patch as object) ?? {});
      job.updated = Date.now();
      return { ...job, running: job.running ?? false };
    },
    cron_rm: (a) => {
      findJob(a.id);
      world.jobs = world.jobs.filter((j) => j.id !== a.id);
      delete world.runs[String(a.id)];
      return null;
    },
    cron_enable: (a) => ((findJob(a.id).enabled = true), null),
    cron_disable: (a) => ((findJob(a.id).enabled = false), null),
    cron_run: (a) => {
      const job = findJob(a.id);
      const run: MockCronRun = {
        id: `${job.id}-${Date.now()}`,
        job_id: job.id,
        status: 'running',
        fired_at: Date.now(),
        started_at: Date.now(),
      };
      world.runs[job.id] = [run, ...runsOf(job.id)];
      job.running = true;
      return { id: job.id, run_id: run.id };
    },
    cron_stop: (a) => {
      const job = findJob(a.id);
      const run = runsOf(job.id).find((r) => r.status === 'running');
      if (run) {
        run.status = 'failed';
        run.ended_at = Date.now();
        run.reason = 'stopped from the app';
      }
      job.running = false;
      return { id: job.id, stopped: Boolean(run) };
    },
    cron_pause_all: () => ((world.paused = true), null),
    cron_resume_all: () => ((world.paused = false), null),
    cron_status: () => ({
      paused: world.paused ?? false,
      concurrency: DEFAULT_SETTINGS.concurrency,
      jobs: world.jobs.map((j) => ({ id: j.id, enabled: j.enabled, next_fire: j.next_fire, running: j.running ?? false })),
    }),
    cron_parse: (a) => mockParse(String(a.schedule ?? a.at ?? ''), Boolean(a.at), a.tz ? String(a.tz) : undefined),
    cron_settings_get: () => ({ ...DEFAULT_SETTINGS, paused: world.paused ?? false }),
    cron_settings_set: () => ({ ...DEFAULT_SETTINGS, paused: world.paused ?? false }),
    // The run id is the fired session's id once the run wrote its file (§9.8).
    cron_open_session: (a) => {
      const run = allRuns().find((r) => r.id === a.runId);
      if (!run) throw err('E_NOT_FOUND', `no run ${JSON.stringify(String(a.runId))}`);
      if (!run.session_path && !(world.sessions ?? new Set()).has(run.id)) {
        throw err('E_NOT_FOUND', 'This run saved no session.');
      }
      return { session: run.id };
    },
  };
}
