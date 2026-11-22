// @vitest-environment jsdom
/**
 * The sidebar header's refresh button (owner, 2026-11-13: "the reload button is missing from the
 * header in the sidebar"): it sits between the bell and "Hide sidebar", and one click re-lists the
 * host's sessions without reloading the app.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { genSession } from '../../dev/mock/gen';
import { setHostClientFactory } from '../../lib/host/factory';
import { setPlatformOverride } from '../../lib/platform';
import { hubs, resetHubs } from '../../lib/session/hub';
import { resetHostCollapse } from '../sidebar/HostGroup';
import { resetListViews } from '../../lib/session/listView';
import type { SessionCard } from '../../lib/host/client';
import { Sidebar } from './Sidebar';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, name: string): SessionCard => ({ session_id: id, name, cwd: '/home/you/proj', run_state: 'idle', updated_at: Date.now() - 90_000 });

beforeEach(() => {
  localStorage.clear();
  setPlatformOverride('tauri-desktop');
  resetHostCollapse();
});
afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetListViews();
});

describe('sidebar header refresh', () => {
  it('sits in the header beside the bell and re-lists the host sessions on click', async () => {
    const cards = [card('a', 'First chat')];
    const scenario: MockScenario = {
      name: 't',
      hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards, sessions: { a: genSession('a'), b: genSession('b') } }],
    };
    setHostClientFactory(createMockHost(scenario).client);
    location.hash = '#/';
    render(() => (
      <SettingsProvider>
        <HostsProvider>
          <HashRouter>
            <Route path="*" component={() => <Sidebar />} />
          </HashRouter>
        </HostsProvider>
      </SettingsProvider>
    ));
    await waitFor(() => expect(screen.getByTestId('srow-a')).toBeTruthy());

    const header = document.querySelector('.sb-top') as HTMLElement;
    const button = within(header).getByTestId('shell-refresh-all');
    expect(button.getAttribute('aria-label')).toBe('Refresh everything');
    expect(button.getAttribute('title')).toBe('Refresh everything');
    expect(button.classList.contains('shell-icon-btn')).toBe(true);
    // Between the bell and the hide-sidebar button, like the other header icons.
    const order = [...header.querySelectorAll('button')].map((el) => el.getAttribute('data-testid') ?? el.getAttribute('aria-label'));
    expect(order.indexOf('shell-refresh-all')).toBe(order.indexOf('shell-needs-you') + 1);
    expect(order.at(-1)).toBe('Hide sidebar');

    // A chat created elsewhere (another window / the CLI) shows up after one click, no app reload.
    cards.push(card('b', 'Second chat'));
    expect(screen.queryByTestId('srow-b')).toBeNull();
    fireEvent.click(button);
    await waitFor(() => expect(screen.getByTestId('srow-b')).toBeTruthy());
    expect(hubs().ensure('this-pc').state.cards.b).toBeTruthy();
    await waitFor(() => expect(button.hasAttribute('disabled')).toBe(false));
  });
});
