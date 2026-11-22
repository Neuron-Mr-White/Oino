/**
 * Test helpers for the HostClient seam (WP-A1).
 *
 * `callBridgeFactory(call)` adapts a `call(op, args, schema)`-style mock (the
 * suites written before the seam mock `lib/ipc`'s `call`) into a
 * `HostClientFactory`: `request(op, args)` → `call(op, {hostId, ...args})`
 * (app-level ops without `hostId`), unwrapping the Result. `test-setup.ts`
 * installs it whenever a suite has mocked `call`, so host ops routed through
 * `hostCall()` still reach that suite's fake. New suites should use
 * `createMockHost(scenario)` instead.
 */
import { createSignal } from 'solid-js';
import { z } from 'zod';
import { APP_HOST, unavailableError, type HostClient, type HostConn } from '../../lib/host/client';
import type { HostClientFactory } from '../../lib/host/factory';

type CallLike = (op: string, args: Record<string, unknown> | undefined, schema: z.ZodType<unknown>) => Promise<{ ok: true; value: unknown } | { ok: false; error: unknown }> | unknown;

export function callBridgeFactory(call: CallLike): HostClientFactory {
  const clients = new Map<string, HostClient>();
  return (hostId) => {
    let client = clients.get(hostId);
    if (client) return client;
    const [conn] = createSignal<HostConn>({ state: 'connected' });
    client = {
      hostId,
      conn,
      async retry() {},
      async request<T>(op: string, args?: Record<string, unknown>): Promise<T> {
        const payload = hostId === APP_HOST ? args : { hostId, ...(args ?? {}) };
        const result = (await call(op, payload, z.unknown())) as { ok: boolean; value?: unknown; error?: unknown } | undefined;
        if (result && typeof result === 'object' && 'ok' in result) {
          if (result.ok) return result.value as T;
          throw result.error;
        }
        return result as T;
      },
      openSession(_req, sink) {
        queueMicrotask(() => sink.closed(unavailableError('Opening sessions')));
        return { cmd: () => Promise.reject(unavailableError('Session commands')), close() {} };
      },
      openEvents: () => ({ close() {} }),
      openTerm: () => ({ write() {}, resize() {}, close() {} }),
      openLsp: () => ({ send() {}, close() {} }),
    };
    clients.set(hostId, client);
    return client;
  };
}
