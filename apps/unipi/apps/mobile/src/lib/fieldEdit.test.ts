// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { diffRange, setFieldValue } from './fieldEdit';

afterEach(() => {
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('diffRange', () => {
  it('finds the smallest replaced span', () => {
    expect(diffRange('see @ab', 'see @UNI-1 ')).toEqual({ start: 5, end: 7, insert: 'UNI-1 ' });
    expect(diffRange('abc', 'abc')).toEqual({ start: 3, end: 3, insert: '' });
    expect(diffRange('a**b**c', 'abc')).toEqual({ start: 1, end: 6, insert: 'b' });
  });
});

describe('setFieldValue (UNI-270)', () => {
  const area = (value: string) => {
    const el = document.createElement('textarea');
    el.value = value;
    document.body.append(el);
    return el;
  };
  it('edits the focused field through execCommand insertText (one native undo step)', () => {
    const el = area('hi @ab rest');
    el.focus();
    const exec = vi.fn((cmd: string, _ui: boolean, text?: string) => {
      expect(cmd).toBe('insertText');
      const { selectionStart: s, selectionEnd: e, value } = el;
      el.value = value.slice(0, s) + text + value.slice(e);
      return true;
    });
    Object.defineProperty(document, 'execCommand', { value: exec, configurable: true });
    expect(setFieldValue(el, 'hi @UNI-7  rest', 10)).toBe(true);
    expect(exec).toHaveBeenCalledWith('insertText', false, 'UNI-7 ');
    expect(el.value).toBe('hi @UNI-7  rest');
    expect(el.selectionStart).toBe(10);
  });
  it('a pure deletion uses execCommand delete', () => {
    const el = area('a**b**c');
    el.focus();
    const exec = vi.fn((cmd: string) => {
      expect(cmd).toBe('delete');
      el.value = el.value.slice(0, el.selectionStart) + el.value.slice(el.selectionEnd);
      return true;
    });
    Object.defineProperty(document, 'execCommand', { value: exec, configurable: true });
    setFieldValue(el, 'ac');
    expect(el.value).toBe('ac');
  });
  it('falls back to setting the value when execCommand is refused or the field is not focused', () => {
    const el = area('x');
    el.focus();
    Object.defineProperty(document, 'execCommand', { value: vi.fn(() => false), configurable: true });
    expect(setFieldValue(el, 'xy', 1, 2)).toBe(false);
    expect(el.value).toBe('xy');
    expect([el.selectionStart, el.selectionEnd]).toEqual([1, 2]);
    const other = area('a');
    const exec = vi.fn(() => true);
    Object.defineProperty(document, 'execCommand', { value: exec, configurable: true });
    expect(setFieldValue(other, 'ab')).toBe(false);
    expect(exec).not.toHaveBeenCalled();
    expect(other.value).toBe('ab');
    expect(document.activeElement).toBe(el);
  });
});
