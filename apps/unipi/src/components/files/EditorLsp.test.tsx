/**
 * Editor + LSP wiring tests (WP-A21): the mock host's scripted language
 * server (`dev/mock/lsp.ts`) over the real `openLsp` seam — attach on
 * file open, the `LspStatus` chip reaching `ready`, `publishDiagnostics`
 * badging the tab, one shared connection per `(root, language)` across
 * tabs, the take-over `E_LSP_TAKEN` surfacing as `taken`, and graceful
 * plain-editing when the host has no server.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockFsNode, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { clearDiagnostics } from '../../lib/lsp/diagnostics';
import { closeAllLsp } from '../../lib/lsp/pool';
import { clearEditorTarget, setEditorTarget } from '../../lib/editorTarget';
import { EditorArea } from './EditorArea';

const HOST = 'local';
const ROOT = '/home/you/p';

const dir = (children: Record<string, MockFsNode>): MockFsNode => ({ kind: 'dir', children });
const file = (text: string): MockFsNode => ({ kind: 'file', text });

const fs = (): Record<string, MockFsNode> => ({
  [ROOT]: dir({
    src: dir({
      'math.ts': file('export function add(a: number, b: number) {\n  return a + b;\n}\n'),
      'other.ts': file('export const other = 1;\n'),
    }),
  }),
});

const scenario = (lsp?: Parameters<ReturnType<typeof createMockHost>['client']>[0] extends never ? never : { name?: string; diagnostics?: { severity?: number; message: string }[]; taken?: boolean }): MockScenario => ({
  name: 'editor-lsp-test',
  hosts: [
    {
      id: HOST,
      name: 'workstation',
      transport: 'local',
      fs: fs(),
      lsp: lsp === undefined ? { name: 'typescript-language-server', diagnostics: [{ severity: 1, message: 'unused' }] } : lsp,
    },
  ],
});

let world: ReturnType<typeof createMockHost>;

beforeEach(() => {
  clearDiagnostics();
  closeAllLsp();
  clearEditorTarget();
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  closeAllLsp();
  clearDiagnostics();
  setHostClientFactory(undefined);
});

function withRouter(body: () => JSX.Element) {
  return render(() => (
    <HashRouter>
      <Route
        path="*"
        component={() => (
          <SettingsProvider>
            <HostsProvider>{body()}</HostsProvider>
          </SettingsProvider>
        )}
      />
    </HashRouter>
  ));
}

describe('EditorArea + LSP (WP-A21)', () => {
  it('opens a tab, attaches the server (ready), and badges diagnostics', async () => {
    world = createMockHost(scenario());
    setHostClientFactory(world.client);
    setEditorTarget({ hostId: HOST, path: `${ROOT}/src/math.ts` });
    withRouter(() => <EditorArea root={() => ROOT} onBinary={vi.fn()} />);

    // The tab appears, the editor mounts, the chip reaches `ready`.
    await screen.findByTestId('files-editor-tabs');
    await screen.findByTestId('editor-tab-math.ts');
    await waitFor(() => expect(document.querySelector('.cm-editor')).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId('files-editor-lsp').dataset.status).toBe('ready'));
    expect(world.log.some((e) => e.op === 'lsp_open')).toBe(true);
    // The mock server pushed its diagnostics on didOpen → the tab badge.
    await waitFor(() => expect(document.querySelector('.feditor-tab-diag')).toBeTruthy());
    expect(document.querySelector('.feditor-tab-diag.err')).toBeTruthy();
  });

  it('shares one pooled connection across tabs of the same language', async () => {
    world = createMockHost(scenario());
    setHostClientFactory(world.client);
    withRouter(() => <EditorArea root={() => ROOT} onBinary={vi.fn()} />);
    setEditorTarget({ hostId: HOST, path: `${ROOT}/src/math.ts` });
    setEditorTarget({ hostId: HOST, path: `${ROOT}/src/other.ts` });
    await screen.findByTestId('editor-tab-other.ts');
    await waitFor(() => expect(screen.getByTestId('files-editor-lsp').dataset.status).toBe('ready'));
    expect(world.log.filter((e) => e.op === 'lsp_open')).toHaveLength(1);
  });

  it('E_LSP_TAKEN surfaces on the chip (take-over from another device)', async () => {
    world = createMockHost(scenario({ taken: true }));
    setHostClientFactory(world.client);
    setEditorTarget({ hostId: HOST, path: `${ROOT}/src/math.ts` });
    withRouter(() => <EditorArea root={() => ROOT} onBinary={vi.fn()} />);
    await waitFor(() => expect(screen.getByTestId('files-editor-lsp').dataset.status).toBe('taken'), { timeout: 4000 });
  });

  it('no server → chip shows off and the editor still works', async () => {
    const [conn] = await import('solid-js').then((m) => m.createSignal({ state: 'connected' as const }));
    const bare = {
      hostId: HOST,
      conn,
      retry: async () => {},
      request: async <T,>(op: string, args?: Record<string, unknown>): Promise<T> => {
        if (op === 'lsp_list') return { root: ROOT, servers: [] } as T;
        return (await world.client(HOST).request(op, args)) as T;
      },
      openSession: () => ({ cmd: async <T,>() => ({} as T), close() {} }),
      openEvents: () => ({ close() {} }),
      openTerm: () => ({ write() {}, resize() {}, close() {} }),
      openLsp: (_req: unknown, sink: { closed(error?: unknown): void }) => {
        queueMicrotask(() => sink.closed({ code: 'E_UNSUPPORTED', message: 'no language servers', retryable: false }));
        return { send() {}, close() {} };
      },
    };
    world = createMockHost(scenario());
    setHostClientFactory(world.client);
    setHostClientFactory((id) => (id === HOST ? (bare as never) : world.client(id)));
    setEditorTarget({ hostId: HOST, path: `${ROOT}/src/math.ts` });
    withRouter(() => <EditorArea root={() => ROOT} onBinary={vi.fn()} />);
    await waitFor(() => expect(document.querySelector('.cm-editor')).toBeTruthy());
    await waitFor(() => expect(screen.getByTestId('files-editor-lsp').dataset.status).toBe('off'));
  });
});
