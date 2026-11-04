/**
 * Goal / ralph view data ([lh-ui]). The loop's state arrives as the `LH_PROGRESS` topic only (no
 * event-log service for goal.json yet), so the timeline is OBSERVED: each change the open view
 * sees (opened, turn n, errored, continued, finished) is appended to a bounded list (30).
 * Pure; the component owns the signal.
 */
import type { LhProgressTopic } from './lh';
import type { LhBy, LhTimelineEntry, LhTone } from './lhGraph';

export const LOOP_LOG_MAX = 30;

export type LoopSnap = {
  kind: 'goal' | 'ralph';
  status: string;
  turn: number;
  error?: string;
};

/** The comparable part of the topic for the loop view. */
export function loopSnap(p: Pick<LhProgressTopic, 'goal' | 'ralph'> | undefined): LoopSnap | undefined {
  if (p?.goal) return { kind: 'goal', status: p.goal.status, turn: p.goal.turn, ...(p.goal.error ? { error: p.goal.error } : {}) };
  if (p?.ralph) return { kind: 'ralph', status: 'active', turn: p.ralph.iteration };
  return undefined;
}

const TERMINAL = new Set(['met', 'done', 'completed', 'failed', 'stopped', 'cancelled', 'abandoned', 'exhausted']);

const toneOf = (status: string): LhTone => (status === 'errored' ? 'error' : status === 'met' || status === 'done' || status === 'completed' ? 'ok' : status === 'failed' ? 'fail' : TERMINAL.has(status) ? 'off' : 'run');

/** Appends the events that took `prev` to `next` (newest last), capped at LOOP_LOG_MAX. Pure. */
export function observeLoop(log: readonly LhTimelineEntry[], prev: LoopSnap | undefined, next: LoopSnap | undefined, now: number): LhTimelineEntry[] {
  if (!next) return [...log];
  const out = [...log];
  const add = (kind: string, title: string, tone: LhTone, by: LhBy, detail?: string) =>
    out.push({ id: `l${now}-${out.length}`, ts: now, kind, by, tone, title, ...(detail ? { detail } : {}) });
  const label = next.kind === 'goal' ? 'Goal' : 'Ralph loop';
  if (!prev) {
    add('opened', `${label} seen`, 'wait', 'host');
    if (next.status === 'errored') add('errored', 'Error', 'error', 'host', next.error);
  } else {
    if (next.turn > prev.turn) add('running', next.kind === 'goal' ? `Turn ${next.turn}` : `Iteration ${next.turn}`, 'run', 'host');
    if (next.status === 'errored' && prev.status !== 'errored') add('errored', 'Error', 'error', 'host', next.error);
    else if (prev.status === 'errored' && next.status !== 'errored') add('continued', 'Continued', 'warn', 'user');
    if (next.status !== prev.status && next.status !== 'errored' && prev.status !== 'errored') {
      add(TERMINAL.has(next.status) ? 'finished' : 'running', TERMINAL.has(next.status) ? `${label} ${next.status}` : `Status: ${next.status}`, toneOf(next.status), 'host');
    }
  }
  return out.slice(-LOOP_LOG_MAX);
}
