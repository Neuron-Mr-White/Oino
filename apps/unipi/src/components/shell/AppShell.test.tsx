import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Root } from '../../main';
import { createMockHost } from '../../dev/mock/host';
import { streamBasic } from '../../dev/mock/scenarios/stream-basic';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetPanes } from '../../lib/panes';
import { registerPaneKind } from '../../lib/panes/kinds';
import { resetHubs } from '../../lib/session/hub';
import { registerChatAction } from './ConversationHeader';
import { runBinding } from './AppShell';
import { bindingForEvent, resetShortcuts } from '../../lib/shortcuts/registry';

// The desktop app's native listeners (quit sheet, tray, window title) have no Tauri here.
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-coffee';
const SESSION = streamBasic.hosts[0]!.cards![0]!.session_id;

function width(w: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: w });
}

function boot(hash: string, w = 1440) {
  width(w);
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  setHostClientFactory(createMockHost(streamBasic).client);
  location.hash = hash;
  return render(() => <Root />);
}

const loaded = () => waitFor(() => expect(screen.getByTestId('conv-title').textContent).not.toBe('Session'));

const key = (k: string, opts: Partial<KeyboardEventInit> & { code?: string } = {}) =>
  fireEvent.keyDown(window, { key: k, code: opts.code ?? `Key${k.toUpperCase()}`, ctrlKey: true, ...opts });

beforeEach(() => {
  localStorage.clear();
  // Native listeners (quit sheet, tray) look for Tauri's internals.
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  resetShortcuts();
  location.hash = '';
  width(1024);
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('shell keys (shortcut registry, Z-94)', () => {
  const k = (key: string, mods: Partial<KeyboardEvent> = {}) => bindingForEvent({ key, code: `Key${key.toUpperCase()}`, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });
  it('maps chords (Cmd or Ctrl)', () => {
    expect(k('b', { metaKey: true })).toEqual({ kind: 'app', id: 'shell.sidebar' });
    expect(k('j', { metaKey: true })).toEqual({ kind: 'app', id: 'shell.bottom' });
    expect(k('b', { metaKey: true, altKey: true })).toEqual({ kind: 'app', id: 'shell.side' });
    expect(k('f', { metaKey: true, shiftKey: true })).toEqual({ kind: 'app', id: 'shell.fullview' });
    expect(k('t', { metaKey: true, shiftKey: true })).toEqual({ kind: 'app', id: 'shell.reopen' });
    expect(k('k', { metaKey: true })).toEqual({ kind: 'app', id: 'shell.palette' });
    expect(k('p', { metaKey: true, shiftKey: true })).toEqual({ kind: 'app', id: 'shell.palette' });
    expect(k('p', { metaKey: true })).toEqual({ kind: 'app', id: 'shell.files' }); // Z-81
    expect(k('f', { metaKey: true })).toEqual({ kind: 'app', id: 'find.open' }); // Z-79
    expect(k('1', { metaKey: true })).toBeUndefined(); // the old Ctrl+1…6 screens are gone
    expect(k('b')).toBeUndefined();
  });

  it('conversation-scoped bindings need a conversation (Z-79)', () => {
    const ctx = { navigate: () => {}, newChatHost: () => 'local', hosts: [] };
    location.hash = '';
    expect(runBinding({ kind: 'app', id: 'find.open' }, ctx)).toBe(false);
    location.hash = `#/s/${HOST}/${SESSION}`;
    expect(runBinding({ kind: 'app', id: 'chat.prev' }, ctx)).toBe(false); // no hub cards → no order yet
  });
});

describe('routes', () => {
  it('/ opens the draft for This computer when nothing was open', async () => {
    boot('#/');
    await waitFor(() => expect(location.hash).toBe('#/new/local'));
    expect(await screen.findByTestId('workspace')).toBeInTheDocument();
    expect(screen.getByTestId('conv-title')).toHaveTextContent('New chat');
  });

  it('/s/:host/:session shows the conversation frame; / reopens it next time', async () => {
    const r = boot(`#/s/${HOST}/${SESSION}`);
    await waitFor(() => expect(screen.getByTestId('conv-title').textContent).not.toBe('Session'));
    expect(screen.getByTestId('sidebar')).toBeInTheDocument();
    expect(screen.getByTestId('rail-board')).toBeInTheDocument();
    expect(screen.getByTestId('rail-settings')).toBeInTheDocument();
    // Files / Git / Preview are not rail pages (Z-1, Z-104)
    expect(screen.queryByText('Files', { selector: '.rail-label' })).toBeNull();
    expect(screen.queryByText('Preview', { selector: '.rail-label' })).toBeNull();
    r.unmount();
    boot('#/');
    await waitFor(() => expect(location.hash).toBe(`#/s/${HOST}/${SESSION}`));
  });

  it('rail pages and desktop-only redirects', async () => {
    boot('#/board');
    await waitFor(() => expect(screen.getByTestId('rail-board')).toHaveAttribute('aria-current', 'page'));
    location.hash = '#/x/unipi.demo%2Fpage';
    expect(await screen.findByTestId('ext-page-missing')).toBeInTheDocument();
    location.hash = '#/preview';
    await waitFor(() => expect(location.hash).not.toBe('#/preview'));
  });
});

describe('frame toggles', () => {
  it('Cmd/Ctrl+B / J / Alt+B / Shift+F and Esc', async () => {
    boot(`#/s/${HOST}/${SESSION}`);
    await loaded();
    key('b');
    expect(screen.queryByTestId('sidebar')).toBeNull();
    expect(screen.getByTestId('sidebar-reveal')).toBeInTheDocument();
    key('b');
    expect(screen.getByTestId('sidebar')).toBeInTheDocument();

    expect(screen.queryByTestId('bottom-panel')).toBeNull();
    key('j');
    expect(screen.getByTestId('bottom-panel')).toBeInTheDocument();
    expect(screen.getByTestId('toggle-bottom')).toHaveAttribute('aria-pressed', 'true');

    expect(screen.queryByTestId('side-pane')).toBeNull();
    key('b', { altKey: true });
    expect(screen.getByTestId('side-pane')).toBeInTheDocument();
    expect(screen.getByTestId('workspace')).toHaveAttribute('data-side', 'push');
    expect(screen.getByTestId('side-empty')).toBeInTheDocument(); // empty pane = the launcher

    key('f', { shiftKey: true });
    expect(screen.getByTestId('workspace')).toHaveAttribute('data-side', 'full');
    fireEvent.click(screen.getByTestId('fullview-chat-tab'));
    expect(screen.getByTestId('workspace')).toHaveAttribute('data-chat');
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.getByTestId('workspace')).toHaveAttribute('data-side', 'push');
  });

  it('below 1280px the side pane is an overlay; a click outside closes it', async () => {
    boot(`#/s/${HOST}/${SESSION}`, 1000);
    await loaded();
    fireEvent.click(screen.getByTestId('toggle-side'));
    expect(screen.getByTestId('workspace')).toHaveAttribute('data-side', 'overlay');
    fireEvent.click(screen.getByTestId('side-scrim'));
    expect(screen.queryByTestId('side-pane')).toBeNull();
  });

  it('tabs: launcher opens, ✕ closes (last one hides the pane), Cmd/Ctrl+Shift+T reopens', async () => {
    const off = registerPaneKind({ kind: 'demo', icon: () => null, title: () => 'Demo', singleton: false, scope: 'project', launcher: { label: 'Demo', order: 1 }, component: () => <p>demo body</p> });
    try {
      boot(`#/s/${HOST}/${SESSION}`);
      // tabs are scoped to the session's project: wait for its card
      await waitFor(() => expect(screen.getByTestId('conv-title').textContent).not.toBe('Session'));
      key('b', { altKey: true });
      fireEvent.click(within(screen.getByTestId('side-empty')).getByTestId('launcher-demo'));
      expect(await screen.findByText('demo body')).toBeInTheDocument();
      const tab = screen.getByRole('tab', { name: /Demo/ });
      expect(tab).toHaveAttribute('aria-selected', 'true');
      fireEvent.click(within(tab).getByLabelText('Close Demo'));
      // Z-91: the tab collapses out for 160 ms before the pane hides.
      await waitFor(() => expect(screen.queryByTestId('side-pane')).toBeNull());
      key('t', { shiftKey: true });
      expect(await screen.findByText('demo body')).toBeInTheDocument();
    } finally {
      off();
    }
  });
});

describe('header ⋯ (Z-2)', () => {
  it('lists the chat-scoped actions in order; unplugged ones stay hidden', async () => {
    boot(`#/s/${HOST}/${SESSION}`);
    await waitFor(() => expect(screen.getByTestId('conv-title').textContent).not.toBe('Session'));
    fireEvent.click(screen.getByTestId('conv-more'));
    const labels = () => screen.getAllByRole('menuitem').map((m) => m.textContent?.trim());
    // "Find in chat" leads (WP-A18: the phone's touch entry for Cmd/Ctrl+F).
    // The mock cards have no session file: no "Copy log path". "Session info" is
    // always there: WP-A17's modal plugs it in with the shell (`SessionModalLayer`).
    // WP-A25's `BuiltinActions` registers Duplicate (`/clone`) and Export
    // (`/export`); WP-A5's `SessionActions` registers Rename… and Fork — the
    // slots A3 reserved stay filled once their WP lands.
    expect(labels()).toEqual(['Find in chat', 'Rename…', 'Fork', 'Duplicate', 'Export…', 'Copy session id', 'Copy path', 'Session info']);
    fireEvent.click(screen.getByTestId('anchored-menu-backdrop'));
    const offs = (['reveal'] as const).map((id) => registerChatAction(id, { run: () => {} }));
    try {
      fireEvent.click(screen.getByTestId('conv-more'));
      expect(labels()).toEqual(['Find in chat', 'Rename…', 'Fork', 'Duplicate', 'Export…', 'Copy session id', 'Copy path', 'Reveal folder', 'Session info']);
    } finally {
      offs.forEach((o) => o());
    }
  });
});
