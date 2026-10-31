/**
 * The draft's first prompt (Z-19, WP-A5): `DraftView` creates the session with
 * `session_new` the moment the user sends, routes to `/s/:host/:id` — and the
 * prompt itself rides along here until that session's stream is live, so the
 * chat opens with the message already on its way (exactly one send, no
 * double-send on reconnect: the queue is taken, not peeked).
 */
import type { OutboxImage } from './types';

export type FirstPrompt = { text: string; images?: OutboxImage[] };

const pending = new Map<string, FirstPrompt>();

const keyOf = (hostId: string, sessionId: string) => `${hostId}/${sessionId}`;

/** Stash the prompt a draft just sent for the session it created. */
export function queueFirstPrompt(hostId: string, sessionId: string, prompt: FirstPrompt): void {
  pending.set(keyOf(hostId, sessionId), prompt);
}

/** Takes (removes) the pending first prompt of a live session, if any. */
export function takeFirstPrompt(hostId: string, sessionId: string): FirstPrompt | undefined {
  const key = keyOf(hostId, sessionId);
  const prompt = pending.get(key);
  pending.delete(key);
  return prompt;
}
