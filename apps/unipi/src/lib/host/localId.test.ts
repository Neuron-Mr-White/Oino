import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
  Channel: class {},
}));

import { createTauriHost } from './tauri';
import { localHostId, resetLocalHostId, resolveHostId, setLocalHostId } from './localId';

afterEach(() => {
  invoke.mockReset();
  resetLocalHostId();
});

describe('the `local` alias is resolved once, centrally', () => {
  it('resolves to the recorded real id; other ids pass through', () => {
    expect(resolveHostId('local')).toBe('local');
    setLocalHostId('abc123');
    expect(resolveHostId('local')).toBe('abc123');
    expect(resolveHostId('pc-two')).toBe('pc-two');
  });

  it('a request on `local` asks list_hosts once and sends the real id', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'list_hosts' ? [{ id: 'abc123', transport: 'local' }, { id: 'pc-two', transport: 'iroh' }] : { ok: true }));
    const host = createTauriHost('local');
    await host.request('fs_roots');
    await host.request('fs_roots');
    expect(invoke.mock.calls.filter(([cmd]) => cmd === 'list_hosts')).toHaveLength(1);
    expect(invoke).toHaveBeenCalledWith('fs_roots', { hostId: 'abc123' });
    expect(localHostId()).toBe('abc123');
  });

  it('a real id never waits for the lookup', () => {
    invoke.mockResolvedValue({ ok: true });
    void createTauriHost('pc-two').request('fs_roots');
    expect(invoke).toHaveBeenCalledWith('fs_roots', { hostId: 'pc-two' });
  });
});
