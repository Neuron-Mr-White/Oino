// @vitest-environment jsdom
/** UNI-248: Settings → Sharing with phones / This computer, and the owner's request dialog. */
import { render, screen, cleanup, fireEvent, waitFor, within } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SharingRequestsRunner, SharingSettings } from './SharingSettings';
import { setPlatformOverride } from '../lib/platform';
import { call } from '../lib/ipc';
import { pairPhoneOpen, resetPairPhone, setPairPhoneTransport } from '../lib/pairPhone';
import { setSharingTransport, SharingStatusSchema, type SharingChange, type SharingStatus, type SharingTransport } from '../lib/sharing';
import { setLocalAdmin } from '../lib/thisComputer';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));
vi.mock('../lib/desktop', () => ({ nativeNotify: vi.fn(async () => true) }));

const raw = {
  version: '0.4.1',
  host_dir: '/tmp/qa-248',
  forward: { enabled: false, any: false, ports: [] },
  browser: { enabled: true, headless: true, idle_minutes: 10, found: { name: 'Google Chrome', path: '/usr/bin/google-chrome' } },
  desktop: { enabled: false, view_only: false, display: 'wayland' },
  push: { enabled: false },
  devices: [{ id: 'aaaaaaaa1111', name: 'Pixel' }],
  requests: [{ id: 'r1', feature: 'desktop', scope: { view_only: true }, device_name: 'Pixel', device_id: 'aaaaaaaa1111', status: 'pending', created_at: Date.now(), note: 'need to check a dialog' }],
  linger: false,
};
let current: SharingStatus;
let changes: SharingChange[];
function transport(extra: Partial<SharingTransport> = {}) {
  setSharingTransport({
    // UNI-253: the tests here don't leave; refuse rather than pretend.
    leave: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
    localHost: async () => ({ ok: true, value: { hostDir: '/tmp/qa-248', hostId: 'local' } }),
    status: async () => ({ ok: true, value: current }),
    change: async (c) => {
      changes.push(c);
      if (c.kind === 'forward_enable') current = { ...current, forward: { ...current.forward, enabled: true, any: c.any, ports: c.ports ?? [] } };
      if (c.kind === 'disable') current = { ...current, [c.feature]: { ...(current as unknown as Record<string, object>)[c.feature], enabled: false } } as SharingStatus;
      if (c.kind === 'approve' || c.kind === 'deny') current = { ...current, requests: [] };
      if (c.kind === 'linger_enable') current = { ...current, linger: true };
      if (c.kind === 'host_update') current = { ...current, version: '0.4.5', update: { mode: 'prompt', latest: '0.4.5', available: false } };
      if (c.kind === 'host_update_mode') current = { ...current, update: { ...current.update!, mode: c.auto ? 'auto' : 'prompt' } };
      return { ok: true, value: current };
    },
    ask: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
    requests: async () => ({ ok: true, value: [] }),
    ...extra,
  });
}

// The supervisor card inside This computer talks to its own commands.
vi.mocked(call).mockImplementation(async (cmd: string) => {
  if (cmd === 'host_runtime') {
    return { ok: true, value: { plan: 'adopt', runBy: 'service', version: current?.version ?? '0.4.1', bin: '/usr/bin/unipi-host', skew: null, banner: null, logTail: [] } } as never;
  }
  if (cmd === 'always_on_state') {
    return { ok: true, value: { installed: false, running: false, bin: null, linger: current?.linger ?? null, lingerHint: null, lingerCommand: null } } as never;
  }
  if (cmd === 'host_log') return { ok: true, value: ['host: ready'] } as never;
  if (cmd === 'host_restart') return { ok: true, value: 'adopt' } as never;
  return { ok: true, value: null } as never;
});

beforeEach(() => {
  current = SharingStatusSchema.parse(raw);
  changes = [];
  setPlatformOverride('tauri-desktop');
  transport();
});
afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  setSharingTransport(undefined);
  vi.clearAllMocks();
});

describe('Settings → Sharing with phones (desktop app)', () => {
  it('shows each switch, flips forwarding on (any port) and browser off', async () => {
    render(() => <SharingSettings />);
    const forward = await screen.findByTestId('share-row-forward');
    expect(forward.dataset.on).toBe('false');
    expect(screen.getByTestId('share-row-browser').dataset.on).toBe('true');
    expect(screen.getByTestId('share-row-browser').textContent).toContain('Uses Google Chrome (/usr/bin/google-chrome)');
    // UNI-268: the window toggle is explained (switches a running browser) and never names other brands.
    expect(screen.getByTestId('share-row-browser').textContent).toContain('a running browser switches within a few seconds');
    expect(screen.getByTestId('share-row-browser').textContent?.toLowerCase()).not.toContain('brave');
    fireEvent.click(within(forward).getByRole('switch'));
    await waitFor(() => expect(screen.getByTestId('share-row-forward').dataset.on).toBe('true'));
    expect(changes[0]).toEqual({ kind: 'forward_enable', any: true });
    fireEvent.click(within(screen.getByTestId('share-row-browser')).getByRole('switch'));
    await waitFor(() => expect(changes[1]).toEqual({ kind: 'disable', feature: 'browser' }));
  });

  it('forwarding: "Only these ports" needs ports, then saves them', async () => {
    current = { ...current, forward: { ...current.forward, enabled: true, any: true } };
    render(() => <SharingSettings />);
    const forward = await screen.findByTestId('share-row-forward');
    fireEvent.click(within(forward).getByRole('radio', { name: 'Only these ports' }));
    expect((await within(forward).findByRole('alert')).textContent).toContain('Type the ports');
    fireEvent.input(within(forward).getByLabelText('Allowed ports'), { target: { value: '3000 x' } });
    fireEvent.click(within(forward).getByText('Save'));
    expect((await within(forward).findByRole('alert')).textContent).toContain('"x"');
    fireEvent.input(within(forward).getByLabelText('Allowed ports'), { target: { value: '3000, 5173' } });
    fireEvent.click(within(forward).getByText('Save'));
    await waitFor(() => expect(changes.at(-1)).toEqual({ kind: 'forward_enable', any: false, ports: [3000, 5173] }));
  });

  it('pending request: allow for 8 hours / deny', async () => {
    render(() => <SharingSettings />);
    const req = await screen.findByTestId('share-request-r1');
    expect(req.textContent).toContain('Pixel asks for desktop (watch only)');
    expect(req.textContent).toContain('need to check a dialog');
    fireEvent.click(within(req).getByRole('radio', { name: 'For 8 hours' }));
    fireEvent.click(within(req).getByTestId('share-approve'));
    await waitFor(() => expect(changes).toEqual([{ kind: 'approve', id: 'r1', for: '8h' }]));
    await waitFor(() => expect(screen.queryByTestId('share-request-r1')).toBeNull());
  });

  it('This computer: "Pair a phone" opens the QR dialog (UNI-266)', async () => {
    resetPairPhone();
    let started = 0;
    setPairPhoneTransport({ start: async () => void started++, cancel: async () => undefined, startHost: async () => undefined });
    render(() => <SharingSettings />);
    await screen.findByTestId('share-this-computer');
    fireEvent.click(screen.getByTestId('pair-phone-open'));
    expect(pairPhoneOpen()).toBe(true);
    await waitFor(() => expect(started).toBe(1));
    setPairPhoneTransport(undefined);
    resetPairPhone();
  });

  it('This computer: one "keep available" row; linger is its sub-line, not a second row', async () => {
    render(() => <SharingSettings />);
    const section = await screen.findByTestId('share-this-computer');
    // The old duplicate row is gone: linger is a sub-line of "Keep available".
    expect(section.textContent).not.toContain('Keep running after logout');
    expect(screen.queryByTestId('linger-state')).toBeNull();
    expect(screen.queryByTestId('linger-enable')).toBeNull();
    await waitFor(() => expect(screen.getByTestId('always-on-linger').textContent).toContain('Also after you log out: off'));
    expect(screen.getByTestId('always-on-detail').textContent).toContain('phones reach this PC only while UniPi is open');
    expect(section.textContent).toContain('/tmp/qa-248');
    // The one path that needs root, behind the button — and through the SAME
    // `sharing_change` the rest of this section uses.
    fireEvent.click(screen.getByTestId('always-on-linger-allow'));
    await waitFor(() => expect(changes).toEqual([{ kind: 'linger_enable' }]));
    await waitFor(() => expect(screen.getByTestId('always-on-linger').textContent).toContain('Also after you log out: on'));
  });

  it('This computer: the rows read in one order — pair, host status, keep available, login, updates, host log', async () => {
    current = { ...current, update: { mode: 'prompt', available: false } };
    setLocalAdmin({
      call: async (req) =>
        req.op === 'host_info'
          ? { version: '0.4.9', proto: '1.14.0', mode: 'service', pid: 42, started_at: 1, host_id: 'h', host_name: 'oi', data_dir: '/tmp/qa-248', tools: { oino: true, unipi_kanboard: true, browser_engines: ['chromium'], cloak: false }, devices: 1 }
          : { devices: [] },
    });
    render(() => <SharingSettings />);
    const section = await screen.findByTestId('share-this-computer');
    await waitFor(() => expect(screen.getByTestId('always-on-linger')).toBeTruthy());
    const text = section.textContent!;
    const markers = [
      ['pair a phone', text.indexOf('Pair a phone')],
      ['host status (run by)', text.indexOf('Run by')],
      ['keep available', text.indexOf('Keep available when UniPi is closed')],
      ['start at login', text.indexOf('Start at login')],
      ['update the host by itself', text.indexOf('Update the host by itself')],
      ['host log', text.indexOf('Host log')],
    ] as const;
    for (const [name, at] of markers) expect(at, `${name} is missing`).toBeGreaterThan(-1);
    const order = markers.map(([, at]) => at);
    expect(order).toEqual([...order].sort((a, b) => a - b));
    // And the old duplicate row never comes back.
    expect(text).not.toContain('Keep running after logout');
    setLocalAdmin(undefined);
  });

  it('This computer: the host status row restarts the host, and the host log row has a title', async () => {
    render(() => <SharingSettings />);
    const section = await screen.findByTestId('share-this-computer');
    await waitFor(() => expect(screen.getByTestId('host-restart')).toBeTruthy());
    expect(section.textContent).toContain('Host log');
    expect(section.textContent).toContain('What this PC');
    fireEvent.click(screen.getByTestId('host-log-open'));
    await waitFor(() => expect(screen.getByTestId('host-log').textContent).toContain('host: ready'));
    fireEvent.click(screen.getByTestId('host-restart'));
    await waitFor(() => expect(vi.mocked(call).mock.calls.some(([c]) => c === 'host_restart')).toBe(true));
  });

  it('unipi-host missing: says how to install, Try again re-reads', async () => {
    const status = vi.fn(async () => ({ ok: false as const, error: { code: 'E_NOT_FOUND' as const, message: "unipi-host isn't installed on this PC", retryable: false } }));
    transport({ status });
    render(() => <SharingSettings />);
    expect((await screen.findByTestId('share-settings')).textContent).toContain("isn't installed");
    expect(screen.getByTestId('share-settings').textContent).toContain('install.sh');
    fireEvent.click(screen.getByText('Try again'));
    await waitFor(() => expect(status).toHaveBeenCalledTimes(2));
  });

  it('This computer: no update block from an older host → no update UI', async () => {
    render(() => <SharingSettings />);
    await screen.findByTestId('share-this-computer');
    expect(screen.queryByTestId('host-update')).toBeNull();
    expect(screen.queryByTestId('host-update-auto')).toBeNull();
  });

  it('This computer: update available → Update runs `unipi-host update`; the auto switch sets the mode', async () => {
    current = SharingStatusSchema.parse({ ...raw, update: { mode: 'prompt', latest: '0.4.5', available: true, checked_at: 1 } });
    render(() => <SharingSettings />);
    expect((await screen.findByTestId('host-update-available')).textContent).toContain('0.4.5 is available');
    fireEvent.click(screen.getByTestId('host-update'));
    await waitFor(() => expect(screen.queryByTestId('host-update')).toBeNull());
    // The version row moved to About › Components; here the update notice is gone once updated.
    expect(screen.queryByTestId('host-update-available')).toBeNull();
    expect(screen.getByTestId('share-this-computer').textContent).not.toContain('unipi-host 0.4');
    expect(screen.getByTestId('share-this-computer').textContent).toContain('asked here first');
    fireEvent.click(within(screen.getByTestId('host-update-auto')).getByRole('switch'));
    await waitFor(() => expect(screen.getByTestId('share-this-computer').textContent).toContain('installed as soon as it is out'));
    expect(changes).toEqual([{ kind: 'host_update' }, { kind: 'host_update_mode', auto: true }]);
  });

  it('This computer: a failed check is shown, unknown modes read as prompt', async () => {
    current = SharingStatusSchema.parse({ ...raw, update: { mode: 'weird', available: false, error: 'offline' } });
    expect(current.update?.mode).toBe('prompt');
    render(() => <SharingSettings />);
    expect((await screen.findByTestId('share-this-computer')).textContent).toContain("Couldn't check for updates: offline");
  });

  it('renders nothing on the phone', () => {
    setPlatformOverride('tauri-mobile');
    const { container } = render(() => <SharingSettings />);
    expect(container.textContent).toBe('');
  });
});

describe('owner dialog (desktop app on the PC)', () => {
  it('a new pending request opens the dialog once; answering closes it', async () => {
    const pending = [{ id: 'r9', feature: 'forward', scope: { any_port: true }, device_name: 'Pixel', device_id: 'a', status: 'pending', created_at: Date.now() }];
    vi.mocked(call).mockImplementation(async (cmd: string) => (cmd === 'local_requests' ? { ok: true, value: pending } : { ok: true, value: null }) as never);
    current = { ...current, requests: [] };
    render(() => <SharingRequestsRunner />);
    const sheet = await screen.findByTestId('share-request-sheet');
    expect(sheet.textContent).toContain('A phone is asking');
    expect(sheet.textContent).toContain('Pixel asks for web apps (any port)');
    fireEvent.click(within(sheet).getByTestId('share-deny'));
    await waitFor(() => expect(changes).toEqual([{ kind: 'deny', id: 'r9' }]));
    pending.length = 0;
    await waitFor(() => expect(screen.queryByTestId('share-request-sheet')).toBeNull());
  });
});
