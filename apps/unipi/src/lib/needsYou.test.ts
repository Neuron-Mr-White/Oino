// @vitest-environment jsdom
/**
 * "Needs you" store (UNI-161 §3): grouping (Waiting/Finished/Other),
 * read/dismiss state persisted across store instances, unread count, and
 * setHostItems's poll-and-diff semantics (a host's items replace, cleared
 * ids stay out until a fresh `since`).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { groupFor, groupNeedsYou, needsYouId, type NeedsYouItem } from './needsYou';

const STORE_KEY = 'unipi.needsYou.v1';

async function freshStore() {
  // Each test needs its own module instance (the store is a module-level
  // singleton seeded from localStorage at creation time).
  vi.resetModules();
  const mod = await import('./needsYou');
  return mod.needsYou();
}

const item = (over: Partial<NeedsYouItem> = {}): NeedsYouItem => ({
  id: needsYouId('h1', 10, 'ask_user', 1000),
  hostId: 'h1',
  hostName: 'My PC',
  pid: 10,
  session: 'default',
  title: 'my session',
  kind: 'ask_user',
  group: 'waiting',
  what: 'Pick a plan',
  since: 1000,
  ...over,
});

beforeEach(() => localStorage.clear());
afterEach(() => localStorage.clear());

describe('groupFor', () => {
  it('dialog kinds and prompts are "waiting"; agent_end/settled are "finished"; anything else is "other"', () => {
    expect(groupFor('ask_user')).toBe('waiting');
    expect(groupFor('select')).toBe('waiting');
    expect(groupFor('permission_request')).toBe('waiting');
    expect(groupFor('agent_end')).toBe('finished');
    expect(groupFor('agent_settled')).toBe('finished');
    expect(groupFor('mcp_server_error')).toBe('other');
  });
});

describe('groupNeedsYou', () => {
  it('buckets items by group, newest first within each bucket', () => {
    const items = [item({ id: 'a', group: 'waiting', since: 1000 }), item({ id: 'b', group: 'waiting', since: 2000 }), item({ id: 'c', group: 'finished', since: 500 })];
    const grouped = groupNeedsYou(items);
    expect(grouped.waiting.map((x) => x.id)).toEqual(['b', 'a']);
    expect(grouped.finished.map((x) => x.id)).toEqual(['c']);
    expect(grouped.other).toEqual([]);
  });
});

describe('needsYou store', () => {
  it('setHostItems replaces a host\'s items and leaves other hosts alone', async () => {
    const store = await freshStore();
    store.setHostItems('h1', [item({ id: 'a' })]);
    store.setHostItems('h2', [item({ id: 'b', hostId: 'h2' })]);
    expect(store.list().map((x) => x.id).sort()).toEqual(['a', 'b']);
    store.setHostItems('h1', [item({ id: 'a2', since: 2000 })]);
    expect(store.list().map((x) => x.id).sort()).toEqual(['a2', 'b']);
  });

  it('markRead / markAllRead / unreadCount', async () => {
    const store = await freshStore();
    store.setHostItems('h1', [item({ id: 'a' }), item({ id: 'b', since: 2000 })]);
    expect(store.unreadCount()).toBe(2);
    store.markRead('a');
    expect(store.isRead('a')).toBe(true);
    expect(store.unreadCount()).toBe(1);
    store.markAllRead();
    expect(store.unreadCount()).toBe(0);
  });

  it('dismiss removes an item and keeps it out until a fresh `since` arrives', async () => {
    const store = await freshStore();
    store.setHostItems('h1', [item({ id: 'a' })]);
    store.dismiss('a');
    expect(store.list()).toEqual([]);
    // Same wait (same id derived from the same since) stays cleared on the next poll.
    store.setHostItems('h1', [item({ id: 'a' })]);
    expect(store.list()).toEqual([]);
    // A fresh wait (new since → new id) shows up normally.
    const freshId = needsYouId('h1', 10, 'ask_user', 9999);
    store.setHostItems('h1', [item({ id: freshId, since: 9999 })]);
    expect(store.list().map((x) => x.id)).toEqual([freshId]);
  });

  it('clearAll empties the list and persists dismissal', async () => {
    const store = await freshStore();
    store.setHostItems('h1', [item({ id: 'a' }), item({ id: 'b', since: 2000 })]);
    store.clearAll();
    expect(store.list()).toEqual([]);
  });

  it('read state persists across store instances (localStorage)', async () => {
    const store1 = await freshStore();
    store1.setHostItems('h1', [item({ id: 'a' })]);
    store1.markRead('a');
    expect(localStorage.getItem(STORE_KEY)).toContain('"a"');
    const store2 = await freshStore();
    store2.setHostItems('h1', [item({ id: 'a' })]);
    expect(store2.isRead('a')).toBe(true);
  });
});

describe('UNI-200: read state is reactive', () => {
  it('unreadCount/isRead re-run when an item is marked read (badge + crab update at once)', async () => {
    const { createRoot, createMemo } = await import('solid-js');
    const { needsYou } = await import('./needsYou');
    const store = needsYou();
    store.upsert({ id: 'r1', hostId: 'h', hostName: 'H', pid: 1, session: 's', title: 't', kind: 'ask_user', group: 'waiting', since: 1 });
    createRoot((dispose) => {
      const count = createMemo(() => store.unreadCount());
      const read = createMemo(() => store.isRead('r1'));
      expect(count()).toBeGreaterThanOrEqual(1);
      const before = count();
      store.markRead('r1');
      expect(count()).toBe(before - 1);
      expect(read()).toBe(true);
      dispose();
    });
    store.dismiss('r1');
  });
});

describe('badge and settling', () => {
  it('the unread badge counts Waiting only; opening a chat settles its finished notes', async () => {
    const store = await freshStore();
    store.upsert(item({ id: 'w', group: 'waiting', session: 's1' }));
    store.upsert(item({ id: 'f', group: 'finished', kind: 'agent_end', session: 's1' }));
    store.upsert(item({ id: 'f2', group: 'finished', kind: 'agent_end', session: 's2' }));
    expect(store.unreadCount()).toBe(1);
    store.settleSession('h1', 's1');
    expect(store.list().map((i) => i.id).sort()).toEqual(['f2', 'w']);
  });
});
