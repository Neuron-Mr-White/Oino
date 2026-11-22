/**
 * The tree's fs topic watches must not re-subscribe when a listing reloads.
 * The host answers every subscribe with an invalidate snapshot, so keying the
 * watches on the whole tree model looped: subscribe → refresh → reload →
 * subscribe … and the pane flickered between the tree and "Nothing here".
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockFsNode } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';

const watchCalls: string[] = [];
vi.mock('../../lib/topics/client', () => ({
  topicsFor: () => ({
    watch: (topic: string, refresh: () => void) => {
      watchCalls.push(topic);
      // Like the host: every (re)subscribe gets an invalidate snapshot.
      queueMicrotask(refresh);
      return () => {};
    },
  }),
}));
vi.mock('../../lib/topics/pane', () => ({ paneVisible: () => () => true }));

const { FileTree } = await import('./FileTree');

const HOST = 'local';
const ROOT = '/home/you/p';
const dir = (children: Record<string, MockFsNode>): MockFsNode => ({ kind: 'dir', children });
const file = (text: string): MockFsNode => ({ kind: 'file', text });

beforeEach(() => {
  watchCalls.length = 0;
  const world = createMockHost({
    name: 'files-topics',
    hosts: [{ id: HOST, name: 'pc', transport: 'local', cards: [], fs: { [ROOT]: dir({ src: dir({ 'a.ts': file('x\n') }), 'README.md': file('# r\n') }) } }],
  });
  setHostClientFactory(world.client);
});

afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
});

const settle = () => new Promise((resolve) => setTimeout(resolve, 200));

describe('FileTree fs topic watches', () => {
  it('subscribes once per watched dir, not on every listing reload', async () => {
    render(() => (
      <HashRouter>
        <Route path="*" component={() => (
          <SettingsProvider>
            <HostsProvider>
              <FileTree hostId={HOST} root={ROOT} roots={() => undefined} openFile={vi.fn()} onPreview={vi.fn()} />
            </HostsProvider>
          </SettingsProvider>
        )} />
      </HashRouter>
    ));
    await screen.findAllByRole('treeitem');
    await settle();
    expect(watchCalls).toEqual([`fs:${ROOT}`]);

    // Expanding a folder adds exactly one watch for it.
    fireEvent.click(screen.getByText('src'));
    await screen.findByText('a.ts');
    await settle();
    expect(watchCalls.filter((t) => t === `fs:${ROOT}/src`)).toHaveLength(1);
    expect(watchCalls.length).toBeLessThanOrEqual(3);
    // The rows stayed on screen through the refreshes.
    expect(screen.queryByTestId('files-empty')).toBeNull();
  });
});
