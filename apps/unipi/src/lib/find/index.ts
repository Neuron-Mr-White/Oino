/**
 * In-chat find (WP-A18, Z-79): the Cmd/Ctrl+F find bar's model. One surface:
 * the loaded conversation text is matched in place (virtual timeline), and
 * "Search whole session" falls back to the host's `search` command
 * (SearchSheet logic) when the loaded text can't answer.
 *
 * - `buildFindIndex` — pure: per-turn match counts over the texts that render
 *   by default (user + steer bubbles, answers), in document order. Matches in
 *   unmounted turns scroll into view (the Timeline adapter), then highlight.
 * - The controller signals drive the FindBar (count "3/17", wrap ↑/↓/Enter/
 *   Shift+Enter) and the Timeline's highlights. A navigation bumps
 *   `navVersion` (monotonic) so re-navigating the *same* index re-scrolls —
 *   the ZCode `chat-find-placement.md` rule.
 */
import { batch, createSignal } from 'solid-js';
import { textOf, type Part } from '../chat/model';
import type { Turn } from '../session/turns';
import type { SessionEntry } from '../session/types';
import type { SessionHandle } from '../session/store';

/** The texts of one turn the timeline renders by default, in order. */
export function turnFindTexts(turn: Turn): Array<{ key: string; text: string }> {
  const out: Array<{ key: string; text: string }> = [];
  const of = (e: SessionEntry | undefined, key: string): void => {
    const text = textOf(e?.message?.content as string | Part[] | undefined);
    if (text.trim()) out.push({ key, text });
  };
  if (turn.kind === 'turn') of(turn.user, turn.user?.id ?? 'user');
  if (turn.kind === 'compaction') of(turn.compaction, turn.compaction?.id ?? 'cmp');
  for (const seg of turn.segments) {
    if (seg.steer) of(seg.steer, seg.steer.id ?? `steer${seg.index}`);
    if (seg.answer) out.push({ key: seg.answer.key, text: seg.answer.text });
  }
  return out;
}

export type FindIndex = {
  query: string;
  /** One entry per turn with ≥ 1 match, in document order. */
  perTurn: Array<{ turnId: string; count: number }>;
  total: number;
};

/** Case-insensitive plain substring, like browser find. Empty query = no index. */
export function buildFindIndex(turns: readonly Turn[], query: string): FindIndex {
  const q = query.trim();
  if (!q) return { query: q, perTurn: [], total: 0 };
  const needle = q.toLowerCase();
  const perTurn: FindIndex['perTurn'] = [];
  let total = 0;
  for (const turn of turns) {
    let count = 0;
    for (const { text } of turnFindTexts(turn)) {
      let at = text.toLowerCase().indexOf(needle);
      while (at >= 0) {
        count += 1;
        at = text.toLowerCase().indexOf(needle, at + needle.length);
      }
    }
    if (count > 0) perTurn.push({ turnId: turn.turnId, count });
    total += count;
  }
  return { query: q, perTurn, total };
}

/** Where global match `index` (0-based) lives: its turn and occurrence there. */
export function locateMatch(index: FindIndex, at: number): { turnId: string; occurrence: number } | undefined {
  if (at < 0 || at >= index.total) return undefined;
  let seen = 0;
  for (const turn of index.perTurn) {
    if (at < seen + turn.count) return { turnId: turn.turnId, occurrence: at - seen };
    seen += turn.count;
  }
  return undefined;
}

// ── The whole-session fallback (SearchSheet logic, one surface) ─────────

/** A `search` command hit (v1 bridge shape, kept by oino hosts that offer it). */
export type SearchHit = { id: string; role: string; snippet: string; marks: Array<[number, number]>; score: number; timestamp?: string };

export const ROLE_LABEL: Record<string, string> = { user: 'You', assistant: 'Agent', tool: 'Ran', output: 'Output', command: 'Command', note: 'Note' };

/** Parses a `search` reply defensively (`null` = this host can't search). */
export function parseSearchHits(data: unknown): SearchHit[] | null {
  if (!Array.isArray(data)) return null;
  const hits: SearchHit[] = [];
  for (const hit of data) {
    if (!hit || typeof hit !== 'object') continue;
    const h = hit as Record<string, unknown>;
    if (typeof h.id !== 'string' && typeof h.entryId !== 'string') continue;
    hits.push({
      id: String(h.id ?? h.entryId),
      role: typeof h.role === 'string' ? h.role : 'assistant',
      snippet: typeof h.snippet === 'string' ? h.snippet : '',
      marks: Array.isArray(h.marks) ? (h.marks as unknown[]).filter((m): m is [number, number] => Array.isArray(m) && typeof m[0] === 'number' && typeof m[1] === 'number') : [],
      score: typeof h.score === 'number' ? h.score : 0,
      ...(typeof h.timestamp === 'string' ? { timestamp: h.timestamp } : {}),
    });
  }
  return hits;
}

// ── The controller ──────────────────────────────────────────────────────

export type FindState = {
  open: boolean;
  query: string;
  /** 0-based index of the active match within the loaded index. */
  active: number;
  /** Bumped on every user navigation (Enter / ↑ / ↓ / button / result). */
  navVersion: number;
};

const [state, setState] = createSignal<FindState>({ open: false, query: '', active: 0, navVersion: 0 });
export const findState = state;

const [whole, setWhole] = createSignal<{ busy: boolean; hits: SearchHit[] | null; unsupported: boolean }>({ busy: false, hits: null, unsupported: false });
export const wholeSession = whole;

/** The timeline's hooks: scrolling a (maybe unmounted) turn into view. */
export type TimelineFindAdapter = { jumpToTurn(turnId: string): void };
let adapter: TimelineFindAdapter | undefined;
export function setFindAdapter(a: TimelineFindAdapter | undefined): void {
  adapter = a;
}

export function openFind(query?: string): void {
  batch(() => setState((s) => ({ open: true, query: query ?? s.query, active: 0, navVersion: s.navVersion + 1 })));
}

export function closeFind(): void {
  batch(() => {
    setState((s) => ({ ...s, open: false }));
    setWhole({ busy: false, hits: null, unsupported: false });
  });
}

export function setFindQuery(query: string): void {
  batch(() => setState((s) => ({ ...s, query, active: 0, navVersion: s.navVersion + 1 })));
  setWhole({ busy: false, hits: null, unsupported: false });
}

/** Steps (wraps) through `total` matches; bumps the navigation version. */
export function stepFind(total: number, delta: 1 | -1): void {
  if (total <= 0) return;
  setState((s) => ({ ...s, active: (s.active + delta + total) % total, navVersion: s.navVersion + 1 }));
}

export function gotoFindMatch(at: number): void {
  setState((s) => ({ ...s, active: Math.max(0, at), navVersion: s.navVersion + 1 }));
}

/** Scrolls the active match's turn into view (positional jump: detach). */
export function revealActiveMatch(index: FindIndex): { turnId: string; occurrence: number } | undefined {
  const target = locateMatch(index, state().active);
  if (target) adapter?.jumpToTurn(target.turnId);
  return target;
}

/** "Search whole session": the host `search` command (null → unsupported). */
export async function searchWholeSession(session: SessionHandle, query: string): Promise<void> {
  const q = query.trim();
  if (!q) return;
  setWhole({ busy: true, hits: null, unsupported: false });
  try {
    const reply = await session.cmd<unknown>({ type: 'search', query: q });
    const hits = parseSearchHits(reply && typeof reply === 'object' && 'hits' in reply ? (reply as { hits: unknown }).hits : reply);
    setWhole(hits === null ? { busy: false, hits: null, unsupported: true } : { busy: false, hits, unsupported: false });
  } catch {
    setWhole({ busy: false, hits: null, unsupported: true });
  }
}

/** A whole-session hit → its turn: jump the branch to it when it isn't current. */
export async function jumpToHit(session: SessionHandle, hit: SearchHit, turnOf: (entryId: string) => string | undefined, jump: (turnId: string) => void): Promise<void> {
  let turnId = turnOf(hit.id);
  if (!turnId) {
    await session.cmd({ type: '_oino/navigate_tree', targetId: hit.id }).catch(() => undefined);
    turnId = turnOf(hit.id);
  }
  if (turnId) jump(turnId);
}
