/**
 * Start sessions from the app — WP-A1 placeholder for the v1 "new chat"
 * layer (herdr/tmux "Where" step, `chat_new` / `chat_stop` / `chat_config`,
 * all deleted with the v1 protocol, D-14).
 *
 * - `newSession(hostId, cwd)` → the host's `session_new` (host.md §7.2)
 *   through the HostClient seam; WP-A5 replaces it with the new-chat draft
 *   (Z-19).
 * - Hidden sessions ("Just hide from list"), per PC, in localStorage.
 * - Pure helpers for the New chat sheet (folder chips, names).
 */
import { createSignal } from 'solid-js';
import { toAppError, type AppError } from '@unipi/protocol';
import { hostClient } from '../host/factory';
import type { SessionCard } from '../host/client';
import type { Result } from '../ipc';
import { cardTarget, type ChatTarget } from './list';

/** Starts a session in `cwd` (or the host's default folder) and returns its row. */
export async function newSession(hostId: string, cwd?: string): Promise<Result<ChatTarget>> {
  try {
    const card = await hostClient(hostId).request<SessionCard>('session_new', cwd ? { cwd } : {});
    if (!card || typeof card.id !== 'string') return { ok: false, error: toAppError(new Error('The host did not return the new session.')) };
    return { ok: true, value: { ...cardTarget(hostId, card), status: 'idle' } };
  } catch (error) {
    return { ok: false, error: toAppError(error) };
  }
}

/** A real host path to start in: the list shortens `/home/<u>` to `~`, which
 * the host can't resolve, so only absolute paths (Unix or `C:\`) are passed;
 * otherwise the host picks its default folder. */
export function hereCwd(cwd: string | undefined): string | undefined {
  return cwd && (cwd.startsWith('/') || /^[A-Za-z]:[\\/]/.test(cwd)) ? cwd : undefined;
}

/** One tap "New chat here": a separate session beside `t`, in the same folder. */
export function newChatHere(t: Pick<ChatTarget, 'hostId' | 'cwdFull' | 'cwd'>): Promise<Result<ChatTarget>> {
  return newSession(t.hostId, hereCwd(t.cwdFull ?? t.cwd));
}

// ── "New chat" sheet open state (Chat list +, desktop palette) ───────────

const [newOpen, setNewOpen] = createSignal(false);
export const newChatOpen = newOpen;
export const setNewChatOpen = (open: boolean) => setNewOpen(open);

// ── Hidden sessions, per PC ──────────────────────────────────────────────

const HIDDEN = (hostId: string) => `unipi.chat.hidden.${hostId}`;
const [hiddenRev, setHiddenRev] = createSignal(0);

/** A session's hide key (its id). */
export function hideKey(t: Pick<ChatTarget, 'session'>): string {
  return t.session;
}

export function readHidden(hostId: string): string[] {
  hiddenRev();
  try {
    const raw = JSON.parse(localStorage.getItem(HIDDEN(hostId)) ?? '[]');
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

function writeHidden(hostId: string, keys: string[]): void {
  try {
    localStorage.setItem(HIDDEN(hostId), JSON.stringify(keys.slice(-200)));
  } catch {
    // ignore
  }
  setHiddenRev((n) => n + 1);
}

export function hideChatFromList(hostId: string, t: Pick<ChatTarget, 'session'>): void {
  const key = hideKey(t);
  const keys = readHidden(hostId);
  if (!keys.includes(key)) writeHidden(hostId, [...keys, key]);
}

export function unhideChat(hostId: string, key: string): void {
  writeHidden(hostId, readHidden(hostId).filter((k) => k !== key));
}

/** Drops hide keys whose session is gone (keeps the list small). */
export function pruneHidden(hostId: string, live: Array<Pick<ChatTarget, 'session'>>): void {
  const alive = new Set(live.map(hideKey));
  const keys = readHidden(hostId);
  const kept = keys.filter((k) => alive.has(k));
  if (kept.length !== keys.length) writeHidden(hostId, kept);
}

// ── Folders ──────────────────────────────────────────────────────────────

/** Recent folders for the folder chips: session folders first, then the
 *  Files roots — distinct, home dropped unless alone, ≤ 8. */
export function folderChoices(sessions: ReadonlyArray<Pick<SessionCard, 'cwd'>> = [], roots: readonly string[] = [], home?: string): string[] {
  const all: string[] = [];
  for (const s of sessions) if (s.cwd && !all.includes(s.cwd)) all.push(s.cwd);
  for (const r of roots) if (!all.includes(r)) all.push(r);
  return all.filter((d) => d !== home || all.length === 1).slice(0, 8);
}

/** `/home/u/proj` → `~/proj` (display only). */
export function shortPath(path: string, home?: string): string {
  if (home && (path === home || path.startsWith(`${home}/`))) return `~${path.slice(home.length)}`;
  return path.replace(/^\/home\/[^/]+/, '~');
}

/** Last path segment (a session's default name). */
export function folderName(path: string): string {
  const parts = path.replace(/\/+$/, '').split('/');
  const last = parts[parts.length - 1];
  return last && last !== '~' ? last : 'session';
}

/** Plain words for a failed start. */
export function errorWords(error: AppError): { text: string; hint?: string } {
  if (error.code === 'E_SCHEMA' || error.code === 'E_UNSUPPORTED') {
    return { text: 'This PC runs an older UniPi host.', hint: 'Update UniPi on the PC (re-run the installer), then try again.' };
  }
  return { text: String(error.message), hint: error.hint };
}
