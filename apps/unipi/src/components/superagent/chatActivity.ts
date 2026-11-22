import { createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import { liveBusy } from '../../lib/session/activity';
import { sessions } from '../../lib/session/store';

/**
 * Which Superagent chat a host shows, and whether that chat itself is working.
 * The Superagent page remembers its chat per host (`unipi.superagent.session.<host>`,
 * so a return lands on the same chat); the sidebar row and the history list read the
 * same id to show the working spinner. Superagent chats are not in the host's session
 * list (cards), so the open session stream is the only truth.
 */
const savedKey = (host: string) => `unipi.superagent.session.${host}`;
const [active, setActive] = createSignal<Record<string, string>>({});

export function readSavedSuperagentSession(host: string): string | undefined {
  try { return localStorage.getItem(savedKey(host)) || undefined; } catch { return undefined; }
}

/** The chat the page picked (a new chat, a history row, a deep link) — and, when `persist`, the one the next visit reopens. */
export function rememberSuperagentSession(host: string, id: string, persist = true): void {
  setActive((m) => (m[host] === id ? m : { ...m, [host]: id }));
  if (!persist) return;
  try { localStorage.setItem(savedKey(host), id); } catch { /* Private storage: the route still keeps the session. */ }
}

/** The host's current Superagent chat id (reactive on the page's picks). */
export function superagentSessionId(host: string | undefined): string | undefined {
  return host ? active()[host] ?? readSavedSuperagentSession(host) : undefined;
}

/** True while an open, live Superagent stream runs, compacts, retries or waits on wake-capable work. */
export function superagentChatBusy(host: string | undefined, id: string | undefined): boolean {
  if (!host || !id) return false;
  const handle = sessions().peek(host, id);
  const state = handle && !handle.closed ? handle.state : undefined;
  return !!state && state.conn === 'live' && liveBusy(state.run);
}

/**
 * The host's Superagent chat is working. Pins the chat's open stream while a component uses
 * this, so the spinner keeps following the run after the page is left (the registry would
 * otherwise evict it after a few other chats).
 */
export function useSuperagentChatWorking(hostId: () => string | undefined): () => boolean {
  const id = createMemo(() => superagentSessionId(hostId()));
  const handle = createMemo(() => { const h = hostId(); const s = id(); return h && s ? sessions().ensure(h, s) : undefined; });
  createEffect(() => {
    const h = handle();
    if (!h || h.closed) return;
    const release = h.acquire();
    onCleanup(release);
  });
  return () => superagentChatBusy(hostId(), id());
}
