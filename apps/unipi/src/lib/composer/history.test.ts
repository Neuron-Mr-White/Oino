/**
 * Prompt history (WP-A12, Z-58; ZCode composer-prompt-history):
 * - append: consecutive duplicates (trim-equal) skipped, `A,B,A` kept, cap 30;
 * - read: stored duplicates are returned as-is (no migration);
 * - walk: ↑ enters at the newest on an empty draft, ↓ leaves back to the
 *   draft being typed;
 * - rollback: a failed send undoes its append only while it is still last.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { PROMPT_HISTORY_LIMIT, appendPrompt, promptHistory, walkDown, walkUp } from './history';

const mem = () => {
  let backing = new Map<string, string>();
  return {
    getItem: (k: string) => backing.get(k) ?? null,
    setItem: (k: string, v: string) => void backing.set(k, v),
    removeItem: (k: string) => void backing.delete(k),
    dump: () => backing,
  };
};

afterEach(() => localStorage.clear());

describe('appendPrompt (write path)', () => {
  it('appends a new prompt', () => {
    expect(appendPrompt([], 'hello')).toEqual(['hello']);
  });

  it('skips a consecutive duplicate, trim-equal (PH01: " A " === "A")', () => {
    expect(appendPrompt(['A'], ' A ')).toEqual(['A']);
  });

  it('keeps non-consecutive repeats (PH02: A,B,A)', () => {
    expect(appendPrompt(['A', 'B'], 'A')).toEqual(['A', 'B', 'A']);
  });

  it('inner whitespace still differs (PH03: "A B" vs "A  B")', () => {
    expect(appendPrompt(['A B'], 'A  B')).toEqual(['A B', 'A  B']);
  });

  it('caps at 30, dropping the oldest', () => {
    let list: string[] = [];
    for (let i = 0; i < 40; i++) list = appendPrompt(list, `p${i}`);
    expect(list).toHaveLength(PROMPT_HISTORY_LIMIT);
    expect(list[0]).toBe('p10');
    expect(list.at(-1)).toBe('p39');
  });

  it('blank text is never recorded', () => {
    expect(appendPrompt(['A'], '  ')).toEqual(['A']);
  });
});

describe('read path', () => {
  it('stored consecutive duplicates stay as stored (PH04: no read-path dedup)', () => {
    const st = mem();
    st.setItem('unipi:composer:history:h%3A%2Fp', JSON.stringify(['A', 'A']));
    expect(promptHistory('h:/p', st).list()).toEqual(['A', 'A']);
  });
});

describe('the walk (↑/↓)', () => {
  const list = ['one', 'two', 'three'];

  it('↑ from null enters at the newest, then walks back, clamping at 0', () => {
    expect(walkUp(list, null)).toBe(2);
    expect(walkUp(list, 2)).toBe(1);
    expect(walkUp(list, 1)).toBe(0);
    expect(walkUp(list, 0)).toBe(0);
  });

  it('↓ walks forward and null past the newest returns to the draft', () => {
    expect(walkDown(list, 0)).toBe(1);
    expect(walkDown(list, 2)).toBeNull();
    expect(walkDown(list, null)).toBeNull();
  });

  it('an empty history never enters the walk', () => {
    expect(walkUp([], null)).toBeNull();
  });
});

describe('promptHistory store', () => {
  it('add persists and rollback undoes only while last (PH05)', () => {
    const st = mem();
    const h = promptHistory('h:/p', st);
    h.add('one');
    h.add('two');
    expect(h.list()).toEqual(['one', 'two']);
    h.rollback('two');
    expect(h.list()).toEqual(['one']);
    // A later send wins: the stale rollback is a no-op.
    h.add('three');
    h.rollback('two');
    expect(h.list()).toEqual(['one', 'three']);
    // Trim-equality on both sides.
    h.add(' three ');
    expect(h.list()).toEqual(['one', 'three']);
  });
});
