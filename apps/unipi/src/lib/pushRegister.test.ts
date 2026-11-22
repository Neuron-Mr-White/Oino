// @vitest-environment jsdom
/**
 * G-12 (WP-A19): the phone's push endpoint reaches the paired host through
 * the HostClient seam (`notify_register`), like every other host op — never
 * a direct Tauri invoke.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { syncPushEndpoint } from './pushRegister';
import { setHostClientFactory } from './host/factory';
import type { HostClient } from './host/client';

type Log = Array<{ op: string; args: unknown }>;

const client = (log: Log): HostClient =>
  ({
    hostId: 'h',
    request: async (op: string, args?: unknown) => {
      log.push({ op, args });
      return { registered: true };
    },
    openSession: () => {
      throw new Error('not needed');
    },
    openEvents: () => ({ close: () => {} }),
    openTerm: () => {
      throw new Error('not needed');
    },
    openLsp: () => {
      throw new Error('not needed');
    },
  }) as unknown as HostClient;

describe('syncPushEndpoint', () => {
  it('sends the endpoint to each host once, over the host seam', async () => {
    const log: Log = [];
    setHostClientFactory(() => client(log));
    // The native bridge reports an endpoint.
    (window as unknown as { UniPiNotify?: unknown }).UniPiNotify = { pushEndpoint: () => 'https://ntfy.example/up', registerPush: () => {}, pushStatus: () => '{"state":"registered"}' };
    try {
      localStorage.removeItem('unipi.push.sent');
      const n = await syncPushEndpoint(['h1', 'h2']);
      expect(n).toBe(2);
      expect(log).toEqual([
        { op: 'notify_register', args: { endpoint: 'https://ntfy.example/up' } },
        { op: 'notify_register', args: { endpoint: 'https://ntfy.example/up' } },
      ]);
      // Already sent: nothing new goes out.
      log.length = 0;
      expect(await syncPushEndpoint(['h1', 'h2'])).toBe(0);
      expect(log).toEqual([]);
    } finally {
      delete (window as unknown as { UniPiNotify?: unknown }).UniPiNotify;
      localStorage.removeItem('unipi.push.sent');
    }
  });

  it('without an endpoint (desktop/web) it is a no-op', async () => {
    expect(await syncPushEndpoint(['h1'])).toBe(0);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  setHostClientFactory(undefined);
});
