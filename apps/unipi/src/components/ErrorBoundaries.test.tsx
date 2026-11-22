// @vitest-environment jsdom
/**
 * Crash containment (WP-A9, Z-89): the root boundary offers Retry / Reload
 * app, a slot entry offers Retry / Report, and — the acceptance case — a
 * throwing side-pane tab shows its own boundary while the conversation in the
 * other region keeps streaming.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { registerPaneKind } from '../lib/panes/kinds';
import { resetPanes } from '../lib/panes';
import { resetLayout, setDesktopOverride } from '../lib/layout';
import { setPlatformOverride } from '../lib/platform';
import { resetHubs } from '../lib/session/hub';
import { setHostClientFactory } from '../lib/host/factory';
import { resetUiRegistries } from '../ui/registries';
import { createMockHost, type MockScenario } from '../dev/mock/host';
import { genCards, genFsTree, genSession } from '../dev/mock/gen';
import { Root } from '../main';
import { ConversationBoundary, reloadApp, RootBoundary, SlotBoundary } from './ErrorBoundaries';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  resetUiRegistries();
  localStorage.clear();
  location.hash = '';
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('RootBoundary / SlotBoundary', () => {
  const Boom = () => {
    throw new Error('kaboom');
  };

  it('chat-vanish: a crashing chat says so and "Reload chat" reopens + re-renders it (never an empty pane)', () => {
    const [broken, setBroken] = createSignal(true);
    const onReload = vi.fn(() => setBroken(false));
    const onError = vi.fn();
    render(() => (
      <ConversationBoundary onReload={onReload} onError={onError}>
        {broken() ? <Boom /> : <p>the chat is back</p>}
      </ConversationBoundary>
    ));
    expect(screen.getByTestId('conv-crashed')).toHaveTextContent('Something went wrong in this chat');
    expect(screen.getByText('kaboom')).toBeInTheDocument();
    expect(onError).toHaveBeenCalledWith(expect.objectContaining({ message: 'kaboom' }));
    fireEvent.click(screen.getByTestId('conv-reload-chat'));
    expect(onReload).toHaveBeenCalledOnce();
    expect(screen.getByText('the chat is back')).toBeInTheDocument();
  });

  it('the root boundary catches, and Retry re-renders the app', () => {
    const [broken, setBroken] = createSignal(true);
    render(() => <RootBoundary>{broken() ? <Boom /> : <p>the app is back</p>}</RootBoundary>);
    expect(screen.getByTestId('root-crashed')).toBeInTheDocument();
    expect(screen.getByText('kaboom')).toBeInTheDocument();
    // Retry resets the boundary; the signal still throws, so fix it first.
    setBroken(false);
    fireEvent.click(screen.getByText('Retry'));
    expect(screen.getByText('the app is back')).toBeInTheDocument();
  });

  it('Reload app asks to reload the whole window', () => {
    const onReload = vi.fn();
    render(() => (
      <RootBoundary onReload={onReload}>
        <Boom />
      </RootBoundary>
    ));
    fireEvent.click(screen.getByText('Reload app'));
    expect(onReload).toHaveBeenCalledTimes(1);
  });

  it('a slot entry failure offers Retry and Report', () => {
    const onReport = vi.fn();
    render(() => (
      <SlotBoundary entryId="e1" onReport={onReport}>
        <Boom />
      </SlotBoundary>
    ));
    expect(screen.getByTestId('ui-entry-failed')).toHaveTextContent('kaboom');
    fireEvent.click(screen.getByText('Report'));
    expect(onReport).toHaveBeenCalledWith('e1', expect.any(Error));
    expect(screen.getByText('Retry')).toBeInTheDocument();
  });

  it('reloadApp never throws where navigation is missing', () => {
    expect(() => reloadApp()).not.toThrow();
  });
});

/* ── a crashing pane tab next to a streaming chat ─────────────────── */

const HOST = 'pc-coffee';
const SESSION = 's001';
const CWD = '/home/you/projects/unipi';

function crashScenario(): MockScenario {
  const base = genSession(SESSION, { seed: 4, turns: 2, toolsPerTurn: 1, thinking: true, cwd: CWD });
  return {
    name: 'crash-boundary',
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        conn: { state: 'connected' },
        cards: genCards(3, { seed: 5 }),
        fs: genFsTree(CWD),
        sessions: { [SESSION]: { snapshot: base.snapshot, events: [] } },
      },
    ],
  };
}

describe('a throwing pane tab keeps the chat streaming (Z-89)', () => {
  it('shows the tab boundary and the timeline still applies new events', async () => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
    localStorage.setItem('unipi.shell.layout', JSON.stringify({ sidebarOpen: true, sidebarW: 264, sideOpen: true, sideW: 480, bottomOpen: false, bottomH: 220 }));
    localStorage.setItem(`unipi.panes:${HOST}|${CWD}`, JSON.stringify({ side: { tabs: [{ id: 'tc', kind: 'crash-demo' }], active: { [SESSION]: 'tc' }, closed: [] }, bottom: { tabs: [], active: {}, closed: [] } }));
    const off = registerPaneKind({
      kind: 'crash-demo',
      icon: () => null as never,
      title: () => 'Board',
      singleton: true,
      scope: 'project',
      component: () => {
        throw new Error('The kanboard bridge returned an unexpected payload');
      },
    });

    try {
      const world = createMockHost(crashScenario());
      setHostClientFactory(world.client);
      location.hash = `#/s/${HOST}/${SESSION}`;
      render(() => <Root />);

      // The chat rendered its snapshot turns…
      await waitFor(() => expect(screen.getByTestId('conversation')).toBeInTheDocument());
      expect(await screen.findByTestId('tab-crashed')).toHaveTextContent('Board stopped working');

      // …and keeps taking events while the tab stays broken. The live tail is
      // outside the virtual list, so it renders without jsdom geometry.
      const ts = Date.now();
      const meta = { timestamp: ts, model: 'claude-sonnet-4-5', provider: 'anthropic' };
      world.push(HOST, SESSION, [
        { session: SESSION, seq: 1, event: { type: 'agent_start' } },
        { session: SESSION, seq: 2, event: { type: 'message_start', message: { role: 'user', content: [{ type: 'text', text: 'Now cache the build' }], ...meta } } },
        { session: SESSION, seq: 3, event: { type: 'message_end', message: { role: 'user', content: [{ type: 'text', text: 'Now cache the build' }], ...meta } } },
        { session: SESSION, seq: 4, event: { type: 'message_start', message: { role: 'assistant', content: [], stopReason: 'stop', ...meta } } },
        {
          session: SESSION,
          seq: 5,
          event: {
            type: 'message_update',
            message: { role: 'assistant', stopReason: 'stop', ...meta },
            assistantMessageEvent: { type: 'text_delta', contentIndex: 0, delta: 'Caching the build now.' },
          },
        },
      ]);
      expect(await screen.findByText('Caching the build now.')).toBeInTheDocument();
      expect(screen.getByTestId('tab-crashed')).toBeInTheDocument();
    } finally {
      off();
    }
  });
});
