/**
 * Background tasks as Running-tab rows ([owner-fix-3/running-pane]): the background-tasks
 * extension's sticky topic `unipi:bg:tasks` (`{tasks: [BgTaskSnapshot]}`, structured) becomes the
 * same row shape the Subagents list uses, so both sections share one component + the filter chips.
 * Pure helpers (no Solid).
 */
import type { RunRowStatus } from './subagentRows';

export const BG_EXT = 'unipi.background-tasks';
/** The extension's work-tray tab id (its actions are `bg:<verb>[:<id>]`). */
export const BG_TAB = 'bg';

export type BgTaskStatus = 'running' | 'completed' | 'failed' | 'killed';

export type BgRow = {
  key: string;
  id: string;
  title: string;
  command: string;
  status: RunRowStatus;
  startedAt?: number;
  endedAt?: number;
  exitCode?: number | null;
  bytes: number;
  /** A terminal notification wakes the agent. */
  wakes: boolean;
  isAgent: boolean;
  error?: string;
  /** Last output lines the snapshot carries (the fallback when the output service is missing). */
  tail: string[];
};

const obj = (v: unknown): Record<string, unknown> | undefined => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined);
const str = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

const STATUS: Record<string, RunRowStatus> = { running: 'running', completed: 'done', done: 'done', failed: 'failed', error: 'failed', killed: 'cancelled', stopped: 'cancelled' };

/** The topic's tasks, newest first; undefined = the extension has not published the topic. */
export function parseBgTasks(data: unknown): BgRow[] | undefined {
  const list = obj(data)?.tasks;
  if (!Array.isArray(list)) return undefined;
  const rows: BgRow[] = [];
  for (const raw of list) {
    const t = obj(raw);
    const id = str(t?.id);
    if (!t || !id) continue;
    const command = str(t.command) ?? '';
    const name = str(t.name);
    const exit = t.exitCode;
    rows.push({
      key: `bg:${id}`,
      id,
      title: name ?? command ?? id,
      command,
      status: STATUS[String(t.status)] ?? 'running',
      ...(num(t.startTime) !== undefined ? { startedAt: num(t.startTime)! } : {}),
      ...(num(t.endTime) !== undefined ? { endedAt: num(t.endTime)! } : {}),
      ...(exit === null || typeof exit === 'number' ? { exitCode: exit as number | null } : {}),
      bytes: num(t.bytesWritten) ?? 0,
      wakes: t.notifyOnCompletion !== false && t.triggerOnCompletion === true,
      isAgent: t.isAgent === true,
      ...(str(t.error) ? { error: str(t.error)! } : {}),
      tail: Array.isArray(t.outputTail) ? t.outputTail.filter((l): l is string => typeof l === 'string') : [],
    });
  }
  return rows.sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0));
}

/** `845 B` / `1.2 KB` / `3.4 MB`. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1).replace(/\.0$/, '')} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Terminal bytes of a poll: `\n` → `\r\n` so a read-only xterm lays the lines out. */
export const toTermText = (text: string): string => text.replace(/\r?\n/g, '\r\n');

export const bgExitText = (r: Pick<BgRow, 'status' | 'exitCode'>): string | undefined =>
  r.status === 'running' ? undefined : r.exitCode === undefined || r.exitCode === null ? (r.status === 'cancelled' ? 'stopped' : undefined) : `exit ${r.exitCode}`;
