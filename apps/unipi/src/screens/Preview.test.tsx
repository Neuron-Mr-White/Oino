// @vitest-environment jsdom
import { render, screen, waitFor, cleanup, fireEvent } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Preview from './Preview';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { call } from '../lib/ipc';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [
  { id: 'host-1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const },
  { id: 'host-2', name: 'laptop', ticket: 't2', paired_at: 0, last_path: 'relay' as const },
];
const disabled = {
  code: 'E_FORWARD_DISABLED',
  message: 'Port forwarding is off on this PC',
  hint: 'On the PC run `unipi-host forward enable --any`',
  retryable: false,
};
const portsData = {
  ports: [
    { port: 22, bind: '0.0.0.0', http: false, allowed: false, process: 'sshd' },
    { port: 5173, bind: '127.0.0.1', http: true, allowed: true, pid: 42, process: 'node', cmd: 'node vite' },
  ],
  any: false,
  allowed: [5173],
};
const forward = (port: number, extra: Record<string, unknown> = {}) => ({
  id: `host-1:${port}`, hostId: 'host-1', remotePort: port, localPort: port, url: `http://127.0.0.1:${port}/`,
  startedAt: 1, active: 0, total: 0, bytesUp: 0, bytesDown: 0, ...extra,
});

type Handler = (args: Record<string, unknown>) => unknown;
let handlers: Record<string, Handler> = {};
function mockIpc(extra: Record<string, Handler> = {}) {
  handlers = {
    list_hosts: () => ({ ok: true, value: hosts }),
    ports_list: () => ({ ok: true, value: portsData }),
    forward_list: () => ({ ok: true, value: [] }),
    forward_start: (args) => ({ ok: true, value: forward(args.port as number) }),
    forward_stop: () => ({ ok: true, value: true }),
    ...extra,
  };
  vi.mocked(call).mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
    (handlers[cmd]?.(args ?? {}) ?? { ok: true, value: null }) as never,
  );
}

function mount() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <SettingsProvider><HostsProvider><Preview /></HostsProvider></SettingsProvider>} />
    </HashRouter>
  ));
}

const tauri = () => Object.assign(window, { __TAURI_INTERNALS__: {} });
afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.resetAllMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  location.hash = '#/';
});
beforeEach(() => mockIpc());

describe('Preview page (UNI-213)', () => {
  it('plain web mode shows "needs the app" and never calls the core', async () => {
    mount();
    expect(await screen.findByTestId('preview-needs-app')).toBeTruthy();
    expect(screen.getByText('Not available in the web version')).toBeTruthy();
    // Same actions as every other web notice: get the app, or go to the board.
    expect(screen.getByTestId('get-app-link').getAttribute('href')).toBe('https://unipi.nrn.one');
    expect(screen.getByText('Open the board').getAttribute('href')).toBe('#/board');
    expect(vi.mocked(call).mock.calls.some(([cmd]) => cmd === 'ports_list')).toBe(false);
  });

  it('no paired PC', async () => {
    tauri();
    mockIpc({ list_hosts: () => ({ ok: true, value: [] }) });
    mount();
    expect(await screen.findByTestId('preview-no-hosts')).toBeTruthy();
  });

  it('forwarding off on another PC: ask it (any port / only these), the command folded below (UNI-248)', async () => {
    tauri();
    mockIpc({ ports_list: () => ({ ok: false, error: disabled }), feature_requests: () => ({ ok: true, value: { requests: [] } }) });
    mount();
    const notice = await screen.findByTestId('preview-disabled');
    expect(notice.textContent).toContain('Web apps are off on this PC');
    expect(screen.getByRole('radio', { name: 'Any port' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByText('Or run this on the PC'));
    expect(notice.textContent).toContain('unipi-oino-host forward enable --any');
    expect(notice.textContent).toContain('unipi-oino-host forward disable');
    // Asking for listed ports sends that scope; the PC says yes → the picker reloads.
    let asked: Record<string, unknown> | undefined;
    mockIpc({
      ports_list: () => ({ ok: false, error: disabled }),
      feature_request: (args) => {
        asked = args;
        return { ok: true, value: { id: 'r1', feature: 'forward', scope: { ports: [5173] }, device_name: 'Pixel', status: 'approved', created_at: 1, decided_at: 2 } };
      },
    });
    fireEvent.click(screen.getByRole('radio', { name: 'Only these ports' }));
    fireEvent.input(screen.getByLabelText('Ports'), { target: { value: '5173' } });
    handlers.ports_list = () => ({ ok: true, value: portsData });
    fireEvent.click(screen.getByTestId('share-ask'));
    await waitFor(() => expect(asked).toBeTruthy());
    expect(asked).toMatchObject({ hostId: 'host-1', feature: 'forward', scope: { ports: [5173] } });
    expect(await screen.findByLabelText('Open port 5173')).toBeTruthy();
  });

  it('no ports', async () => {
    tauri();
    mockIpc({ ports_list: () => ({ ok: true, value: { ports: [], any: true, allowed: [] } }) });
    mount();
    expect(await screen.findByTestId('preview-empty')).toBeTruthy();
  });

  it('transport errors show a retryable banner', async () => {
    tauri();
    mockIpc({ ports_list: () => ({ ok: false, error: { code: 'E_TRANSPORT', message: 'PC offline', retryable: true } }) });
    mount();
    // The shared offline state (UNI-218 F56): "Can't reach <PC>", the reason, Retry.
    const state = await screen.findByTestId('offline-state');
    expect(state.textContent).toContain("Can't reach");
    expect(state.textContent).toContain('PC offline');
    expect(screen.getByRole('button', { name: /Retry/ })).toBeTruthy();
  });

  it('lists web servers first with the not-allowed badge on others', async () => {
    tauri();
    mount();
    await screen.findByLabelText('Open port 5173');
    const labels = screen.getAllByRole('button', { name: /^Open port/ }).map((b) => b.getAttribute('aria-label'));
    expect(labels).toEqual(['Open port 5173', 'Open port 22']);
    expect(screen.getByText('Web servers')).toBeTruthy();
    expect(screen.getByText('not allowed')).toBeTruthy();
    expect(screen.getByText(/only forwards ports 5173/)).toBeTruthy();
  });

  it('opening a port starts a forward and shows the viewer with URL bar + controls', async () => {
    tauri();
    mount();
    (await screen.findByLabelText('Open port 5173')).click();
    const viewer = await screen.findByTestId('preview-viewer');
    expect(vi.mocked(call)).toHaveBeenCalledWith('forward_start', { hostId: 'host-1', port: 5173, localPort: undefined }, expect.anything());
    const frame = viewer.querySelector('iframe')!;
    expect(frame.getAttribute('src')).toBe('http://127.0.0.1:5173/');
    // UNI-224: the previewed site can't navigate the app away (no top navigation, no popups).
    const sandbox = frame.getAttribute('sandbox') ?? '';
    expect(sandbox).toContain('allow-scripts');
    expect(sandbox).toContain('allow-same-origin');
    expect(sandbox).not.toMatch(/allow-top-navigation|allow-popups/);
    expect((screen.getByLabelText('Address') as HTMLInputElement).value).toBe('/');
    expect(screen.getByLabelText('Reload')).toBeTruthy();
    expect(screen.getByLabelText('Open in browser')).toBeTruthy();
    // URL bar navigation.
    const address = screen.getByLabelText('Address') as HTMLInputElement;
    fireEvent.input(address, { target: { value: 'docs/' } });
    fireEvent.submit(address.closest('form')!);
    await waitFor(() => expect(viewer.querySelector('iframe')!.getAttribute('src')).toBe('http://127.0.0.1:5173/docs/'));
    // Open in the phone's browser through the bridge (UNI-224) — never window.open, which
    // loaded the page over the app itself in the Android WebView.
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const openUrl = vi.fn(() => '{"ok":true}');
    Object.assign(window, { UniPiFiles: { openUrl } });
    screen.getByLabelText('Open in browser').click();
    expect(openUrl).toHaveBeenCalledWith('http://127.0.0.1:5173/docs/');
    expect(open).not.toHaveBeenCalled();
    delete (window as { UniPiFiles?: unknown }).UniPiFiles;
    // Stop closes the viewer.
    screen.getByLabelText('Stop preview').click();
    await waitFor(() => expect(screen.queryByTestId('preview-viewer')).toBeNull());
    expect(vi.mocked(call)).toHaveBeenCalledWith('forward_stop', { id: 'host-1:5173' }, expect.anything());
  });

  it('UNI-233: on This computer, a web app opens straight on 127.0.0.1 (no forward) and there is no Desktop tab', async () => {
    tauri();
    mockIpc({ list_hosts: () => ({ ok: true, value: [{ id: 'me', name: 'This computer', ticket: 't', paired_at: 0, last_path: 'local', transport: 'local' }] }) });
    mount();
    (await screen.findByLabelText('Open port 5173')).click();
    const viewer = await screen.findByTestId('preview-viewer');
    expect(viewer.querySelector('iframe')!.getAttribute('src')).toBe('http://127.0.0.1:5173/');
    expect(vi.mocked(call).mock.calls.some((c) => c[0] === 'forward_start')).toBe(false);
    screen.getByLabelText('Stop preview').click();
    await waitFor(() => expect(screen.queryByTestId('preview-viewer')).toBeNull());
    expect(vi.mocked(call).mock.calls.some((c) => c[0] === 'forward_stop')).toBe(false);
    expect(screen.queryByRole('tab', { name: /Desktop/ })).toBeNull();
    expect(screen.getByRole('tab', { name: /Browser/ })).toBeTruthy();
  });

  it('a closed port when opening is explained inline', async () => {
    tauri();
    mockIpc({ forward_start: () => ({ ok: false, error: { code: 'E_NOT_FOUND', message: 'Nothing is listening on port 9999 on this PC', retryable: false } }) });
    mount();
    const input = await screen.findByLabelText('Other port');
    fireEvent.input(input, { target: { value: '9999' } });
    fireEvent.submit(input.closest('form')!);
    const error = await screen.findByTestId('preview-start-error');
    expect(error.textContent).toContain('Port 9999 is closed');
  });

  it('a port outside the allow-list shows the command to allow it', async () => {
    tauri();
    mockIpc({ forward_start: () => ({ ok: false, error: { code: 'E_UNAUTHORIZED', message: "Port 22 is not in this PC's forwarding allow-list", retryable: false } }) });
    mount();
    (await screen.findByLabelText('Open port 22')).click();
    const error = await screen.findByTestId('preview-start-error');
    expect(error.textContent).toContain('unipi-oino-host forward enable --port 22');
  });

  it('manual port box validates', async () => {
    tauri();
    mount();
    const input = await screen.findByLabelText('Other port');
    fireEvent.input(input, { target: { value: '70000' } });
    fireEvent.submit(input.closest('form')!);
    expect((await screen.findByRole('alert')).textContent).toBe('Ports go from 1 to 65535');
    expect(vi.mocked(call).mock.calls.some(([cmd]) => cmd === 'forward_start')).toBe(false);
  });

  it('running forwards are listed and can be stopped', async () => {
    tauri();
    let running = [forward(5173, { active: 2 })];
    mockIpc({ forward_list: () => ({ ok: true, value: running }), forward_stop: () => { running = []; return { ok: true, value: true }; } });
    mount();
    const list = await screen.findByTestId('preview-running');
    expect(list.textContent).toContain('2 live connections');
    screen.getByLabelText('Stop forwarding port 5173').click();
    await waitFor(() => expect(screen.queryByTestId('preview-running')).toBeNull());
  });

  it('Back (Escape stands in for Android Back) and the ← button both close the viewer (UNI-224)', async () => {
    tauri();
    mount();
    (await screen.findByLabelText('Open port 5173')).click();
    await screen.findByTestId('preview-viewer');
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('preview-viewer')).toBeNull());
    expect(screen.getByLabelText('Open port 5173')).toBeTruthy();
    (await screen.findByLabelText('Open port 5173')).click();
    await screen.findByTestId('preview-viewer');
    screen.getByLabelText('Back to ports').click();
    await waitFor(() => expect(screen.queryByTestId('preview-viewer')).toBeNull());
  });

  it('the viewer reports a port that closed and a lost connection', async () => {
    tauri();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      let state: Record<string, unknown> = {};
      mockIpc({ forward_list: () => ({ ok: true, value: [forward(5173, state)] }) });
      mount();
      (await screen.findByLabelText('Open port 5173')).click();
      await screen.findByTestId('preview-viewer');
      expect(screen.queryByTestId('preview-banner')).toBeNull();
      state = { lastError: { error: { code: 'E_NOT_FOUND', message: 'x', retryable: false }, at: 5 } };
      await vi.advanceTimersByTimeAsync(2100);
      expect((await screen.findByTestId('preview-banner')).textContent).toContain('Nothing is listening on port 5173');
      state = { lastError: { error: { code: 'E_TRANSPORT', message: 'x', retryable: true }, at: 6 } };
      await vi.advanceTimersByTimeAsync(2100);
      await waitFor(() => expect(screen.getByTestId('preview-banner').textContent).toContain('Connection to the PC was lost'));
      // Forward gone (stopped elsewhere): offer the way back.
      mockIpc({ forward_list: () => ({ ok: true, value: [] }) });
      await vi.advanceTimersByTimeAsync(2100);
      await waitFor(() => expect(screen.getByTestId('preview-banner').textContent).toContain('This preview was stopped'));
      screen.getByText('Ports').click();
      await waitFor(() => expect(screen.queryByTestId('preview-viewer')).toBeNull());
    } finally {
      vi.useRealTimers();
    }
  });

  it('?host= picks the PC', async () => {
    tauri();
    location.hash = '#/preview?host=host-2';
    mount();
    await screen.findByLabelText('Open port 5173');
    expect(screen.getByLabelText('Choose a PC').textContent).toContain('laptop');
    expect(vi.mocked(call)).toHaveBeenCalledWith('ports_list', { hostId: 'host-2' }, expect.anything());
  });
});
