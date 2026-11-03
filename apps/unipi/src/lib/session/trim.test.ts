import { describe, expect, it } from 'vitest';
import { initialState, prependEntries, trimHistory } from './reducer';
import { createTurnsIndex } from './turns';
import type { SessionEntry } from './types';

/** `n` turns of user + assistant, chained by parentId. */
function chat(n: number, from = 0): SessionEntry[] {
  const out: SessionEntry[] = [];
  let parent: string | undefined = from ? `a${from - 1}` : undefined;
  for (let i = from; i < from + n; i++) {
    out.push({ type: 'message', id: `u${i}`, parentId: parent, timestamp: '2026-11-03T00:00:00Z', message: { role: 'user', content: `q${i}` } } as SessionEntry);
    out.push({ type: 'message', id: `a${i}`, parentId: `u${i}`, timestamp: '2026-11-03T00:00:00Z', message: { role: 'assistant', content: [{ type: 'text', text: `r${i}` }] } } as SessionEntry);
    parent = `a${i}`;
  }
  return out;
}

function stateWith(entries: SessionEntry[], history = true) {
  const s = initialState('h', 's');
  s.entries = entries;
  s.byId = Object.fromEntries(entries.map((e, i) => [e.id, i]));
  s.leafId = entries.at(-1)?.id ?? null;
  if (history) s.history = { total: 10_000, hasMore: false };
  return s;
}

describe('trimHistory (perf-app)', () => {
  it('keeps the newest entries cut at a user message and makes the cut the cursor', () => {
    const s = stateWith(chat(500));
    const rev = s.entriesRev;
    expect(trimHistory(s, 101)).toBe(true);
    expect(s.entries.length).toBe(100);
    expect(s.entries[0]!.id).toBe('u450');
    expect(s.history).toEqual({ total: 10_000, hasMore: true, cursor: 'u450' });
    expect(s.byId.u450).toBe(0);
    expect(s.entriesRev).toBe(rev + 1);
    // The turns index rebuilds from the kept tail (first entry's parent is gone).
    const turns = createTurnsIndex().update(s);
    expect(turns.length).toBe(50);
    expect(turns.at(-1)!.entries.at(-1)!.id).toBe('a499');
  });

  it('round-trips: an older page prepends again after a trim', () => {
    const s = stateWith(chat(300));
    trimHistory(s, 100);
    prependEntries(s, { entries: chat(250).slice(400, 500 - 0), total: 10_000, hasMore: true, cursor: 'u200' });
    expect(s.entries[0]!.id).toBe('u200');
    expect(new Set(s.entries.map((e) => e.id)).size).toBe(s.entries.length);
  });

  it('never trims an unpaged chat, a short one or while a page loads', () => {
    expect(trimHistory(stateWith(chat(500), false), 100)).toBe(false);
    expect(trimHistory(stateWith(chat(10)), 100)).toBe(false);
    const s = stateWith(chat(500));
    s.history!.loading = true;
    expect(trimHistory(s, 100)).toBe(false);
  });
});
