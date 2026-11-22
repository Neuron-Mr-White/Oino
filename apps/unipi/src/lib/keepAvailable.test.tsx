// @vitest-environment jsdom
/** UNI-238 (A7): "Keep available when UniPi is closed" turns on by default
 *  right after the FIRST phone pairs (decision 3), the pair dialog says so,
 *  and the linger step is only ever offered behind a button. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { resetKeepAvailableOffer, keepAvailableNote, offerKeepAvailable } from './keepAvailable';
import { setPlatformOverride } from './platform';
import { resetLayout } from './layout';
import { setSharingTransport } from './sharing';

/** What `changeSharing` (the section's own transport) was asked to do. */
const seen: Array<{ kind: string }> = [];

type Call = [cmd: string, args?: unknown];
const call = vi.fn(async (): Promise<{ ok: boolean; value: unknown }> => ({ ok: true, value: null }));
vi.mock('./ipc', () => ({ call: (...a: unknown[]) => (call as unknown as (...x: unknown[]) => unknown)(...a) }));

const calls = (): Call[] => (call as unknown as { mock: { calls: Call[] } }).mock.calls;
const alwaysOnValue = (over: Record<string, unknown> = {}) => ({
  installed: false, running: false, bin: null, linger: null, lingerHint: null, lingerCommand: null, ...over,
});

beforeEach(() => {
  setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
  resetLayout();
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
  (call as unknown as { mockImplementation: (f: (cmd?: string) => Promise<{ ok: boolean; value: unknown }>) => void }).mockImplementation(
    async (cmd = '') => ({ ok: true, value: cmd === 'always_on_state' ? alwaysOnValue() : null }),
  );
  resetKeepAvailableOffer();
});
afterEach(() => {
  cleanup();
  setPlatformOverride(undefined);
  setSharingTransport(undefined);
});

describe('keep available: default on after the first pair (UNI-238)', () => {
  it('the first pairing turns the service on, a second one does not', async () => {
    expect(await offerKeepAvailable()).toBe('on');
    expect(calls().some(([c, a]) => c === 'always_on_set' && (a as { on: boolean }).on === true)).toBe(true);
    call.mockReset();
    expect(await offerKeepAvailable()).toBe('already');
    expect(calls().some(([c]) => c === 'always_on_set')).toBe(false);
  });

  it('a host without the op leaves it off rather than breaking pairing', async () => {
    (call as unknown as { mockImplementation: (f: () => Promise<{ ok: boolean; value: unknown }>) => void }).mockImplementation(async () => ({ ok: false, value: null }));
    expect(await offerKeepAvailable()).toBe('failed');
  });
});

describe('the pair dialog success note', () => {
  it('says the PC stays available, and mentions the linger step when needed', () => {
    expect(keepAvailableNote(alwaysOnValue())).toBeUndefined();
    expect(keepAvailableNote(alwaysOnValue({ installed: true, linger: true }))).toContain('even after a restart');
    const note = keepAvailableNote(alwaysOnValue({ installed: true, linger: false }));
    expect(note).toContain('close Unipi');
    expect(note).toContain('log out');
  });
});

describe('the linger prompt is only offered behind a button', () => {
  it('the Settings card shows the command and an Allow button, never running it', { timeout: 20000 }, async () => {
    (call as unknown as { mockImplementation: (f: (cmd?: string) => Promise<{ ok: boolean; value: unknown }>) => void }).mockImplementation(async (cmd = '') => ({
      ok: true,
      value: cmd === 'always_on_state'
        ? alwaysOnValue({ installed: true, running: true, linger: false, lingerHint: 'needs your password', lingerCommand: 'pkexec loginctl enable-linger oi' })
        : cmd === 'host_runtime'
          ? { plan: 'adopt', runBy: 'service', version: '0.4.9', bin: '/x', skew: null, banner: null, logTail: [] }
          : null,
    }));
    const { HostSupervisorCard } = await import('../components/HostSupervisorCard');
    const { setPlatformOverride } = await import('../lib/platform');
    const { resetLayout } = await import('../lib/layout');
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
    resetLayout();
    render(() => <HostSupervisorCard />);
    const note = await screen.findByTestId('always-on-linger');
    expect(note.textContent).toContain('pkexec loginctl enable-linger oi');
    // Nothing ran root by itself.
    fireEvent.click(screen.getByTestId('always-on-linger-allow'));
    // The card goes through the section's own `changeSharing` transport.
    await waitFor(() => expect(seen).toEqual([{ kind: 'linger_enable' }]));
  });
});
