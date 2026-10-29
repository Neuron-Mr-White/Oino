/**
 * UNI-161 §2/§3: wires the native notification bridge into the app at
 * startup — pushes the stored E2E key down to the native side (for
 * background decrypt), listens for `unipi://chat` deep links (notification
 * taps while the app runs), and replays one pending action queued while
 * the app was closed (an ask_user option / permission allow-deny tapped
 * from a notification): opens the right chat and sends `answer` once the
 * bridge connects.
 */
import { openChat, updateOpenTarget } from './chat/open';
import { chatGroups, chatLists, type ChatTarget } from './chat/list';
import type { ChatListData } from './chat/transport';
import { parseChatDeepLink, readPendingAction } from './notifyDeepLink';
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

/** The chat-list row for `pid` on `hostId` in a `chat_list` snapshot. */
export function findChatTarget(hostId: string, data: ChatListData | undefined, pid: number): ChatTarget | undefined {
  for (const group of chatGroups(hostId, data)) {
    const hit = group.items.find((t) => t.pid === pid);
    if (hit) return hit;
  }
  return undefined;
}

/** Opens the chat a `unipi://chat?host=&pid=&dialog=` link points at.
 *  Only `hostId`/`pid` are known from the link itself, so the chat opens
 *  at once with placeholders, then takes its labels (session name,
 *  workspace) from the cached chat list and again after a fresh
 *  `chat_list` — seen on the Redmi: the header otherwise stayed
 *  "<PC> / <PC> ·" for good (nothing else fills them in). */
export function openChatFromDeepLink(hosts: readonly HostInfo[], url: string): void {
  const parsed = parseChatDeepLink(url);
  if (!parsed) return;
  const hostId = resolveHostId(hosts, parsed.host);
  if (!hostId || parsed.pid === undefined) return;
  const hostName = hosts.find((h) => h.id === hostId)?.name ?? '';
  const target: ChatTarget = {
    hostId,
    session: '',
    workspaceId: '',
    workspaceLabel: '',
    tabId: '',
    paneId: '',
    pid: parsed.pid,
    title: hostName,
    status: 'idle',
    focused: false,
  };
  const lists = chatLists();
  const fill = () => {
    const known = findChatTarget(hostId, lists.get(hostId)?.data, parsed.pid!);
    if (known) updateOpenTarget(known);
    return known;
  };
  openChat(findChatTarget(hostId, lists.get(hostId)?.data, parsed.pid) ?? target, hostName);
  void lists.load(hostId).then(fill, () => undefined);
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
