/**
 * WP-A25, G-15 `/scoped-models`: the cycle set the picker edits —
 * pi's `enabledModels` patterns, where checkboxes own the exact references and
 * globs/thinking suffixes stay as written.
 */
import { describe, expect, it } from 'vitest';
import { cycleKeyOf, cycleKeysOf, cycleLabel, cycleList, isExactPattern, toggleCycleKey } from './cycle';

describe('isExactPattern', () => {
  it('accepts provider/model, rejects globs and thinking suffixes', () => {
    expect(isExactPattern('anthropic/claude-opus-5-5')).toBe(true);
    expect(isExactPattern('anthropic/*')).toBe(false);
    expect(isExactPattern('anthropic/claude-opus-5-5:high')).toBe(false);
    expect(isExactPattern('claude')).toBe(false);
    expect(isExactPattern('/model')).toBe(false);
    expect(isExactPattern('provider/')).toBe(false);
  });
});

describe('cycleKeysOf', () => {
  it('splits the stored value; an unset value means “every model”', () => {
    expect(cycleKeysOf(undefined)).toEqual({ exact: [], custom: [] });
    expect(cycleKeysOf(null)).toEqual({ exact: [], custom: [] });
    expect(cycleKeysOf(['a/x', 'b/*', 'c/y:low', 'a/x', '', 42])).toEqual({ exact: ['a/x'], custom: ['b/*', 'c/y:low'] });
  });
});

describe('toggleCycleKey', () => {
  it('adds and removes exact keys, keeping custom patterns', () => {
    expect(toggleCycleKey([], [], 'a/x', true)).toEqual(['a/x']);
    expect(toggleCycleKey(['a/x'], ['b/*'], 'a/x', false)).toEqual(['b/*']);
    expect(toggleCycleKey(['a/x'], [], 'b/y', true)).toEqual(['a/x', 'b/y']);
  });

  it('an empty set deletes the setting (every model cycles again)', () => {
    expect(toggleCycleKey(['a/x'], [], 'a/x', false)).toBeUndefined();
  });
});

describe('cycleLabel', () => {
  it('reads the set', () => {
    expect(cycleLabel([], [], 12)).toBe('Cycle set · every model (12)');
    expect(cycleLabel(['a/x', 'b/y'], [], 12)).toBe('Cycle set · 2 of 12 models');
    expect(cycleLabel(['a/x'], ['c/*'], 12)).toBe('Cycle set · 1 of 12 models + 1 pattern');
  });
});

describe('cycleKeyOf', () => {
  it('is the provider/id form `enabledModels` stores', () => {
    expect(cycleKeyOf('anthropic', 'claude-opus-5-5')).toBe('anthropic/claude-opus-5-5');
  });
});

describe('F-04 cycleList', () => {
  const usable = ['a/x', 'a/y', 'b/z'];
  it('uses the stored set (globs resolved, level suffix dropped), only usable models', () => {
    expect(cycleList({ patterns: ['b/z:high', 'a/*', 'gone/q'], curated: [], usable })).toEqual(['b/z', 'a/x', 'a/y']);
  });
  it('falls back to the curated list, then to a small catalogue; never a big one', () => {
    expect(cycleList({ patterns: [], curated: ['a/y', 'nope/x'], usable })).toEqual(['a/y']);
    expect(cycleList({ patterns: [], curated: [], usable })).toEqual(usable);
    const big = Array.from({ length: 40 }, (_, i) => `p/m${i}`);
    expect(cycleList({ patterns: [], curated: [], usable: big })).toBeUndefined();
  });
});
