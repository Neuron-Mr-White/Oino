// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { DEFAULT_TURN_HEIGHT, HEIGHT_CACHE_MAX, TurnHeightCache } from './heightCache';

describe('TurnHeightCache (§3.6)', () => {
  it('estimate falls back until a row is measured', () => {
    const c = new TurnHeightCache();
    expect(c.estimate('t1')).toBe(DEFAULT_TURN_HEIGHT);
    expect(c.estimate('t1', 200)).toBe(200);
    c.set('t1', 240);
    expect(c.estimate('t1')).toBe(240);
    expect(c.estimate(undefined)).toBe(DEFAULT_TURN_HEIGHT);
  });

  it('ignores non-positive / non-finite heights', () => {
    const c = new TurnHeightCache();
    c.set('t', 0);
    c.set('t', -5);
    c.set('t', NaN);
    expect(c.size).toBe(0);
  });

  it('re-writing refreshes the eviction order (active turns stay warm)', () => {
    const c = new TurnHeightCache(3);
    c.set('a', 10);
    c.set('b', 20);
    c.set('c', 30);
    c.set('a', 11); // a is now the warmest
    c.set('d', 40); // evicts b, not a
    expect(c.get('b')).toBeUndefined();
    expect(c.get('a')).toBe(11);
    expect(c.get('c')).toBe(30);
    expect(c.get('d')).toBe(40);
  });

  it('caps at max entries', () => {
    const c = new TurnHeightCache(HEIGHT_CACHE_MAX);
    for (let i = 0; i < HEIGHT_CACHE_MAX + 50; i++) c.set(`t${i}`, 10 + i);
    expect(c.size).toBe(HEIGHT_CACHE_MAX);
    expect(c.get('t0')).toBeUndefined();
    expect(c.get(`t${HEIGHT_CACHE_MAX + 49}`)).toBeDefined();
  });

  it('clear() resets on session switch', () => {
    const c = new TurnHeightCache();
    c.set('t1', 100);
    c.clear();
    expect(c.size).toBe(0);
    expect(c.estimate('t1')).toBe(DEFAULT_TURN_HEIGHT);
  });
});
