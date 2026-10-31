// @vitest-environment jsdom
/**
 * UNI-226 (ported from v1's ChatHeaderMenu/MessageMenu to the WP-A5 row menu):
 * menus mounted the way the app mounts them — reading state after
 * `onDismiss()` unmounts them — must still run the picked item. Reading a
 * prop after the dismiss used to throw "Stale read from <Show>", so every
 * row of the session menu did nothing while the icon action bar worked.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { createMockHost, type MockScenario } from '../dev/mock/host';
import { setHostClientFactory } from '../lib/host/factory';
import { resetHubs } from '../lib/session/hub';
import { openRowMenu, openRowMenuState } from '../lib/session/actions';
import type { SessionCard } from '../lib/host/client';
import { SessionActions } from './session/SessionActions';
import { SessionList } from './sidebar/SessionList';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
});

let world: ReturnType<typeof createMockHost>;
const boot = () => {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: [card('a')] }] };
  world = createMockHost(scenario);
  setHostClientFactory(world.client);
  location.hash = '#/';
  render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={() => (<><SessionList variant="sidebar" /><SessionActions /></>)} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('the row menu inside <Show> (UNI-226)', () => {
  it('Pin still sends its op after the menu dismisses', async () => {
    boot();
    const row = await waitFor(() => screen.getByTestId('srow-a'));
    fireEvent.contextMenu(row);
    await waitFor(() => expect(screen.getByRole('menu')).toBeTruthy());
    fireEvent.click(screen.getByRole('menuitem', { name: 'Pin' }));
    // The menu unmounted on pick; the op must have gone out anyway.
    expect(screen.queryByRole('menu')).toBeNull();
    await waitFor(() =>
      expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_meta_set' && (e.args as { session?: string; pinned?: boolean })?.session === 'a' && (e.args as { pinned?: boolean }).pinned === true)).toBe(true),
    );
  });

  it('opening the same row again after a dismiss works (fresh state, no stale read)', async () => {
    boot();
    await waitFor(() => screen.getByTestId('srow-a'));
    fireEvent.contextMenu(screen.getByTestId('srow-a'));
    await waitFor(() => expect(screen.getByRole('menu')).toBeTruthy());
    fireEvent.click(screen.getByRole('menuitem', { name: 'Pin' }));
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    // Pinning moved the row into the Pinned group — re-query it.
    fireEvent.contextMenu(screen.getByTestId('srow-a'));
    await waitFor(() => expect(screen.getByRole('menu')).toBeTruthy());
    expect(openRowMenuState()?.card.session_id).toBe('a');
    fireEvent.click(screen.getByRole('menuitem', { name: 'Unpin' }));
    await waitFor(() =>
      expect(world.log.some((e) => e.kind === 'request' && e.op === 'session_meta_set' && (e.args as { pinned?: boolean })?.pinned === false)).toBe(true),
    );
  });

  it('openRowMenu outside a render does not throw (the module signal is not Show-scoped)', () => {
    boot();
    const rect = new DOMRect(4, 8, 120, 36);
    expect(() => openRowMenu('this-pc', card('zz'), rect)).not.toThrow();
    expect(openRowMenuState()?.card.session_id).toBe('zz');
  });
});
