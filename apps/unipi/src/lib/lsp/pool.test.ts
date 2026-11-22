import { afterEach, describe, expect, it } from 'vitest';
import type { AppError } from '@unipi/protocol';
import type { HostClient, HostConn } from '../host/client';
import { createSignal } from 'solid-js';
import { createMockHost, type MockScenario } from '../host/mock';
import { setHostClientFactory } from '../host/factory';
import { acquireLsp, closeAllLsp, lspPoolSize } from './pool';

const FS = {
  '/proj': {
    kind: 'dir' as const,
    children: {
      'a.ts': { kind: 'file' as const, text: 'const mockUnused = 1;\nexport const answer = 42;\n' },
      'other.ts': { kind: 'file' as const, text: 'export const other = 1;\n' },
    },
  },
};

const scenario = (lsp?: MockScenario['hosts'][number]['lsp']): MockScenario => ({
  name: 'lsp-test',
  hosts: [{ id: 'pc1', name: 'pc', conn: { state: 'connected' }, fs: FS, lsp }],
});

const hooks = { displayFile: async () => null };

afterEach(() => {
  closeAllLsp();
  setHostClientFactory(undefined);
});

describe('lsp pool (WP-A21)', () => {
  it('acquires a client and shares it across tabs of the same (root, language)', async () => {
    const world = createMockHost(scenario({ name: 'typescript-language-server' }));
    setHostClientFactory(world.client);
    const host = world.client('pc1');

    const a = await acquireLsp(host, '/proj', '/proj/a.ts', hooks);
    expect(a).not.toBeNull();
    expect(a!.client).not.toBeNull();
    expect(a!.serverName).toBe('typescript-language-server');
    expect(a!.status()).toBe('ready');

    // A second .ts file on the same root+host shares the connection.
    const b = await acquireLsp(host, '/proj', '/proj/other.ts', hooks);
    expect(b!.client).toBe(a!.client);
    expect(lspPoolSize()).toBe(1);
    // And the host saw exactly one lsp_open.
    expect(world.log.filter((l) => l.op === 'lsp_open')).toHaveLength(1);

    // Releasing the last reference detaches (the host keeps it warm).
    a!.release();
    expect(lspPoolSize()).toBe(1);
    b!.release();
    expect(lspPoolSize()).toBe(0);
  });

  it('different language keys get different connections', async () => {
    const world = createMockHost(scenario({}));
    setHostClientFactory(world.client);
    const host = world.client('pc1');
    const a = await acquireLsp(host, '/proj', '/proj/a.ts', hooks);
    const b = await acquireLsp(host, '/proj', '/proj/other.rs', hooks);
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(a!.client).not.toBe(b!.client);
    expect(lspPoolSize()).toBe(2);
  });

  it('degrades to null when openLsp closes immediately (unsupported host)', async () => {
    const [conn] = createSignal<HostConn>({ state: 'connected' });
    const host: HostClient = {
      hostId: 'old',
      conn,
      retry: async () => {},
      request: async <T,>(): Promise<T> => ({} as T),
      openSession: () => ({ cmd: async <T,>() => ({} as T), close() {} }),
      openEvents: () => ({ close() {} }),
      openTerm: () => ({ write() {}, resize() {}, close() {} }),
      openLsp: (_req, sink) => {
        queueMicrotask(() => sink.closed({ code: 'E_UNSUPPORTED', message: 'no lsp', retryable: false } as AppError));
        return { send() {}, close() {} };
      },
    };
    const pooled = await acquireLsp(host, '/proj', '/proj/a.ts', hooks);
    expect(pooled).toBeNull();
    expect(lspPoolSize()).toBe(0);
  });

  it('performs the initialize handshake once (the server saw initialize exactly once)', async () => {
    const world = createMockHost(scenario({}));
    setHostClientFactory(world.client);
    const host = world.client('pc1');
    const a = await acquireLsp(host, '/proj', '/proj/a.ts', hooks);
    expect(a).not.toBeNull();
    await new Promise((r) => setTimeout(r, 50));
    const inits = world.log.filter((l) => l.kind === 'lsp' && l.op === 'initialize');
    expect(inits).toHaveLength(1);
  });
});
