/**
 * Session selectors (app spec §3.3, WP-A2): the derived values every surface
 * shares — title, project folder, running, needs-you, cost — for a hub card
 * and/or an open session, plus sessions grouped by project (D-9 data:
 * pinned → running → needs-you → recent inside a project, projects by their
 * latest activity) and the connection banner (Z-50).
 */
import { isInternalSession } from './internal';
import type { SessionCard } from '../host/client';
import type { SessionMeta } from './hub';
import { isProvisional, type SessionState } from './types';

export const NO_WORKSPACE_LABEL = 'No workspace';

const folderOf = (cwd?: string) => (cwd ? cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd : NO_WORKSPACE_LABEL);

/**
 * True when `cwd` is a "No workspace" chat's private scratch folder — `<data dir>/oino/scratch/<session id>`
 * (Linux `~/.local/share/oino/scratch/…`, macOS `~/Library/Application Support/oino/scratch/…`,
 * Windows `%LOCALAPPDATA%\\oino\\scratch\\…`). The host picks it; the app only recognises the shape.
 */
export function isScratchCwd(cwd?: string | null): boolean {
  return !!cwd && /[\\/]oino[\\/]scratch[\\/][^\\/]+[\\/]?$/.test(cwd);
}

/**
 * True when `cwd` means "No workspace" (D-18): absent, a user's bare home directory (older chats ran
 * there) or a UniPi scratch folder ({@link isScratchCwd}).
 */
export function isHomeCwd(cwd?: string | null): boolean {
  if (!cwd) return true;
  if (isScratchCwd(cwd)) return true;
  // `/` (an empty path once trimmed) counts as No workspace too.
  return !cwd.replace(/[\\/]+$/, '') || isBareHomeCwd(cwd);
}

/** True when `cwd` is a user's bare home directory itself (older chats, cloned from UniPi, ran there). */
export function isBareHomeCwd(cwd?: string | null): boolean {
  if (!cwd) return false;
  const clean = cwd.replace(/[\\/]+$/, '');
  return !!clean && (clean === '~' || /^\/home\/[^/]+$|^\/Users\/[^/]+$|^\/root$|^[A-Za-z]:[\\/]Users[\\/][^\\/]+$/.test(clean));
}

/**
 * True when the chat may "Move to a workspace…" (one way): it runs in its scratch folder, or —
 * an older chat — in the bare home folder (the host checks again; the home is never copied from).
 */
export function isMovableCwd(cwd?: string | null): boolean {
  return isScratchCwd(cwd) || isBareHomeCwd(cwd);
}

/** Card / session display title: the session name, else its project folder (or `fallback` when in No workspace). */
export function sessionTitle(card?: SessionCard, state?: Pick<SessionState, 'name'>, fallback = 'New chat'): string {
  const name = state?.name?.trim() || card?.name?.trim();
  if (name) return name;
  if (card?.cwd && !isHomeCwd(card.cwd)) return folderOf(card.cwd);
  return fallback;
}

/** The project folder a session belongs to (`""` = none). */
export function sessionCwd(card?: SessionCard, state?: Pick<SessionState, 'info'>): string {
  const fromState = state?.info && typeof state.info.cwd === 'string' ? state.info.cwd : undefined;
  return (card?.cwd || fromState || '').replace(/[\\/]+$/, '');
}

/** `~/…` for display. */
export function shortCwd(cwd: string): string {
  return cwd.replace(/^\/home\/[^/]+/, '~').replace(/^\/Users\/[^/]+/, '~');
}

/** Running: the open session's run state wins over the (slower) card. */
export function isRunning(card?: SessionCard, state?: Pick<SessionState, 'run'>): boolean {
  if (state) return state.run.running || state.run.phase === 'compacting' || state.run.phase === 'retrying';
  return card?.run_state === 'working';
}

/** Needs you: an open dialog (session) or the card's flag. */
export function needsYou(card?: SessionCard, state?: Pick<SessionState, 'dialogs'>): boolean {
  if (state && state.dialogs.length) return true;
  return card?.needs_you === true;
}

export function isCrashed(card?: SessionCard, state?: Pick<SessionState, 'banner'>): boolean {
  return state?.banner?.kind === 'crashed' || card?.run_state === 'crashed';
}

/** Total cost (USD) of the session's assistant messages on the loaded entries. */
export function sessionCost(state: Pick<SessionState, 'entries'>): number {
  let total = 0;
  for (const e of state.entries) {
    const m = e.message;
    if (m?.role === 'assistant') total += m.usage?.cost?.total ?? 0;
  }
  return total;
}

/** Last activity (epoch ms) of a card. */
export const lastActivity = (card: SessionCard): number => {
  const c = card as SessionCard & { last_activity?: unknown };
  return typeof c.last_activity === 'number' ? c.last_activity : (card.updated_at ?? 0);
};

export type ProjectGroup = {
  /** The project folder (`""` = no folder). */
  cwd: string;
  label: string;
  cards: SessionCard[];
  running: number;
  needsYou: number;
  lastActivity: number;
};

/** One host's sidebar sections (Z-8/Z-14): the Pinned group first, then the projects. */
export type SessionGroups = { pinned: SessionCard[]; projects: ProjectGroup[] };

/** Order inside a project: pinned → running → needs-you → most recent (Z-9 / D-9). */
export function compareCards(a: SessionCard, b: SessionCard, meta: Readonly<Record<string, SessionMeta>> = {}): number {
  const rank = (c: SessionCard) => (meta[c.session_id]?.pinned ? 0 : c.run_state === 'working' ? 1 : c.needs_you ? 2 : 3);
  return rank(a) - rank(b) || lastActivity(b) - lastActivity(a) || a.session_id.localeCompare(b.session_id);
}

/** Order inside the Pinned group (Z-14): running → needs-you → most recent. */
function comparePinned(a: SessionCard, b: SessionCard): number {
  const rank = (c: SessionCard) => (c.run_state === 'working' ? 0 : c.needs_you ? 1 : 2);
  return rank(a) - rank(b) || lastActivity(b) - lastActivity(a) || a.session_id.localeCompare(b.session_id);
}

/** Sessions grouped by project folder (D-9), projects by latest activity; home/empty cwd groups as "No workspace" (D-18). */
export function projectsOf(cards: Readonly<Record<string, SessionCard>> | readonly SessionCard[], meta: Readonly<Record<string, SessionMeta>> = {}): ProjectGroup[] {
  const list = Array.isArray(cards) ? cards : Object.values(cards);
  const byCwd = new Map<string, ProjectGroup>();
  for (const card of list) {
    const rawCwd = sessionCwd(card);
    const cwd = isHomeCwd(rawCwd) ? '' : rawCwd;
    let g = byCwd.get(cwd);
    if (!g) {
      g = { cwd, label: cwd ? folderOf(cwd) : NO_WORKSPACE_LABEL, cards: [], running: 0, needsYou: 0, lastActivity: 0 };
      byCwd.set(cwd, g);
    }
    g.cards.push(card);
    if (card.run_state === 'working') g.running += 1;
    if (card.needs_you) g.needsYou += 1;
    g.lastActivity = Math.max(g.lastActivity, lastActivity(card));
  }
  const groups = [...byCwd.values()];
  for (const g of groups) g.cards.sort((a, b) => compareCards(a, b, meta));
  groups.sort((a, b) => b.lastActivity - a.lastActivity || a.label.localeCompare(b.label));
  return groups;
}

/**
 * The sidebar's view of one host's cards (Z-8/Z-9/Z-14): pinned sessions in
 * their own group on top (the projects do not repeat them), then the project
 * groups with pinned → running → needs-you → recent inside.
 */
export function sessionGroups(cards: Readonly<Record<string, SessionCard>> | readonly SessionCard[], meta: Readonly<Record<string, SessionMeta>> = {}): SessionGroups {
  const list = (Array.isArray(cards) ? cards : Object.values(cards)).filter((c) => !isInternalSession(c.session_id));
  const pinned = list.filter((c) => meta[c.session_id]?.pinned).sort(comparePinned);
  const pinnedIds = new Set(pinned.map((c) => c.session_id));
  const rest = list.filter((c) => !pinnedIds.has(c.session_id));
  return { pinned, projects: projectsOf(rest, meta) };
}

/** The card's line changes (`+N −M`, Z-12) when the host reports a summary. */
export function changesOf(card: SessionCard): { add: number; del: number } | undefined {
  const c = card as SessionCard & { changes?: { add?: unknown; del?: unknown } };
  const changes = c.changes;
  if (!changes || typeof changes.add !== 'number' || typeof changes.del !== 'number') return undefined;
  return { add: changes.add, del: changes.del };
}

/** The connection banner (Z-50 "Lost the connection · Retry"); undefined while live / connecting. */
export function connBanner(state: Pick<SessionState, 'conn' | 'connError' | 'banner'>): { text: string; action: 'retry' } | undefined {
  if (state.conn !== 'reconnecting' && state.conn !== 'lost') return undefined;
  return { text: state.conn === 'lost' ? 'Lost the connection' : 'Lost the connection · reconnecting…', action: 'retry' };
}

/** Outbox bubble words (Z-50): waiting / sending / lost. */
export function outboxLabel(status: SessionState['outbox'][number]['status']): string | undefined {
  if (status === 'waiting') return 'Waiting for connection…';
  if (status === 'lost') return 'Not sent';
  if (status === 'sending') return 'Sending…';
  return undefined;
}

/** Entries the host has not confirmed yet (live run tail). */
export const provisionalCount = (state: Pick<SessionState, 'entries'>): number => state.entries.filter((e) => isProvisional(e.id)).length;
