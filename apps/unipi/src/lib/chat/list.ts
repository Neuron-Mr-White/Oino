/**
 * Chat list (UNI-113): every pi pane in a PC's herdr sessions, grouped
 * session → workspace, from `chat_list`. Pure helpers + a small cache.
 */
import { createRoot } from 'solid-js';
import { createStore } from 'solid-js/store';
import { chatTransport, type AgentStatus, type ChatListData, type ChatPane } from './transport';

/** The bridge's "needs you" mark (UNI-161): an open dialog, or idle right
 *  after an agent_end no phone has seen yet. */
export type ChatWaiting = { kind: string; title?: string; since: number };

export type ChatTarget = {
  hostId: string;
  session: string;
  workspaceId: string;
  workspaceLabel: string;
  tabId: string;
  paneId: string;
  /** null = pi without a reachable bridge (old unipi / not restarted). */
  pid: number | null;
  title: string;
  cwd?: string;
  /** The pane's real folder (`cwd` is shortened for display: `~/…`). */
  cwdFull?: string;
  status: AgentStatus;
  focused: boolean;
  waiting?: ChatWaiting;
  /** UNI-221: wake-capable work still pending while pi is idle (bg task
   *  with a wake, background subagent, fusion handoff) — the row reads
   *  "Working…" with this as detail, never idle/finished. */
  pendingWork?: string;
  /** UNI-260: where the pi runs (absent = herdr). For tmux, `workspaceId`
   *  is the window id and `paneId` the tmux pane id. */
  mux?: 'herdr' | 'tmux';
  /** UNI-269: the long-horizon working mode (goal/ralph/swarm/graph) for the row's badge. */
  lhMode?: string;
  /** UNI-269: a parked owner's mode ("Graph · paused"). */
  lhPaused?: string;
};

export type ChatGroup = { session: string; workspaceId: string; label: string; status: AgentStatus; items: ChatTarget[]; mux?: 'tmux' };

const short = (cwd?: string) => (cwd ? cwd.replace(/^\/home\/[^/]+/, '~') : undefined);

/** Groups pi panes by herdr session + workspace (non-pi panes dropped). */
export function chatGroups(hostId: string, data: ChatListData | undefined, protectedSessions: readonly string[] = []): ChatGroup[] {
  const out: ChatGroup[] = [];
  for (const session of data?.herdr.sessions ?? []) {
    if (!session.running || protectedSessions.includes(session.name)) continue;
    for (const ws of session.workspaces) {
      const items: ChatTarget[] = [];
      for (const tab of ws.tabs) {
        for (const pane of tab.panes) {
          if (pane.agent?.name !== 'pi' && !pane.chat) continue;
          items.push(target(hostId, session.name, ws.id, ws.label, tab.id, pane));
        }
      }
      if (items.length) out.push({ session: session.name, workspaceId: ws.id, label: ws.label, status: ws.agent_status, items });
    }
  }
  // UNI-260: pis in tmux (only those with a bridge; one group per session).
  for (const session of data?.tmux?.sessions ?? []) {
    if (protectedSessions.includes(session.name)) continue;
    const items: ChatTarget[] = [];
    for (const w of session.windows) {
      for (const pane of w.panes) {
        if (!pane.chat) continue;
        const name = pane.chat.session_name?.trim();
        items.push({
          hostId,
          session: session.name,
          workspaceId: w.id,
          workspaceLabel: session.name,
          tabId: '',
          paneId: pane.id,
          pid: pane.chat.pid,
          title: name || w.name || session.name,
          cwd: short(pane.chat.cwd ?? pane.cwd),
          cwdFull: pane.chat.cwd ?? pane.cwd ?? undefined,
          status: paneStatus(pane as unknown as ChatPane),
          focused: pane.active,
          waiting: effectiveWaiting(pane.chat.waiting, pane.chat.pending_work),
          ...(pane.chat.pending_work ? { pendingWork: pane.chat.pending_work } : {}),
          ...lhModeOf(pane.chat),
          mux: 'tmux',
        });
      }
    }
    if (items.length) out.push({ session: session.name, workspaceId: session.id, label: session.name, status: 'unknown', items, mux: 'tmux' });
  }
  return out;
}

/** A pane's agent status (UNI-212): herdr's own `agent_status` can miss a
 * run (a busy pi listed as "Idle"), so the bridge's `running` flag wins
 * when it says pi is working; "needs you" (blocked) still beats it. */
export function paneStatus(pane: ChatPane): AgentStatus {
  const herdr = pane.agent?.status ?? 'unknown';
  if (herdr === 'blocked') return herdr;
  if (pane.chat?.running) return 'working';
  // UNI-221: idle but wake-capable work is pending — still working.
  if (pane.chat?.pending_work) return 'working';
  return herdr;
}

/** UNI-221: a bare "run finished" mark (agent_end/agent_settled) is not a
 *  finish while wake-capable work is still pending — that work will re-wake
 *  pi; only the final settle counts. Dialogs (ask_user, …) still need you. */
export function effectiveWaiting(waiting: ChatWaiting | undefined, pendingWork: string | undefined): ChatWaiting | undefined {
  if (!waiting) return undefined;
  if (pendingWork && (waiting.kind === 'agent_end' || waiting.kind === 'agent_settled')) return undefined;
  return waiting;
}

/** UNI-221: the one plain working wording the app shares with the TUI. */
export const WORKING_WORD = 'Working…';

/** The status the list shows for a row: the open (live) chat's run state
 * — what its transcript shows — upgrades a stale polled "Idle" to
 * "Working". Never downgrades: herdr's "working" may also mean held by
 * background work that will wake pi again. */
export function liveStatus(target: Pick<ChatTarget, 'status' | 'hostId' | 'pid'>, open: { hostId: string; pid: number; running: boolean; waiting?: string } | undefined): AgentStatus {
  if (!open || open.hostId !== target.hostId || open.pid !== target.pid) return target.status;
  if (target.status === 'blocked') return target.status;
  // UNI-221: idle with wake-capable work pending is still "working".
  return open.running || open.waiting ? 'working' : target.status;
}

/** UNI-221: a row merged with the open (live) chat's run state — status via
 *  liveStatus, and the live `run.waiting` (pending work) beats the polled
 *  record, so the row flips "Working…" ↔ idle the moment the bridge pushes. */
export function withLive<T extends ChatTarget>(target: T, open: { hostId: string; pid: number; running: boolean; waiting?: string; lhMode?: { mode: string; paused?: string } } | undefined): T {
  const status = liveStatus(target, open);
  if (!open || open.hostId !== target.hostId || open.pid !== target.pid) return { ...target, status };
  const pendingWork = open.running ? undefined : open.waiting;
  const out = { ...target, status, pendingWork, waiting: effectiveWaiting(target.waiting, pendingWork) };
  // UNI-269: the open chat's live working mode beats the polled one.
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

function target(hostId: string, session: string, workspaceId: string, workspaceLabel: string, tabId: string, pane: ChatPane): ChatTarget {
  const name = pane.chat?.session_name?.trim();
  const fromTitle = (pane.terminal_title ?? pane.title ?? '').replace(/^π\s*-\s*/, '').trim();
  return {
    hostId,
    session,
    workspaceId,
    workspaceLabel,
    tabId,
    paneId: pane.id,
    pid: pane.chat?.pid ?? null,
    title: name || fromTitle || workspaceLabel,
    cwd: short(pane.chat?.cwd ?? pane.cwd),
    cwdFull: pane.chat?.cwd ?? pane.cwd ?? undefined,
    status: paneStatus(pane),
    focused: pane.focused,
    waiting: effectiveWaiting(pane.chat?.waiting, pane.chat?.pending_work),
    ...(pane.chat?.pending_work ? { pendingWork: pane.chat.pending_work } : {}),
    ...lhModeOf(pane.chat),
  };
}

/** UNI-269: the row's mode badge fields from the host's chat ref. */
function lhModeOf(chat: ChatPane['chat']): Pick<ChatTarget, 'lhMode' | 'lhPaused'> {
  return {
    ...(chat?.lh_mode ? { lhMode: chat.lh_mode } : {}),
    ...(chat?.lh_paused ? { lhPaused: chat.lh_paused } : {}),
  };
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

/** One "needs you" row (bell drawer / lib/needsYou.ts) per waiting pane in a
 *  host's chat_list snapshot — pure, so the bell's poller and tests share it. */
export function waitingTargets(hostId: string, data: ChatListData | undefined): Array<ChatTarget & { waiting: ChatWaiting }> {
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

type Entry = { data?: ChatListData; error?: string; loading: boolean; at?: number };

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
