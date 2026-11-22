// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { clearExpansion, dropSessionExpansion, expansionOpen, expansionSize, setExpansion, toggleExpansion, EXPANSION_MAX } from './expansion';

afterEach(clearExpansion);

describe('expansion memory (§3.3)', () => {
  it('falls back until the user toggles', () => {
    expect(expansionOpen('s1', 'fold:a', false)).toBe(false);
    expect(expansionOpen('s1', 'fold:a', true)).toBe(true);
    setExpansion('s1', 'fold:a', true);
    expect(expansionOpen('s1', 'fold:a', false)).toBe(true);
    toggleExpansion('s1', 'fold:a', false);
    expect(expansionOpen('s1', 'fold:a', true)).toBe(false);
  });

  it('keys are scoped per session', () => {
    setExpansion('s1', 'tool:t1', true);
    expect(expansionOpen('s2', 'tool:t1', false)).toBe(false);
    dropSessionExpansion('s1');
    expect(expansionOpen('s1', 'tool:t1', false)).toBe(false);
  });

  it('LRU caps at 5000', () => {
    for (let i = 0; i < EXPANSION_MAX + 10; i++) setExpansion('s', `k${i}`, true);
    expect(expansionSize()).toBe(EXPANSION_MAX);
    expect(expansionOpen('s', 'k0', false)).toBe(false);
  });
});
