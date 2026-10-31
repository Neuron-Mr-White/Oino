/**
 * UNI-161 §5 (migrated to v2 stores, P0-06): an ongoing Android notification
 * while a v2 session (`sessions()` / `hubs()`) is working — "Running `npm test`
 * · 3m 5s" (from `liveActivity`) with a Stop button — cleared when the run
 * settles or the session closes. Updates are throttled (≥ 5 s apart unless the
 * activity title changes).
 *
 * The Stop tap (or an `ask_user` / approval action tap) goes through
 * `NotifyActionReceiver` (`readPendingAction`); `handlePendingAction()` latches
 * it so cold-start taps wait until the v2 session stream is `live` before
 * dispatching `abort()` or `dialogResponse(...)`.
 */
import { createEffect, createRoot, on } from 'solid-js';
import type { SessionCard } from './host/client';
import { formatDuration, liveActivity, type LiveTools, type Part } from './chat/model';
import { readPendingAction, type PendingAction } from './notifyDeepLink';
import { hubs } from './session/hub';
import { activeChatTargetFor } from './session/open';
import { compareCards, isRunning, sessionCwd, sessionTitle } from './session/selectors';
import { sessions, type SessionHandle } from './session/store';
import { dialogResponse } from '../ui/dialogs/adapter';
import { sessionUi } from '../ui/registries';

export const RUNNING_NOTIFICATION_ID = 900;
const MIN_GAP_MS = 5000;

type Native = {
  show?: (id: number, title: string, body: string, ongoing: boolean, actionsJson: string | null) => void;
  cancel?: (id: number) => void;
};
const native = (): Native | undefined => (window as unknown as { UniPiNotify?: Native }).UniPiNotify;

/** The notification's text for a running chat (pure, tested). */
export function runningText(input: { session: string; title: string; detail?: string; since?: number; now: number }): { title: string; body: string } {
  const what = input.detail ? `${input.title} ${input.detail}` : input.title;
  const time = input.since ? ` · ${formatDuration(Math.max(0, input.now - input.since))}` : '';
  return { title: input.session || 'UniPi is working', body: `${what}${time}` };
}

/**
 * UNI-223: whether the ongoing notification needs re-posting. Only when
 * its text changed: the activity (a new tool, Thinking → Writing) right
 * away, the elapsed-time tick at most every MIN_GAP_MS.
 */
export function runningUpdateDue(
  last: { title: string; text: string; at: number } | undefined,
  next: { title: string; text: string },
  now: number,
  minGapMs = MIN_GAP_MS,
): boolean {
  if (!last) return true;
  if (next.text === last.text) return false;
  if (next.title !== last.title) return true;
  return now - last.at >= minGapMs;
}

type RunningSnapshot = {
  hostId: string;
  sessionId: string;
  sessionName: string;
  title: string;
  detail?: string;
  since?: number;
};

/** Adapts a v2 `SessionHandle` + optional `SessionCard` into `liveActivity` inputs. */
function snapshotFromHandle(handle: SessionHandle, card?: SessionCard): RunningSnapshot {
  const st = handle.state;
  const sid = handle.sessionId();
  const cwd = sessionCwd(card, st) || undefined;
  const dialogs = sessionUi(handle.hostId, sid).dialogs();
  const dialogList = dialogs.length ? dialogs : st.dialogs;

  let act: { title: string; detail?: string };
  if (!dialogList.length && st.run.phase === 'compacting') {
    act = { title: 'Compacting' };
  } else if (!dialogList.length && st.run.phase === 'retrying') {
    act = { title: 'Retrying' };
  } else {
    const tools: LiveTools = {};
    for (const [id, tool] of Object.entries(st.liveTools)) {
      tools[id] = {
        name: tool.name,
        args: tool.args,
        text: tool.partial,
        done: tool.done,
        isError: tool.isError,
      };
    }
    const parts = Array.isArray(st.streaming?.message.content) ? (st.streaming.message.content as Part[]) : undefined;
    act = liveActivity(
      {
        dialogs: dialogList,
        streaming: st.streaming ? { id: 'live', parts: parts ?? [] } : undefined,
        tools,
        run: { running: st.run.running },
      },
      cwd,
    );
  }
  return {
    hostId: handle.hostId,
    sessionId: sid,
    sessionName: sessionTitle(card, st) || 'UniPi',
    title: act.title,
    detail: act.detail,
    since: st.run.startedAt,
  };
}

/** Ordered open `SessionHandle`s: currently routed session first, then MRU. */
function orderedOpenHandles(): SessionHandle[] {
  const reg = sessions();
  const out: SessionHandle[] = [];
  const seen = new Set<string>();
  const active = activeChatTargetFor();
  if (active?.kind === 'session') {
    const h = reg.peek(active.hostId, active.sessionId);
    if (h) {
      out.push(h);
      seen.add(h.key);
    }
  }
  const keys = reg.keys();
  for (let i = keys.length - 1; i >= 0; i -= 1) {
    const key = keys[i]!;
    if (seen.has(key)) continue;
    const slash = key.indexOf('/');
    if (slash <= 0) continue;
    const h = reg.peek(key.slice(0, slash), key.slice(slash + 1));
    if (h) {
      out.push(h);
      seen.add(h.key);
    }
  }
  return out;
}

/** Picks the primary running session across v2 `sessions()` and `hubs()`. */
export function currentRunningSession(): RunningSnapshot | undefined {
  for (const handle of orderedOpenHandles()) {
    if (handle.state.conn !== 'live') continue;
    const card = hubs().peek(handle.hostId)?.state.cards[handle.sessionId()];
    if (isRunning(card, handle.state)) return snapshotFromHandle(handle, card);
  }
  // Fallback: a working session card on an open hub whose stream isn't open yet.
  for (const hostId of hubs().keys()) {
    const hub = hubs().peek(hostId);
    if (!hub) continue;
    const working = Object.values(hub.state.cards)
      .filter((card) => {
        const open = sessions().peek(hostId, card.session_id);
        if (open && open.state.conn === 'live') return false;
        return card.run_state === 'working';
      })
      .sort((a, b) => compareCards(a, b, hub.state.meta));
    const top = working[0];
    if (top) {
      return {
        hostId,
        sessionId: top.session_id,
        sessionName: sessionTitle(top) || 'UniPi',
        title: top.needs_you ? 'Waiting for you' : 'Working',
        since: top.updated_at,
      };
    }
  }
  return undefined;
}

let latchedPending: PendingAction | undefined;
let lastRunningTarget: { hostId: string; sessionId: string } | undefined;

/** Whether an unfulfilled notification action is waiting for a live v2 session. */
export function hasLatchedPendingAction(): boolean {
  return latchedPending !== undefined;
}

/**
 * Handles one pending notification action (tapped while the app was closed or
 * in the background):
 * - Stop (`RUNNING_NOTIFICATION_ID`) aborts the running v2 session;
 * - Dialog option (`notificationId = dialogIndex + 1000`) sends `dialogResponse`
 *   on the live v2 session that owns the pending dialog.
 *
 * Latches a freshly read native action until a live v2 session is ready to
 * consume it, so cold-start deep links never drop the tap during stream connect.
 */
export function handlePendingAction(): boolean {
  const pending = latchedPending ?? readPendingAction();
  if (!pending) return false;
  latchedPending = pending;

  if (pending.notificationId === RUNNING_NOTIFICATION_ID) {
    if (pending.action !== 'stop') {
      latchedPending = undefined;
      return false;
    }
    const target = currentRunningSession() ?? (lastRunningTarget ? { ...lastRunningTarget } : undefined);
    if (!target) return false;
    const handle = sessions().ensure(target.hostId, target.sessionId);
    if (handle.state.conn !== 'live') return false;
    latchedPending = undefined;
    void handle.abort().catch(() => undefined);
    return true;
  }

  const dialogIndex = pending.notificationId - 1000;
  if (dialogIndex < 0) {
    latchedPending = undefined;
    return false;
  }
  for (const handle of orderedOpenHandles()) {
    if (handle.state.conn !== 'live') continue;
    const reg = sessionUi(handle.hostId, handle.sessionId());
    const dialogs = reg.dialogs();
    if (!dialogs.length) continue;
    const chosen = dialogs.find((d) => d.id === String(dialogIndex)) ?? dialogs[dialogIndex] ?? dialogs[0];
    if (!chosen) continue;
    latchedPending = undefined;
    reg.dismissDialog(chosen.id);
    void handle.cmd(dialogResponse(chosen.id, pending.action)).catch(() => undefined);
    return true;
  }
  return false;
}

/** Starts the watcher on v2 `sessions()` / `hubs()`; returns a stop function. */
export function startRunningNotification(enabled: () => boolean): () => void {
  const n = native();
  if (!n?.show) return () => {};
  let last: { title: string; text: string; at: number } | undefined;
  const clear = () => {
    if (last) n.cancel?.(RUNNING_NOTIFICATION_ID);
    last = undefined;
  };
  const dispose = createRoot((disposeRoot) => {
    const tick = () => {
      if (!enabled()) return clear();
      const snap = currentRunningSession();
      if (!snap) return clear();
      lastRunningTarget = { hostId: snap.hostId, sessionId: snap.sessionId };
      const now = Date.now();
      const text = runningText({
        session: snap.sessionName,
        title: snap.title,
        detail: snap.detail,
        since: snap.since,
        now,
      });
      const next = { title: snap.title, text: `${text.title}\n${text.body}` };
      if (!runningUpdateDue(last, next, now)) return;
      n.show!(RUNNING_NOTIFICATION_ID, text.title, text.body, true, JSON.stringify([{ label: 'Stop', action: 'stop' }]));
      last = { ...next, at: now };
    };
    createEffect(
      on(
        () => {
          const onFlag = enabled();
          const handles = orderedOpenHandles();
          const handleSig = handles.map((h) => {
            const st = h.state;
            const content = st.streaming?.message.content;
            const partLen = Array.isArray(content) ? content.length : typeof content === 'string' ? 1 : 0;
            const dialogsLen = sessionUi(h.hostId, h.sessionId()).dialogs().length;
            return `${h.key}:${st.conn}:${st.run.running}:${st.run.phase}:${partLen}:${Object.keys(st.liveTools).length}:${dialogsLen}`;
          });
          const hubSig = hubs()
            .keys()
            .map((hostId) => {
              const hub = hubs().peek(hostId);
              if (!hub) return hostId;
              const working = Object.values(hub.state.cards)
                .filter((c) => c.run_state === 'working')
                .map((c) => `${c.session_id}:${c.needs_you ? 1 : 0}`);
              return `${hostId}:${working.join(',')}`;
            });
          return [onFlag, handleSig.join('|'), hubSig.join('|')] as const;
        },
        tick,
      ),
    );
    const timer = setInterval(tick, MIN_GAP_MS);
    const onVisible = () => {
      if (document.visibilityState === 'visible') handlePendingAction();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
      clear();
      disposeRoot();
    };
  });
  return dispose;
}

/** Test seam: clears latched pending action / last running target. */
export function resetRunningNotification(): void {
  latchedPending = undefined;
  lastRunningTarget = undefined;
}
