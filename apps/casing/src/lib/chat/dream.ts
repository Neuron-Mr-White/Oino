/**
 * UNI-118: Dream (background memory consolidation + skill proposals) as the
 * bridge sends it (`hello.dream`, `dream_status` pushes, `dream_detail`
 * replies — docs/m5/PROTOCOL.md §9), plus the pure pieces of the Dream sheet.
 */

import { createSignal } from 'solid-js';

/** The Dream sheet's open state — module-level so the desktop palette can open it too. */
export const [dreamSheetOpen, setDreamSheetOpen] = createSignal(false);

export type DreamRunStatus = 'running' | 'finished' | 'failed' | 'stopped';
export type DreamDecision = 'pending' | 'approved' | 'rejected';

export interface DreamProposal {
  id: string;
  name: string;
  kind: 'skill' | 'check';
  decision: DreamDecision;
}

export interface DreamRun {
  id: string;
  status: DreamRunStatus;
  startedAt: number;
  endedAt?: number;
  manual: boolean;
  sessions: number;
  events: number;
  pending: number;
  hasReport: boolean;
  proposals: DreamProposal[];
  summary?: string[];
  error?: string;
}

export interface DreamStatus {
  v: 1;
  enabled: boolean;
  due?: { due: boolean; reason: string };
  minSessions: number;
  minGapHours: number;
  lastRunAt: number;
  running: boolean;
  runs: DreamRun[];
}

export interface DreamDetail {
  id: string;
  report: string;
  steps: Array<{ kind: string; text: string }>;
  log: string[];
}

export type DreamAction = 'run' | 'stop' | 'approve' | 'reject' | 'dismiss';

export function isDreamStatus(v: unknown): v is DreamStatus {
  return !!v && typeof v === 'object' && (v as DreamStatus).v === 1 && Array.isArray((v as DreamStatus).runs) && typeof (v as DreamStatus).enabled === 'boolean';
}

export const RUN_LABEL: Record<DreamRunStatus, string> = { running: 'Running', finished: 'Finished', failed: 'Failed', stopped: 'Stopped' };
export const DECISION_LABEL: Record<DreamDecision, string> = { pending: 'Waiting', approved: 'Approved', rejected: 'Rejected' };

/** `42s`, `3m 05s`, `1h 12m`. */
export function durationText(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
}

/** "Today 14:02", "Yesterday 09:10", "Oct 3, 14:02". */
export function startedText(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  const today = new Date(now);
  const y = new Date(now - 86_400_000);
  if (d.toDateString() === today.toDateString()) return `Today ${time}`;
  if (d.toDateString() === y.toDateString()) return `Yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}, ${time}`;
}

/** One-line run facts: `6 sessions · 2 waiting · 3m 05s`. */
export function runFacts(run: DreamRun, now = Date.now()): string {
  const parts = [`${run.sessions} session${run.sessions === 1 ? '' : 's'}`];
  if (run.pending > 0) parts.push(`${run.pending} waiting`);
  else if (run.proposals.length > 0) parts.push(`${run.proposals.length} proposal${run.proposals.length === 1 ? '' : 's'}`);
  parts.push(durationText((run.endedAt ?? now) - run.startedAt));
  return parts.join(' · ');
}

/** Pending proposals across every run (the ⋯ menu badge). */
export function pendingCount(status: DreamStatus | undefined): number {
  return status?.runs.reduce((n, r) => n + r.pending, 0) ?? 0;
}

/** Headline for the status card. */
export function statusLine(status: DreamStatus, now = Date.now()): string {
  if (status.running) return 'A dream is running in the background';
  if (!status.enabled) return 'Background dreaming is off';
  if (status.due?.due) return 'Due — starts the next time pi opens';
  const last = status.lastRunAt ? `Last dream ${startedText(status.lastRunAt, now).toLowerCase()}` : 'No dream yet';
  return status.due ? `${last} · ${status.due.reason}` : last;
}
