import { describe, expect, it } from 'vitest';
import { changeEnd, EditHistory, undoKey } from './undo';

function typed(h: EditHistory, start: string, chars: string) {
  let text = start;
  for (const c of chars) {
    text += c;
    h.record(text, text.length, 'type');
  }
  return text;
}

describe('EditHistory (UNI-270)', () => {
  it('coalesces typing into word groups', () => {
    const h = new EditHistory({ now: () => 0 });
    h.record('', 0, 'other');
    typed(h, '', 'hello world again');
    expect(h.undo()?.text).toBe('hello world ');
    expect(h.undo()?.text).toBe('hello ');
    expect(h.undo()?.text).toBe('');
    expect(h.undo()).toBeUndefined();
  });

  it('redo walks forward again; new typing after an undo clears redo', () => {
    const h = new EditHistory({ now: () => 0 });
    h.record('', 0, 'other');
    typed(h, '', 'one two');
    expect(h.undo()?.text).toBe('one ');
    expect(h.redo()?.text).toBe('one two');
    expect(h.redo()).toBeUndefined();
    h.undo();
    typed(h, 'one ', 'x');
    expect(h.canRedo()).toBe(false);
    expect(h.undo()?.text).toBe('one ');
  });

  it('a pause, a caret jump or a different kind starts a new group', () => {
    let t = 0;
    const h = new EditHistory({ now: () => t, groupMs: 1000 });
    h.record('', 0, 'other');
    h.record('a', 1, 'type');
    t = 5000;
    h.record('ab', 2, 'type');
    expect(h.undo()?.text).toBe('a');
    h.redo();
    // Caret jump: typing at the start of "ab".
    h.record('xab', 1, 'type');
    expect(h.undo()?.text).toBe('ab');
    h.redo();
    // Delete after type: own group; consecutive Backspaces merge.
    h.record('xa', 2, 'delete');
    h.record('x', 1, 'delete');
    expect(h.undo()?.text).toBe('xab');
  });

  it('paste/chip/pick ("other") is always its own step', () => {
    const h = new EditHistory({ now: () => 0 });
    h.record('', 0, 'other');
    h.record('hi', 2, 'type');
    h.record('hi /skill:review ', 17, 'other');
    h.record('hi /skill:review pasted', 23, 'other');
    expect(h.undo()?.text).toBe('hi /skill:review ');
    expect(h.undo()?.text).toBe('hi');
  });

  it('puts the caret where the undone change was', () => {
    const h = new EditHistory({ now: () => 0 });
    h.record('ac', 0, 'other');
    h.record('abbbc', 4, 'other');
    expect(h.undo()).toEqual({ text: 'ac', caret: 1 });
    expect(h.redo()).toEqual({ text: 'abbbc', caret: 4 });
    expect(changeEnd('abc', 'abc')).toBe(3);
  });

  it('reset leaves exactly one way back; the limit drops the oldest steps', () => {
    const h = new EditHistory({ now: () => 0, limit: 3 });
    h.record('', 0, 'other');
    for (const t of ['a', 'b', 'c', 'd', 'e']) h.record(t, 1, 'other');
    expect(h.reachable()).toEqual(['b', 'c', 'd', 'e']);
    h.reset('sent text');
    h.record('', 0, 'other');
    expect(h.undo()?.text).toBe('sent text');
    expect(h.undo()).toBeUndefined();
  });
});

describe('undoKey', () => {
  const k = (key: string, mods: Partial<KeyboardEvent> = {}) => undoKey({ key, code: '', ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });
  it('maps Ctrl/⌘+Z, Ctrl/⌘+Shift+Z and Ctrl/⌘+Y', () => {
    expect(k('z', { ctrlKey: true })).toBe('undo');
    expect(k('z', { metaKey: true })).toBe('undo');
    expect(k('Z', { ctrlKey: true, shiftKey: true })).toBe('redo');
    expect(k('Z', { metaKey: true, shiftKey: true })).toBe('redo');
    expect(k('y', { ctrlKey: true })).toBe('redo');
    // Non-latin layout: the physical key.
    expect(undoKey({ key: 'я', code: 'KeyZ', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false })).toBe('undo');
  });
  it('ignores plain keys, Alt combos and other letters', () => {
    expect(k('z')).toBeNull();
    expect(k('z', { ctrlKey: true, altKey: true })).toBeNull();
    expect(k('x', { ctrlKey: true })).toBeNull();
    expect(k('Y', { ctrlKey: true, shiftKey: true })).toBeNull();
  });
});
