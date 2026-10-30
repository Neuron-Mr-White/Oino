/**
 * UNI-161 §5: an ongoing Android notification while the open chat's pi is
 * working — "Running `npm test` · 3 min" (from liveActivity) with a Stop
 * button — cleared when pi goes idle or the chat closes. Updates are
 * throttled (≥ 5 s apart unless the activity title changes).
 *
 * The Stop tap goes through NotifyActionReceiver (it stores a pending
 * action and brings the app forward); `takeRunningStop()` consumes it on
 * resume and aborts the run.
 */
import { createEffect, createRoot, on } from 'solid-js';
import { openChatState } from './chat/open';
import { formatDuration, liveActivity } from './chat/model';
import { readPendingAction } from './notifyDeepLink';

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
  return { title: input.session || 'pi is working', body: `${what}${time}` };
}

/**
 * UNI-223: whether the ongoing notification needs re-posting. Only when
 * its text changed: the activity (a new tool, Thinking → Writing) right
 * away, the elapsed-time tick at most every MIN_GAP_MS. Re-posting the
 * same text did nothing useful, and on the old high-importance channel
 * every post buzzed the phone — a buzz per chat change. (Native side: the
 * running notice now has its own silent low-importance channel and
 * setOnlyAlertOnce, see NotificationBridge.kt.)
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

/**
 * Handles one pending notification action (tapped while the app was in the
 * background): Stop on the running notification aborts the open chat;
 * anything else (an ask_user option / permission) is answered on the open
 * chat's dialog (notification id = dialogId + 1000). One-shot.
 */
export function handlePendingAction(): void {
  const pending = readPendingAction();
  if (!pending) return;
  const chat = openChatState()?.chat;
  if (!chat) return;
  if (pending.notificationId === RUNNING_NOTIFICATION_ID) {
    if (pending.action === 'stop') chat.abort();
    return;
  }
  const dialogId = pending.notificationId - 1000;
  if (dialogId >= 0) chat.answer(dialogId, pending.action);
}

/** Starts the watcher; returns a stop function. `enabled()` = the user's notification switch. */
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
      const open = openChatState();
      const st = open?.chat.state;
      if (!enabled() || !open || !st || !st.run.running || st.conn !== 'live') return clear();
      const act = liveActivity(st, st.session?.cwd);
      const now = Date.now();
      const text = runningText({ session: st.session?.name || open.target.title || 'pi', title: act.title, detail: act.detail, since: st.runStartedAt, now });
      const next = { title: act.title, text: `${text.title}\n${text.body}` };
      if (!runningUpdateDue(last, next, now)) return;
      n.show!(RUNNING_NOTIFICATION_ID, text.title, text.body, true, JSON.stringify([{ label: 'Stop', action: 'stop' }]));
      last = { ...next, at: now };
    };
    createEffect(on(() => {
      const st = openChatState()?.chat.state;
      return [st?.run.running, st?.conn, st?.streaming?.parts.length, Object.keys(st?.tools ?? {}).length] as const;
    }, tick));
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
