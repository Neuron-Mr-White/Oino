import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';
vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });

// HostClient seam (WP-A1): suites that mock `lib/ipc`'s `call` keep answering
// host ops (now routed through `hostCall()` → HostClient) via a bridge; the
// rest get the default factory (Tauri `invoke`, mocked per suite). Imports are
// dynamic so a suite's `vi.mock('@tauri-apps/api/core')` still applies.
import { beforeEach } from 'vitest';
beforeEach(async () => {
  const ipc = await import('./lib/ipc');
  const { setHostClientFactory } = await import('./lib/host/factory');
  let real = false;
  try {
    real = (ipc as { IPC_REAL?: boolean }).IPC_REAL === true;
  } catch {
    // A vi.mock factory without the marker throws on access: mocked.
  }
  if (!real) {
    const { callBridgeFactory } = await import('./dev/mock/testing');
    setHostClientFactory(callBridgeFactory(ipc.call));
  } else setHostClientFactory(undefined);
});
