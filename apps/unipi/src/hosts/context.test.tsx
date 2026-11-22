// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import '../test-setup';
import { SettingsProvider } from '../settings/context';
import { createMemoryAdapter } from '../settings/adapter';
import { HostsProvider, useHosts } from './context';

const call = vi.hoisted(() => vi.fn());
vi.mock('../lib/ipc', () => ({ call }));

afterEach(() => { cleanup(); call.mockReset(); });

it('discovers the local host after an initially empty list without remounting', async () => {
  call.mockResolvedValueOnce({ ok: true, value: [] }).mockResolvedValue({
    ok: true, value: [{ id: 'pc', name: 'This computer', transport: 'local' }],
  });
  function View() { const hosts = useHosts(); return <div>{hosts.hosts().map((host) => host.name)}</div>; }
  render(() => <SettingsProvider adapter={createMemoryAdapter(undefined)}><HostsProvider><View /></HostsProvider></SettingsProvider>);
  await waitFor(() => expect(screen.getByText('This computer')).toBeTruthy(), { timeout: 3000 });
  expect(call.mock.calls.filter(([name]) => name === 'list_hosts').length).toBeGreaterThan(1);
});
