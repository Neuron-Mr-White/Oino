/**
 * [owner-fix-9] "The note goes with the answer": while an `ask_user` card waits in a chat,
 * the composer stays open, and a message sent from it first submits the card with its
 * current picks (unpicked questions count as skipped), then goes out as a normal message.
 *
 * Each waiting `AskUserCard` registers here under its session key; the composer reads it for
 * its placeholder and calls `answerPendingAsk` on send. Oldest registration first.
 */
import { createSignal } from 'solid-js';

export type PendingAsk = { id: string; submit: () => void };

const [pending, setPending] = createSignal<ReadonlyMap<string, readonly PendingAsk[]>>(new Map());

/** Registers a waiting card; returns the unregister function. */
export function registerPendingAsk(sessionKey: string, ask: PendingAsk): () => void {
  setPending((m) => {
    const next = new Map(m);
    next.set(sessionKey, [...(m.get(sessionKey) ?? []).filter((a) => a.id !== ask.id), ask]);
    return next;
  });
  return () =>
    setPending((m) => {
      const list = (m.get(sessionKey) ?? []).filter((a) => a.id !== ask.id);
      const next = new Map(m);
      if (list.length) next.set(sessionKey, list);
      else next.delete(sessionKey);
      return next;
    });
}

/** A question card is waiting in this chat (reactive). */
export function hasPendingAsk(sessionKey: string): boolean {
  return (pending().get(sessionKey)?.length ?? 0) > 0;
}

/** Submits the oldest waiting card of this chat with its current picks; `false` when none waits. */
export function answerPendingAsk(sessionKey: string): boolean {
  const ask = pending().get(sessionKey)?.[0];
  if (!ask) return false;
  ask.submit();
  return true;
}
