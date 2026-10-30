import { describe, expect, it } from 'vitest';
import { boardKey, isTyping, stepSelection, type KeyContext } from './keys';

const ctx = (over: Partial<KeyContext> = {}): KeyContext => ({ typing: false, overlay: false, taskOpen: false, project: true, filtered: false, ...over });
const key = (k: string, mods: Partial<KeyboardEvent> = {}) => ({ key: k, ctrlKey: false, metaKey: false, altKey: false, ...mods });

describe('board keys', () => {
  it('maps the web UI keymap', () => {
    expect(boardKey(key('c'), ctx())).toEqual({ kind: 'new' });
    expect(boardKey(key('/'), ctx())).toEqual({ kind: 'search' });
    expect(boardKey(key('j'), ctx())).toEqual({ kind: 'select', delta: 1 });
    expect(boardKey(key('ArrowUp'), ctx())).toEqual({ kind: 'select', delta: -1 });
    expect(boardKey(key('Enter'), ctx())).toEqual({ kind: 'open' });
    expect(boardKey(key('b'), ctx())).toEqual({ kind: 'view', view: 'board' });
    expect(boardKey(key('l'), ctx())).toEqual({ kind: 'view', view: 'list' });
    expect(boardKey(key('r'), ctx())).toEqual({ kind: 'scope', scope: 'in_review' });
    expect(boardKey(key('x'), ctx())).toEqual({ kind: 'scope', scope: 'blocked' });
    expect(boardKey(key('?'), ctx())).toEqual({ kind: 'help' });
  });
  it('stays out of the way while typing, with overlays, modifiers, or no project', () => {
    expect(boardKey(key('c'), ctx({ typing: true }))).toBeUndefined();
    expect(boardKey(key('c'), ctx({ overlay: true }))).toBeUndefined();
    expect(boardKey(key('k', { ctrlKey: true }), ctx())).toBeUndefined();
    expect(boardKey(key('c'), ctx({ project: false }))).toBeUndefined();
    expect(boardKey(key('?'), ctx({ project: false }))).toEqual({ kind: 'help' });
  });
  it('J/K/Enter only drive the board while no task is open; Esc clears the filter', () => {
    expect(boardKey(key('j'), ctx({ taskOpen: true }))).toBeUndefined();
    expect(boardKey(key('Enter'), ctx({ taskOpen: true }))).toBeUndefined();
    expect(boardKey(key('Escape'), ctx({ filtered: true }))).toEqual({ kind: 'clear' });
    expect(boardKey(key('Escape'), ctx({ filtered: true, taskOpen: true }))).toBeUndefined();
    expect(boardKey(key('Escape'), ctx())).toBeUndefined();
  });
  it('steps the selection and clamps at the ends', () => {
    expect(stepSelection([], null, 1)).toBeUndefined();
    expect(stepSelection(['a', 'b', 'c'], null, 1)).toBe('a');
    expect(stepSelection(['a', 'b', 'c'], null, -1)).toBe('c');
    expect(stepSelection(['a', 'b', 'c'], 'b', 1)).toBe('c');
    expect(stepSelection(['a', 'b', 'c'], 'c', 1)).toBe('c');
    expect(stepSelection(['a', 'b', 'c'], 'gone', -1)).toBe('c');
  });
  it('detects text fields', () => {
    expect(isTyping(document.createElement('textarea'))).toBe(true);
    expect(isTyping(document.createElement('button'))).toBe(false);
    expect(isTyping(null)).toBe(false);
  });
});

describe('selection ring CSS', () => {
  it('beats the running card outline (J/K onto a running task stays visible)', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const css = readFileSync(resolve(__dirname, '../../components/board/dashboard.css'), 'utf8');
    // `.bcard.running` (board.css, loaded later) has the same specificity as `.bcard.kb-selected`.
    expect(css).toMatch(/\.bcard\.running\.kb-selected[^{]*\{[^}]*wm-cyan/);
  });
});
