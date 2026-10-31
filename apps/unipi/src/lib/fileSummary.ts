/**
 * `unipi.file-summary` seam (WP-A27, app spec §3.13, D-1): the `turn` / `diff` /
 * `undo` / `undoFrom` services the extension serves over
 * `_oino/service_call{target:"unipi.file-summary"}` (`extensions/unipi/
 * file-summary/src/services.rs`), the per-turn summary cache the native footer
 * and A7's "Also undo file changes" both read, and the pure helpers (statuses,
 * `N files +X −Y`) they render with.
 *
 * Wire shapes are the extension's (`FileRecord` is serde camelCase), narrowed
 * defensively: the footer re-asks every 500 ms while a turn runs, so the hot
 * path never validates — a malformed reply just renders nothing.
 */
import { createSignal } from 'solid-js';
import type { UndoFiles } from '../components/timeline/context';
import type { SessionHandle } from './session/store';

/** The extension's id (the `_oino/service_call` target and the owner gate). */
export const FILE_SUMMARY_EXT = 'unipi.file-summary';

/** The reserved id of the native turn footer (§8; the manifest declares it). */
export const TURN_FILES_ENTRY = 'unipi.file-summary/turn-files';

/** A running turn's footer is re-asked at most this often (§3.13, §6.7). */
export const TURN_FILES_REFRESH_MS = 500;

/** Why a path has no snapshot (`skipped`): listed, never undone. */
export type SkipReason = 'too_large' | 'unreadable' | (string & {});

/** One touched path of a turn (`store::FileRecord`). */
export type TurnFileRecord = {
  /** Display path: project-relative inside the session cwd, absolute otherwise. */
  path: string;
  /** The file existed before the turn (`false` ⇒ the turn created it). */
  existed: boolean;
  beforeSha?: string;
  /** sha256 after the turn; absent ⇒ deleted (`or` never comparable). */
  afterSha?: string;
  add: number;
  del: number;
  /** `too_large` / `unreadable`: listed with no diff and no undo. */
  skipped?: SkipReason;
};

/** What `turn{turnEntryId}` answers. */
export type TurnSummary = {
  turnEntryId: string;
  files: TurnFileRecord[];
  /** Some path of the turn was put back by an undo. */
  undone: boolean;
  undonePaths: string[];
  /** The summary comes from the kv in-progress index (the turn is still running). */
  running: boolean;
};

/** What `diff{turnEntryId, path}` answers. `diff: null` + `reason` = nothing to show. */
export type TurnFileDiff = {
  turnEntryId: string;
  path: string;
  diff: string | null;
  truncated: boolean;
  /** `not_found` / `no_snapshot` / `unresolvable` / `unreadable` / a payload error. */
  reason?: string;
};

/** What `undo{turnEntryId, paths?, force?}` answers. */
export type FileUndoReply = {
  restored: string[];
  deleted: string[];
  /** Files changed since the turn ended: they need `force` (§3.13). */
  conflicts: string[];
  failed: string[];
  /** `false` = the turn is not in the session file (nothing was tried). */
  found: boolean;
};

/** What `undoFrom{turnEntryId, force?}` answers. */
export type FileUndoFromReply = FileUndoReply & { turns: string[] };

/** A turn's file was changed since it ended; the caller must confirm `force`. */
export class FileUndoConflict extends Error {
  readonly paths: string[];
  constructor(paths: string[]) {
    const n = paths.length;
    super(`${n} file${n === 1 ? ' was' : 's were'} changed since that turn — undo them from the turn's file list to override`);
    this.name = 'FileUndoConflict';
    this.paths = paths;
  }
}

/* ── wire ─────────────────────────────────────────────────────────── */

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : 0);
const list = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);

/** `_oino/service_call{target:"unipi.file-summary", name, payload}`. */
function call<T>(s: SessionHandle, name: string, payload: Record<string, unknown>): Promise<T> {
  return s.cmd<T>({ type: '_oino/service_call', target: FILE_SUMMARY_EXT, name, payload });
}

const recordOf = (v: unknown): TurnFileRecord | undefined => {
  if (!v || typeof v !== 'object') return undefined;
  const o = v as Record<string, unknown>;
  const path = str(o.path);
  if (!path) return undefined;
  return {
    path,
    existed: o.existed === true,
    beforeSha: str(o.beforeSha),
    afterSha: str(o.afterSha),
    add: num(o.add),
    del: num(o.del),
    skipped: str(o.skipped),
  };
};

/** `turn{turnEntryId}` — one turn's summary (running turns come from the kv index). */
export async function fetchTurnSummary(s: SessionHandle, turnEntryId: string): Promise<TurnSummary> {
  const reply = await call<unknown>(s, 'turn', { turnEntryId });
  const o = (reply && typeof reply === 'object' ? reply : {}) as Record<string, unknown>;
  return {
    turnEntryId: str(o.turnEntryId) ?? turnEntryId,
    files: (Array.isArray(o.files) ? o.files : []).map(recordOf).filter((f): f is TurnFileRecord => !!f),
    undone: o.undone === true,
    undonePaths: list(o.undonePaths),
    running: o.running === true,
  };
}

/** `diff{turnEntryId, path}` — the unified diff before the turn → now (≤ 1 MiB). */
export async function fetchTurnFileDiff(s: SessionHandle, turnEntryId: string, path: string): Promise<TurnFileDiff> {
  const reply = await call<unknown>(s, 'diff', { turnEntryId, path });
  const o = (reply && typeof reply === 'object' ? reply : {}) as Record<string, unknown>;
  return {
    turnEntryId: str(o.turnEntryId) ?? turnEntryId,
    path: str(o.path) ?? path,
    diff: str(o.diff) ?? null,
    truncated: o.truncated === true,
    reason: str(o.reason),
  };
}

const undoReplyOf = (reply: unknown): FileUndoReply => {
  const o = (reply && typeof reply === 'object' ? reply : {}) as Record<string, unknown>;
  return {
    restored: list(o.restored),
    deleted: list(o.deleted),
    conflicts: list(o.conflicts),
    failed: list(o.failed),
    found: o.found === true,
  };
};

/** `undo{turnEntryId, paths?, force?}` — restore/delete; conflicts need `force`. */
export async function undoFiles(
  s: SessionHandle,
  turnEntryId: string,
  opts: { paths?: string[]; force?: boolean } = {},
): Promise<FileUndoReply> {
  const reply = await call<unknown>(s, 'undo', {
    turnEntryId,
    ...(opts.paths ? { paths: opts.paths } : {}),
    ...(opts.force ? { force: true } : {}),
  });
  return undoReplyOf(reply);
}

/** `undoFrom{turnEntryId, force?}` — this and every later turn, newest first (Z-28). */
export async function undoFilesFrom(s: SessionHandle, turnEntryId: string, force = false): Promise<FileUndoFromReply> {
  const reply = await call<unknown>(s, 'undoFrom', { turnEntryId, ...(force ? { force: true } : {}) });
  return { ...undoReplyOf(reply), turns: list((reply as { turns?: unknown } | undefined)?.turns) };
}

/* ── the summary cache (one live copy per (session, turn)) ─────────── */

const [summaries, setSummaries] = createSignal<Record<string, TurnSummary | undefined>>({});
const keyOf = (sessionKey: string, turnId: string): string => `${sessionKey}|${turnId}`;

/** The last summary the footer (or a refresh) read for this turn. */
export function turnSummary(sessionKey: string, turnId: string): TurnSummary | undefined {
  return summaries()[keyOf(sessionKey, turnId)];
}

/** Publishes a summary (the footer's fetch, a test, an undo's refresh). */
export function setTurnSummary(sessionKey: string, turnId: string, summary: TurnSummary | undefined): void {
  setSummaries((cur) => ({ ...cur, [keyOf(sessionKey, turnId)]: summary }));
}

/** Reads the service and publishes the reply. */
export async function refreshTurnSummary(s: SessionHandle, turnId: string): Promise<TurnSummary> {
  const summary = await fetchTurnSummary(s, turnId);
  setTurnSummary(s.key, turnId, summary);
  return summary;
}

/** Re-reads the turns an undo touched (they are all in this session). */
export async function refreshSummaries(s: SessionHandle, turnIds: readonly string[]): Promise<void> {
  await Promise.all(
    turnIds.map((turnId) =>
      fetchTurnSummary(s, turnId).then(
        (summary) => setTurnSummary(s.key, turnId, summary),
        () => undefined,
      ),
    ),
  );
}

/** Test seam: forget every cached summary. */
export function resetFileSummaries(): void {
  setSummaries({});
}

/* ── pure helpers (the footer and the edit menu share them) ────────── */

export type FileStatus = 'new' | 'modified' | 'deleted' | 'skipped' | 'undone';

/** The row status, in the extension's order (`footer::status`). */
export function statusOf(file: TurnFileRecord, undone: boolean): FileStatus {
  if (file.skipped) return 'skipped';
  if (undone) return 'undone';
  if (!file.existed) return 'new';
  if (!file.afterSha) return 'deleted';
  return 'modified';
}

/** Is this path undone in the turn's summary? */
export function isUndone(summary: TurnSummary | undefined, path: string): boolean {
  return !!summary?.undonePaths.includes(path);
}

/** Can any of the turn's files be undone? (`skipped` files and done paths never.) */
export function canUndoSummary(summary: TurnSummary | undefined): boolean {
  return !!summary?.files.some((file) => !file.skipped && !isUndone(summary, file.path));
}

/** `true` when the turn has files the edit menu could undo (A7's Z-28 option). */
export function canUndoFiles(sessionKey: string, turnId: string): boolean {
  return canUndoSummary(turnSummary(sessionKey, turnId));
}

/** The header's totals: file count, added and removed lines (skipped files count 0). */
export function totalsOf(files: readonly TurnFileRecord[]): { count: number; add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const file of files) {
    add += file.add;
    del += file.del;
  }
  return { count: files.length, add, del };
}

/** `3 files` / `1 file`. */
export function fileCountLabel(count: number): string {
  return `${count} file${count === 1 ? '' : 's'}`;
}

/** `+12 −04` — the extension zero-pads to two digits (`footer.rs`). */
export function statLabel(add: number, del: number, pad = true): string {
  const n = (v: number) => (pad ? String(v).padStart(2, '0') : String(v));
  return `+${n(add)} −${n(del)}`;
}

/** A row's own counts: `+2 −0`. */
export function rowStatLabel(file: TurnFileRecord): string {
  return statLabel(file.add, file.del, false);
}

/**
 * A7's "Also undo file changes from this turn on" (Z-28): the capability the
 * timeline context carries once `unipi.file-summary` is enabled. `canUndo`
 * reads the cache the footer fills; a turn without a summary stays hidden
 * rather than offering a destructive action it cannot honour.
 *
 * `undoFrom` never forces: a conflict rejects with {@link FileUndoConflict} so
 * the caller keeps the files (and its own prompt) instead of discarding newer
 * work silently.
 */
export function fileSummaryCapability(s: SessionHandle): UndoFiles {
  return {
    canUndo: (turnId: string) => canUndoFiles(s.key, turnId),
    undoFrom: async (turnId: string) => {
      const reply = await undoFilesFrom(s, turnId);
      if (reply.conflicts.length) throw new FileUndoConflict(reply.conflicts);
      await refreshSummaries(s, reply.turns.length ? reply.turns : [turnId]);
    },
  };
}
