// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { tabbables, wrapTab } from './focusTrap';

afterEach(() => {
  document.body.innerHTML = '';
});

function dialog() {
  document.body.innerHTML = `
    <button id="outside">behind</button>
    <div id="d" role="dialog">
      <button id="close">x</button>
      <textarea id="title"></textarea>
      <input type="file" hidden id="file" />
      <button id="create" disabled>Create</button>
    </div>`;
  return document.getElementById('d')!;
}
const tab = (shift = false) => new KeyboardEvent('keydown', { key: 'Tab', shiftKey: shift, cancelable: true });

describe('dialog Tab wrap', () => {
  it('skips disabled and hidden controls when finding the ends', () => {
    expect(tabbables(dialog()).map((e) => e.id)).toEqual(['close', 'title']);
  });
  it('Tab on the last real control wraps to the first (a disabled Create at the end no longer lets focus escape)', () => {
    const d = dialog();
    document.getElementById('title')!.focus();
    const e = tab();
    expect(wrapTab(e, d)).toBe(true);
    expect(e.defaultPrevented).toBe(true);
    expect(document.activeElement?.id).toBe('close');
  });
  it('Shift+Tab on the first wraps to the last; in the middle the browser moves focus', () => {
    const d = dialog();
    document.getElementById('close')!.focus();
    expect(wrapTab(tab(true), d)).toBe(true);
    expect(document.activeElement?.id).toBe('title');
    const mid = tab(true);
    expect(wrapTab(mid, d)).toBe(false);
    expect(mid.defaultPrevented).toBe(false);
  });
  it('focus outside the dialog comes back in', () => {
    const d = dialog();
    document.getElementById('outside')!.focus();
    wrapTab(tab(), d);
    expect(document.activeElement?.id).toBe('close');
  });
});
