import { afterEach, describe, expect, it } from 'vitest';
import { availabilityText, boardPcs, pcAvailability, resetPcAvailability } from './pcs';
import { setBoardTransport } from './transport';
import { fakeBoard } from './testing';

afterEach(() => {
  setBoardTransport(undefined);
  resetPcAvailability();
});

describe('board PCs', () => {
  it('lists the hosts the app knows (no synthetic web PC in v2)', () => {
    expect(boardPcs([{ id: 'a', name: 'coffee' }])).toEqual([{ id: 'a', name: 'coffee' }]);
    expect(boardPcs([])).toEqual([]);
  });

  it('checks availability per PC: ok / offline / not installed / error', async () => {
    const fake = fakeBoard({
      availability: {
        ok: { state: 'ok' },
        off: { state: 'offline' },
        none: { state: 'not_installed', hint: 'install unipi-kanboard' },
        bad: { state: 'error', message: 'connection reset' },
      },
    });
    setBoardTransport(fake.transport);
    const avail = pcAvailability();
    const pending = avail.check('ok');
    expect(avail.availability('ok')).toEqual({ state: 'checking' });
    await pending;
    await Promise.all(['off', 'none', 'bad'].map((id) => avail.check(id)));
    expect(availabilityText(avail.availability('ok'))).toBe('Board ready');
    expect(availabilityText(avail.availability('off'))).toBe('Offline');
    expect(availabilityText(avail.availability('none'))).toBe('install unipi-kanboard');
    expect(availabilityText(avail.availability('bad'))).toBe('connection reset');
    expect(availabilityText(undefined)).toBe('Checking…');
  });

  it('a throwing transport counts as an error state', async () => {
    setBoardTransport({
      availability: async () => {
        throw new Error('boom');
      },
      cli: async () => {
        throw new Error('unreachable');
      },
    });
    expect(await pcAvailability().check('x')).toEqual({ state: 'error', message: 'boom' });
  });
});
