import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Root } from '../../main';
import { createMockHost } from '../../dev/mock/host';
import { sidebarStates } from '../../dev/mock/scenarios/sidebar';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetPanes } from '../../lib/panes';
import { resetHubs } from '../../lib/session/hub';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

function boot(hash: string) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  setHostClientFactory(createMockHost(sidebarStates).client);
  location.hash = hash;
  return render(() => <Root />);
}

beforeEach(() => {
  localStorage.clear();
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  location.hash = '';
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('global host selector ([ui-shell] #1)', () => {
  it('sits at the bottom of the sidebar, defaults to This computer and lists every PC', async () => {
    boot('#/');
    const row = await screen.findByTestId('host-row');
    expect(screen.getByTestId('sidebar').contains(row)).toBe(true);
    await waitFor(() => expect(row.textContent).toContain('workstation'));
    fireEvent.click(row);
    expect(await screen.findByTestId('hs-host-this-pc')).toBeTruthy();
    expect(screen.getByTestId('hs-host-pc-offline').textContent).toContain('old-laptop');
    expect(screen.getByTestId('hs-add').textContent).toContain('Add a PC');
    expect(screen.getByTestId('hs-manage').textContent).toContain('Manage hosts');
    expect(screen.getByTestId('hs-host-this-pc').getAttribute('aria-checked')).toBe('true');
  });

  it('picking a PC is the app-wide selection, remembered', async () => {
    boot('#/');
    fireEvent.click(await screen.findByTestId('host-row'));
    fireEvent.click(await screen.findByTestId('hs-host-pc-offline'));
    await waitFor(() => expect(screen.getByTestId('host-row').textContent).toContain('old-laptop'));
    expect(localStorage.getItem('unipi.selectedHost')).toBe('pc-offline');
  });

  it('Manage hosts opens Settings › Hosts', async () => {
    boot('#/');
    fireEvent.click(await screen.findByTestId('host-row'));
    fireEvent.click(await screen.findByTestId('hs-manage'));
    await waitFor(() => expect(location.hash).toContain('/settings/hosts'));
  });
});
