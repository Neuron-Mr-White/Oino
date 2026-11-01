// @vitest-environment jsdom
/**
 * WP-A25 acceptance: the Agent-capability pages (G-8..G-11) and Providers
 * (G-15) inside the one Settings screen.
 *
 * - the MCP page shows the `mcp` namespace form + the extension's Manage rows;
 * - a Manage row runs the extension command in an open chat (MockHost log) and
 *   shows that chat, where U10 renders the panel;
 * - with no chat the rows say nothing redundant but the R8 block offers
 *   "New chat here" (the form still works — the settings form is chat-free).
 */
import { HostsProvider } from '../../hosts/context';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setPlatformOverride } from '../../lib/platform';
import { setDesktopOverride } from '../../lib/layout';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import '../../test-setup';
import Settings from '../Settings';
import { SettingsProvider } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { capMcpNoChat, providersNoChat } from '../../dev/mock/scenarios/capabilities';
import { resetHubs } from '../../lib/session/hub';
import { sessions, resetSessions } from '../../lib/session/store';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), Channel: class {} }));

function renderAt(section: string) {
  location.hash = `#/settings/${section}`;
  return render(() => (
    <SettingsProvider adapter={createMemoryAdapter(undefined)}>
      <HashRouter>
        <Route path="/settings/:section?" component={() => <HostsProvider><Settings /></HostsProvider>} />
      </HashRouter>
    </SettingsProvider>
  ));
}

/** Opens a session in the app's registry (what a chat shown in a pane does). */
function openChat(world: ReturnType<typeof createMockHost>, hostId: string, sessionId: string): Promise<void> {
  const handle = sessions().ensure(hostId, sessionId);
  return new Promise((resolve) => {
    const stop = setInterval(() => {
      if (handle.state.conn === 'live') {
        clearInterval(stop);
        resolve();
      }
    }, 20);
    setTimeout(() => {
      clearInterval(stop);
      resolve();
    }, 3000);
  });
}

describe('Settings › MCP (G-8)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    resetSessions();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('renders the mcp namespace form and the extension’s Manage rows', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    const r = renderAt('mcp');
    await waitFor(() => expect(r.getByTestId('cap-mcp-form')).toBeTruthy());
    // U11's form engine rendered the namespace's own fields.
    await waitFor(() => expect(r.getByText('Load MCP servers')).toBeTruthy());
    for (const id of ['mcp.status', 'mcp.servers', 'mcp.catalog', 'mcp.sync', 'mcp.reload']) {
      expect(r.getByTestId(`manage-${id}`), id).toBeTruthy();
    }
    // R8 / P1-07: no chat ⇒ a hint says where the panel opens; the rows still work.
    expect(r.getByTestId('cap-mcp-nochat')).toBeTruthy();
  });

  it('P1-07: with no chat open, Manage opens the latest chat on this PC and runs there', async () => {
    const base = capMcpNoChat.hosts[0]!;
    // The hub lists cap001 as this PC's most recent chat.
    const latest = { session_id: 'cap001', run_state: 'idle' as const, last_activity: Date.now() + 60_000 };
    const world = createMockHost({ ...capMcpNoChat, hosts: [{ ...base, cards: [latest, ...(base.cards ?? [])] }] });
    setHostClientFactory(world.client);
    const r = renderAt('mcp');
    await waitFor(() => expect(r.getByTestId('manage-mcp.servers')).toBeTruthy());
    r.getByTestId('manage-mcp.servers').click();
    await waitFor(
      () => expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt' && (entry.args as { message?: string }).message === '/mcp-settings')).toBe(true),
      { timeout: 4000 },
    );
  });

  it('with a chat open, Manage runs the extension command there (the panel opens in that chat)', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    await openChat(world, 'local', 'cap001');
    const r = renderAt('mcp');
    await waitFor(() => expect(r.getByTestId('manage-mcp.servers')).toBeTruthy());
    expect(r.queryByTestId('cap-mcp-nochat')).toBeNull();
    r.getByTestId('manage-mcp.servers').click();
    await waitFor(() => {
      expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt' && (entry.args as { message?: string }).message === '/mcp-settings')).toBe(true);
    });
    // The click also shows that chat, where U10 renders the extension's panel.
    await waitFor(() => expect(location.hash).toBe('#/s/local/cap001'));
  });
});

describe('Settings › Providers (G-15 /login, /logout)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    resetSessions();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('lists each provider with its credential state and the actions it supports', async () => {
    const world = createMockHost(providersNoChat);
    setHostClientFactory(world.client);
    const r = renderAt('providers');
    await waitFor(() => expect(r.getByTestId('provider-anthropic')).toBeTruthy());
    expect(r.getByTestId('provider-signin-anthropic')).toBeTruthy(); // oauth
    expect(r.queryByTestId('provider-signin-openai')).toBeNull(); // api key only
    expect(r.getByTestId('provider-signout-anthropic')).toBeTruthy(); // stored credential
    expect(r.getByTestId('provider-openai').textContent).toContain('API key saved');
    expect(r.getByTestId('provider-deepseek').textContent).toContain('from DEEPSEEK_API_KEY');
    expect(r.getByTestId('provider-google').textContent).toContain('Not configured');
  });

  it('saving an API key round-trips through host_cmd › set_api_key', async () => {
    const world = createMockHost(providersNoChat);
    setHostClientFactory(world.client);
    const r = renderAt('providers');
    await waitFor(() => expect(r.getByTestId('provider-key-openai')).toBeTruthy());
    r.getByTestId('provider-key-openai').click();
    const input = (await waitFor(() => r.getByTestId('provider-key-input-openai'))) as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'sk-test' } });
    await waitFor(() => expect((r.getByTestId('provider-key-save-openai') as HTMLButtonElement).disabled).toBe(false));
    r.getByTestId('provider-key-save-openai').click();
    await waitFor(() => expect(r.queryByTestId('provider-key-form-openai')).toBeNull());
    expect(world.log.some((entry) => entry.op === 'host_cmd' && (entry.args as { cmd?: { kind?: string } }).cmd?.kind === 'set_api_key')).toBe(true);
  });

  it('Sign in needs a chat (R8) and says so', async () => {
    const world = createMockHost(providersNoChat);
    setHostClientFactory(world.client);
    const r = renderAt('providers');
    await waitFor(() => expect(r.getByTestId('provider-signin-anthropic')).toBeTruthy());
    expect((r.getByTestId('provider-signin-anthropic') as HTMLButtonElement).disabled).toBe(true);
    expect(r.getByTestId('providers-nochat')).toBeTruthy();
  });
});
