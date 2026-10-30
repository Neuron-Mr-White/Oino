/**
 * Composer send + per-host send history (DESIGN.md §3.5). The bytes
 * live here; the recents store keeps the history (last 50 per host).
 */
import { composeSend } from './termToolbar';

export { composeSend };

export const SEND_HISTORY_MAX = 50;

/** Pushes a send onto a host's history: deduped, newest first. */
export function pushSendHistory(history: string[], text: string): string[] {
  const trimmed = text.trim();
  if (trimmed === '') return history;
  const without = history.filter((entry) => entry !== trimmed);
  return [trimmed, ...without].slice(0, SEND_HISTORY_MAX);
}

/** Parses a stored history blob, keeping only non-empty strings. */
export function parseStoredSendHistory(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  return raw.filter((entry): entry is string => typeof entry === 'string' && entry.length > 0);
}
