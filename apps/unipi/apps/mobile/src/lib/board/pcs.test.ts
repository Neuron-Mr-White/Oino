import { afterEach, describe, expect, it } from 'vitest';
import { availabilityText, boardPcs, DEV_PC, pcAvailability, resetPcAvailability } from './pcs';
import { DEV_HOST_ID, setBoardTransport } from './transport';
import { fakeBoard } from './testing';

afterEach(() => {
  setBoardTransport(undefined);
  resetPcAvailability();
});

describe('board PCs', () => {
  it('lists paired hosts, or the dev PC in a browser without hosts', () => {
    expect(boardPcs([{ id: 'a', name: 'coffee' }], true)).toEqual([{ id: 'a', name: 'coffee' }]);
    expect(boardPcs([], true)).toEqual([]);
    expect(boardPcs([], false)).toEqual([DEV_PC]);
    expect(DEV_PC).toEqual({ id: DEV_HOST_ID, name: 'dev (vite proxy)', dev: true });
  });

  it('checks availability per PC: ok / offline / not installed / error', async () => {
    const fake = fakeBoard({
      availability: {
        ok: { state: 'ok', version: '3.0.0' },
        off: { state: 'offline' },
        none: { state: 'not_installed', hint: 'install unipi-host' },
        bad: { state: 'error', message: 'connection reset' },
      },
    });
    setBoardTransport(fake.transport);
    const avail = pcAvailability();
    const pending = avail.check('ok');
    expect(avail.availability('ok')).toEqual({ state: 'checking' });
    await pending;
    await Promise.all(['off', 'none', 'bad'].map((id) => avail.check(id)));
    expect(availabilityText(avail.availability('ok'))).toBe('Board 3.0.0');
    expect(availabilityText(avail.availability('off'))).toBe('Offline');
    expect(availabilityText(avail.availability('none'))).toBe('install unipi-host');
    expect(availabilityText(avail.availability('bad'))).toBe('connection reset');
    expect(availabilityText(undefined)).toBe('Checking…');
  });

  it('a throwing transport counts as an error state', async () => {
    setBoardTransport({
      availability: async () => {
        throw new Error('boom');
      },
      request: async () => ({ status: 500, contentType: '', body: new Uint8Array() }),
      events: () => () => {},
    });
    expect(await pcAvailability().check('x')).toEqual({ state: 'error', message: 'boom' });
  });
});
