// @vitest-environment jsdom
/**
 * w5-ui D1: Settings › Memory (and every capability page) crashed the app with "Stale read from <Show>"
 * when opened from another settings section through the router's transition (1.0.10: lazy Settings route).
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { ErrorBoundary, Suspense, lazy } from 'solid-js';
import { MemoryRouter, Route, createMemoryHistory, useNavigate } from '@solidjs/router';
import { SettingsProvider } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';
import { setHostClientFactory } from '../../lib/host/factory';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { createMockHost } from '../../dev/mock/host';
import { setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetHubs } from '../../lib/session/hub';
import { MemoryPage } from './MemoryPage';

let go!: (to: string) => void;
function Nav() {
  go = useNavigate();
  return null;
}
const Other = () => <div data-testid="other">General</div>;
const LazyMemory = lazy(async () => ({ default: MemoryPage }));

beforeEach(() => {
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  setHostClientFactory(createMockHost(SCENARIOS['cap-dream']!).client);
});
afterEach(() => {
  cleanup();
  resetHubs();
  setPlatformOverride(undefined);
  setDesktopOverride(undefined);
  setHostClientFactory(undefined);
});

describe('capability pages', () => {
  it('Settings › Memory renders when opened from another section (router transition, lazy route)', async () => {
    const history = createMemoryHistory();
    history.set({ value: '/settings/general', replace: true });
    const view = render(() => (
      <ErrorBoundary fallback={(e) => <div data-testid="crashed">{String(e?.message ?? e)}</div>}>
        <SettingsProvider adapter={createMemoryAdapter(undefined)}>
          <MemoryRouter history={history} root={(p) => <><Nav /><Suspense>{p.children}</Suspense></>}>
            <Route path="/settings/general" component={Other} />
            <Route path="/settings/memory" component={LazyMemory} />
          </MemoryRouter>
        </SettingsProvider>
      </ErrorBoundary>
    ));
    await waitFor(() => expect(view.getByTestId('other')).toBeTruthy());
    go('/settings/memory');
    await waitFor(() => expect(view.container.querySelector('[data-testid=cap-memory-form]')).toBeTruthy(), { timeout: 4000 });
    await new Promise((r) => setTimeout(r, 300));
    expect(view.queryByTestId('crashed')?.textContent).toBeUndefined();
    expect(view.container.querySelector('[data-testid=cap-memory-form]')).toBeTruthy();
  });
});
