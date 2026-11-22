// @vitest-environment jsdom
/**
 * w6-ui N1: Settings › Notifications (always) and Memory › Dream (intermittently) crashed the whole app with
 * "Stale read from <Show>" — NamespaceForm's resource source re-read a non-keyed <Show> accessor. Walks EVERY
 * settings page through the router's lazy-route transition, fast (150 ms) and slow, and asserts no ErrorBoundary.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { ErrorBoundary, Suspense, lazy } from 'solid-js';
import { MemoryRouter, Route, createMemoryHistory, useNavigate } from '@solidjs/router';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { createMemoryAdapter } from '../settings/adapter';
import { setHostClientFactory } from '../lib/host/factory';
import { SCENARIOS } from '../dev/mock/scenarios';
import { createMockHost, type MockScenario } from '../dev/mock/host';
import { setDesktopOverride } from '../lib/layout';
import { setPlatformOverride } from '../lib/platform';
import { resetHubs } from '../lib/session/hub';
import { SETTINGS_SECTIONS } from './settings/catalog';

const NOTIFY_NS = {
  namespace: 'notify',
  title: 'Notifications',
  description: 'Needs-you fan-out',
  extensionId: 'unipi.notify',
  defaults: { enabled: true },
  sections: [{ id: 'main', title: 'Events', fields: [{ key: 'enabled', type: 'boolean', label: 'Notify', default: true }] }],
};

function scenario(): MockScenario {
  const base = SCENARIOS['cap-dream']!;
  const hosts = base.hosts.map((h) => ({
    ...h,
    settings: {
      ...h.settings,
      namespaces: [...(h.settings?.namespaces ?? []), NOTIFY_NS as never],
      extensions: [...(h.settings?.extensions ?? []), { id: 'unipi.notify', enabled: true, loaded: true, path: '~/.oino/extensions/unipi.notify' }],
    },
  }));
  return { ...base, hosts };
}

let go!: (to: string) => void;
function Nav() {
  go = useNavigate();
  return null;
}
const Other = () => <div data-testid="other">elsewhere</div>;
const LazySettings = lazy(async () => {
  await new Promise((r) => setTimeout(r, 5));
  return import('./Settings');
});

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

beforeEach(() => {
  window.scrollTo = (() => {}) as never;
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  setHostClientFactory(createMockHost(scenario()).client);
});
afterEach(() => {
  cleanup();
  resetHubs();
  setPlatformOverride(undefined);
  setDesktopOverride(undefined);
  setHostClientFactory(undefined);
});

function mount() {
  const history = createMemoryHistory();
  history.set({ value: '/other', replace: true });
  return render(() => (
    <ErrorBoundary fallback={(e) => <div data-testid="crashed">{String(e?.message ?? e)}</div>}>
      <SettingsProvider adapter={createMemoryAdapter(undefined)}><HostsProvider>
        <MemoryRouter history={history} root={(p) => <><Nav /><Suspense>{p.children}</Suspense></>}>
          <Route path="/other" component={Other} />
          <Route path={['/settings', '/settings/:section', '/settings/:section/:sub']} component={LazySettings} />
        </MemoryRouter>
      </HostsProvider></SettingsProvider>
    </ErrorBoundary>
  ));
}

const ids = () => SETTINGS_SECTIONS.map((s) => s.id);
const crashed = (view: ReturnType<typeof mount>) => view.queryByTestId('crashed')?.textContent;

describe('settings pages never crash with a stale <Show> read', () => {
  it('walks every page with fast navigation (150 ms)', async () => {
    const view = mount();
    await waitFor(() => expect(view.getByTestId('other')).toBeTruthy());
    go('/settings/general');
    await waitFor(() => expect(view.container.querySelector('[data-testid=settings-page]')).toBeTruthy(), { timeout: 6000 });
    for (const round of [ids(), [...ids()].reverse()]) {
      for (const id of round) {
        go(`/settings/${id}`);
        await sleep(150);
        expect(crashed(view), `after ${id}`).toBeUndefined();
      }
    }
    await sleep(400);
    expect(crashed(view)).toBeUndefined();
  }, 60_000);

  it('reaches Notifications and Memory (Dream) from every other page, slow navigation through the lazy route', async () => {
    const view = mount();
    await waitFor(() => expect(view.getByTestId('other')).toBeTruthy());
    for (const target of ['notifications', 'memory']) {
      for (const from of ['other', ...ids().filter((i) => i !== target)]) {
        go(from === 'other' ? '/other' : `/settings/${from}`);
        await sleep(from === 'other' ? 10 : 60);
        go(`/settings/${target}`);
        await sleep(from === 'other' ? 120 : 90);
        expect(crashed(view), `${from} → ${target}`).toBeUndefined();
      }
      await sleep(400);
      expect(crashed(view), `settled on ${target}`).toBeUndefined();
    }
  }, 120_000);

  it('renders the Notifications PC form and the Dream settings form', async () => {
    const view = mount();
    await waitFor(() => expect(view.getByTestId('other')).toBeTruthy());
    go('/settings/notifications');
    await waitFor(() => expect(view.container.querySelector('[data-testid=notify-pc-form]')).toBeTruthy(), { timeout: 6000 });
    go('/settings/memory');
    await waitFor(() => expect(view.container.querySelector('[data-testid=dream-settings-form]')).toBeTruthy(), { timeout: 6000 });
    expect(crashed(view)).toBeUndefined();
  }, 30_000);
});
