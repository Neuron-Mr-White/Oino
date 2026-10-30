/**
 * The active terminal target for the /term route (DESIGN §3.3).
 * `target: null` means the Skip flow: a plain login shell.
 *
 * Backed by the live-session store (lib/liveSessions): setting a
 * target resumes its background connection when there is one, or
 * starts a new live entry; the previous terminal stays connected in
 * the background.
 */
import { activeLive, clearActiveLive, openLive, type LiveTarget } from './liveSessions';

export type TermTarget = LiveTarget;

export function setTermTarget(next: TermTarget | null) {
  if (next) openLive(next);
  else clearActiveLive();
}
export function getTermTarget(): TermTarget | null {
  return activeLive()?.target ?? null;
}
