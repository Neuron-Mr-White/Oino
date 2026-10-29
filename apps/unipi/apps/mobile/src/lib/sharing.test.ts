import { afterEach, describe, expect, it } from 'vitest';
import {
  ago,
  askStateFrom,
  devicesLine,
  featureOn,
  isOldHost,
  isThisComputer,
  loadLocalHost,
  parsePorts,
  pendingRequests,
  requestTitle,
  sameHost,
  scopeText,
  setSharingTransport,
  SharingStatusSchema,
  untilText,
  type SharingStatus,
  type SharingTransport,
} from './sharing';
import { setPlatformOverride } from './platform';

const base: SharingStatus = SharingStatusSchema.parse({
  version: '0.4.1',
  host_dir: '/tmp/qa-248',
  forward: { enabled: true, any: false, ports: [], devices: [] },
  browser: { enabled: false, headless: true, idle_minutes: 10 },
  desktop: { enabled: true, view_only: true, display: 'wayland' },
  push: { enabled: false },
  devices: [{ id: 'aaaaaaaa1111', name: 'Pixel' }],
  requests: [
    { id: 'r1', feature: 'push', device_name: 'Pixel', device_id: 'aaaaaaaa1111', status: 'pending', created_at: 5 },
    { id: 'r2', feature: 'desktop', device_name: 'Pixel', device_id: 'aaaaaaaa1111', status: 'denied', created_at: 9 },
    { id: 'r3', feature: 'forward', scope: { ports: [5173] }, device_name: 'Tab', device_id: 'b', status: 'pending', created_at: 7 },
  ],
});

afterEach(() => {
  setPlatformOverride(undefined);
  setSharingTransport(undefined);
});

describe('sharing helpers (UNI-248)', () => {
  it('feature on/off: forward with no ports is off', () => {
    expect(featureOn(base, 'forward')).toBe(false);
    expect(featureOn({ ...base, forward: { ...base.forward, ports: [5173] } }, 'forward')).toBe(true);
    expect(featureOn({ ...base, forward: { ...base.forward, any: true } }, 'forward')).toBe(true);
    expect(featureOn(base, 'desktop')).toBe(true);
    expect(featureOn(base, 'push')).toBe(false);
  });

  it('device lines name paired devices (id, prefix ≥ 8)', () => {
    expect(devicesLine(base, [])).toBe('All paired devices');
    expect(devicesLine(base, ['aaaaaaaa'])).toBe('Only Pixel');
    expect(devicesLine(base, ['Tablet'])).toBe('Only Tablet');
    expect(devicesLine(base, ['f'.repeat(64)])).toBe('Only ffffffff… (not paired)');
  });

  it('ports parse and explain mistakes', () => {
    expect(parsePorts('5173, 8080 5173')).toEqual({ ok: true, ports: [5173, 8080] });
    expect(parsePorts('')).toMatchObject({ ok: false });
    expect(parsePorts('abc')).toMatchObject({ ok: false, message: '"abc" isn\'t a port number' });
    expect(parsePorts('70000')).toMatchObject({ ok: false });
    expect(parsePorts(Array.from({ length: 17 }, (_, i) => i + 1).join(','))).toMatchObject({ ok: false, message: 'At most 16 ports' });
  });

  it('pending requests newest first; titles and scopes in plain words', () => {
    expect(pendingRequests(base).map((r) => r.id)).toEqual(['r3', 'r1']);
    expect(pendingRequests(undefined)).toEqual([]);
    expect(requestTitle(base.requests[2]!)).toBe('Tab asks for web apps (port 5173)');
    expect(scopeText('desktop', { view_only: true })).toBe('watch only');
    expect(scopeText('forward', { any_port: true })).toBe('any port');
    expect(scopeText('browser', { headed: true })).toBe('window shown on the PC');
    expect(scopeText('push', {})).toBeUndefined();
  });

  it('time words', () => {
    expect(ago(1000, 1000)).toBe('just now');
    expect(ago(0, 5 * 60_000)).toBe('5 min ago');
    expect(ago(0, 3 * 3_600_000)).toBe('3 h ago');
    expect(untilText(undefined)).toBe('until it is turned off');
    expect(untilText(60 * 60_000, 0)).toBe('for 60 more min');
    expect(untilText(8 * 3_600_000, 0)).toBe('for 8 more h');
  });

  it('ask state follows the latest request; old hosts are recognised', () => {
    expect(askStateFrom(undefined)).toEqual({ kind: 'idle' });
    expect(askStateFrom({ ...base.requests[0]!, status: 'approved' }).kind).toBe('approved');
    expect(isOldHost({ code: 'E_SCHEMA', message: 'unknown variant `feature_request`', retryable: false })).toBe(true);
    expect(isOldHost({ code: 'E_TRANSPORT', message: 'x', retryable: true })).toBe(false);
  });

  it('this computer = same public endpoint id, desktop app only', async () => {
    expect(sameHost('ABCD', { hostDir: '/x', hostId: 'abcd' })).toBe(true);
    expect(sameHost('abcd', { hostDir: '/x' })).toBe(false);
    expect(sameHost(undefined, { hostDir: '/x', hostId: 'abcd' })).toBe(false);
    const t = { localHost: async () => ({ ok: true as const, value: { hostDir: '/tmp/qa-248', hostId: 'abcd' } }) } as unknown as SharingTransport;
    setPlatformOverride('tauri-mobile');
    setSharingTransport(t);
    expect(await loadLocalHost()).toBeUndefined();
    expect(isThisComputer('abcd')).toBe(false);
    setPlatformOverride('tauri-desktop');
    setSharingTransport(t);
    await loadLocalHost();
    expect(isThisComputer('abcd')).toBe(true);
    expect(isThisComputer('ffff')).toBe(false);
  });
});
