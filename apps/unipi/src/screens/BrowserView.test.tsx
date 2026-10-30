// @vitest-environment jsdom
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Preview from './Preview';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { call } from '../lib/ipc';
import { encodeFrame, setBrowserTransport, type BrowserTransport, type StreamMsg } from '../lib/browser';
import type { Result } from '../lib/ipc';
import type { BrowserStatusData } from '@unipi/protocol';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [{ id: 'host-1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

// jsdom has no ResizeObserver / createImageBitmap / canvas 2d.
class RO {
  observe() {}
  disconnect() {}
}
Object.assign(globalThis, { ResizeObserver: RO, createImageBitmap: vi.fn(async () => ({ close() {} })) });
HTMLCanvasElement.prototype.getContext = (() => ({ fillRect() {}, drawImage() {}, fillStyle: '' })) as never;

const status = (extra: Partial<BrowserStatusData> = {}): Result<BrowserStatusData> => ({
  ok: true,
  value: { enabled: true, browser: { name: 'Google Chrome', path: '/usr/bin/google-chrome' }, running: false, headless: true, idle_minutes: 10, tabs: [], ...extra },
});

type Fake = BrowserTransport & { cmds: unknown[]; msg: (m: StreamMsg) => void; frame: (b: Uint8Array) => void; opened: unknown[] };
let windowResult: Result<BrowserStatusData> | undefined;
function fake(statusResult: Result<BrowserStatusData>, openResult?: Awaited<ReturnType<BrowserTransport['open']>>): Fake {
  const f: Fake = {
    cmds: [],
    opened: [],
    msg: () => {},
    frame: () => {},
    status: vi.fn(async () => statusResult),
    close: vi.fn(async () => ({ ok: true as const, value: null })),
    window: vi.fn(async (_h: string, show: boolean): Promise<Result<BrowserStatusData>> =>
      windowResult ?? (statusResult.ok ? { ok: true, value: { ...statusResult.value, headless: !show } } : statusResult),
    ),
    open: vi.fn(async (_h, args, onMsg, onFrame) => {
      f.opened.push(args);
      f.msg = onMsg;
      f.frame = onFrame;
      return openResult ?? { ok: true as const, value: { streamId: 's1', open: { browser: { name: 'Google Chrome', path: '/c' }, headless: true, target: 'T1', tabs: [] } } };
    }),
    cmd: vi.fn(async (_id, cmd) => {
      f.cmds.push(cmd);
    }),
    stop: vi.fn(),
  };
  setBrowserTransport(f);
  return f;
}

function mount() {
  location.hash = '#/preview?mode=browser';
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <SettingsProvider><HostsProvider><Preview /></HostsProvider></SettingsProvider>} />
    </HashRouter>
  ));
}

beforeEach(() => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  vi.mocked(call).mockImplementation(async (cmd: string) =>
    (cmd === 'list_hosts' ? { ok: true, value: hosts } : cmd === 'forward_list' ? { ok: true, value: [] } : { ok: true, value: null }) as never,
  );
});
afterEach(() => {
  cleanup();
  setBrowserTransport(undefined);
  vi.clearAllMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  location.hash = '#/';
});

describe('Preview → Browser mode (UNI-215)', () => {
  it('off on another PC: ask it to turn this on (UNI-248), command folded below, separate profile', async () => {
    fake(status({ enabled: false }));
    mount();
    const notice = await screen.findByTestId('browser-disabled');
    expect(notice.textContent).toContain('The remote browser is off on this PC');
    expect(notice.textContent).toContain('Ask');
    expect(notice.textContent).toContain('separate Chrome on the PC');
    expect(notice.textContent?.toLowerCase()).not.toContain('brave');
    expect(notice.textContent).toContain('Show its window on the PC');
    fireEvent.click(screen.getByText('Or run this on the PC'));
    expect(notice.textContent).toContain('unipi-host browser enable');
    expect(notice.textContent).toContain('unipi-host browser disable');
    expect(screen.getByRole('tab', { name: /Browser/ }).getAttribute('aria-selected')).toBe('true');
  });

  it('browser not found: how to install', async () => {
    fake(status({ browser: undefined, install_hint: 'x' }));
    mount();
    const notice = await screen.findByTestId('browser-not-found');
    expect(notice.textContent).toContain("Google Chrome isn't installed on the PC");
    expect(notice.textContent).toContain('Install Google Chrome on the PC');
    expect(notice.textContent).toContain('winget install Google.Chrome');
    expect(notice.textContent).toContain('brew install --cask google-chrome');
    expect(notice.textContent).toContain('--path');
    expect(notice.textContent?.toLowerCase()).not.toContain('brave');
  });

  it('PC unreachable: retryable error banner', async () => {
    fake({ ok: false, error: { code: 'E_TRANSPORT', message: 'Could not reach coffee', retryable: true } });
    mount();
    const state = await screen.findByTestId('offline-state');
    expect(state.textContent).toContain("Can't reach");
    expect(state.textContent).toContain('Could not reach coffee');
  });

  it('ready: open a URL → starting → live with tabs, page, frames and acks', async () => {
    const f = fake(status());
    mount();
    await screen.findByTestId('browser-ready');
    expect(screen.getByTestId('browser-ready').textContent).toContain('starts when you open it');
    const urlBox = screen.getByPlaceholderText('example.com or localhost:5173') as HTMLInputElement;
    fireEvent.input(urlBox, { target: { value: 'localhost:5173' } });
    urlBox.focus();
    fireEvent.click(screen.getByTestId('browser-start'));
    expect(document.activeElement).not.toBe(urlBox); // the keyboard drops before the stream shows
    expect(await screen.findByTestId('browser-viewer')).toBeTruthy();
    await waitFor(() => expect(f.opened[0]).toMatchObject({ url: 'localhost:5173' }));
    expect(screen.getByTestId('browser-starting').textContent).toContain('Waiting for the first picture');

    f.msg({ kind: 'event', event: { kind: 'tabs', tabs: [{ id: 'T1', url: 'http://localhost:5173/', title: 'Vite App' }, { id: 'T2', url: 'https://example.com/', title: '' }], active: 'T1' } });
    f.msg({ kind: 'event', event: { kind: 'page', url: 'http://localhost:5173/', title: 'Vite App', loading: false, can_back: true, can_forward: false } });
    f.frame(encodeFrame(1, 412, 780, new Uint8Array([0xff, 0xd8, 0xff, 0])));
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'ack', seq: 1 }));
    await waitFor(() => expect(screen.queryByTestId('browser-starting')).toBeNull());

    const strip = screen.getByTestId('browser-tabstrip');
    expect(strip.textContent).toContain('Vite App');
    expect(strip.textContent).toContain('example.com');
    expect((screen.getByLabelText('Address') as HTMLInputElement).value).toBe('localhost:5173');
    expect((screen.getByLabelText('Page back') as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByLabelText('Page forward') as HTMLButtonElement).disabled).toBe(true);

    fireEvent.click(screen.getByLabelText('Page back'));
    fireEvent.click(screen.getByLabelText('Reload'));
    fireEvent.click(screen.getByText('example.com'));
    fireEvent.click(screen.getByLabelText('New tab'));
    fireEvent.click(screen.getByLabelText('Close tab Vite App'));
    const address = screen.getByLabelText('Address');
    fireEvent.focus(address);
    fireEvent.input(address, { target: { value: 'rust lang' } });
    fireEvent.submit(address.closest('form')!);
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'navigate', url: 'rust lang' }));
    expect(f.cmds).toEqual(
      expect.arrayContaining([{ op: 'back' }, { op: 'reload' }, { op: 'tab_activate', id: 'T2' }, { op: 'tab_new' }, { op: 'tab_close', id: 'T1' }]),
    );

    // Typing on the canvas (desktop keyboard).
    const canvas = screen.getByTestId('browser-canvas');
    fireEvent.keyDown(canvas, { key: 'x' });
    fireEvent.keyDown(canvas, { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'key', kind: 'press', key: 'Enter', code: 'Enter', modifiers: 0 }));
    expect(f.cmds).toContainEqual({ op: 'text', text: 'x' });

    // Soft keyboard: the hidden input is diffed into insertText.
    fireEvent.click(screen.getByLabelText('Type'));
    const ime = screen.getByLabelText('Type into the page') as HTMLTextAreaElement;
    ime.value = 'hi';
    fireEvent.input(ime);
    await waitFor(() => expect(f.cmds).toContainEqual({ op: 'text', text: 'hi' }));

    // Loading shows Stop instead of Reload.
    f.msg({ kind: 'event', event: { kind: 'page', url: 'https://example.com/', title: 'Example', loading: true, can_back: true, can_forward: false } });
    expect(await screen.findByLabelText('Stop loading')).toBeTruthy();
  });

  it('crashed → Restart re-opens; closed while idle explains why', async () => {
    const f = fake(status({ running: true, tabs: [{ id: 'T9', url: 'https://a.dev/', title: 'A' }] }));
    mount();
    await screen.findByTestId('browser-tabs-list');
    fireEvent.click(screen.getByText('A'));
    await waitFor(() => expect(f.opened[0]).toMatchObject({ target: 'T9' }));
    f.msg({ kind: 'event', event: { kind: 'crashed', message: 'The page crashed. Reload to try again.' } });
    const problem = await screen.findByTestId('browser-problem');
    expect(problem.textContent).toContain('The browser crashed');
    fireEvent.click(screen.getByText('Restart'));
    await waitFor(() => expect(f.opened).toHaveLength(2));
    f.msg({ kind: 'event', event: { kind: 'closed', reason: 'idle' } });
    expect((await screen.findByTestId('browser-problem')).textContent).toContain('nobody was watching');
  });

  it('connection lost and disabled-at-open states', async () => {
    const f = fake(status());
    mount();
    fireEvent.click(await screen.findByTestId('browser-start'));
    await screen.findByTestId('browser-viewer');
    await waitFor(() => expect(f.opened).toHaveLength(1));
    f.msg({ kind: 'error', message: 'Connection to the PC was lost', code: 'E_TRANSPORT' });
    expect((await screen.findByTestId('browser-problem')).textContent).toContain('Connection lost');
    fireEvent.click(screen.getByText('Back'));
    await waitFor(() => expect(screen.queryByTestId('browser-viewer')).toBeNull());

    fake(status(), { ok: false, error: { code: 'E_BROWSER_DISABLED', message: 'The remote browser is off on this PC', hint: 'x', retryable: false } });
    fireEvent.click(screen.getByTestId('browser-start'));
    const problem = await screen.findByTestId('browser-problem');
    expect(problem.textContent).toContain('unipi-host browser enable');
  });

  it('launch_failed: the message explains itself, the stderr tail is behind Details (UNI-287)', async () => {
    fake(status(), {
      ok: false,
      error: {
        code: 'E_BROWSER_UNAVAILABLE',
        message: 'Chromium exited before DevTools came up — it could not write its profile directory /home/me/.unipi/app-host/browser-profile: Permission denied',
        retryable: true,
        detail: { reason: 'launch_failed', stderr: ['Failed to create /home/me/.unipi/app-host/browser-profile/SingletonLock: Permission denied'] },
      },
    });
    mount();
    fireEvent.click(await screen.findByTestId('browser-start'));
    await new Promise((r) => setTimeout(r, 300));
    const problem = await screen.findByTestId('browser-problem');
    expect(problem.textContent).toContain('Permission denied');
    // The browser's own tail is behind a closed "Details" disclosure until you open it.
    const details = screen.getByText('Details') as HTMLElement;
    expect((details.closest('details') as HTMLDetailsElement).open).toBe(false);
    fireEvent.click(details);
    expect((details.closest('details') as HTMLDetailsElement).open).toBe(true);
    expect(screen.getByTestId('browser-problem-stderr').textContent).toContain('SingletonLock');
  });

  it('menu: Show on PC relaunches with a window, the viewer reconnects by itself, then Hide (UNI-268)', async () => {
    const f = fake(status({ can_show_window: true }));
    mount();
    fireEvent.click(await screen.findByTestId('browser-start'));
    await screen.findByTestId('browser-viewer');
    await waitFor(() => expect(f.opened).toHaveLength(1));
    expect(screen.queryByTestId('browser-headed-note')).toBeNull();
    fireEvent.click(screen.getByTestId('browser-menu-button'));
    const show = await screen.findByRole('menuitem', { name: /Show window on coffee/ });
    fireEvent.click(show);
    await waitFor(() => expect(f.window).toHaveBeenCalledWith('host-1', true));
    // The PC relaunched its browser (same tabs): no error, we reopen on our own.
    f.msg({ kind: 'event', event: { kind: 'closed', reason: 'relaunch' } });
    expect((await screen.findByTestId('browser-starting')).textContent).toContain('Switching the window on coffee');
    expect(screen.queryByTestId('browser-problem')).toBeNull();
    await waitFor(() => expect(f.opened).toHaveLength(2), { timeout: 2000 });
    // The new stream says headed: the note shows and the menu offers Hide.
    const fresh = fake(status({ can_show_window: true }), { ok: true, value: { streamId: 's2', open: { browser: { name: 'Google Chrome', path: '/c' }, headless: false, target: 'T1', tabs: [] } } });
    fireEvent.click(screen.getByTestId('browser-menu-button'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Show window on coffee/ }));
    await waitFor(() => expect(fresh.window).toHaveBeenCalledWith('host-1', true));
    f.msg({ kind: 'event', event: { kind: 'closed', reason: 'relaunch' } });
    await waitFor(() => expect(fresh.opened).toHaveLength(1), { timeout: 2000 });
    expect((await screen.findByTestId('browser-headed-note')).textContent).toContain('Also showing in a window on coffee');
    fireEvent.click(screen.getByTestId('browser-menu-button'));
    fireEvent.click(await screen.findByRole('menuitem', { name: /Hide the window/ }));
    await waitFor(() => expect(fresh.window).toHaveBeenCalledWith('host-1', false));
  });

  it('menu: no Show on PC when the PC has no screen; an old host explains the update', async () => {
    fake(status({ can_show_window: false }));
    mount();
    fireEvent.click(await screen.findByTestId('browser-start'));
    await screen.findByTestId('browser-viewer');
    fireEvent.click(screen.getByTestId('browser-menu-button'));
    await screen.findByRole('menuitem', { name: /Quit the browser on coffee/ });
    expect(screen.queryByRole('menuitem', { name: /Show on/ })).toBeNull();
    cleanup();

    windowResult = { ok: false, error: { code: 'E_SCHEMA', message: 'unknown op', retryable: false } };
    try {
      const f = fake(status());
      mount();
      fireEvent.click(await screen.findByTestId('browser-start'));
      await screen.findByTestId('browser-viewer');
      await waitFor(() => expect(f.opened).toHaveLength(1));
      fireEvent.click(screen.getByTestId('browser-menu-button'));
      fireEvent.click(await screen.findByRole('menuitem', { name: /Show window on coffee/ }));
      expect((await screen.findByRole('alert')).textContent).toContain('Update unipi-host on the PC');
    } finally {
      windowResult = undefined;
    }
  });

  it('quit the browser on the PC', async () => {
    const f = fake(status({ running: true, tabs: [{ id: 'T1', url: 'about:blank', title: '' }] }));
    mount();
    await screen.findByText('New tab');
    fireEvent.click(screen.getByText('Quit the browser on the PC'));
    await waitFor(() => expect(f.close).toHaveBeenCalledWith('host-1'));
  });

  it('web mode: the Preview page shows its graceful notice', async () => {
    delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    const f = fake(status());
    mount();
    expect(await screen.findByTestId('preview-needs-app')).toBeTruthy();
    expect(f.status).not.toHaveBeenCalled();
  });
});
