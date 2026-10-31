/**
 * "Mention in chat" mailbox + subscription API (UNI-155, P0-07):
 * routes `@<path>` and task-id mentions from Files, Editor, Board and FileRef
 * menus into the target chat's composer.
 *
 * - Callers push via `queueMention(key, text)` (`hostId:sessionId`,
 *   `hostId/sessionId`, or `draft:hostId`).
 * - A mounted composer subscribes via `subscribeMention` /
 *   `connectComposerMentions` (matching both `host:session` and `host/session`)
 *   to insert at the live caret.
 * - When no live subscriber is mounted for `key`, `queueMention` also writes
 *   the mention into `composerDraft(sessionKey)` and keeps the one-shot
 *   mailbox populated for `pendingMention()` / `consumeMentionFor()`, rolling
 *   back the pre-applied draft append if a subscriber drains the same pending
 *   entry on mount so nothing is ever lost or duplicated.
 */
import { createRenderEffect, createSignal, onCleanup } from 'solid-js';
import { composerDraft, focusComposer, setComposerText } from '../composer/drafts';
import { appendTokenText } from '../composer/tokens';

export type PendingMention = { key: string; text: string };
export type MentionListener = (text: string) => void;

type StoredMention = PendingMention & {
  sessionKey?: string;
  prevDraft?: string;
  appliedDraft?: string;
};

const [pending, setPendingSignal] = createSignal<StoredMention | null>(null);
const listeners = new Map<string, Set<MentionListener>>();

/** Normalizes v1 `hostId:sessionId` and v2 `hostId/sessionId` keys (`draft:hostId` stays as-is). */
export function toComposerSessionKey(key: string): string {
  if (!key || key.startsWith('draft:')) return key;
  const slash = key.indexOf('/');
  if (slash > 0) return key;
  const colon = key.indexOf(':');
  return colon > 0 ? `${key.slice(0, colon)}/${key.slice(colon + 1)}` : key;
}

function keysMatch(a: string, b: string): boolean {
  return a === b || toComposerSessionKey(a) === toComposerSessionKey(b);
}

/** Queues `text` to be inserted into the chat identified by `key`'s draft. */
export function queueMention(key: string, text: string): void {
  const sessionKey = toComposerSessionKey(key);
  const subs = listeners.get(sessionKey);
  if (subs && subs.size > 0) {
    setPendingSignal(null);
    for (const fn of subs) fn(text);
    focusComposer(sessionKey);
    return;
  }
  const prevDraft = sessionKey ? composerDraft(sessionKey).text : undefined;
  const appliedDraft = sessionKey && prevDraft !== undefined ? appendTokenText(prevDraft, text) : undefined;
  if (sessionKey && appliedDraft !== undefined) {
    setComposerText(sessionKey, appliedDraft);
    focusComposer(sessionKey);
  }
  setPendingSignal({ key, text, sessionKey, prevDraft, appliedDraft });
}

export function pendingMention(): PendingMention | null {
  const cur = pending();
  return cur ? { key: cur.key, text: cur.text } : null;
}

/** Consumes the pending mention for `key`, if any (clears it either way
 * once read so it isn't re-applied on the next render). */
export function consumeMentionFor(key: string): string | null {
  const current = pending();
  if (!current) return null;
  setPendingSignal(null);
  return keysMatch(current.key, key) ? current.text : null;
}

/**
 * Subscribes a composer to mentions targeting `key` (`hostId/sessionId`,
 * `hostId:sessionId`, or `draft:hostId`); immediately drains any queued mention
 * for `key` and returns the unsubscribe function.
 */
export function subscribeMention(key: string, listener: MentionListener): () => void {
  const sessionKey = toComposerSessionKey(key);
  const set = listeners.get(sessionKey) ?? new Set<MentionListener>();
  set.add(listener);
  listeners.set(sessionKey, set);

  const cur = pending();
  if (cur && keysMatch(cur.key, sessionKey)) {
    setPendingSignal(null);
    if (cur.sessionKey && cur.appliedDraft !== undefined && cur.prevDraft !== undefined && composerDraft(cur.sessionKey).text === cur.appliedDraft) {
      setComposerText(cur.sessionKey, cur.prevDraft);
    }
    listener(cur.text);
    focusComposer(sessionKey);
  }

  return () => {
    set.delete(listener);
    if (!set.size) listeners.delete(sessionKey);
  };
}

/**
 * One-line reactive hook for `Composer.tsx`:
 * `connectComposerMentions(key, (text) => commit(appendTokenText(draft().text, text), ...))`
 */
export function connectComposerMentions(key: () => string, insertText: (text: string) => void): void {
  createRenderEffect(() => {
    const k = key();
    onCleanup(subscribeMention(k, insertText));
  });
}

/** Test seam: clears queued mentions and listeners. */
export function resetMentions(): void {
  setPendingSignal(null);
  listeners.clear();
}
