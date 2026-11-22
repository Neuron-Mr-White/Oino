/**
 * perf-open: a big session opens at its tail (snapshot `page`, `waking`), older history
 * pages in with `get_entries{before: cursor}` ("load earlier"), and `{kind:"woken"}`
 * replaces the provisional state once the runtime is up.
 */
import { describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost, type MockScenario } from '../host/mock';
import type { RpcCommand, SessionSnapshot } from '../host/client';
import { createSession, type SessionHandle } from './store';
import { applyEvent, applySnapshot, initialState, prependEntries } from './reducer';
import type { SessionEntry } from './types';

/** `n` turns (user + assistant) with ids u<i>/a<i>, linear. */
function conversation(from: number, to: number): SessionEntry[] {
  const out: SessionEntry[] = [];
  for (let i = from; i < to; i++) {
    const parentU = i === 0 ? null : `a${i - 1}`;
    out.push({ type: 'message', id: `u${i}`, parentId: parentU, timestamp: '2026-11-02T00:00:00.000Z', message: { role: 'user', content: `q${i}`, timestamp: i } } as SessionEntry);
    out.push({ type: 'message', id: `a${i}`, parentId: `u${i}`, timestamp: '2026-11-02T00:00:01.000Z', message: { role: 'assistant', content: [{ type: 'text', text: `a${i}` }], timestamp: i } } as SessionEntry);
  }
  return out;
}

function pagedSnapshot(): SessionSnapshot {
  return {
    session_id: 's1',
    reset: true,
    seq: 0,
    state: { sessionId: 's1', isStreaming: false, model: { provider: 'p', id: 'm', name: 'm' } },
    entries: conversation(8, 10),
    leaf_id: 'a9',
    page: { total: 20, has_more: true, cursor: 'u8' },
    waking: true,
  } as SessionSnapshot;
}

describe('reducer: paged history', () => {
  it('a paged snapshot records total / has-more / cursor and the waking flag', () => {
    const s = initialState('h1', 's1');
    applySnapshot(s, pagedSnapshot());
    expect(s.entries.map((e) => e.id)).toEqual(['u8', 'a8', 'u9', 'a9']);
    expect(s.history).toEqual({ total: 20, hasMore: true, cursor: 'u8' });
    expect(s.waking).toBe(true);
    // A later full snapshot without a page (a small session) drops both.
    applySnapshot(s, { session_id: 's1', reset: true, seq: 0, entries: conversation(0, 1), leaf_id: 'a0' } as SessionSnapshot);
    expect(s.history).toBeUndefined();
    expect(s.waking).toBeUndefined();
  });

  it('prepends an older page once, keeping order and the new cursor', () => {
    const s = initialState('h1', 's1');
    applySnapshot(s, pagedSnapshot());
    const rev = s.entriesRev;
    prependEntries(s, { entries: conversation(5, 9), total: 20, hasMore: true, cursor: 'u5' });
    expect(s.entries.map((e) => e.id)).toEqual(['u5', 'a5', 'u6', 'a6', 'u7', 'a7', 'u8', 'a8', 'u9', 'a9']);
    expect(s.byId.u8).toBe(6);
    expect(s.entriesRev).toBe(rev + 1);
    expect(s.history).toEqual({ total: 20, hasMore: true, cursor: 'u5' });
    prependEntries(s, { entries: conversation(0, 5), total: 20, hasMore: false });
    expect(s.entries).toHaveLength(20);
    expect(s.history).toEqual({ total: 20, hasMore: false });
  });

  it('woken replaces the provisional state; a leaf past ours asks for a catch-up', () => {
    const s = initialState('h1', 's1');
    applySnapshot(s, pagedSnapshot());
    const r = applyEvent(s, {
      session: 's1',
      seq: 1,
      event: {
        kind: 'woken',
        state: { sessionId: 's1', isStreaming: false, sessionName: 'Big chat', thinkingLevel: 'high', model: { provider: 'p', id: 'm', name: 'Model M', contextWindow: 1 } },
        commands: [{ name: 'plan' }],
        ui: { rev: 3 },
        topics: [],
        leafId: 'x1',
      },
    });
    expect(s.waking).toBeUndefined();
    expect(s.name).toBe('Big chat');
    expect(s.model?.name).toBe('Model M');
    expect(s.thinkingLevel).toBe('high');
    expect(s.commands).toEqual([{ name: 'plan' }]);
    expect(r.catchUp).toBe('a9');
    expect(r.ui?.[0]).toMatchObject({ type: 'oino_ui_snapshot', rev: 3 });
    // The same leaf: nothing to catch up.
    const s2 = initialState('h1', 's1');
    applySnapshot(s2, pagedSnapshot());
    expect(applyEvent(s2, { session: 's1', seq: 1, event: { kind: 'woken', leafId: 'a9' } }).catchUp).toBeUndefined();
  });

  it('wake_failed shows a banner', () => {
    const s = initialState('h1', 's1');
    applySnapshot(s, pagedSnapshot());
    applyEvent(s, { session: 's1', seq: 1, event: { kind: 'wake_failed', error: 'no model' } });
    expect(s.banner?.kind).toBe('wake_failed');
    expect(s.banner?.text).toContain('no model');
    expect(s.waking).toBeUndefined();
  });
});

describe('store: load earlier', () => {
  const tick = () => new Promise((r) => setTimeout(r, 0));

  function open(getEntries: (cmd: RpcCommand) => unknown): { s: SessionHandle; asked: RpcCommand[]; dispose: () => void } {
    const asked: RpcCommand[] = [];
    const scenario: MockScenario = {
      name: 'paged',
      hosts: [
        {
          id: 'h1',
          name: 'pc',
          sessions: {
            s1: {
              snapshot: pagedSnapshot(),
              cmd: {
                get_entries: (cmd) => {
                  asked.push(cmd);
                  return getEntries(cmd);
                },
              },
            },
          },
        },
      ],
    };
    const world = createMockHost(scenario);
    return createRoot((dispose) => ({ s: createSession('h1', 's1', { client: world.client('h1') }), asked, dispose }));
  }

  it('fetches the page before the cursor and prepends it from its oldest KNOWN turn start (chat-load: whole turns only)', async () => {
    const { s, asked, dispose } = open(() => ({ entries: conversation(4, 8), total: 20, hasMore: true, cursor: 'u4' }));
    for (let i = 0; i < 5 && s.state.conn !== 'live'; i++) await tick();
    expect(s.state.conn).toBe('live');
    expect(s.state.history?.cursor).toBe('u8');
    await s.loadEarlier!();
    expect(asked).toEqual([{ type: 'get_entries', before: 'u8', limit: 200 }]);
    // u4 opens the page, but what precedes it is unknown (a steer looks the same): the chunk
    // starts at u5 (its previous message a4 is loaded) and u4/a4 come with the next chunk.
    expect(s.state.entries[0]!.id).toBe('u5');
    expect(s.state.entries).toHaveLength(10);
    expect(s.state.history).toMatchObject({ total: 20, hasMore: true, cursor: 'u5' });
    expect(s.state.history?.loading).toBeFalsy();
    // Nothing older: a page with hasMore false ends the paging.
    dispose();
  });

  it('a failed fetch leaves the history and shows an error until retried', async () => {
    let fail = true;
    const { s, asked, dispose } = open(() => {
      if (fail) throw new Error('boom');
      return { entries: conversation(0, 8), total: 20, hasMore: false };
    });
    for (let i = 0; i < 5 && s.state.conn !== 'live'; i++) await tick();
    await s.loadEarlier!();
    expect(s.state.entries).toHaveLength(4);
    expect(s.state.history?.error).toBeTruthy();
    expect(s.state.history?.loading).toBe(false);
    fail = false;
    await s.loadEarlier!();
    expect(asked).toHaveLength(2);
    expect(s.state.entries).toHaveLength(20);
    expect(s.state.history).toMatchObject({ hasMore: false });
    expect(s.state.history?.error).toBeUndefined();
    // No more pages: further calls do not ask.
    await s.loadEarlier!();
    expect(asked).toHaveLength(2);
    dispose();
  });
});
