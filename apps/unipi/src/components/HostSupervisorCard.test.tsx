// @vitest-environment jsdom
/** UNI-235/A5 + UNI-238/A7: the supervisor card in Settings → This computer —
 * the crash banner, the version-skew card, "Keep available when UniPi is
 * closed" with the linger state as a sub-line of that row, and the pkexec
 * prompt only ever behind a button. The host status row and the host log row
 * are tested in SharingSettings.test.tsx (they live in ThisComputer). */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setPlatformOverride } from '../lib/platform';
import { resetLayout } from '../lib/layout';
import { HostSupervisorCard } from './HostSupervisorCard';
import { resetKeepAvailableOffer } from '../lib/keepAvailable';
import { setSharingTransport } from '../lib/sharing';

/** What `changeSharing` (the section's own transport) was asked to do. */
const seen: Array<{ kind: string }> = [];

type Call = [cmd: string, args?: unknown];
const call = vi.fn(async (): Promise<{ ok: boolean; value: unknown }> => ({ ok: true, value: null }));
vi.mock('../lib/ipc', () => ({ call: (...a: unknown[]) => (call as unknown as (...x: unknown[]) => unknown)(...a) }));

const calls = (): Call[] => (call as unknown as { mock: { calls: Call[] } }).mock.calls;
const runtime = (over: Record<string, unknown> = {}) => ({
  plan: 'adopt', runBy: 'service', version: '0.4.9', bin: '/home/u/.local/bin/unipi-host', skew: null, banner: null, logTail: ['ok'], ...over,
});
const always = (over: Record<string, unknown> = {}) => ({
  installed: false, running: false, bin: null, linger: null, lingerHint: null, lingerCommand: null, ...over,
});
const mockCalls = (r: Record<string, unknown>, a: Record<string, unknown>) => {
  (call as unknown as { mockImplementation: (f: (cmd?: string) => Promise<{ ok: boolean; value: unknown }>) => void }).mockImplementation(async (cmd = '') => ({
    ok: true,
    value: cmd === 'host_runtime' ? r : cmd === 'always_on_state' ? a : null,
  }));
};

beforeEach(() => {
  setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
  resetLayout();
  resetKeepAvailableOffer();
  seen.length = 0;
  setSharingTransport({
    localHost: async () => ({ ok: true, value: { hostDir: '/x', hostId: 'h' } }),
    status: async () => ({ ok: true, value: {} }) as never,
    change: async (c: unknown) => {
      seen.push(c as { kind: string });
      return { ok: true, value: {} } as never;
    },
    ask: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
    requests: async () => ({ ok: true, value: [] }),
  } as never);
  call.mockReset();
  mockCalls(runtime(), always());
});
afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  setSharingTransport(undefined);
});

describe('host supervisor card (UNI-235)', () => {
  it('shows the crash banner after repeated failures, pointing at the host log', async () => {
    mockCalls(runtime({ plan: 'spawn', runBy: 'app', banner: 'The host on this PC stopped (exit code 1). Restarting…' }), always());
    render(() => <HostSupervisorCard />);
    const banner = await screen.findByTestId('host-banner');
    expect(banner.textContent).toContain('exit code 1');
    expect(banner.textContent).toContain('host log');
  });

  it('the skew card copies the newer bundled host over ~/.local/bin', async () => {
    mockCalls(runtime({ plan: 'adopt', skew: 'ours-newer' }), always());
    render(() => <HostSupervisorCard />);
    const skew = await screen.findByTestId('host-skew');
    expect(skew.textContent).toContain('~/.local/bin/unipi-host');
    fireEvent.click(screen.getByTestId('host-skew-update'));
    await waitFor(() => expect(calls().some(([c]) => c === 'host_update_service')).toBe(true));
  });
});

describe('keep available when UniPi is closed (UNI-238)', () => {
  it('off → the button turns the service on', async () => {
    render(() => <HostSupervisorCard />);
    const detail = await screen.findByTestId('always-on-detail');
    await waitFor(() => expect(detail.textContent).toContain('phones reach this PC only while UniPi is open'));
    fireEvent.click(screen.getByTestId('always-on-toggle'));
    await waitFor(() => expect(calls().some(([c, a]) => c === 'always_on_set' && (a as { on: boolean }).on === true)).toBe(true));
  });

  it('on → the button turns it off', async () => {
    mockCalls(runtime(), always({ installed: true, running: true, bin: '/home/u/.local/bin/unipi-host' }));
    render(() => <HostSupervisorCard />);
    const detail = await screen.findByTestId('always-on-detail');
    await waitFor(() => expect(detail.textContent).toContain('systemd user service'));
    fireEvent.click(screen.getByTestId('always-on-toggle'));
    await waitFor(() => expect(calls().some(([c, a]) => c === 'always_on_set' && (a as { on: boolean }).on === false)).toBe(true));
  });

  it('the linger state is a sub-line of this row, and root only behind the button', async () => {
    mockCalls(runtime(), always({ installed: true, running: true, linger: false, lingerHint: 'needs your password', lingerCommand: 'pkexec loginctl enable-linger oi' }));
    render(() => <HostSupervisorCard />);
    const sub = await screen.findByTestId('always-on-linger');
    // A sub-line of "Keep available", not a row of its own: it never says
    // "Keep running after logout".
    expect(sub.textContent).toContain('Also after you log out: off');
    expect(sub.textContent).toContain('pkexec loginctl enable-linger oi');
    expect(screen.queryByText('Keep running after logout')).toBeNull();
    // Nothing ran root by itself.
    expect(seen).toEqual([]);
    fireEvent.click(screen.getByTestId('always-on-linger-allow'));
    // The card goes through the section's own `changeSharing` transport.
    await waitFor(() => expect(seen).toEqual([{ kind: 'linger_enable' }]));
  });

  it('linger on says so in the same sub-line', async () => {
    mockCalls(runtime(), always({ installed: true, running: true, linger: true }));
    render(() => <HostSupervisorCard />);
    expect((await screen.findByTestId('always-on-linger')).textContent).toContain('Also after you log out: on');
    expect(screen.queryByTestId('always-on-linger-allow')).toBeNull();
  });
});
