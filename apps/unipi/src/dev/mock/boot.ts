/**
 * Capture harness boot (app spec §3.15, WP-A1). `vite.config.capture.ts`
 * imports this ahead of `lib/host/factory.ts`; with `?scenario=<name>` in the
 * URL (hash or search) it installs the mock host world for that scenario on
 * `window.__UNIPI_HOST__` and the shell (`shell=desktop|phone`) as the mock
 * platform. Without `scenario=` it does nothing. Never in production builds.
 */
import { createMockHost } from './host';
import { SCENARIOS } from './scenarios';
import { setBlobTransport } from '../../lib/blob';

type BootWindow = Window & { __UNIPI_MOCK_PLATFORM__?: 'tauri-desktop' | 'tauri-mobile' | 'web'; __UNIPI_MOCK_WORLD__?: unknown };

function param(name: string): string | undefined {
  const fromSearch = new URLSearchParams(location.search).get(name);
  if (fromSearch) return fromSearch;
  const q = location.hash.indexOf('?');
  return q >= 0 ? (new URLSearchParams(location.hash.slice(q + 1)).get(name) ?? undefined) : undefined;
}

export function bootCapture(): void {
  if (typeof window === 'undefined') return;
  const name = param('scenario');
  if (!name) return;
  const scenario = SCENARIOS[name];
  if (!scenario) {
    console.error(`[capture] unknown scenario "${name}"; known: ${Object.keys(SCENARIOS).join(', ')}`);
    return;
  }
  const speed = param('speed');
  const live = param('live');
  const world = createMockHost({ ...scenario, speed: live === '1' ? Number(speed ?? 1) || 1 : Number(speed ?? scenario.speed ?? 0) });
  const w = window as BootWindow;
  w.__UNIPI_MOCK_LOG__ = [];
  w.__UNIPI_HOST__ = world.client;
  w.__UNIPI_MOCK_WORLD__ = world;
  w.__UNIPI_MOCK_PLATFORM__ = param('shell') === 'phone' ? 'tauri-mobile' : 'tauri-desktop';
  // Blob channel over the same mock host (WP-A20 preview bytes, A9 image
  // chips, A13 uploads): the capture runs in a browser, where the real
  // transport has no iroh endpoint.
  //
  // Deferred one microtask: this module is prepended to `lib/host/factory.ts`
  // and `lib/blob.ts` imports that module, so calling into blob *while it is
  // still in the import cycle* hits its temporal dead zone ("Cannot access
  // 'override' before initialization") and would abort the rest of the boot.
  const off = () => {
    throw new Error('File transfer needs the app (not available in a browser capture).');
  };
  queueMicrotask(() => {
    setBlobTransport({
      putFile: off,
      getFile: off,
      putBytes: (hostId, bytes, mime, name) => world.client(hostId).request('blob_put_bytes', { bytes: Array.from(bytes), mime, name }),
      getBytes: (hostId, ref) => world.client(hostId).request('blob_get_bytes', ref),
    });
  });
  // Captures start clean: no settings/hidden-chat state from an earlier run.
  if (param('keep') !== '1') {
    try {
      for (const key of Object.keys(localStorage)) if (key.startsWith('unipi.')) localStorage.removeItem(key);
    } catch {
      // storage unavailable: fine
    }
  }
  scenario.setup?.();
}

bootCapture();
