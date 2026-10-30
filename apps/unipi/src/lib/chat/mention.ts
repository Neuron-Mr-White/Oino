/**
 * "Mention in chat" (UNI-155): the Files screen's long-press action that
 * inserts `@<path>` into a chat's draft. The chat may be in the
 * background (its `ChatView` instance owns the actual draft signal), so
 * this is a tiny mailbox keyed the same way as `ChatView`'s `key()`
 * (`${hostId}:${pid}`): `ChatView` drains it in an effect and folds the
 * text into its own draft state.
 */
import { createSignal } from 'solid-js';

export type PendingMention = { key: string; text: string };

const [pending, setPendingSignal] = createSignal<PendingMention | null>(null);

/** Queues `text` to be appended to the chat identified by `key`'s draft. */
export function queueMention(key: string, text: string): void {
  setPendingSignal({ key, text });
}

export function pendingMention() {
  return pending();
}

/** Consumes the pending mention for `key`, if any (clears it either way
 * once read so it isn't re-applied on the next render). */
export function consumeMentionFor(key: string): string | null {
  const current = pending();
  if (!current) return null;
  setPendingSignal(null);
  return current.key === key ? current.text : null;
}
