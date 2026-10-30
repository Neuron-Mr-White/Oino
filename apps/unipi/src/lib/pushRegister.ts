/**
 * UNI-161: background push via UnifiedPush (the ntfy app as distributor).
 *
 * Flow: `registerPush()` asks the native side to register with the
 * distributor → PushServiceImpl.onNewEndpoint stores the endpoint URL →
 * `syncPushEndpoint()` reads it and sends it to each paired host
 * (`notify_register`), which writes ~/.unipi/config/notify/app-endpoints.json
 * so the PC's notify publishes there too. Messages are our `unipi1:`
 * envelope (encrypted with the per-pairing key), decrypted natively.
 */
import { invoke } from '@tauri-apps/api/core';

export type PushState = 'registered' | 'not_registered' | 'no_distributor' | 'unsupported';
export type PushStatus = { state: PushState; distributor?: string; endpoint?: string };

type Native = {
  registerPush?: () => void;
  pushStatus?: () => string;
  pushEndpoint?: () => string;
  requestPermission?: () => void;
};
const native = (): Native | undefined => (window as unknown as { UniPiNotify?: Native }).UniPiNotify;

export function pushStatus(): PushStatus {
  const n = native();
  if (!n?.pushStatus) return { state: 'unsupported' };
  try {
    const parsed = JSON.parse(n.pushStatus()) as { state?: PushState; distributor?: string };
    const endpoint = n.pushEndpoint?.() || undefined;
    return { state: parsed.state ?? 'not_registered', distributor: parsed.distributor, endpoint };
  } catch {
    return { state: 'not_registered' };
  }
}

/** Asks for the notification permission (Android 13+) and registers with the distributor. */
export function registerPush(): boolean {
  const n = native();
  if (!n?.registerPush) return false;
  n.requestPermission?.();
  n.registerPush();
  return true;
}

const SENT_KEY = 'unipi.push.sent';
/** Hosts that already have this endpoint: `{[hostId]: endpoint}`. */
function sentMap(): Record<string, string> {
  try {
    return JSON.parse(localStorage.getItem(SENT_KEY) ?? '{}') as Record<string, string>;
  } catch {
    return {};
  }
}

/**
 * Sends the current endpoint to every paired host that doesn't have it yet.
 * Cheap to call often (startup, resume, after Register). Returns how many
 * hosts were updated.
 */
export async function syncPushEndpoint(hostIds: readonly string[], send: (hostId: string, endpoint: string) => Promise<unknown> = defaultSend): Promise<number> {
  const endpoint = native()?.pushEndpoint?.();
  if (!endpoint) return 0;
  const sent = sentMap();
  let n = 0;
  for (const hostId of hostIds) {
    if (sent[hostId] === endpoint) continue;
    try {
      await send(hostId, endpoint);
      sent[hostId] = endpoint;
      n++;
    } catch {
      // Host offline: retried on the next sync.
    }
  }
  localStorage.setItem(SENT_KEY, JSON.stringify(sent));
  return n;
}

const defaultSend = (hostId: string, endpoint: string) => invoke('notify_register', { hostId, endpoint });
