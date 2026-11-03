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
  it('keeps at least the newest `keep` entries, cut back at the user message of that turn, and makes the cut the cursor', () => {
    const s = stateWith(chat(500));
    const rev = s.entriesRev;
    expect(trimHistory(s, 101)).toBe(true);
    // 1000 - 101 = a449 → back to u449 (render-loop: was forward to u450, fewer than `keep`).
    expect(s.entries.length).toBe(102);
    expect(s.entries[0]!.id).toBe('u449');
    expect(s.history).toEqual({ total: 10_000, hasMore: true, cursor: 'u449' });
    expect(s.byId.u449).toBe(0);
    expect(s.entriesRev).toBe(rev + 1);
    // The turns index rebuilds from the kept tail (first entry's parent is gone).
    const turns = createTurnsIndex().update(s);
    expect(turns.length).toBe(51);
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

  it('render-loop: one giant subagent turn + a short open-ended last turn keeps the giant turn (never a few-entry page)', () => {
    // The owner's 'Frontend Whitelabeling Strategy' shape: 1527 entries, the user messages
    // at 605 and 1512 — 900 subagent-step entries in one turn, 15 in the open-ended last one.
    const entries: SessionEntry[] = [];
    let parent: string | undefined;
    const push = (e: Partial<SessionEntry> & { id: string }) => {
      entries.push({ parentId: parent, timestamp: '2026-11-03T00:00:00Z', ...e } as SessionEntry);
      parent = e.id;
    };
    for (let i = 0; i < 1527; i++) {
      if (i === 0 || i === 605 || i === 1512) push({ type: 'message', id: `u${i}`, message: { role: 'user', content: `q${i}` } } as never);
      else push({ type: 'custom', id: `c${i}`, customType: 'subagent-step', data: { kind: 'text', agentId: 'x' } } as never);
    }
    const s = stateWith(entries);
    expect(trimHistory(s, 600)).toBe(true);
    // The old forward cut kept 15 entries (u1512…): a short page the timeline paged straight back in.
    expect(s.entries[0]!.id).toBe('u605');
    expect(s.entries.length).toBe(922);
    // Below LOADED_MAX now: the next trim at the bottom does nothing.
    expect(s.entries.length).toBeLessThan(1200);
  });

  it('render-loop: a kept turn longer than `keep` falls back to the forward cut (bounded)', () => {
    const entries: SessionEntry[] = [];
    let parent: string | undefined;
    for (let i = 0; i < 3000; i++) {
      const id = i === 0 || i === 2990 ? `u${i}` : `c${i}`;
      entries.push({ type: i === 0 || i === 2990 ? 'message' : 'custom', id, parentId: parent, timestamp: '2026-11-03T00:00:00Z', message: { role: 'user', content: 'q' } } as SessionEntry);
      parent = id;
    }
    const s = stateWith(entries);
    expect(trimHistory(s, 160)).toBe(true);
    expect(s.entries[0]!.id).toBe('u2990');
    expect(s.entries.length).toBe(10);
  });
});
