// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { clearScrollMemory, readScrollMemory, scrollMemoryKey, scrollMemorySize, writeScrollMemory, SCROLL_MEMORY_MAX } from './scrollMemory';

afterEach(clearScrollMemory);

describe('scroll memory (§3.3/§3.6)', () => {
  it('stores a pixel position or the bottom-lock flag per key', () => {
    const k1 = scrollMemoryKey('main', 'pc1', 's1');
    const k2 = scrollMemoryKey('main', 'pc1', 's2');
    writeScrollMemory(k1, { scrollTop: 1234 });
    writeScrollMemory(k2, { bottomLocked: true });
    expect(readScrollMemory(k1)).toEqual({ scrollTop: 1234 });
    expect(readScrollMemory(k2)).toEqual({ bottomLocked: true });
    expect(readScrollMemory(scrollMemoryKey('main', 'pc1', 's3'))).toBeUndefined();
  });

  it('keys isolate panes, hosts and sessions', () => {
    expect(scrollMemoryKey('main', 'a', 's')).not.toBe(scrollMemoryKey('side', 'a', 's'));
    expect(scrollMemoryKey('main', 'a', 's')).not.toBe(scrollMemoryKey('main', 'b', 's'));
  });

  it('LRU: writing refreshes order; evicts the coldest past 200', () => {
    const keys = Array.from({ length: SCROLL_MEMORY_MAX }, (_, i) => scrollMemoryKey('main', 'h', `s${i}`));
    for (const k of keys) writeScrollMemory(k, { scrollTop: 1 });
    expect(scrollMemorySize()).toBe(SCROLL_MEMORY_MAX);
    writeScrollMemory(keys[0]!, { scrollTop: 2 }); // warm s0
    writeScrollMemory(scrollMemoryKey('main', 'h', 'new'), { scrollTop: 3 });
    expect(scrollMemorySize()).toBe(SCROLL_MEMORY_MAX);
    expect(readScrollMemory(keys[0]!)).toEqual({ scrollTop: 2 });
    expect(readScrollMemory(keys[1]!)).toBeUndefined(); // coldest evicted
  });
});
