import type { StatusNotice } from '../api';

const KEY = 'unipi.superagent.notices.dismissed.v1';
const KEEP = 100;

/** One notice, one key: the project and the moment the host made it. */
export const noticeKey = (n: Pick<StatusNotice, 'projectId' | 'at'>) => `${n.projectId ?? ''}@${n.at ?? ''}`;

export function readDismissed(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? '[]') as unknown;
    return Array.isArray(raw) ? raw.filter((x): x is string => typeof x === 'string') : [];
  } catch { return []; }
}

/** Remembers a dismissal (newest kept) and returns the new list. A full or blocked storage only loses the memory, never the dismissal for this view. */
export function dismissNotice(n: Pick<StatusNotice, 'projectId' | 'at'>): string[] {
  const next = [...new Set([...readDismissed(), noticeKey(n)])].slice(-KEEP);
  try { localStorage.setItem(KEY, JSON.stringify(next)); } catch { /* storage unavailable */ }
  return next;
}

/** The notices the work pane draws: `board_created` only, with an id, not dismissed yet. */
export function visibleNotices(notices: readonly StatusNotice[] | undefined, dismissed: readonly string[]): StatusNotice[] {
  return (notices ?? []).filter((n) => n.kind === 'board_created' && !!n.projectId && !dismissed.includes(noticeKey(n)));
}
