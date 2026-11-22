/**
 * [chat-tree] Tree jumps (`_oino/navigate_tree`): the store reloads the new branch's tail
 * page after its own jump, and a `session_tree` notice from the host (another window or the
 * phone moved the leaf) reloads it too — once, never when the view already shows that leaf.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createMockHost, type MockScenario, type MockSession, type MockWorld } from '../host/mock';
import type { RpcCommand, SessionSnapshot } from '../host/client';
import { createSession, BRANCH_PAGE, type SessionHandle } from './store';
import { initialState, replaceBranch, applyEvents } from './reducer';
import type { SessionEntry } from './types';

const msg = (id: string, parentId: string | null, role: 'user' | 'assistant', text: string, at: number): SessionEntry =>
  ({
    type: 'message',
    id,
    parentId,
    timestamp: new Date(1_793_350_800_000 + at * 1000).toISOString(),
    message:
      role === 'user'
        ? { role, content: [{ type: 'text', text }], timestamp: 1_793_350_800_000 + at * 1000 }
        : { role, content: [{ type: 'text', text }], stopReason: 'stop', timestamp: 1_793_350_800_000 + at * 1000, usage: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0 } },
  }) as SessionEntry;

const U1 = msg('u1', null, 'user', 'first question', 1);
const A1 = msg('a1', 'u1', 'assistant', 'first answer', 2);
const U2 = msg('u2', 'a1', 'user', 'second question', 3);
const A2 = msg('a2', 'u2', 'assistant', 'second answer', 4);

function snapshot(): SessionSnapshot {
  return {
    session_id: 's1',
    reset: true,
    seq: 5,
    state: { sessionId: 's1', isStreaming: false, isCompacting: false },
    leaf_id: 'a2',
    entries: [U1, A1, U2, A2],
    commands: [],
  } as unknown as SessionSnapshot;
}

type Log = { navigations: RpcCommand[]; pages: RpcCommand[] };

function world(log: Log, leaf: { id: string | null }): { w: MockWorld; session: MockSession; nextSeq: () => number } {
  const branchTo = (id: string | null) => {
    const all = [U1, A1, U2, A2];
    if (!id) return [];
    const at = all.findIndex((e) => e.id === id);
    return all.slice(0, at + 1);
  };
  let w: MockWorld | undefined;
  let seq = 5;
  const session: MockSession = {
    snapshot: snapshot(),
    cmd: {
      '_oino/navigate_tree': (cmd) => {
        log.navigations.push(cmd);
        const target = String(cmd.targetId);
        const entry = [U1, A1, U2, A2].find((e) => e.id === target)!;
        leaf.id = entry.message?.role === 'user' ? (entry.parentId ?? null) : entry.id;
        w!.push('h1', 's1', [{ session: 's1', seq: ++seq, event: { type: 'session_tree', leafId: leaf.id } }]);
        return entry.message?.role === 'user' ? { cancelled: false, editorText: 'second question' } : { cancelled: false };
      },
      get_entries: (cmd) => {
        log.pages.push(cmd);
        const entries = branchTo(leaf.id);
        return { entries, leafId: leaf.id, total: entries.length, hasMore: false, cursor: entries[0]?.id ?? null };
      },
    },
  };
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: session } }] };
  w = createMockHost(scenario);
  return { w, session, nextSeq: () => ++seq };
}

const tick = () => new Promise((r) => setTimeout(r, 0));
const frames: Array<() => void> = [];
const frame = () => {
  for (const f of frames.splice(0)) f();
};

let disposers: Array<() => void> = [];
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
});

function open(w: MockWorld): SessionHandle {
  return createRoot((dispose) => {
    const s = createSession('h1', 's1', { client: w.client('h1'), schedule: (f) => void frames.push(f) });
    disposers.push(() => (s.close(), dispose()));
    return s;
  });
}

describe('tree jumps (chat-tree)', () => {
  it('Return here: navigate moves the leaf and the timeline shows the new branch (one page load)', async () => {
    const log: Log = { navigations: [], pages: [] };
    const leaf = { id: 'a2' as string | null };
    const { w } = world(log, leaf);
    const s = open(w);
    await tick();
    expect(s.turns().map((t) => t.turnId)).toEqual(['u1', 'u2']);
    const reply = await s.navigate!('a1');
    expect(reply.cancelled).toBe(false);
    expect(s.state.leafId).toBe('a1');
    expect(s.turns().map((t) => t.turnId)).toEqual(['u1']);
    // The host's `session_tree` notice arrives too: it is ours, nothing reloads again.
    frame();
    await tick();
    frame();
    await tick();
    expect(log.pages).toHaveLength(1);
    expect(log.pages[0]).toMatchObject({ type: 'get_entries', limit: BRANCH_PAGE });
    expect(log.navigations[0]).toEqual({ type: '_oino/navigate_tree', targetId: 'a1' });
  });

  it('a user message lands on its parent and hands its text back; summarize rides along', async () => {
    const log: Log = { navigations: [], pages: [] };
    const leaf = { id: 'a2' as string | null };
    const { w } = world(log, leaf);
    const s = open(w);
    await tick();
    const reply = await s.navigate!('u2', { summarize: true });
    expect(reply.editorText).toBe('second question');
    expect(log.navigations[0]).toEqual({ type: '_oino/navigate_tree', targetId: 'u2', summarize: true });
    expect(s.state.leafId).toBe('a1');
    expect(s.state.entries.map((e) => e.id)).toEqual(['u1', 'a1']);
    expect(s.state.branchRev).toBe(1);
  });

  it('another client moved the leaf: the notice reloads the branch once', async () => {
    const log: Log = { navigations: [], pages: [] };
    const leaf = { id: 'a2' as string | null };
    const { w, session, nextSeq } = world(log, leaf);
    const s = open(w);
    await tick();
    // The phone jumped back to the first answer.
    leaf.id = 'a1';
    w.push('h1', 's1', [{ session: 's1', seq: nextSeq(), event: { type: 'session_tree', leafId: 'a1' } }]);
    frame();
    await tick();
    await tick();
    expect(log.pages).toHaveLength(1);
    expect(s.turns().map((t) => t.turnId)).toEqual(['u1']);
    // A replayed notice for the leaf we already show does nothing.
    w.push('h1', 's1', [{ session: 's1', seq: nextSeq(), event: { type: 'session_tree', leafId: 'a1' } }]);
    frame();
    await tick();
    expect(log.pages).toHaveLength(1);
    void session;
  });
});

describe('replaceBranch', () => {
  it('keeps shared entries by identity, resets paging and bumps the revs', () => {
    const s = initialState('h1', 's1');
    s.entries = [U1, A1, U2, A2];
    s.byId = { u1: 0, a1: 1, u2: 2, a2: 3 };
    s.leafId = 'a2';
    s.history = { total: 400, hasMore: true, cursor: 'u1' };
    const before = s.entriesRev;
    replaceBranch(s, { entries: [structuredClone(U1), structuredClone(A1)], leafId: 'a1', total: 2, hasMore: false, cursor: 'u1' });
    expect(s.entries[0]).toBe(U1);
    expect(s.entries[1]).toBe(A1);
    expect(s.byId).toEqual({ u1: 0, a1: 1 });
    expect(s.leafId).toBe('a1');
    expect(s.history).toEqual({ total: 2, hasMore: false });
    expect(s.entriesRev).toBe(before + 1);
    expect(s.branchRev).toBe(1);
  });

  it('a jump before the first message leaves an empty branch (leaf null)', () => {
    const s = initialState('h1', 's1');
    s.entries = [U1, A1];
    s.byId = { u1: 0, a1: 1 };
    s.leafId = 'a1';
    replaceBranch(s, { entries: [], leafId: null, total: 0, hasMore: false, cursor: null });
    expect(s.entries).toEqual([]);
    expect(s.leafId).toBeNull();
  });

  it('`session_tree` is a branch notice, not a timeline row', () => {
    const s = initialState('h1', 's1');
    const r = applyEvents(s, [{ session: 's1', seq: 1, event: { type: 'session_tree', leafId: 'x' } }]);
    expect(r.branchMoved).toBe('x');
    const r2 = applyEvents(s, [{ session: 's1', seq: 2, event: { type: 'session_tree', leafId: null } }]);
    expect(r2.branchMoved).toBeNull();
  });
});
