/**
 * UNI-161 §2/§3 + G-12 (WP-A19): wires the native notification bridge into
 * the app at startup — pushes the stored E2E key down to the native side
 * (for background decrypt), listens for `unipi://s/<host>/<session>` deep
 * links (notification taps while the app runs: needs-you, cron runs and
 * completions all open their session's route), and replays one pending
 * action queued while the app was closed (an ask_user option or permission
 * allow-deny) on the open chat.
 */
import { parseChatDeepLink, readPendingAction } from './notifyDeepLink';
import { sessionHref } from '../components/shell/Sidebar';
import type { HostInfo } from './schemas';

type NativeNotifyBridge = {
  setKey?: (keyB64: string) => void;
  registerPush?: () => void;
  requestPermission?: () => void;
};

function bridge(): NativeNotifyBridge | undefined {
  return (window as unknown as { UniPiNotify?: NativeNotifyBridge }).UniPiNotify;
}

/** Pushes the current key down to the native side so background delivery
 *  (UnifiedPush while the app isn't running) can decrypt. No-op without
 *  the native bridge (desktop/web) or without a key configured yet. */
export function syncNotifyKeyToNative(keyB64: string): void {
  bridge()?.setKey?.(keyB64 ?? '');
}

/** Resolves a deep link's `host` (a paired host id OR — from the pi
 *  side — a bare machine hostname) to a paired host id, preferring an
 *  exact id match, then a case-insensitive name match. */
export function resolveHostId(hosts: readonly HostInfo[], host: string | undefined): string | undefined {
  if (!host) return hosts[0]?.id;
  const byId = hosts.find((h) => h.id === host);
  if (byId) return byId.id;
  const byName = hosts.find((h) => h.name.toLowerCase() === host.toLowerCase());
  return byName?.id ?? hosts[0]?.id;
}

/** Opens a session's route from anywhere (the HashRouter follows the hash
 *  change; `useNavigate` needs a component scope). */
export function openSessionRoute(hostId: string, sessionId: string): void {
  // The ordinary session route presents Superagent as a workspace chat. Resolve
  // its identity first so notification actions return to the restricted view.
  void import('../components/superagent/api').then(async ({ superagentApi }) => {
    const result = await superagentApi.sessions(hostId);
    const superagent = result.ok && result.value.some((row) => row.sessionId === sessionId);
    location.hash = superagent
      ? `#/superagent?host=${encodeURIComponent(hostId)}&session=${encodeURIComponent(sessionId)}`
      : `#${sessionHref(hostId, sessionId)}`;
  }).catch(() => { location.hash = `#${sessionHref(hostId, sessionId)}`; });
}

/** Opens the session a `unipi://s/<host>/<session>` (or v1
 *  `unipi://chat?host=&session=`) link points at — the conversation route;
 *  the hub fills its title. A v1 link (`pid=` only) has no session: it just
 *  goes to the Chat list. */
export function openChatFromDeepLink(hosts: readonly HostInfo[], url: string): void {
  const parsed = parseChatDeepLink(url);
  if (!parsed) return;
  const hostId = resolveHostId(hosts, parsed.host);
  const session = parsed.session;
  if (!hostId || !session) {
    location.hash = '#/chat';
    return;
  }
  openSessionRoute(hostId, session);
}

/** Replays one pending action (ask_user option tapped from a notification
 *  while the app was closed) once a chat for the given target is open and
 *  connected: sends `answer`. Best-effort — a missing dialog id or a
 *  disconnected chat just drops it, same as a stale/duplicate tap. */
export function replayPendingAction(answer: (dialogId: number, value: string) => void): void {
  const pending = readPendingAction();
  if (!pending) return;
  // notificationId doubles as the dialog id for ask_user/permission
  // notifications (PushServiceImpl sets it to dialogId ?? pid, +1000).
  const dialogId = pending.notificationId - 1000;
  if (dialogId < 0) return;
  answer(dialogId, pending.action);
}
