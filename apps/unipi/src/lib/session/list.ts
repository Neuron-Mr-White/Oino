/**
 * Session list — WP-A1 placeholder for the v1 herdr/tmux chat list: the
 * host's session cards (`sessions_list`, host.md §7.2) as `ChatTarget` rows,
 * grouped by project folder. Pure helpers + a small per-host cache.
 * WP-A2/A4 replace it with the hub store (`lib/session/hub.ts`).
 */
import { createRoot } from 'solid-js';
import { createStore } from 'solid-js/store';
import type { SessionCard } from '../host/client';
import { chatTransport, type AgentStatus, type SessionListData } from './link';
import { sessionTitle } from './selectors';

export type { AgentStatus } from './link';

/** The session's "needs you" mark (an open dialog, or idle after an
 *  agent_end nobody has seen yet). */
export type ChatWaiting = { kind: string; title?: string; since: number };

/** One session row (and what the chat layer opens). */
export type ChatTarget = {
  hostId: string;
  /** The oino session id. */
  session: string;
  title: string;
  /** Display folder (`~/…`). */
  cwd?: string;
  /** The real folder (`cwd` is shortened for display). */
  cwdFull?: string;
  status: AgentStatus;
  waiting?: ChatWaiting;
  /** Wake-capable work still pending while idle (bg task, background
   *  subagent): the row reads "Working…" with this as detail. */
  pendingWork?: string;
  /** The long-horizon working mode (goal/ralph/swarm/graph) for the row's badge. */
  lhMode?: string;
  /** A parked owner's mode ("Graph · paused"). */
  lhPaused?: string;
};

/** Sessions of one project folder. */
export type ChatGroup = { key: string; label: string; cwd?: string; status: AgentStatus; items: ChatTarget[] };

const short = (cwd?: string) => (cwd ? cwd.replace(/^\/home\/[^/]+/, '~') : undefined);
const folderOf = (cwd?: string) => (cwd ? cwd.replace(/\/+$/, '').split('/').pop() || cwd : 'No folder');

const optStr = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);

/** A host session card (host.md §4.2) → a row. `needs_you` → a dialog wait since the last activity. */
export function cardTarget(hostId: string, card: SessionCard): ChatTarget {
  const pendingWork = optStr(card.pending_work);
  const lhMode = optStr(card.lh_mode);
  const lhPaused = optStr(card.lh_paused);
  const waiting: ChatWaiting | undefined = card.needs_you ? { kind: 'dialog', since: card.updated_at ?? 0 } : undefined;
  return {
    hostId,
    session: card.session_id,
    // Same name the sidebar shows (a chat in the bare home folder is "New chat", not the folder).
    title: sessionTitle(card),
    cwd: short(card.cwd),
    cwdFull: card.cwd,
    status: card.run_state === 'working' ? 'working' : card.run_state === 'idle' || card.run_state === 'taken_over' ? 'idle' : 'unknown',
    waiting: effectiveWaiting(waiting, pendingWork),
    ...(pendingWork ? { pendingWork } : {}),
    ...(lhMode ? { lhMode } : {}),
    ...(lhPaused ? { lhPaused } : {}),
  };
}

/** A bare row for a session known only by id (notification / needs-you deep links);
 *  `updateOpenTarget` fills in the rest once the list loads. */
export function bareTarget(hostId: string, session: string, title: string): ChatTarget {
  return { hostId, session, title, status: 'unknown' };
}

/** Groups a host's sessions by project folder (host order kept inside a group). */
export function chatGroups(hostId: string, data: SessionListData | undefined): ChatGroup[] {
  const out: ChatGroup[] = [];
  const byKey = new Map<string, ChatGroup>();
  for (const card of data?.sessions ?? []) {
    const key = card.cwd ?? '';
    let group = byKey.get(key);
    if (!group) {
      group = { key, label: folderOf(card.cwd), cwd: card.cwd, status: 'unknown', items: [] };
      byKey.set(key, group);
      out.push(group);
    }
    group.items.push(cardTarget(hostId, card));
  }
  for (const group of out) group.status = groupStatus(group.items);
  return out;
}

function groupStatus(items: ChatTarget[]): AgentStatus {
  if (items.some((t) => t.status === 'blocked')) return 'blocked';
  if (items.some((t) => t.status === 'working')) return 'working';
  return items[0]?.status ?? 'unknown';
}

/** A bare "run finished" mark (agent_end/agent_settled) is not a finish
 *  while wake-capable work is still pending — that work will re-wake the
 *  agent; only the final settle counts. Dialogs (ask_user, …) still need you. */
export function effectiveWaiting(waiting: ChatWaiting | undefined, pendingWork: string | undefined): ChatWaiting | undefined {
  if (!waiting) return undefined;
  if (pendingWork && (waiting.kind === 'agent_end' || waiting.kind === 'agent_settled')) return undefined;
  return waiting;
}

/** The one plain working wording the app shares with the TUI. */
export const WORKING_WORD = 'Working…';

type LiveOpen = { hostId: string; session: string; running: boolean; waiting?: string; lhMode?: { mode: string; paused?: string } };

/** The status the list shows for a row: the open (live) session's run state
 * upgrades a stale "Idle" to "Working". Never downgrades. */
export function liveStatus(target: Pick<ChatTarget, 'status' | 'hostId' | 'session'>, open: LiveOpen | undefined): AgentStatus {
  if (!open || open.hostId !== target.hostId || open.session !== target.session) return target.status;
  if (target.status === 'blocked') return target.status;
  return open.running || open.waiting ? 'working' : target.status;
}

/** A row merged with the open (live) session's run state. */
export function withLive<T extends ChatTarget>(target: T, open: LiveOpen | undefined): T {
  const status = liveStatus(target, open);
  if (!open || open.hostId !== target.hostId || open.session !== target.session) return { ...target, status };
  const pendingWork = open.running ? undefined : open.waiting;
  const out = { ...target, status, pendingWork, waiting: effectiveWaiting(target.waiting, pendingWork) };
  if (open.lhMode) {
    const mode = open.lhMode.mode && open.lhMode.mode !== 'none' ? open.lhMode.mode : undefined;
    const paused = open.lhMode.paused && open.lhMode.paused !== 'none' && open.lhMode.paused !== mode ? open.lhMode.paused : undefined;
    delete out.lhMode;
    delete out.lhPaused;
    if (mode) out.lhMode = mode;
    if (paused) out.lhPaused = paused;
  }
  return out;
}

/** "Needs you" first (UNI-161): waiting items (open dialog, or idle after an
 *  agent_end not yet seen) float to the top, oldest-waiting first; everything
 *  else keeps its original (host) order after that. Pure — callers re-sort
 *  whenever the list or its waiting flags change. */
export function needsYouFirst<T extends { waiting?: ChatWaiting }>(items: readonly T[]): T[] {
  const waiting = items.filter((x) => x.waiting);
  const rest = items.filter((x) => !x.waiting);
  waiting.sort((a, b) => (a.waiting!.since ?? 0) - (b.waiting!.since ?? 0));
  return [...waiting, ...rest];
}

/** One "needs you" row (bell drawer / lib/needsYou.ts) per waiting session in a
 *  host's session list — pure, so the bell and tests share it. */
export function waitingTargets(hostId: string, data: SessionListData | undefined): Array<ChatTarget & { waiting: ChatWaiting }> {
  const out: Array<ChatTarget & { waiting: ChatWaiting }> = [];
  for (const group of chatGroups(hostId, data)) {
    for (const item of group.items) if (item.waiting) out.push(item as ChatTarget & { waiting: ChatWaiting });
  }
  return out;
}

export const STATUS_WORD: Record<AgentStatus, string> = {
  working: 'Working',
  blocked: 'Needs you',
  done: 'Done',
  idle: 'Idle',
  unknown: 'Unknown',
};

type Entry = { data?: SessionListData; error?: string; loading: boolean; at?: number };

function createChatLists() {
  const [lists, setLists] = createStore<Record<string, Entry>>({});
  const inflight = new Map<string, Promise<void>>();
  function load(hostId: string): Promise<void> {
    const running = inflight.get(hostId);
    if (running) return running;
    setLists(hostId, (e) => ({ ...(e ?? {}), loading: true }));
    const p = chatTransport()
      .list(hostId)
      .then((data) => setLists(hostId, { data, error: undefined, loading: false, at: Date.now() }))
      .catch((error: unknown) =>
        setLists(hostId, (e) => ({ ...(e ?? {}), error: String((error as { message?: string })?.message ?? error), loading: false })),
      )
      .finally(() => inflight.delete(hostId));
    inflight.set(hostId, p);
    return p;
  }
  return { get: (hostId: string): Entry | undefined => lists[hostId], load };
}

let shared: ReturnType<typeof createChatLists> | undefined;
export function chatLists() {
  if (!shared) shared = createRoot(() => createChatLists());
  return shared;
}

const LAST = 'unipi.chat.last';
export function readLastChatHost(): string | undefined {
  try {
    return localStorage.getItem(LAST) ?? undefined;
  } catch {
    return undefined;
  }
}
export function rememberChatHost(hostId: string): void {
  try {
    localStorage.setItem(LAST, hostId);
  } catch {
    // ignore
  }
}
