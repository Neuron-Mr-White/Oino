// @vitest-environment jsdom
/**
 * The MCP servers card (owner 2026-11-06): the two per-server controls talk to
 * the extension's `mcp.server-action` service — the switch enables / disables a
 * server (it persists and starts or stops it), Reconnect restarts a running
 * one — and both say what they did.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { toasts } from '../../lib/toasts';

const h = vi.hoisted(() => ({
  calls: [] as { name: string; payload?: Record<string, unknown> }[],
  enabled: true,
  status: 'running',
  error: '' as string,
}));

vi.mock('../../lib/settings/run', () => ({
  settingsCommandChat: async () => ({
    cmd: async (cmd: { name: string; payload?: Record<string, unknown> }) => {
      h.calls.push(cmd);
      const server = () => ({
        name: 'refero',
        source: 'global',
        enabled: h.enabled,
        transport: 'http',
        status: h.status,
        toolCount: 3,
        tools: ['design'],
        ...(h.error ? { error: h.error } : {}),
      });
      if (cmd.name === 'mcp.servers') return { servers: [server()], errors: [], totalTools: 3 };
      if (cmd.name === 'mcp.server-action') {
        const action = String(cmd.payload?.action);
        if (action === 'enable') h.enabled = true;
        if (action === 'disable') h.enabled = false;
        if (action === 'reconnect') h.status = 'running';
        return { ok: true, servers: { servers: [server()], errors: [], totalTools: 3 } };
      }
      return {};
    },
  }),
}));

const { McpServers } = await import('./McpServers');

afterEach(() => {
  cleanup();
  toasts().clear();
  h.calls.length = 0;
  h.enabled = true;
  h.status = 'running';
  h.error = '';
});

const renderRow = () => render(() => <McpServers hostId="this-pc" />);

describe('the MCP servers card', () => {
  it('the switch is labelled with the state and enables / disables the server', async () => {
    renderRow();
    const toggle = await screen.findByTestId('mcp-en-refero');
    expect(toggle.getAttribute('aria-label')).toBe('refero enabled');
    // The focusable switch is the native input inside the Kobalte root.
    fireEvent.click(toggle.querySelector('input')!);
    await waitFor(() => expect(h.calls.some((c) => c.name === 'mcp.server-action' && c.payload?.action === 'disable')).toBe(true));
    await waitFor(() => expect(screen.getByTestId('mcp-en-refero').getAttribute('aria-label')).toBe('refero disabled'));
  });

  it('Reconnect restarts the server and says so', async () => {
    renderRow();
    const button = await screen.findByTestId('mcp-reconnect-refero');
    fireEvent.click(button);
    await waitFor(() => expect(h.calls.some((c) => c.name === 'mcp.server-action' && c.payload?.action === 'reconnect')).toBe(true));
    await waitFor(() => expect(toasts().list().some((t) => t.message.includes('refero'))).toBe(true));
  });

  it('a server that fails says why', async () => {
    h.error = 'MCP initialize handshake failed: HTTP 415';
    renderRow();
    await screen.findByTestId('mcp-err-refero');
    expect(screen.getByTestId('mcp-err-refero').textContent).toContain('HTTP 415');
  });
});
