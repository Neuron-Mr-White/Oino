import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  checkDesktopUpdate,
  desktopUpdateState,
  dismissDesktopUpdate,
  installDesktopUpdate,
  promptOpen,
  resetDesktopUpdate,
  RECHECK_MS,
  setDesktopUpdateTransport,
} from './desktopUpdate';

const available = { current: '0.4.4', kind: 'in_app', version: '0.4.5', notes: 'Fixes', downloadPage: 'https://unipi.nrn.one/' };

afterEach(() => {
  resetDesktopUpdate();
  setDesktopUpdateTransport(undefined);
});

describe('desktop app updates (UNI-263)', () => {
  it('a newer version opens the prompt; nothing installs until the user agrees', async () => {
    const install = vi.fn(async () => {});
    setDesktopUpdateTransport({ check: async () => available, install });
    await checkDesktopUpdate();
    expect(desktopUpdateState().kind).toBe('available');
    expect(promptOpen()).toBe(true);
    expect(install).not.toHaveBeenCalled();
  });

  it('"Later" hides the prompt for that version only', async () => {
    let feed: Record<string, unknown> = available;
    setDesktopUpdateTransport({ check: async () => feed, install: vi.fn() });
    await checkDesktopUpdate(true);
    dismissDesktopUpdate();
    expect(promptOpen()).toBe(false);
    feed = { ...available, version: '0.4.6' };
    await checkDesktopUpdate(true);
    expect(promptOpen()).toBe(true);
  });

  it('install reports progress, passes the version the user agreed to, and keeps errors retryable', async () => {
    const seen: string[] = [];
    let fail = true;
    setDesktopUpdateTransport({
      check: async () => available,
      install: async (expect, onProgress) => {
        seen.push(expect);
        onProgress({ kind: 'progress', done: 10, total: 100 });
        expect === '0.4.5' && seen.push(desktopUpdateState().kind);
        if (fail) throw { code: 'E_INTERNAL', message: 'Download failed: signature mismatch' };
        onProgress({ kind: 'installing' });
      },
    });
    await checkDesktopUpdate(true);
    await installDesktopUpdate();
    expect(seen).toEqual(['0.4.5', 'downloading']);
    const s = desktopUpdateState();
    expect(s.kind).toBe('error');
    expect(s.kind === 'error' && s.message).toContain('signature mismatch');
    expect(promptOpen()).toBe(true);
    fail = false;
    await installDesktopUpdate();
    expect(desktopUpdateState().kind).toBe('installing');
  });

  it('package installs (deb/tar.gz) never install in-app', async () => {
    const install = vi.fn(async () => {});
    setDesktopUpdateTransport({ check: async () => ({ ...available, kind: 'manual' }), install });
    await checkDesktopUpdate(true);
    await installDesktopUpdate();
    expect(install).not.toHaveBeenCalled();
    expect(desktopUpdateState().kind).toBe('available');
  });

  it('up to date, rate limit, quiet background failures, loud forced ones', async () => {
    const check = vi.fn(async () => ({ current: '0.4.5', kind: 'in_app', downloadPage: 'https://unipi.nrn.one/' }));
    setDesktopUpdateTransport({ check, install: vi.fn() });
    await checkDesktopUpdate(false, 100_000_000);
    expect(desktopUpdateState().kind).toBe('current');
    expect(promptOpen()).toBe(false);
    await checkDesktopUpdate(false, 100_000_000 + RECHECK_MS - 1);
    expect(check).toHaveBeenCalledTimes(1);
    check.mockRejectedValueOnce(new Error('offline'));
    await checkDesktopUpdate(false, 100_000_000 + RECHECK_MS + 1);
    expect(desktopUpdateState().kind).toBe('current');
    check.mockRejectedValueOnce(new Error('offline'));
    await checkDesktopUpdate(true);
    expect(desktopUpdateState()).toMatchObject({ kind: 'error', message: 'offline' });
    expect(promptOpen()).toBe(false);
    check.mockResolvedValueOnce({ garbage: true } as never);
    await checkDesktopUpdate(true);
    expect(desktopUpdateState().kind).toBe('error');
  });
});
