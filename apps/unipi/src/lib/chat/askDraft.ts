/**
 * [OIN-98] In-progress answers of an `ask_user` card (picked options + typed "Other" text),
 * kept per session key + tool call id so they survive tab/session switches and card remounts.
 * Held in memory and mirrored to sessionStorage (survives a reload). Cleared on submit/skip
 * and when the call settles.
 */
import type { AskAnswer, AskSettled } from './askUser';

const mem = new Map<string, AskAnswer[]>();
const storageKey = (sessionKey: string, callId: string) => `unipi:ask:draft:${sessionKey}:${callId}`;
const store = (): Storage | undefined => {
  try {
    return typeof sessionStorage === 'undefined' ? undefined : sessionStorage;
  } catch {
    return undefined;
  }
};

export function readAskDraft(sessionKey: string, callId: string): AskAnswer[] {
  const k = storageKey(sessionKey, callId);
  const hit = mem.get(k);
  if (hit) return hit;
  try {
    const raw = store()?.getItem(k);
    const parsed: unknown = raw ? JSON.parse(raw) : undefined;
    if (Array.isArray(parsed)) {
      mem.set(k, parsed as AskAnswer[]);
      return parsed as AskAnswer[];
    }
  } catch {
    /* unreadable draft: start empty */
  }
  return [];
}

export function writeAskDraft(sessionKey: string, callId: string, answers: AskAnswer[]): void {
  const k = storageKey(sessionKey, callId);
  if (!answers.some((a) => a && !a.skipped)) return clearAskDraft(sessionKey, callId);
  mem.set(k, answers);
  try {
    store()?.setItem(k, JSON.stringify(answers));
  } catch {
    /* storage full or blocked: memory copy still works */
  }
}

export function clearAskDraft(sessionKey: string, callId: string): void {
  const k = storageKey(sessionKey, callId);
  mem.delete(k);
  try {
    store()?.removeItem(k);
  } catch {
    /* ignore */
  }
}

/**
 * ASK-STALE: an answer sent from a card whose result has not landed yet. A card that remounts in that
 * window (chat switch, scroll away and back, restore) would otherwise find its call `running` with
 * the dialog already gone and sit there as a dead "Opening the question…" form. Memory only: after a
 * reload the host's snapshot is the truth (it re-sends the dialog when the answer never arrived).
 */
export type AskSentRecord = { settled: AskSettled; answers: AskAnswer[] };
const sentMem = new Map<string, AskSentRecord>();

export const readAskSent = (sessionKey: string, callId: string): AskSentRecord | undefined => sentMem.get(storageKey(sessionKey, callId));
export const writeAskSent = (sessionKey: string, callId: string, record: AskSentRecord): void => void sentMem.set(storageKey(sessionKey, callId), record);
export const clearAskSent = (sessionKey: string, callId: string): void => void sentMem.delete(storageKey(sessionKey, callId));
