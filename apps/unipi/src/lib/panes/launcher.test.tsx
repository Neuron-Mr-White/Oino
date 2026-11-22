// @vitest-environment jsdom
/**
 * w5-ui D2: Files was missing from the side-pane "+" launcher and the chat ⋯ menu — 1.0.10 made FilesTab lazy, and
 * its `setPaneComponent('files', …)` only ran once the module loaded, so `launchableKinds` (which hides kinds without a
 * view) never offered it. The launcher lists every kind a desktop offers, without the lazy modules having loaded.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockFsNode } from '../../dev/mock/host';
import { setHostClientFactory } from '../host/factory';
import { setPlatformOverride } from '../platform';
import { launchableKinds, paneKind } from './kinds';
// What `main.tsx` / AppShell import eagerly: the session tabs (Tree, Side chat) and the Git pane. Files, Browser and
// Terminal are deliberately NOT imported — they are lazy.
import '../../components/panes/tabs';
import '../../components/status/StatusPane';

beforeEach(() => setPlatformOverride('tauri-desktop'));
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  setPlatformOverride(undefined);
});

describe('pane launcher (desktop)', () => {
  it('lists Files, Git, Tree, Side chat, Browser and Terminal for a chat', () => {
    expect(launchableKinds('side', true).map((k) => k.launcher!.label)).toEqual(['Files', 'Git', 'Tree', 'Side chat', 'Browser', 'Terminal']);
  });

  it('every launchable kind has a component (lazy ones included) and none is a dead surface', () => {
    for (const k of launchableKinds('side', true)) expect(paneKind(k.kind)!.component, k.kind).toBeTypeOf('function');
  });

  it('opening Files loads the lazy tree module and renders the tree', async () => {
    const ROOT = '/home/you/p';
    const dir = (children: Record<string, MockFsNode>): MockFsNode => ({ kind: 'dir', children });
    const file = (text: string): MockFsNode => ({ kind: 'file', text });
    const world = createMockHost({
      name: 'launcher-files',
      hosts: [{ id: 'local', name: 'workstation', transport: 'local', cards: [], fs: { [ROOT]: dir({ src: dir({}), 'README.md': file('# hi\n') }) } } as never],
    });
    setHostClientFactory(world.client);
    const Files = paneKind('files')!.component!;
    render(() => (
      <HashRouter>
        <Route
          path="*"
          component={() => (
            <SettingsProvider>
              <HostsProvider>
                <Files tab={{ id: 't-files', kind: 'files', params: {} }} area="side" hostId="local" projectKey={ROOT} close={() => undefined} setTitle={() => undefined} />
              </HostsProvider>
            </SettingsProvider>
          )}
        />
      </HashRouter>
    ));
    expect((await screen.findAllByRole('treeitem', undefined, { timeout: 15000 })).length).toBeGreaterThan(0);
    expect(screen.getByText('README.md')).toBeTruthy();
    // The module's own `setPaneComponent('files', FilesTab)` swapped in the real component.
    expect(paneKind('files')!.component).toBeTypeOf('function');
  }, 25000);
});
