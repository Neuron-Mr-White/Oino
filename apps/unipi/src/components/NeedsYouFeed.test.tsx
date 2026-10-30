// @vitest-environment jsdom
/**
 * The hub-fed needs-you source (WP-A19, Z-83/Z-86): `session_changed`
 * transitions (a wait rising, a run finishing) become drawer items; a wait
 * that ends or a run that restarts removes them; a completion watched in
 * this window is marked unread when the window blurs right after, and
 * cleared again on return (`session_focus`).
 */
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { createMockHost, type MockScenario } from '../dev/mock/host';
import { setHostClientFactory } from '../lib/host/factory';
import { resetHubs } from '../lib/session/hub';
import { resetNeedsYou, needsYou } from '../lib/needsYou';
import type { SessionCard } from '../lib/host/client';
import { NeedsYouFeed } from './NeedsYouBell';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: 1000,
  ...over,
});

let world: ReturnType<typeof createMockHost>;
const boot = (cards: SessionCard[], hub: NonNullable<MockScenario['hosts'][number]['hub']>, hash = '#/') => {
  const scenario: MockScenario = {
    name: 'feed',
    hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards, hub }],
  };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = hash;
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => <NeedsYouFeed />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const log = () => world.log.filter((e) => e.kind === 'request').map((e) => `${e.op}:${JSON.stringify(e.args)}`);

beforeEach(() => {
  localStorage.clear();
  resetNeedsYou();
});
afterEach(() => {
  cleanup();
  window.dispatchEvent(new Event('focus'));
  setHostClientFactory(undefined);
  resetHubs();
});

describe('NeedsYouFeed', () => {
  it('cold cards seed (no items), then a rising wait becomes a Waiting item (Z-83)', async () => {
    const idle = card('w2', { run_state: 'idle' });
    boot(
      [card('w1', { run_state: 'working' }), idle],
      [{ at: 20, event: { kind: 'session_changed', card: { ...idle, needs_you: true, updated_at: 2000 } } }],
    );
    await waitFor(() => expect(needsYou().list()).toHaveLength(1));
    const item = needsYou().list()[0]!;
    expect(item).toMatchObject({ session: 'w2', hostName: 'workstation', group: 'waiting', kind: 'input_needed' });
    expect(needsYou().unreadCount()).toBe(1);
  });

  it('a working → idle transition becomes a Finished item; the wait ending removes it', async () => {
    const working = card('w1', { run_state: 'working', updated_at: 1000 });
    boot([working], [{ at: 20, event: { kind: 'session_changed', card: { ...working, run_state: 'idle', updated_at: 3000 } } }]);
    await waitFor(() => expect(needsYou().list()).toHaveLength(1));
    expect(needsYou().list()[0]).toMatchObject({ kind: 'agent_end', group: 'finished' });

    // The wait ends (or a new run starts): the item leaves the drawer.
    world.client('this-pc').emit({ kind: 'session_changed', card: { ...working, run_state: 'working', updated_at: 4000 } });
    await waitFor(() => expect(needsYou().list()).toHaveLength(0));
  });

  it('Z-86: a completion watched here is marked unread on blur, cleared on return', async () => {
    const hasFocus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const working = card('w1', { run_state: 'working', updated_at: 1000 });
    boot([working], [], '#/s/this-pc/w1');
    // The hub stream is up and the cold list seeded before the run finishes.
    await waitFor(() => expect(world.log.some((e) => e.op === 'sessions_list')).toBe(true));
    world.client('this-pc').emit({ kind: 'session_changed', card: { ...working, run_state: 'idle', updated_at: 2000 } });
    await waitFor(() => expect(needsYou().list()).toHaveLength(1));
    // Minimizing right after: the session is marked unread (persisted meta).
    window.dispatchEvent(new Event('blur'));
    await waitFor(() => expect(log().some((l) => l.startsWith('session_meta_set:') && l.includes('"unread":true'))).toBe(true));
    // Coming back: focus is re-reported and the host clears the dot.
    window.dispatchEvent(new Event('focus'));
    await waitFor(() => expect(log().filter((l) => l.startsWith('session_focus:')).at(-1)).toBe('session_focus:{"session":"w1"}'));
    hasFocus.mockRestore();
  });
});
