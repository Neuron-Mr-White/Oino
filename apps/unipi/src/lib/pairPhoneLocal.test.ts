// @vitest-environment jsdom
/** UNI-233: Pair a phone uses the host's own pair_begin/pair_status over this
 * PC's local socket when it can, and the `unipi-host pair --json` subprocess
 * for older hosts. Closing the dialog cancels the code. */
import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), Channel: class { onmessage?: (e: unknown) => void } }));

import { closePairPhone, pairPhoneState, startPairPhone } from './pairPhone';

afterEach(() => {
  invoke.mockReset();
  vi.useRealTimers();
});

describe('pair a phone over the local socket', () => {
  it('pair_begin → waiting; pair_status polls until paired', async () => {
    vi.useFakeTimers();
    let polls = 0;
    invoke.mockImplementation(async (cmd: string, args: { req?: { op: string } }) => {
      expect(cmd).toBe('local_admin');
      if (args.req?.op === 'pair_begin') return { link: 'unipi://pair?t=x&c=K7QX2M9P', code: 'K7QX2M9P', expires_at: 1760000600, host_name: 'oi-desktop' };
      if (args.req?.op === 'pair_status') return ++polls < 2 ? { state: 'waiting', expires_at: 1760000600 } : { state: 'paired', device: 'Redmi' };
      throw new Error(`unexpected ${cmd}`);
    });
    await startPairPhone();
    expect(pairPhoneState()).toMatchObject({ kind: 'waiting', code: 'K7QX2M9P', host: 'oi-desktop' });
    await vi.advanceTimersByTimeAsync(2000);
    expect(pairPhoneState()).toMatchObject({ kind: 'paired', device: 'Redmi' });
    expect(invoke.mock.calls.some((c) => c[0] === 'pair_phone_start')).toBe(false);
  });

  it('an older host (no socket) falls back to the CLI subprocess', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'local_admin') throw { code: 'E_NOT_FOUND', message: 'no local host' };
      if (cmd === 'pair_phone_start') return undefined;
      throw new Error(cmd);
    });
    await startPairPhone();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(['local_admin', 'pair_phone_start']);
  });

  it('closing the dialog cancels the code on the host', async () => {
    invoke.mockImplementation(async (_cmd: string, args: { req?: { op: string } }) =>
      args.req?.op === 'pair_begin' ? { link: 'l', code: 'ABCD1234', expires_at: 1, host_name: 'h' } : { state: 'waiting', expires_at: 1 },
    );
    await startPairPhone();
    closePairPhone();
    await new Promise((r) => setTimeout(r, 0));
    expect(invoke.mock.calls.at(-1)).toEqual(['local_admin', { req: { op: 'pair_status', code: 'ABCD1234', cancel: true } }]);
  });
});
