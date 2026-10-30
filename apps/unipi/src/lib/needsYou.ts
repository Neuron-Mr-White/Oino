/**
 * "Needs you" bell + drawer (UNI-161): aggregates every paired host's chat
 * list (its sessions' `waiting` state — see lib/session/list.ts ChatWaiting) into
 * one flat feed across the whole app, grouped Waiting / Finished / Other for
 * the drawer, with a bell badge. Read/dismissed state persists locally
 * (localStorage) so it survives app restarts; it's keyed by host+pid+kind so
 * the same pi waiting on a second question after the first was read shows up
 * again (the `since` timestamp changes).
 *
 * Sources feeding `pushNeedsYouItems` (not this module's job to poll):
 * - live session-list `waiting` state (UNI-161 §2; hub events after WP-A19), read by whoever owns the
 *   PC picker/Chat screen already (reuses that poll, no new one here);
 * - live bridge `dialog`/`dialog_end` events from any chat the app has open
 *   right now (ChatView forwards them);
 * - ntfy history (UNI-161 §4), fed in by lib/notify's poll.
 */
import { createRoot, createSignal } from 'solid-js';
import { createStore, produce } from 'solid-js/store';

export type NeedsYouGroup = 'waiting' | 'finished' | 'other';

export type NeedsYouItem = {
  /** Stable across updates of the SAME wait (host+pid+kind+since): used as the dedup/read key. */
  id: string;
  hostId: string;
  hostName: string;
  pid: number | null;
  session: string;
  title: string;
  /** What's waiting: the dialog kind, or 'agent_end'/'agent_settled' for a finished run, or 'other'. */
  kind: string;
  group: NeedsYouGroup;
  /** The question/need text shown under the title, when there is one. */
  what?: string;
  since: number;
};

const groupFor = (kind: string): NeedsYouGroup => {
  if (kind === 'agent_end' || kind === 'agent_settled' || kind === 'workflow_end') return 'finished';
  if (kind === 'select' || kind === 'confirm' || kind === 'input' || kind === 'editor' || kind === 'ask_user' || kind === 'custom' || kind === 'permission_request' || kind === 'input_needed')
    return 'waiting';
  return 'other';
};

const STORE_KEY = 'unipi.needsYou.v1';

type Persisted = { readIds: string[]; clearedIds: string[] };

function loadPersisted(): Persisted {
  try {
    const raw = localStorage.getItem(STORE_KEY);
    const parsed = raw ? (JSON.parse(raw) as Partial<Persisted>) : {};
    return { readIds: Array.isArray(parsed.readIds) ? parsed.readIds : [], clearedIds: Array.isArray(parsed.clearedIds) ? parsed.clearedIds : [] };
  } catch {
    return { readIds: [], clearedIds: [] };
  }
}

function savePersisted(p: Persisted): void {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(p));
  } catch {
    // storage full/unavailable: read/dismissed state just won't survive a restart.
  }
}

function createNeedsYou() {
  const [items, setItems] = createStore<Record<string, NeedsYouItem>>({});
  const persisted = loadPersisted();
  const readIds = new Set(persisted.readIds);
  const clearedIds = new Set(persisted.clearedIds);

  // readIds is a plain Set: this signal makes isRead/unreadCount reactive (UNI-200: the
  // badge and the crab didn't update on Mark all read / opening an item until the next poll).
  const [readVersion, setReadVersion] = createSignal(0);
  const persist = () => {
    savePersisted({ readIds: [...readIds], clearedIds: [...clearedIds] });
    setReadVersion((n) => n + 1);
  };

  /** Replaces every item from one host with a fresh snapshot (poll-and-diff: a
   *  host whose list no longer carries an id simply drops it — the waiting
   *  state ended or the session's gone). Cleared ids are skipped; a NEW
   *  since for an id the user cleared un-clears it (a fresh wait). */
  function setHostItems(hostId: string, fresh: readonly NeedsYouItem[]): void {
    setItems(
      produce((draft) => {
        for (const id of Object.keys(draft)) {
          if (draft[id]!.hostId === hostId && !fresh.some((f) => f.id === id)) delete draft[id];
        }
        for (const item of fresh) {
          if (clearedIds.has(item.id)) continue;
          draft[item.id] = item;
        }
      }),
    );
  }

  /** One item from a live bridge event (dialog opened in a chat the app has
   *  open right now) or an ntfy history entry. */
  function upsert(item: NeedsYouItem): void {
    if (clearedIds.has(item.id)) return;
    setItems(item.id, item);
  }

  function remove(id: string): void {
    setItems(produce((draft) => void delete draft[id]));
  }

  function list(): NeedsYouItem[] {
    return Object.values(items).sort((a, b) => b.since - a.since);
  }

  function isRead(id: string): boolean {
    readVersion();
    return readIds.has(id);
  }

  function markRead(id: string): void {
    if (readIds.has(id)) return;
    readIds.add(id);
    persist();
  }

  function markAllRead(): void {
    for (const item of list()) readIds.add(item.id);
    persist();
  }

  /** Dismiss one (swipe/✕): gone from the drawer until a fresh wait (new `since`) replaces it. */
  function dismiss(id: string): void {
    clearedIds.add(id);
    readIds.delete(id);
    remove(id);
    persist();
  }

  /** Clear everything currently shown. */
  function clearAll(): void {
    for (const item of list()) clearedIds.add(item.id);
    setItems(produce((draft) => {
      for (const id of Object.keys(draft)) delete draft[id];
    }));
    persist();
  }

  function unreadCount(): number {
    readVersion();
    return list().filter((x) => !readIds.has(x.id)).length;
  }

  return { list, setHostItems, upsert, remove, isRead, markRead, markAllRead, dismiss, clearAll, unreadCount };
}

let shared: ReturnType<typeof createNeedsYou> | undefined;
/** The one shared "needs you" store (bell badge + drawer), app-wide. */
export function needsYou() {
  if (!shared) shared = createRoot(() => createNeedsYou());
  return shared;
}

/** A stable item id: one wait, across polls, until it ends or a fresh one replaces it. */
export function needsYouId(hostId: string, pid: number | null, kind: string, since: number): string {
  return `${hostId}:${pid ?? '-'}:${kind}:${since}`;
}

/** Groups items for the drawer (Waiting / Finished / Other), each newest-first. */
export function groupNeedsYou(items: readonly NeedsYouItem[]): Record<NeedsYouGroup, NeedsYouItem[]> {
  const out: Record<NeedsYouGroup, NeedsYouItem[]> = { waiting: [], finished: [], other: [] };
  for (const item of items) out[item.group].push(item);
  for (const key of Object.keys(out) as NeedsYouGroup[]) out[key].sort((a, b) => b.since - a.since);
  return out;
}

export { groupFor };
