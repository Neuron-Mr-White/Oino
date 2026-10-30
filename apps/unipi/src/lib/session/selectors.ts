/**
 * Session selectors (app spec §3.3, WP-A2): the derived values every surface
 * shares — title, project folder, running, needs-you, cost — for a hub card
 * and/or an open session, plus sessions grouped by project (D-9 data:
 * pinned → running → needs-you → recent inside a project, projects by their
 * latest activity) and the connection banner (Z-50).
 */
import type { SessionCard } from '../host/client';
import type { SessionMeta } from './hub';
import { isProvisional, type SessionState } from './types';

const folderOf = (cwd?: string) => (cwd ? cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || cwd : 'No folder');

/** Card / session display title: the session name, else its folder. */
export function sessionTitle(card?: SessionCard, state?: Pick<SessionState, 'name'>): string {
  const name = state?.name?.trim() || card?.name?.trim();
  return name || folderOf(card?.cwd);
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

/** Order inside a project: pinned → running → needs-you → most recent (Z-9 / D-9). */
export function compareCards(a: SessionCard, b: SessionCard, meta: Readonly<Record<string, SessionMeta>> = {}): number {
  const rank = (c: SessionCard) => (meta[c.session_id]?.pinned ? 0 : c.run_state === 'working' ? 1 : c.needs_you ? 2 : 3);
  return rank(a) - rank(b) || lastActivity(b) - lastActivity(a) || a.session_id.localeCompare(b.session_id);
}

/** Sessions grouped by project folder (D-9), projects by latest activity. */
export function projectsOf(cards: Readonly<Record<string, SessionCard>> | readonly SessionCard[], meta: Readonly<Record<string, SessionMeta>> = {}): ProjectGroup[] {
  const list = Array.isArray(cards) ? cards : Object.values(cards);
  const byCwd = new Map<string, ProjectGroup>();
  for (const card of list) {
    const cwd = sessionCwd(card);
    let g = byCwd.get(cwd);
    if (!g) {
      g = { cwd, label: folderOf(cwd || undefined), cards: [], running: 0, needsYou: 0, lastActivity: 0 };
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
