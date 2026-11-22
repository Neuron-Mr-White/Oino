/**
 * What the Superagent Workbench was showing, kept per host so it survives leaving the page and
 * coming back (route change, component unmount) and a reload: the open section, project filter,
 * search text, the ticket on show and the queue order it was opened from, the pager positions,
 * the side pane, the scroll positions, and every half-typed reply.
 *
 * Held in memory and mirrored to sessionStorage (same pattern as lib/chat/askDraft.ts and
 * components/draft/draftStored.ts). Replies live apart from the view state because they change on
 * every keystroke while the view carries the (large) pane text.
 */
import type { PaneImage, ReaderEntry } from '../board/ui';

export type WorkbenchFilter = 'approve' | 'review' | 'escalate';
export type TaskRef = { project: string; task: string };
/** How the Review queue is ordered: the host's own order (default), priority, newest update, oldest waiting, or ticket id. */
export type WorkbenchSort = 'default' | 'priority' | 'updated' | 'waiting' | 'id';
export const WORKBENCH_SORTS: readonly WorkbenchSort[] = ['default', 'priority', 'updated', 'waiting', 'id'];
const PRIORITY_IDS = ['urgent', 'high', 'medium', 'low', 'none'];

export type WorkbenchView = {
  /** The section that was open; absent: the Workbench was closed. */
  open?: WorkbenchFilter;
  project?: string;
  search?: string;
  /** Review queue filters (priority ids, label names; empty = off) and order. Absent: all tickets, the host's order. */
  priorities?: string[];
  labels?: string[];
  sort?: WorkbenchSort;
  /** The ticket on show in Review, and the queue it was opened from (to find its neighbour when it is gone). */
  task?: TaskRef;
  queue?: TaskRef[];
  /** Pager positions of Approve and Escalate. */
  approveAt?: number;
  questionAt?: number;
  /** The side pane: the full report or a file (`reader`) or an image. */
  reader?: ReaderEntry;
  image?: PaneImage;
  /** Scroll offsets: the list, and each ticket by id. */
  listScroll?: number;
  ticketScroll?: Record<string, number>;
};

/** A pane text above this is not kept (sessionStorage is small); the report reopens with its ticket anyway. */
const MAX_PANE_TEXT = 200_000;

const viewMem = new Map<string, WorkbenchView>();
const draftMem = new Map<string, Record<string, string>>();
const viewKey = (host: string) => `unipi:superagent:workbench:${host}`;
const draftKey = (host: string) => `unipi:superagent:workbench:drafts:${host}`;
const store = (): Storage | undefined => {
  try {
    return typeof sessionStorage === 'undefined' ? undefined : sessionStorage;
  } catch {
    return undefined;
  }
};
const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const filterOf = (v: unknown): WorkbenchFilter | undefined => (v === 'approve' || v === 'review' || v === 'escalate' ? v : undefined);
const ref = (v: unknown): TaskRef | undefined => (isRecord(v) && typeof v.project === 'string' && typeof v.task === 'string' ? { project: v.project, task: v.task } : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : undefined);

/** A tolerant read of a stored view: wrong shapes are dropped, not fatal. */
function parseView(raw: unknown): WorkbenchView {
  if (!isRecord(raw)) return {};
  const out: WorkbenchView = {};
  const open = filterOf(raw.open);
  if (open) out.open = open;
  if (typeof raw.project === 'string' && raw.project) out.project = raw.project;
  if (typeof raw.search === 'string' && raw.search) out.search = raw.search;
  const strings = (v: unknown, allowed?: string[]) => (Array.isArray(v) ? [...new Set(v.filter((x): x is string => typeof x === 'string' && !!x && (!allowed || allowed.includes(x))))] : []);
  const priorities = strings(raw.priorities, PRIORITY_IDS);
  if (priorities.length) out.priorities = priorities;
  const labels = strings(raw.labels);
  if (labels.length) out.labels = labels;
  if (typeof raw.sort === 'string' && raw.sort !== 'default' && (WORKBENCH_SORTS as readonly string[]).includes(raw.sort)) out.sort = raw.sort as WorkbenchSort;
  const task = ref(raw.task);
  if (task) out.task = task;
  if (Array.isArray(raw.queue)) out.queue = raw.queue.map(ref).filter((r): r is TaskRef => !!r);
  const approveAt = num(raw.approveAt);
  if (approveAt) out.approveAt = approveAt;
  const questionAt = num(raw.questionAt);
  if (questionAt) out.questionAt = questionAt;
  if (isRecord(raw.reader) && typeof raw.reader.title === 'string' && typeof raw.reader.text === 'string') out.reader = raw.reader as ReaderEntry;
  if (isRecord(raw.image) && Array.isArray(raw.image.items) && typeof raw.image.index === 'number') out.image = raw.image as PaneImage;
  const listScroll = num(raw.listScroll);
  if (listScroll) out.listScroll = listScroll;
  if (isRecord(raw.ticketScroll)) {
    const entries = Object.entries(raw.ticketScroll).filter((e): e is [string, number] => num(e[1]) !== undefined);
    if (entries.length) out.ticketScroll = Object.fromEntries(entries);
  }
  return out;
}

export function readWorkbenchView(host: string): WorkbenchView {
  const hit = viewMem.get(host);
  if (hit) return hit;
  let view: WorkbenchView = {};
  try {
    const raw = store()?.getItem(viewKey(host));
    if (raw) view = parseView(JSON.parse(raw));
  } catch {
    /* unreadable: start empty */
  }
  viewMem.set(host, view);
  return view;
}

/** Merges `patch` into the host's view; an `undefined` value removes that field. */
export function patchWorkbenchView(host: string, patch: Partial<WorkbenchView>): WorkbenchView {
  const next: WorkbenchView = { ...readWorkbenchView(host), ...patch };
  for (const k of Object.keys(next) as (keyof WorkbenchView)[]) if (next[k] === undefined || next[k] === '' || next[k] === 0 || next[k] === 'default' || (Array.isArray(next[k]) && k !== 'queue' && !(next[k] as unknown[]).length)) delete next[k];
  viewMem.set(host, next);
  try {
    const keep = next.reader && next.reader.text.length > MAX_PANE_TEXT ? { ...next, reader: undefined } : next;
    if (Object.keys(keep).length) store()?.setItem(viewKey(host), JSON.stringify(keep));
    else store()?.removeItem(viewKey(host));
  } catch {
    /* storage full or blocked: the memory copy still works */
  }
  return next;
}

export function readDrafts(host: string): Record<string, string> {
  const hit = draftMem.get(host);
  if (hit) return hit;
  let drafts: Record<string, string> = {};
  try {
    const raw = store()?.getItem(draftKey(host));
    const parsed: unknown = raw ? JSON.parse(raw) : undefined;
    if (isRecord(parsed)) drafts = Object.fromEntries(Object.entries(parsed).filter((e): e is [string, string] => typeof e[1] === 'string' && e[1].trim() !== ''));
  } catch {
    /* unreadable: start empty */
  }
  draftMem.set(host, drafts);
  return drafts;
}

export const readDraft = (host: string, key: string): string => readDrafts(host)[key] ?? '';

/** Keeps a reply under `key` (a ticket note `slug/ID`, or `answer:slug/ID`); blank text removes it, which is what sending does. */
export function writeDraft(host: string, key: string, text: string): void {
  const drafts = { ...readDrafts(host) };
  if (text.trim()) drafts[key] = text;
  else if (key in drafts) delete drafts[key];
  else return;
  draftMem.set(host, drafts);
  try {
    if (Object.keys(drafts).length) store()?.setItem(draftKey(host), JSON.stringify(drafts));
    else store()?.removeItem(draftKey(host));
  } catch {
    /* ignore */
  }
}

const same = (a: TaskRef, b: TaskRef) => a.project === b.project && a.task === b.task;

/**
 * The ticket to show on return. The one that was open when it is still listed; otherwise the
 * nearest one after it in the order it was opened from (then the nearest before it); with no
 * usable order, undefined (the caller opens the first row).
 */
export function resolveTicket(selected: TaskRef | undefined, queue: readonly TaskRef[] | undefined, live: readonly TaskRef[]): TaskRef | undefined {
  if (!selected) return undefined;
  const still = live.find((r) => same(r, selected));
  if (still) return still;
  const order = queue ?? [];
  const at = order.findIndex((r) => same(r, selected));
  if (at < 0) return undefined;
  for (let i = at + 1; i < order.length; i++) { const hit = live.find((r) => same(r, order[i]!)); if (hit) return hit; }
  for (let i = at - 1; i >= 0; i--) { const hit = live.find((r) => same(r, order[i]!)); if (hit) return hit; }
  return undefined;
}

/** Test hook (and sign-out): forget everything, memory and storage. */
export function resetWorkbenchState(): void {
  for (const host of [...viewMem.keys()]) store()?.removeItem(viewKey(host));
  for (const host of [...draftMem.keys()]) store()?.removeItem(draftKey(host));
  viewMem.clear();
  draftMem.clear();
  try {
    const s = store();
    if (s) for (let i = s.length - 1; i >= 0; i--) { const k = s.key(i); if (k?.startsWith('unipi:superagent:workbench:')) s.removeItem(k); }
  } catch {
    /* ignore */
  }
}
