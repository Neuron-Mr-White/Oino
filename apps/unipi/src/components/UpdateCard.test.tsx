// @vitest-environment jsdom
/** The one Update card (G-14, WP-A24): desktop body + prompt; one card, one surface. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DesktopUpdatePrompt, UpdateCard } from './UpdateCard';
import { checkDesktopUpdate, resetDesktopUpdate, setDesktopUpdateTransport } from '../lib/desktopUpdate';
import { setPlatformOverride } from '../lib/platform';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')) }));
vi.mock('../lib/board/share', () => ({ openExternal: vi.fn(() => true) }));
import { openExternal } from '../lib/board/share';

const info = { current: '0.4.4', kind: 'in_app', version: '0.4.5', notes: 'Faster chats', downloadPage: 'https://unipi.nrn.one/' };
type Install = (v: string, onProgress: (p: never) => void) => Promise<void>;
let install: ReturnType<typeof vi.fn<Install>>;

beforeEach(() => {
  setPlatformOverride('tauri-desktop');
  install = vi.fn<Install>(async (_v, onProgress) => {
    onProgress({ kind: 'progress', done: 5 * 1024 * 1024, total: 10 * 1024 * 1024 } as never);
    await new Promise(() => {}); // the app restarts before this resolves
  });
});
afterEach(() => {
  cleanup();
  resetDesktopUpdate();
  setDesktopUpdateTransport(undefined);
  setPlatformOverride(undefined);
  vi.clearAllMocks();
});

describe('the one Update card (desktop)', () => {
  it('card: Check for updates → up to date', async () => {
    setDesktopUpdateTransport({ check: async () => ({ current: '0.4.5', kind: 'in_app', downloadPage: 'https://unipi.nrn.one/' }), install });
    render(() => <UpdateCard />);
    fireEvent.click(screen.getByTestId('desktop-update-check'));
    await waitFor(() => expect(screen.getByTestId('update-card').textContent).toContain('UniPi is up to date'));
    expect(screen.getByTestId('update-card').textContent).toContain('Installed: 0.4.5');
  });

  it('prompt opens for a newer version; Update and restart downloads with progress', async () => {
    setDesktopUpdateTransport({ check: async () => info, install });
    render(() => <DesktopUpdatePrompt />);
    await checkDesktopUpdate(true);
    const sheet = await screen.findByTestId('desktop-update-prompt');
    expect(sheet.textContent).toContain('UniPi 0.4.5 is available');
    expect(sheet.textContent).toContain('Faster chats');
    expect(install).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('desktop-update-install'));
    await waitFor(() => expect(screen.getByTestId('desktop-update-progress').textContent).toContain('5.0 MB of 10.0 MB'));
    expect(install.mock.calls[0][0]).toBe('0.4.5');
    expect(screen.queryByTestId('desktop-update-later')).toBeNull();
  });

  it('Later closes the prompt; the card still offers the update', async () => {
    setDesktopUpdateTransport({ check: async () => info, install });
    render(() => (
      <>
        <DesktopUpdatePrompt />
        <UpdateCard />
      </>
    ));
    await checkDesktopUpdate(true);
    fireEvent.click(await screen.findByTestId('desktop-update-later'));
    await waitFor(() => expect(screen.queryByTestId('desktop-update-prompt')).toBeNull());
    expect(screen.getByTestId('update-card').textContent).toContain('0.4.5 is available');
  });

  it('package installs get a Download link instead of an install button', async () => {
    setDesktopUpdateTransport({ check: async () => ({ ...info, kind: 'manual' }), install });
    render(() => <UpdateCard />);
    await checkDesktopUpdate(true);
    await waitFor(() => expect(screen.getByTestId('desktop-update-download').textContent).toContain('Download 0.4.5'));
    expect(screen.queryByTestId('desktop-update-install')).toBeNull();
    fireEvent.click(screen.getByTestId('desktop-update-download'));
    expect(openExternal).toHaveBeenCalledWith('https://unipi.nrn.one/');
  });

  it('a failed check says why', async () => {
    setDesktopUpdateTransport({ check: async () => Promise.reject({ message: 'Update check failed: offline' }), install });
    render(() => <UpdateCard />);
    fireEvent.click(screen.getByTestId('desktop-update-check'));
    expect((await screen.findByTestId('desktop-update-error')).textContent).toContain('offline');
  });
});
