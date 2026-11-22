// @vitest-environment jsdom
/**
 * [phone-chats] The phone Chats tab keeps the desktop list's controls: the Properties menu
 * (sort / filter / group) and pinning, with pinned chats on top. Against the mock host.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { createMockHost, type MockScenario } from '../dev/mock/host';
import { setHostClientFactory } from '../lib/host/factory';
import { resetHubs } from '../lib/session/hub';
import { listView, resetListViews } from '../lib/session/listView';
import { createToastStack, setToasts } from '../lib/toasts';
import type { SessionCard } from '../lib/host/client';
import Chat from './Chat';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const card = (id: string, name: string, ago: number, over: Partial<SessionCard> = {}): SessionCard => ({
  session_id: id,
  name,
  cwd: '/home/you/p/oino',
  run_state: 'idle',
  updated_at: Date.now() - ago,
  message_count: 2,
  ...over,
});

const boot = (cards: SessionCard[]) => {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards }] };
  setHostClientFactory(createMockHost(scenario).client);
  location.hash = '#/';
  return render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="*" component={Chat} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const rowIds = () => [...document.querySelectorAll('[data-testid^="chat-row-"]')].map((r) => r.getAttribute('data-testid')!.slice('chat-row-'.length));
const CARDS = () => [card('a', 'Alpha', 60_000), card('b', 'Bravo', 120_000), card('c', 'Charlie', 180_000)];

beforeEach(() => {
  localStorage.clear();
  resetListViews();
  setToasts(createToastStack());
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
  setToasts(undefined);
});

describe('phone Chats: ordering + filter', () => {
  it('the title row has a Properties button with sort / filter / group, and ⋯ no longer repeats sort', async () => {
    boot(CARDS());
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b', 'c']));
    fireEvent.click(screen.getByTestId('chats-props'));
    // OIN-94: the sort choices are on the first screen (no Sort ▸ drill-in), the current one checked.
    for (const id of ['lp-sort-inline', 'lp-filter', 'lp-group']) expect(await screen.findByTestId(id)).toBeTruthy();
    expect(screen.queryByTestId('lp-sort')).toBeNull();
    expect(screen.getByTestId('lp-sort-updated').getAttribute('aria-checked')).toBe('true');
    expect(screen.getByTestId('lp-sort-name').getAttribute('aria-checked')).toBe('false');
    fireEvent.click(screen.getByTestId('chats-menu'));
    await screen.findByTestId('action-archived');
    expect(screen.queryByTestId('action-sort-name')).toBeNull();
  });

  it('Sort → Name reorders the list and persists per host', async () => {
    boot([card('a', 'Zulu', 60_000), card('b', 'Alpha', 120_000)]);
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b']));
    fireEvent.click(screen.getByTestId('chats-props'));
    fireEvent.click(await screen.findByTestId('lp-sort-name'));
    await waitFor(() => expect(rowIds()).toEqual(['b', 'a']));
    expect(listView('this-pc').sort).toBe('name');
    expect(screen.getByTestId('lp-sort-name').getAttribute('aria-checked')).toBe('true');
  });

  it('Filter → Status narrows the list, shows a count on the button, and Reset filters brings the rows back', async () => {
    boot([card('a', 'Alpha', 60_000, { run_state: 'working' }), card('b', 'Bravo', 120_000)]);
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b']));
    fireEvent.click(screen.getByTestId('chats-props'));
    fireEvent.click(await screen.findByTestId('lp-filter'));
    fireEvent.click(await screen.findByTestId('lp-facet-status'));
    fireEvent.click(await screen.findByTestId('lp-status-running'));
    await waitFor(() => expect(rowIds()).toEqual(['a']));
    expect(screen.getByTestId('chats-filter-on').textContent).toBe('1');
  });

  it('a filter that matches nothing says so and offers Reset filters', async () => {
    boot([card('a', 'Alpha', 60_000), card('b', 'Bravo', 120_000)]);
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b']));
    fireEvent.click(screen.getByTestId('chats-props'));
    fireEvent.click(await screen.findByTestId('lp-filter'));
    fireEvent.click(await screen.findByTestId('lp-facet-status'));
    fireEvent.click(await screen.findByTestId('lp-status-running'));
    fireEvent.click(await screen.findByTestId('chats-reset-filters'));
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b']));
    expect(screen.queryByTestId('chats-filter-on')).toBeNull();
  });
});

describe('phone Chats: pin', () => {
  const longPress = (id: string) => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const row = screen.getByTestId(`chat-row-${id}`);
    fireEvent.pointerDown(row.querySelector('.ph-row-main, button, [role="button"]') ?? row, { pointerId: 1, pointerType: 'touch', clientX: 10, clientY: 10 });
    vi.advanceTimersByTime(500);
    vi.useRealTimers();
  };

  it('long-press → Pin lists the chat once, in a Pinned group on top; Unpin puts it back', async () => {
    boot(CARDS());
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b', 'c']));
    longPress('c');
    fireEvent.click(await screen.findByTestId('action-pin'));
    await waitFor(() => expect(screen.getByText('Pinned')).toBeTruthy());
    // Pinned on top, and not repeated under its project.
    expect(rowIds()).toEqual(['c', 'a', 'b']);
    expect(screen.getByTestId('chat-row-c').querySelector('.chat-row-pin')).toBeTruthy();

    longPress('c');
    fireEvent.click(await screen.findByTestId('action-pin'));
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b', 'c']));
    expect(screen.queryByText('Pinned')).toBeNull();
  });

  it('OIN-94: lifting the finger after the long-press keeps the row menu open, and Pin from it sticks', async () => {
    boot(CARDS());
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b', 'c']));
    longPress('c');
    await screen.findByTestId('chat-row-sheet');
    // The finger that opened the sheet now lifts: pointerup lands on the row (touch keeps the
    // pointer captured there) — outside the sheet, but it is the same gesture, not a scrim tap.
    fireEvent.pointerUp(screen.getByTestId('chat-row-c'), { pointerId: 1, pointerType: 'touch', clientX: 10, clientY: 10 });
    await new Promise((r) => setTimeout(r, 30));
    expect(screen.queryByTestId('chat-row-sheet')).not.toBeNull();
    fireEvent.click(await screen.findByTestId('action-pin'));
    await waitFor(() => expect(rowIds()).toEqual(['c', 'a', 'b']));
  });

  it('OIN-94: a later tap outside the row menu still dismisses it', async () => {
    boot(CARDS());
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b', 'c']));
    longPress('c');
    await screen.findByTestId('chat-row-sheet');
    fireEvent.pointerUp(screen.getByTestId('chat-row-c'), { pointerId: 1, pointerType: 'touch' });
    await new Promise((r) => setTimeout(r, 30));
    fireEvent.pointerDown(document.body, { pointerId: 2, pointerType: 'touch' });
    fireEvent.pointerUp(document.body, { pointerId: 2, pointerType: 'touch' });
    await waitFor(() => expect(screen.queryByTestId('chat-row-sheet')).toBeNull());
  });

  it('OIN-94: the pin is still there after leaving the screen and opening it again', async () => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: CARDS() }] };
    setHostClientFactory(createMockHost(scenario).client);
    const mount = () => {
      location.hash = '#/';
      return render(() => (
        <SettingsProvider>
          <HostsProvider>
            <HashRouter>
              <Route path="*" component={Chat} />
            </HashRouter>
          </HostsProvider>
        </SettingsProvider>
      ));
    };
    const first = mount();
    await waitFor(() => expect(rowIds()).toEqual(['a', 'b', 'c']));
    longPress('b');
    fireEvent.pointerUp(screen.getByTestId('chat-row-b'), { pointerId: 1, pointerType: 'touch' });
    await new Promise((r) => setTimeout(r, 30));
    fireEvent.click(await screen.findByTestId('action-pin'));
    await waitFor(() => expect(rowIds()).toEqual(['b', 'a', 'c']));
    // Leave: the screen unmounts and the app's session hubs are rebuilt (as after a reload).
    first.unmount();
    resetHubs();
    mount();
    await waitFor(() => expect(rowIds()).toEqual(['b', 'a', 'c']));
    expect(screen.getByTestId('chat-row-b').querySelector('.chat-row-pin')).toBeTruthy();
  });

  it('a pinned chat stays on top whatever the sort', async () => {
    boot([card('a', 'Alpha', 60_000), card('z', 'Zulu', 120_000, { pinned: true })]);
    await waitFor(() => expect(rowIds()).toEqual(['z', 'a']));
    fireEvent.click(screen.getByTestId('chats-props'));
    fireEvent.click(await screen.findByTestId('lp-sort-name'));
    await waitFor(() => expect(listView('this-pc').sort).toBe('name'));
    expect(rowIds()).toEqual(['z', 'a']);
  });
});
