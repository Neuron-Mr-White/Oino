import { createRoot } from 'solid-js';
import { createStore } from 'solid-js/store';
import { expect, it, vi } from 'vitest';
import { rememberedAutonomy, resetRememberedAutonomy } from './chatAutonomy';

vi.mock('../host/localId', () => ({ resolveHostId: (id: string) => id }));
const { createSessionRegistry } = await import('./store');

/** N2: the registry (LRU) remembers each chat's level before it lets the handle go. */
it('keeps the level of a chat after the registry evicted its handle', async () => {
  resetRememberedAutonomy();
  const [state] = createStore({ conn: 'live', waking: false, info: { cwd: '/p', autonomy: 'high' }, entries: [] as unknown[] });
  const make = (_h: string, id: string) => {
    const h = { hostId: 'h', sessionId: () => id, state: id === 'zm' ? state : createStore({ conn: 'live', waking: false, info: { cwd: '/q', autonomy: 'low' }, entries: [] })[0], refs: 0, closed: false, close: () => { h.closed = true; } };
    return h as never;
  };
  const reg = createRoot(() => createSessionRegistry(1, make));
  reg.ensure('h', 'zm');
  await Promise.resolve();
  expect(rememberedAutonomy('h', 'zm')).toBe('high');
  reg.ensure('h', 'other'); // evicts zm
  expect(reg.peek('h', 'zm')).toBeUndefined();
  expect(rememberedAutonomy('h', 'zm')).toBe('high');
});
