/**
 * Live (background-capable) terminal sessions (DESIGN §3.3, UNI-128
 * follow-up). App-level, not component state: a terminal that leaves
 * the screen through the yellow "–" button, the knob drag-down or
 * Android Back stays connected — its xterm instance, scrollback and
 * PTY attach stream live on in a hidden TerminalScreen owned by the
 * app shell's terminal layer — so reopening it is instant.
 *
 * Only `closeLive` (the dashboard's "Close connection") or the cap
 * eviction ends an entry. Ending one unmounts its TerminalScreen,
 * which closes our attach stream (`term_close`): the host frees the
 * PTY *client* only. The herdr workspace / tmux session keeps running
 * on the PC; nothing here ever kills a real session.
 */
import { batch, createSignal } from 'solid-js';
import type { AttachTarget } from '@unipi/protocol';
import { recentKey } from './recents';

/** What a terminal attaches to (`target: null` = plain login shell). */
export type LiveTarget = {
  hostId: string;
  target: AttachTarget | null;
  label: string;
  sublabel: string;
};

export type LiveEntry = {
  /** `recentKey(hostId, target)`: the same key as the recents list. */
  readonly key: string;
  readonly target: LiveTarget;
};

export type LiveStatus = 'connecting' | 'live' | 'reconnecting' | 'ended';

/** At most this many live connections; opening one more closes the
 * least recently used background connection (detach only). */
export const MAX_LIVE = 4;

const [entries, setEntries] = createSignal<readonly LiveEntry[]>([]);
const [activeKey, setActiveKey] = createSignal<string | null>(null);
const [usedAt, setUsedAt] = createSignal<Record<string, number>>({});
const [statuses, setStatuses] = createSignal<Record<string, LiveStatus>>({});
const snapshots = new Map<string, () => string | undefined>();
let clock = 0;

/** Monotonic "last used" stamp (Date.now ties within one tick). */
function stamp(): number {
  clock = Math.max(clock + 1, Date.now());
  return clock;
}

export function liveKey(target: Pick<LiveTarget, 'hostId' | 'target'>): string {
  return recentKey(target.hostId, target.target);
}

/**
 * Makes `target` the active terminal: resumes its live entry if there
 * is one, otherwise adds a new entry (connects when its screen
 * mounts). Returns the keys evicted to stay within MAX_LIVE.
 */
export function openLive(target: LiveTarget): { key: string; evicted: string[] } {
  // One batch: observers must never see the new active key before
  // its entry exists (the layer would treat /term as target-less).
  return batch(() => openLiveNow(target));
}

function openLiveNow(target: LiveTarget): { key: string; evicted: string[] } {
  const key = liveKey(target);
  const list = entries();
  const used = { ...usedAt(), [key]: stamp() };
  setUsedAt(used);
  setActiveKey(key);
  if (list.some((entry) => entry.key === key)) return { key, evicted: [] };
  const next = [...list, { key, target: { ...target } }];
  const evicted: string[] = [];
  while (next.length > MAX_LIVE) {
    // Least recently used, never the one being opened.
    let victim = -1;
    for (let i = 0; i < next.length; i += 1) {
      if (next[i].key === key) continue;
      if (victim < 0 || (used[next[i].key] ?? 0) < (used[next[victim].key] ?? 0)) victim = i;
    }
    if (victim < 0) break;
    evicted.push(next[victim].key);
    next.splice(victim, 1);
  }
  setEntries(next);
  for (const gone of evicted) forget(gone);
  return { key, evicted };
}

/** Closes our connection to `key` (detach only; see file header). */
export function closeLive(key: string): void {
  if (!entries().some((entry) => entry.key === key)) return;
  batch(() => {
    if (activeKey() === key) setActiveKey(null);
    setEntries(entries().filter((entry) => entry.key !== key));
    forget(key);
  });
}

function forget(key: string): void {
  snapshots.delete(key);
  setStatuses((previous) => {
    const { [key]: _gone, ...rest } = previous;
    return rest;
  });
  setUsedAt((previous) => {
    const { [key]: _gone, ...rest } = previous;
    return rest;
  });
}

/** Clears the active terminal (the /term route then goes Home). */
export function clearActiveLive(): void {
  setActiveKey(null);
}

/** Entries in insertion order (stable: drives the terminal layer). */
export function liveEntries(): readonly LiveEntry[] {
  return entries();
}

/** Entries most recently used first (the dashboard's Ongoing group). */
export function liveByRecency(): LiveEntry[] {
  const used = usedAt();
  return [...entries()].sort((a, b) => (used[b.key] ?? 0) - (used[a.key] ?? 0));
}

export function activeLiveKey(): string | null {
  return activeKey();
}

export function activeLive(): LiveEntry | undefined {
  const key = activeKey();
  return key === null ? undefined : entries().find((entry) => entry.key === key);
}

export function isLive(key: string): boolean {
  return entries().some((entry) => entry.key === key);
}

export function setLiveStatus(key: string, status: LiveStatus): void {
  if (!isLive(key) || statuses()[key] === status) return;
  setStatuses((previous) => ({ ...previous, [key]: status }));
}

export function liveStatus(key: string): LiveStatus {
  return statuses()[key] ?? 'connecting';
}

/** A screen registers how to read its visible text (shell previews). */
export function registerLiveSnapshot(key: string, read: () => string | undefined): () => void {
  snapshots.set(key, read);
  return () => {
    if (snapshots.get(key) === read) snapshots.delete(key);
  };
}

export function liveSnapshot(key: string): string | undefined {
  return snapshots.get(key)?.();
}

/** Test hook. */
export function resetLiveSessions(): void {
  setEntries([]);
  setActiveKey(null);
  setUsedAt({});
  setStatuses({});
  snapshots.clear();
}
