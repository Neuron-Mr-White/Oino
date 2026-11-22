// @vitest-environment jsdom
/**
 * Session actions (WP-A5): deleting a chat never starts a new one.
 *
 * [owner-fix-8] Deleting the chat that is on screen used to land on a new
 * chat: the route still pointed at the deleted session, the host answered
 * E_NOT_FOUND and the "this chat is gone" handler opened a draft. The app now
 * moves to the next chat (the neighbour in the sidebar order); deleting any
 * other chat changes nothing.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../host/factory';
import { resetHubs, hubs } from './hub';
import { archiveMany, archiveSession, deleteSession, nextChatAfter } from './actions';
import { resetSessions } from './store';
import type { SessionCard } from '../host/client';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  file: `/home/you/.oino/s/${id}.jsonl`,
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
  ...over,
});

let world: ReturnType<typeof createMockHost>;

const boot = async (cards: SessionCard[]): Promise<void> => {
  const scenario: MockScenario = {
    name: 't',
    hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards }],
  };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  const hub = hubs().ensure('this-pc');
  await new Promise((resolve) => {
    const stop = setInterval(() => {
      if (hub.state.loaded) {
        clearInterval(stop);
        resolve(undefined);
      }
    }, 10);
  });
};

const hash = () => location.hash;

beforeEach(() => {
  localStorage.clear();
  location.hash = '#/';
});
afterEach(() => {
  setHostClientFactory(undefined);
  resetHubs();
  resetSessions();
  localStorage.clear();
});

describe('deleting a chat (owner-fix-8)', () => {
  it('leaves the app on the next chat when the chat on screen is deleted', async () => {
    await boot([card('s1'), card('s2'), card('s3')]);
    location.hash = '#/s/this-pc/s2';
    await deleteSession('this-pc', 's2');
    // the next chat in the sidebar order — never a new chat
    expect(hash()).toBe('#/s/this-pc/s3');
    expect(hash()).not.toContain('/new/');
  });

  it('stays where it is when another chat is deleted', async () => {
    await boot([card('s1'), card('s2'), card('s3')]);
    location.hash = '#/s/this-pc/s2';
    await deleteSession('this-pc', 's3');
    expect(hash()).toBe('#/s/this-pc/s2');
  });

  it('picks the chat above when the deleted one was the last', async () => {
    await boot([card('s1'), card('s2')]);
    location.hash = '#/s/this-pc/s2';
    await deleteSession('this-pc', 's2');
    expect(hash()).toBe('#/s/this-pc/s1');
  });

  it('names the next chat from the sidebar order', async () => {
    await boot([card('s1'), card('s2'), card('s3')]);
    expect(nextChatAfter('this-pc', 's2')).toEqual({ hostId: 'this-pc', sessionId: 's3' });
    expect(nextChatAfter('this-pc', 's3')).toEqual({ hostId: 'this-pc', sessionId: 's2' });
    expect(nextChatAfter('this-pc', 'gone')).toEqual({ hostId: 'this-pc', sessionId: 's1' });
  });

  it('leaves the shown chat BEFORE archiving so the mounted route cannot reopen or resurrect it', async () => {
    await boot([card('s1'), card('s2'), card('s3')]);
    location.hash = '#/s/this-pc/s2';
    await archiveSession('this-pc', 's2');
    expect(hash()).toBe('#/s/this-pc/s3');
    expect(hubs().peek('this-pc')?.state.cards.s2).toBeUndefined();
  });

  it('navigates to /new/<host> when archiving the only remaining chat on screen', async () => {
    await boot([card('only')]);
    location.hash = '#/s/this-pc/only';
    await archiveSession('this-pc', 'only');
    expect(hash()).toBe('#/new/this-pc');
    expect(hubs().peek('this-pc')?.state.cards.only).toBeUndefined();
  });

  it('archiveMany moves off a shown chat that is in the batch to the next unarchived chat', async () => {
    await boot([card('s1'), card('s2'), card('s3')]);
    location.hash = '#/s/this-pc/s2';
    await archiveMany('this-pc', ['s2', 's3']);
    expect(hash()).toBe('#/s/this-pc/s1');
  });
});
