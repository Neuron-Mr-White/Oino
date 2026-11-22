/**
 * The picker's zoxide seam: the host's `fs_zoxide` answers for every PC (the phone, remote PCs and
 * the desktop's own); the own PC falls back to the Tauri `local_zoxide` only when the host cannot.
 * "Not installed" is an answer, never an error.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

const fs = vi.hoisted(() => ({ zoxide: vi.fn() }));
vi.mock('./fs', () => ({ fsZoxide: (...a: unknown[]) => fs.zoxide(...a) }));
const local = vi.hoisted(() => ({ own: false, zoxide: vi.fn() }));
vi.mock('./openLocal', () => ({ isOwnPcFile: () => local.own, localZoxide: (...a: unknown[]) => local.zoxide(...a) }));

import { zoxideQuery } from './zoxide';

afterEach(() => {
  fs.zoxide.mockReset();
  local.zoxide.mockReset();
  local.own = false;
});

describe('zoxideQuery', () => {
  it('returns the host\'s scope-filtered hits for the phone and remote PCs', async () => {
    fs.zoxide.mockResolvedValue({ ok: true, value: { available: true, hits: [{ path: '/home/u/oino', score: 9 }, { path: '/home/u/web', score: 3 }] } });
    expect(await zoxideQuery('pc', 'oi', { limit: 1 })).toEqual({ available: true, hits: [{ path: '/home/u/oino', score: 9 }] });
    expect(fs.zoxide).toHaveBeenCalledWith('pc', 'oi', 1);
    expect(local.zoxide).not.toHaveBeenCalled();
  });

  it('keeps "zoxide is not installed" a normal answer', async () => {
    fs.zoxide.mockResolvedValue({ ok: true, value: { available: false, hits: [] } });
    expect(await zoxideQuery('pc', '')).toEqual({ available: false, hits: [] });
  });

  it('a host that cannot answer (older, offline) is "not available" off this PC', async () => {
    fs.zoxide.mockResolvedValue({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown op' } });
    expect(await zoxideQuery('pc', 'x')).toEqual({ available: false, hits: [] });
    fs.zoxide.mockRejectedValue(new Error('offline'));
    expect(await zoxideQuery('pc', 'x')).toEqual({ available: false, hits: [] });
    expect(local.zoxide).not.toHaveBeenCalled();
  });

  it('own PC: falls back to local_zoxide, without the home, / and scratch folders', async () => {
    local.own = true;
    fs.zoxide.mockResolvedValue({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown op' } });
    local.zoxide.mockResolvedValue({ ok: true, hits: [{ path: '/home/u', score: 99 }, { path: '/tmp/x', score: 50 }, { path: '/home/u/oino', score: 9 }] });
    expect(await zoxideQuery('local', 'o', { home: '/home/u' })).toEqual({ available: true, hits: [{ path: '/home/u/oino', score: 9 }] });
    local.zoxide.mockResolvedValue({ ok: false, message: 'zoxide isn\'t installed' });
    expect(await zoxideQuery('local', 'o')).toEqual({ available: false, hits: [] });
  });
});
