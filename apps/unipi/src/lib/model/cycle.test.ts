/**
 * WP-A25, G-15 `/scoped-models`: the cycle set the picker edits —
 * pi's `enabledModels` patterns, where checkboxes own the exact references and
 * globs/thinking suffixes stay as written.
 */
import { describe, expect, it } from 'vitest';
import { cycleKeyOf, cycleKeysOf, cycleLabel, isExactPattern, toggleCycleKey } from './cycle';

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
