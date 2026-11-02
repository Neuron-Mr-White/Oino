// @vitest-environment jsdom
/**
 * ext-install: Settings › Extensions › "Install extension…" — the desktop folder pick and
 * the phone's .zip upload hand a host path to `host_cmd install_extension`; a same-id
 * package asks before replacing; a bad package shows the host's reason; after "Restart
 * agent host" the new extension's own settings page opens; user-installed rows offer
 * Remove (bundled ones do not).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import '../../test-setup';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';
import { ExtensionsSection } from './ExtensionsSection';
import { setPlatformOverride } from '../../lib/platform';
import { setDesktopOverride } from '../../lib/layout';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost, type MockExtension, type MockNamespace, type MockScenario } from '../../dev/mock/host';
import { resetHubs } from '../../lib/session/hub';
import { setBlobTransport, type BlobTransport } from '../../lib/blob';
import type { HostClient } from '../../lib/host/client';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args), Channel: class {} }));

type Cmd = { kind: string } & Record<string, unknown>;

/** A host whose `install_extension` / `remove_extension` are scripted by the test. */
function world(handler: (cmd: Cmd) => unknown) {
  const extensions: MockExtension[] = [
    { id: 'unipi.memory', enabled: true, loaded: true, path: '/usr/share/oino/extensions/unipi.memory', removable: false },
    { id: 'acme.old', enabled: true, loaded: true, path: '/home/me/.oino/extensions/acme.old', removable: true },
  ];
  const namespaces: MockNamespace[] = [
    { namespace: 'memory', title: 'Memory', extensionId: 'unipi.memory', fields: [{ key: 'topK', type: 'number', title: 'Results' }] },
  ];
  const scenario: MockScenario = {
    name: 'ext-install',
    hosts: [{ id: 'local', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: [], settings: { pi: {}, namespaces, extensions } }],
  };
  const mock = createMockHost(scenario);
  const calls: Cmd[] = [];
  setHostClientFactory((hostId: string) => {
    const client = mock.client(hostId);
    return new Proxy(client, {
      get(target, prop, receiver) {
        if (prop === 'request') {
          return async (op: string, args?: Record<string, unknown>) => {
            const cmd = (args?.cmd ?? {}) as Cmd;
            if (op === 'host_cmd' && (cmd.kind === 'install_extension' || cmd.kind === 'remove_extension')) {
              calls.push(cmd);
              return handler(cmd);
            }
            return target.request(op, args);
          };
        }
        const value = Reflect.get(target, prop, receiver);
        return typeof value === 'function' ? value.bind(target) : value;
      },
    }) as unknown as HostClient;
  });
  return { extensions, namespaces, calls };
}

function renderSection() {
  const [ns, setNs] = createSignal<string | undefined>();
  return render(() => (
    <SettingsProvider adapter={createMemoryAdapter(undefined)}>
      <HostsProvider>
        <ExtensionsSection namespace={ns()} onNamespace={setNs} />
      </HostsProvider>
    </SettingsProvider>
  ));
}

const OMNI = { id: 'omniroute.bridge', name: 'OmniRoute Bridge', version: '1.2.0' };

function installed(extra: Record<string, unknown> = {}) {
  return { installed: true, ...OMNI, path: '/home/me/.oino/extensions/omniroute.bridge', warnings: [], settingsNamespaces: ['omniroute'], needsRestart: true, ...extra };
}

describe('Install extension… (desktop)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    invoke.mockReset();
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
  });

  it('folder → install → Restart agent host → the extension\'s own settings page opens', async () => {
    const w = world(() => installed());
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'pick_folder') return '/home/me/Downloads/omniroute-bridge';
      if (cmd === 'restart_host') {
        // The restarted host has loaded the package and its settings namespace.
        w.extensions.push({ id: OMNI.id, enabled: true, loaded: true, path: '/home/me/.oino/extensions/omniroute.bridge' });
        w.namespaces.push({
          namespace: 'omniroute',
          title: 'OmniRoute Bridge',
          extensionId: OMNI.id,
          fields: [
            { key: 'baseUrl', type: 'string', title: 'Base URL' },
            { key: 'apiKey', type: 'secret', title: 'API key' },
          ],
        });
        return null;
      }
      throw new Error(`unexpected ${cmd}`);
    });
    renderSection();
    fireEvent.click(await screen.findByTestId('ext-install-open'));
    fireEvent.click(await screen.findByTestId('ext-install-folder'));
    const notice = await screen.findByTestId('ext-install-restart');
    expect(w.calls).toEqual([{ kind: 'install_extension', path: '/home/me/Downloads/omniroute-bridge' }]);
    expect(notice.textContent).toContain('Installed OmniRoute Bridge v1.2.0');
    fireEvent.click(screen.getByTestId('ext-install-restart-host'));
    const page = await screen.findByTestId('namespace-page', {}, { timeout: 3000 });
    expect(page.textContent).toContain('OmniRoute Bridge');
    expect(invoke).toHaveBeenCalledWith('restart_host', {});
  });

  it('a same-id package asks before replacing, then sends replace: true', async () => {
    const w = world((cmd) =>
      cmd.replace
        ? installed({ replaced: '1.0.0', warnings: ['OmniRoute Bridge ships with the app (v1.1.0); this v1.2.0 replaces it on this PC until you remove it'] })
        : { installed: false, ...OMNI, confirm: { ...OMNI, installedVersion: '1.0.0' }, warnings: [], settingsNamespaces: [], needsRestart: false },
    );
    invoke.mockImplementation(async (cmd: string) => (cmd === 'pick_folder' ? '/tmp/omni' : null));
    renderSection();
    fireEvent.click(await screen.findByTestId('ext-install-open'));
    fireEvent.click(await screen.findByTestId('ext-install-folder'));
    const ask = await screen.findByTestId('ext-install-confirm');
    expect(ask.textContent).toContain('OmniRoute Bridge v1.0.0 is already installed. Replace it with v1.2.0?');
    fireEvent.click(screen.getByTestId('ext-install-replace'));
    const notice = await screen.findByTestId('ext-install-restart');
    expect(notice.textContent).toContain('Replaced OmniRoute Bridge v1.0.0 with v1.2.0');
    expect(screen.getByTestId('ext-install-warning').textContent).toContain('ships with the app');
    expect(w.calls.map((c) => c.replace)).toEqual([undefined, true]);
  });

  it('an invalid package shows the host\'s reason and nothing installs', async () => {
    world(() => {
      throw { code: 'E_SCHEMA', message: 'invalid extension package: checksum mismatch for `component.wasm`', retryable: false };
    });
    invoke.mockImplementation(async (cmd: string) => (cmd === 'pick_folder' ? '/tmp/bad' : null));
    renderSection();
    fireEvent.click(await screen.findByTestId('ext-install-open'));
    fireEvent.click(await screen.findByTestId('ext-install-folder'));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('checksum mismatch');
    expect(screen.queryByTestId('ext-install-restart')).toBeNull();
    expect(screen.getByTestId('ext-install-open')).toBeTruthy();
  });

  it('a cancelled folder pick changes nothing', async () => {
    const w = world(() => installed());
    invoke.mockImplementation(async () => null);
    renderSection();
    fireEvent.click(await screen.findByTestId('ext-install-open'));
    fireEvent.click(await screen.findByTestId('ext-install-folder'));
    await screen.findByTestId('ext-install-open');
    expect(w.calls).toEqual([]);
  });

  it('only user-installed rows offer Remove; removing asks first and offers the restart', async () => {
    const w = world(() => ({ id: 'acme.old', path: '/home/me/.oino/extensions/acme.old', bundledRemains: false, needsRestart: true }));
    renderSection();
    await screen.findByTestId('ext-remove-acme.old');
    expect(screen.queryByTestId('ext-remove-unipi.memory')).toBeNull();
    fireEvent.click(screen.getByTestId('ext-remove-acme.old'));
    fireEvent.click(await screen.findByTestId('ext-remove-yes-acme.old'));
    const notice = await screen.findByTestId('ext-install-restart');
    expect(notice.textContent).toContain('Removed acme.old');
    expect(w.calls).toEqual([{ kind: 'remove_extension', id: 'acme.old' }]);
  });
});

describe('Install extension… (phone)', () => {
  beforeEach(() => {
    setPlatformOverride('tauri-mobile');
    setDesktopOverride(false);
    invoke.mockReset();
  });
  afterEach(() => {
    cleanup();
    resetHubs();
    setPlatformOverride(undefined);
    setDesktopOverride(undefined);
    setHostClientFactory(undefined);
    setBlobTransport(undefined);
  });

  it('uploads a .zip to the PC and installs the uploaded path; no folder choice, manual restart', async () => {
    const w = world(() => installed());
    const putBytes = vi.fn(async () => ({ sha256: 'ab'.repeat(32), size: 4, deduped: false, path: '/home/me/.oino/host/blobs/abab' }));
    setBlobTransport({ putBytes } as unknown as BlobTransport);
    renderSection();
    const input = (await screen.findByTestId('ext-install-file')) as HTMLInputElement;
    const click = vi.spyOn(input, 'click').mockImplementation(() => {});
    fireEvent.click(screen.getByTestId('ext-install-open'));
    expect(click).toHaveBeenCalled();
    expect(screen.queryByTestId('ext-install-folder')).toBeNull();
    const file = new File([new Uint8Array([0x50, 0x4b, 3, 4])], 'omniroute-bridge.zip', { type: 'application/zip' });
    Object.defineProperty(input, 'files', { value: [file], configurable: true });
    fireEvent.change(input);
    await screen.findByTestId('ext-install-restart');
    expect(putBytes).toHaveBeenCalled();
    expect(w.calls).toEqual([{ kind: 'install_extension', path: '/home/me/.oino/host/blobs/abab' }]);
    expect(screen.getByTestId('ext-install-restart-manual').textContent).toContain('Restart UniPi on the PC');
  });
});
