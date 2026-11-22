/**
 * F-11: a cron run's chat is titled with its job ("Nightly build"), not the folder it ran in,
 * and a failed run reads as failed. The sidebar card only says `source: 'cron'`; the run and
 * job records (`cron_runs` / `cron_list`) carry the name and the outcome, matched by run id
 * (= the fired session's id). Loaded lazily per host, at most every 15 s while rows ask.
 */
import { createSignal } from 'solid-js';
import { cronList, cronRuns } from './cron';

export type CronRunLabel = { jobName: string; status: string; reason?: string };

const [labels, setLabels] = createSignal<Record<string, CronRunLabel>>({});
const asked = new Map<string, number>();
const REFRESH_MS = 15_000;

/** The label of the run behind a session id (reactive), asking the host when stale. */
export function cronRunLabel(hostId: string, sessionId: string): CronRunLabel | undefined {
  const key = `${hostId}\u0000${sessionId}`;
  const found = labels()[key];
  const last = asked.get(hostId) ?? 0;
  if (!found && Date.now() - last > REFRESH_MS) {
    asked.set(hostId, Date.now());
    void (async () => {
      const [jobs, runs] = await Promise.all([cronList(hostId), cronRuns(hostId, undefined, 200)]);
      if (!jobs.ok || !runs.ok) return;
      const names = new Map(jobs.value.jobs.map((j) => [j.id, j.name]));
      const next: Record<string, CronRunLabel> = {};
      for (const run of runs.value.runs) {
        next[`${hostId}\u0000${run.id}`] = { jobName: names.get(run.job_id) ?? run.job_id, status: run.status, ...(run.reason ? { reason: run.reason } : {}) };
      }
      setLabels((cur) => ({ ...cur, ...next }));
    })();
  }
  return found;
}

/** A run that did not succeed. */
export const cronRunFailed = (l: CronRunLabel | undefined): boolean => l?.status === 'failed' || l?.status === 'timeout';
