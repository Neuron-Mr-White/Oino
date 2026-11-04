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
import { resetSettingsMemory } from '../../lib/settings/memory';
import { resetSettingsCommandSessions } from '../../lib/settings/run';
import { resetSubagentsView } from '../settings/SubagentsPage';
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
    resetSettingsMemory();
    resetSessions();
    resetSettingsCommandSessions();
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
  });

  it('owner5 #5: with no chat open, a UI action runs in Settings’ own session, in a Settings sheet', async () => {
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
    expect(document.querySelector('[data-testid=settings-cmd-sheet]')).toBeTruthy();
    expect(location.hash).toBe('#/settings/mcp');
  });

  it('owner5 #5: with a chat open, a UI action still runs in Settings’ own session (never the chat) and stays in Settings', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    await openChat(world, 'local', 'cap001');
    const r = renderAt('mcp');
    await waitFor(() => expect(r.getByTestId('manage-mcp.servers')).toBeTruthy());
    r.getByTestId('manage-mcp.servers').click();
    await waitFor(() => {
      expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt' && (entry.args as { message?: string }).message === '/mcp-settings')).toBe(true);
    });
    // Never navigates: the sheet is the surface.
    expect(location.hash).toBe('#/settings/mcp');
    await waitFor(() => expect(document.querySelector('[data-testid=settings-cmd-sheet]')).toBeTruthy());
  });

  it('owner5 #5: a background action (Sync the catalog) shows its progress inline on the row', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    await openChat(world, 'local', 'cap001');
    const r = renderAt('mcp');
    await waitFor(() => expect(r.getByTestId('manage-mcp.sync')).toBeTruthy());
    r.getByTestId('manage-mcp.sync').click();
    await waitFor(() => expect(r.getByTestId('manage-mcp.sync-result').textContent).toMatch(/Running|Started|Done|catalog/i));
    await waitFor(() => {
      expect(world.log.some((entry) => entry.kind === 'cmd' && entry.op === 'prompt' && (entry.args as { message?: string }).message === '/mcp-sync')).toBe(true);
    });
    expect(location.hash).toBe('#/settings/mcp');
    expect(document.querySelector('[data-testid=settings-cmd-sheet]')).toBeNull();
  });
});

describe('Settings › Skills hub (owner5 #10)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    resetSettingsMemory();
    resetSessions();
    resetSettingsCommandSessions();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('opens the hub in a Settings sheet: no navigation, no command, no flash', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    const r = renderAt('skills');
    await waitFor(() => expect(r.getByTestId('manage-skills.hub')).toBeTruthy());
    r.getByTestId('manage-skills.hub').click();
    await waitFor(() => expect(document.querySelector('[data-testid=settings-form-sheet]')).toBeTruthy());
    expect(location.hash).toBe('#/settings/skills');
    expect(world.log.some((entry) => entry.kind === 'cmd' && (entry.args as { message?: string }).message === '/skills')).toBe(false);
    expect(document.querySelector('.settings-flash-target')).toBeNull();
  });
});

describe('Settings › Subagents › Agent profiles (owner5 #17)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    resetSessions();
    resetSettingsCommandSessions();
    resetSettingsMemory();
    resetSubagentsView();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('Agent profiles is a Settings sub-page: built-ins read-only, Back returns; nothing is sent to a chat', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    const r = renderAt('subagents');
    await waitFor(() => expect(r.getByTestId('manage-agents.profiles')).toBeTruthy());
    r.getByTestId('manage-agents.profiles').click();
    await waitFor(() => expect(r.getByTestId('agent-profiles-page')).toBeTruthy());
    await waitFor(() => expect(r.getByTestId('agent-subagent_explore')).toBeTruthy());
    expect(r.getByTestId('agent-subagent_general')).toBeTruthy();
    r.getByTestId('agent-subagent_explore').click();
    await waitFor(() => expect(document.querySelector('[data-testid=agent-editor]')).toBeTruthy());
    expect((document.querySelector('[data-testid=agent-name]') as HTMLInputElement).disabled).toBe(true);
    expect(document.querySelector('[data-testid=agent-save]')).toBeNull();
    expect(document.querySelector('[data-testid=agent-duplicate]')).toBeTruthy();
    expect(location.hash).toBe('#/settings/subagents');
    expect(world.log.some((entry) => entry.kind === 'cmd' && (entry.args as { message?: string }).message === '/agents')).toBe(false);
  });

  it('agent-disable: every profile has an Enabled switch; off = dimmed + Disabled chip, saved as subagents.disabled; the last one stays on', async () => {
    const world = createMockHost(capMcpNoChat);
    setHostClientFactory(world.client);
    const writes = () =>
      world.log
        .filter((e) => e.kind === 'request' && e.op === 'host_cmd')
        .map((e) => (e.args as { cmd: { kind: string; key?: string; value?: unknown; scope?: string } }).cmd)
        .filter((c) => c.kind === 'set_extension_setting' || c.kind === 'unset_extension_setting');
    const r = renderAt('subagents');
    await waitFor(() => expect(r.getByTestId('manage-agents.profiles')).toBeTruthy());
    r.getByTestId('manage-agents.profiles').click();
    const sw = (id: string) => r.getByTestId(`agent-switch-${id}`).querySelector('input') as HTMLInputElement;
    await waitFor(() => expect(sw('subagent_general').disabled).toBe(false));
    expect(sw('subagent_general').checked).toBe(true);
    expect(r.queryByTestId('agent-off-subagent_general')).toBeNull();
    // Off: the built-in stays listed, dimmed with a chip; the default note names the fallback.
    fireEvent.click(sw('subagent_general'));
    await waitFor(() => expect(r.getByTestId('agent-off-subagent_general').textContent).toBe('Disabled'));
    expect(r.getByTestId('agent-row-subagent_general').hasAttribute('data-off')).toBe(true);
    expect(r.getByTestId('agent-default-note').textContent).toContain('runs subagent_explore');
    await waitFor(() => expect(writes().at(-1)).toMatchObject({ kind: 'set_extension_setting', key: 'disabled', value: ['subagent_general'], scope: 'global' }));
    // The last enabled profile can't be switched off (row and detail sheet).
    await waitFor(() => expect(sw('subagent_explore').disabled).toBe(true));
    r.getByTestId('agent-subagent_explore').click();
    const q = (id: string) => document.querySelector(`[data-testid=${id}]`) as HTMLElement | null;
    await waitFor(() => expect(q('agent-enabled')?.textContent).toContain('at least one must stay on'));
    expect((q('agent-enabled-switch')!.querySelector('input') as HTMLInputElement).disabled).toBe(true);
    // Back on from the detail sheet of the disabled one: the global list is unset again.
    [...document.querySelectorAll('[data-testid=agent-editor] button')].find((b) => b.textContent === 'Close')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await waitFor(() => expect(q('agent-editor')).toBeNull());
    r.getByTestId('agent-subagent_general').click();
    await waitFor(() => expect(q('agent-enabled')?.textContent).toContain('Hidden from the agent'));
    fireEvent.click(q('agent-enabled-switch')!.querySelector('input')!);
    await waitFor(() => expect(writes().at(-1)).toMatchObject({ kind: 'unset_extension_setting', key: 'disabled', scope: 'global' }));
    await waitFor(() => expect(r.queryByTestId('agent-off-subagent_general')).toBeNull());
    expect(r.queryByTestId('agent-default-note')).toBeNull();
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
    resetSettingsMemory();
    resetSessions();
    resetSettingsCommandSessions();
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
