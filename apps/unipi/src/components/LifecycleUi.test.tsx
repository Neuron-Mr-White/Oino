// @vitest-environment jsdom
/** UNI-237 (A6): the Quit confirm and "Start at login" — closing the window
 * always keeps the app in the tray (decision 2), and a real Quit asks first
 * when the host is the app's own child and phones are paired. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setPlatformOverride } from '../lib/platform';
import { resetLayout } from '../lib/layout';

type Call = [cmd: string, args?: unknown];
const listen = vi.fn();
const call = vi.fn(async (): Promise<{ ok: boolean; value: unknown }> => ({ ok: true, value: null }));
vi.mock('../lib/ipc', () => ({ call: (...a: unknown[]) => (call as unknown as (...x: unknown[]) => unknown)(...a) }));
// `@tauri-apps/api/event` is desktop-only; capture the listener instead.
vi.mock('@tauri-apps/api/event', () => ({ listen: (...a: unknown[]) => (listen as unknown as (...x: unknown[]) => unknown)(...a) }));

import { QuitRunner, useAutostart } from './LifecycleUi';

const calls = (): Call[] => (call as unknown as { mock: { calls: Call[] } }).mock.calls;

beforeEach(() => {
  setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
  resetLayout();
  listen.mockReset();
  call.mockReset();
  (call as unknown as { mockImplementation: (f: (cmd?: string) => Promise<{ ok: boolean; value: unknown }>) => void }).mockImplementation(async (cmd?: string) => ({
    ok: true,
    value: cmd === 'autostart_state' ? false : null,
  }));
});
afterEach(cleanup);

describe('Quit confirm (UNI-237)', () => {
  it('asks, then Quit exits for real and Keep in tray just closes', async () => {
    let handler: ((e: { payload: { message: string; paired: number } }) => void) | undefined;
    listen.mockImplementation(async (_event: string, h: (e: unknown) => void) => {
      handler = h as unknown as typeof handler;
      return () => {};
    });
    render(() => <QuitRunner />);
    await waitFor(() => expect(listen).toHaveBeenCalledWith('unipi-quit-confirm', expect.any(Function)));
    expect(screen.queryByTestId('quit-sheet')).toBeNull();
    handler!({ payload: { message: 'Phones won’t reach this PC until you open Unipi again', paired: 2 } });
    const sheet = await screen.findByTestId('quit-sheet');
    expect(sheet.textContent).toContain('open Unipi again');
    fireEvent.click(screen.getByTestId('quit-keep'));
    await waitFor(() => expect(screen.queryByTestId('quit-sheet')).toBeNull());
    handler!({ payload: { message: 'again', paired: 1 } });
    fireEvent.click(await screen.findByTestId('quit-confirm'));
    await waitFor(() => expect(calls().some(([c]) => c === 'quit_now')).toBe(true));
  });
});

describe('Start at login (UNI-237)', () => {
  it('reads, flips and persists the setting through the plugin command', async () => {
    call.mockImplementation(async (cmd = '') => ({ ok: true, value: cmd === 'autostart_state' ? false : null }));
    let api: ReturnType<typeof useAutostart> | undefined;
    render(() => {
      const a = useAutostart();
      api = a;
      return (
        <button type="button" data-testid="auto" onClick={() => void a.set(!a.on())}>
          autostart
        </button>
      );
    });
    await waitFor(() => expect(api!.on()).toBe(false));
    fireEvent.click(screen.getByTestId('auto'));
    await waitFor(() => expect(calls().some(([c, a]) => c === 'autostart_set' && (a as { enabled: boolean }).enabled === true)).toBe(true));
    await waitFor(() => expect(api!.on()).toBe(true));
  });
});
