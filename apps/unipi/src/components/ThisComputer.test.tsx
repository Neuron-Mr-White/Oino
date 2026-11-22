// @vitest-environment jsdom
/** UNI-233: Settings → This computer over the local admin ops — who runs the
 * host, the paired phones with Unpair (asks first), and the older-host case. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SharingSettings } from './SharingSettings';
import { setPlatformOverride } from '../lib/platform';
import { setSharingTransport, SharingStatusSchema, type SharingStatus } from '../lib/sharing';
import { seenText, setLocalAdmin, refreshThisComputer, thisComputer } from '../lib/thisComputer';

vi.mock('../lib/ipc', () => ({ call: vi.fn(async () => ({ ok: true, value: [] })) }));
vi.mock('../lib/desktop', () => ({ nativeNotify: vi.fn(async () => true) }));

const status: SharingStatus = SharingStatusSchema.parse({
  version: '0.4.8', host_dir: '/home/u/.unipi/app-host',
  forward: { enabled: false, any: false, ports: [] }, browser: { enabled: false, headless: true, idle_minutes: 10 }, desktop: { enabled: false, view_only: false, display: 'wayland' }, push: { enabled: false },
  devices: [{ id: 'aa11bb22cc33', name: 'Pixel 9' }], requests: [], linger: true,
});
const info = { version: '0.4.8', proto: '1.14.0', mode: 'service', pid: 42, started_at: 1, host_id: 'h', host_name: 'oi-desktop', data_dir: '/home/u/.unipi/app-host', tools: { oino: true, unipi_kanboard: true, browser_engines: ['chromium'], cloak: false }, devices: 2 };
let devices: Array<{ id: string; name: string; paired_at: number; last_seen?: number }>;
const calls: Array<Record<string, unknown>> = [];

beforeEach(() => {
  devices = [
    { id: 'aa11bb22cc33dd44', name: 'Pixel 9', paired_at: 1760000000, last_seen: Math.floor(Date.now() / 1000) - 120 },
    { id: 'ee55ff6677889900', name: 'A very long phone name that keeps going and going', paired_at: 1760000000 },
  ];
  calls.length = 0;
  setPlatformOverride('tauri-desktop');
  setSharingTransport({
    localHost: async () => ({ ok: true, value: { hostDir: '/x', hostId: 'h' } }),
    status: async () => ({ ok: true, value: status }),
    change: async () => ({ ok: true, value: status }),
      leave: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
    ask: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
    requests: async () => ({ ok: true, value: [] }),
  });
  setLocalAdmin({
    call: async (req) => {
      calls.push(req);
      if (req.op === 'host_info') return info;
      if (req.op === 'devices_list') return { devices };
      if (req.op === 'device_revoke') {
        devices = devices.filter((d) => !d.id.startsWith(String(req.id)));
        return null;
      }
      throw { code: 'E_SCHEMA', message: 'unknown op' };
    },
  });
});
afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  setSharingTransport(undefined);
  setLocalAdmin(undefined);
});

describe('This computer (UNI-233)', () => {
  it('says who runs the host and lists the phones; Unpair asks, then removes it', async () => {
    render(() => <SharingSettings />);
    await waitFor(() => expect(screen.getByTestId('tc-host-line').textContent).toContain('Run by the background service'));
    const rows = await screen.findAllByTestId('tc-phone');
    expect(rows).toHaveLength(2);
    expect(rows[0]!.textContent).toContain('Pixel 9');
    expect(rows[0]!.textContent).toContain('seen 2 min ago');
    fireEvent.click(rows[0]!.querySelector('[data-testid=tc-unpair]')!);
    const sheet = await screen.findByTestId('tc-unpair-sheet');
    expect(sheet.textContent).toContain('Unpair Pixel 9?');
    expect(calls.some((c) => c.op === 'device_revoke')).toBe(false);
    fireEvent.click(screen.getByTestId('tc-unpair-confirm'));
    await waitFor(() => expect(screen.getAllByTestId('tc-phone')).toHaveLength(1));
    expect(calls.find((c) => c.op === 'device_revoke')).toEqual({ op: 'device_revoke', id: 'aa11bb22cc33dd44' });
  });

  it('an older host (no local socket) keeps the section without the phones list', async () => {
    setLocalAdmin({ call: async () => { throw { code: 'E_NOT_FOUND', message: "unipi-host isn't running" }; } });
    await refreshThisComputer();
    expect(thisComputer().kind).toBe('unsupported');
    render(() => <SharingSettings />);
    await screen.findByTestId('share-this-computer');
    expect(screen.queryByTestId('tc-phones')).toBeNull();
    expect(screen.getByTestId('tc-host-line').textContent).not.toContain('Run by');
  });

  it('seenText', () => {
    const now = 1_760_100_000_000;
    expect(seenText({ id: 'x', name: 'x', paired_at: 1_760_000_000 }, now)).toBe('paired yesterday');
    expect(seenText({ id: 'x', name: 'x', paired_at: 1, last_seen: 1_760_100_000 - 30 }, now)).toBe('seen just now');
    expect(seenText({ id: 'x', name: 'x', paired_at: 1, last_seen: 1_760_100_000 - 7200 }, now)).toBe('seen 2 h ago');
  });
});
