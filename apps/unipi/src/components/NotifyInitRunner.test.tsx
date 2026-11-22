// @vitest-environment jsdom
import { render, cleanup, waitFor } from '@solidjs/testing-library';
import { describe, it, expect, vi, afterEach } from 'vitest';
import '../test-setup';
import { NotifyInitRunner } from './NotifyInitRunner';
import { SettingsProvider, useSettings, useSettingsApi } from '../settings/context';
import { createMemoryAdapter } from '../settings/adapter';
import { HostsProvider } from '../hosts/context';
import { createMockHost } from '../dev/mock/host';
import { setHostClientFactory } from '../lib/host/factory';
import type { SessionSnapshot } from '../lib/host/client';
import roundtrip from '../lib/session/fixtures/roundtrip.json';
import { resetSessions, sessions } from '../lib/session/store';
import { resetRunningNotification } from '../lib/runningNotification';
import { installUiSinks, resetUiRegistries, sessionUi } from '../ui/registries';

vi.mock('./MascotFeedRunner', () => ({ MascotFeedRunner: () => <div /> }));
vi.mock('../lib/pushRegister', () => ({ syncPushEndpoint: vi.fn(async () => 0) }));
vi.mock('../lib/ipc', () => ({ call: vi.fn(async () => ({ ok: true, value: [] })) }));

/** Every key the native bridge was handed (UNI-286: it used to get ''). */
const keys: string[] = [];
let pendingActionJson: string | null = null;
Object.defineProperty(window, 'UniPiNotify', {
  configurable: true,
  writable: true,
  value: {
    setKey: (keyB64: string) => keys.push(keyB64),
    takePendingAction: () => {
      const out = pendingActionJson;
      pendingActionJson = null;
      return out;
    },
  },
});

let api: ReturnType<typeof useSettingsApi> | undefined;
function ApiGrab() {
  api = useSettingsApi();
  return <div />;
}
function Probe() {
  const [settings] = useSettings();
  return <div data-testid="key">{settings.notify.key}</div>;
}

afterEach(() => {
  cleanup();
  api = undefined;
  keys.length = 0;
  pendingActionJson = null;
  resetRunningNotification();
  resetUiRegistries();
  resetSessions();
  setHostClientFactory(undefined);
  vi.clearAllMocks();
});

describe('NotifyInitRunner key sync (UNI-286)', () => {
  it('with no key anywhere, native is never told to clear one', async () => {
    const adapter = createMemoryAdapter(undefined);
    render(() => (
      <SettingsProvider adapter={adapter}>
        <HostsProvider>
          <NotifyInitRunner />
          <ApiGrab />
          <Probe />
        </HostsProvider>
      </SettingsProvider>
    ));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(keys.filter(Boolean)).toEqual([]);
  });

  it('hands the stored key to native once the settings have loaded', async () => {
    // A key that survived an app update: native SharedPreferences keeps it,
    // while the JS settings used to be dropped and re-defaulted on load.
    const adapter = createMemoryAdapter({ version: 1, notify: { key: 'c3RvcmVk' } });
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <HostsProvider>
          <NotifyInitRunner />
          <ApiGrab />
          <Probe />
        </HostsProvider>
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('key').textContent).toBe('c3RvcmVk'));
    expect(keys).toContain('c3RvcmVk');
  });

  it('never pushes an empty key (that is what wiped native on every start)', async () => {
    const adapter = createMemoryAdapter({ version: 1, notify: { key: 'c3RvcmVk' } });
    const r = render(() => (
      <SettingsProvider adapter={adapter}>
        <HostsProvider>
          <NotifyInitRunner />
          <ApiGrab />
          <Probe />
        </HostsProvider>
      </SettingsProvider>
    ));
    await waitFor(() => expect(r.getByTestId('key').textContent).toBe('c3RvcmVk'));
    // The user clears the key: native keeps its copy (inert either way, since
    // nothing publishes to that endpoint any more).
    expect(api!.update('notify.key', '')).toBeUndefined();
    await waitFor(() => expect(r.getByTestId('key').textContent).toBe(''));
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(keys).not.toContain('');
  });

  it('latches a cold-start pending notification action and replays it once the v2 session is live', async () => {
    installUiSinks();
    const rt = roundtrip as unknown as { fresh: SessionSnapshot };
    const snap: SessionSnapshot = { ...rt.fresh, seq: 0, entries: [], ui: undefined };
    const world = createMockHost({
      name: 'notify-init-v2',
      hosts: [{ id: 'local', name: 'PC', sessions: { s1: { snapshot: snap, events: [] } } }],
    });
    setHostClientFactory(world.client);
    pendingActionJson = JSON.stringify({ notificationId: 1000, action: 'allow' });

    render(() => (
      <SettingsProvider adapter={createMemoryAdapter(undefined)}>
        <HostsProvider>
          <NotifyInitRunner />
        </HostsProvider>
      </SettingsProvider>
    ));

    // Session connects *after* NotifyInitRunner mounted (cold-start deep link).
    const handle = sessions().ensure('local', 's1');
    await waitFor(() => expect(handle.state.conn).toBe('live'));
    sessionUi('local', 's1').applyRequest({
      type: 'extension_ui_request',
      id: 'dlg-cold',
      method: 'select',
      title: 'Approve?',
      options: ['allow', 'deny'],
    });
    await waitFor(() =>
      expect(
        world.log.some(
          (e) =>
            e.kind === 'cmd' &&
            e.op === 'extension_ui_response' &&
            (e.args as { id?: string; value?: string }).id === 'dlg-cold' &&
            (e.args as { value?: string }).value === 'allow',
        ),
      ).toBe(true),
    );
  });
});
