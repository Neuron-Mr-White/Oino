/**
 * Sessions the app runs for itself (Settings' command runtime, owner5 #5):
 * never listed with the user's chats. Remembered across restarts (a command
 * reply can give the runtime a session file).
 */
import { createSignal } from 'solid-js';

const KEY = 'unipi.internalSessions';

function load(): Set<string> {
  try {
    const raw = JSON.parse(globalThis.localStorage?.getItem(KEY) ?? '[]');
    return new Set(Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : []);
  } catch {
    return new Set();
  }
}

const [ids, setIds] = createSignal<ReadonlySet<string>>(load());

export function markInternalSession(sessionId: string): void {
  if (ids().has(sessionId)) return;
  const next = new Set(ids()).add(sessionId);
  setIds(next);
  try {
    globalThis.localStorage?.setItem(KEY, JSON.stringify([...next].slice(-100)));
  } catch {
    // best effort
  }
}

export function isInternalSession(sessionId: string): boolean {
  return ids().has(sessionId);
}
