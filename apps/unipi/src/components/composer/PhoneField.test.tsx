// @vitest-environment jsdom
/**
 * The phone field's inline chips (owner spec 2026-11-03): the textarea stays the
 * only input (Gboard composes natively), a mirror layer draws the chips, and
 * chips stay atomic — Backspace next to one (or over its trailing space) takes
 * the whole token, an edit that still cut into one is widened to the token, and
 * nothing is rewritten while a composition is open.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, describe, expect, it } from 'vitest';
import { PhoneField, repairBrokenChips } from './PhoneField';
import { attachmentToken, imageToken } from '../../lib/composer/chips';

afterEach(() => cleanup());

const opts = { commands: new Set(['skill:review', 'graph']), requireTrailingSpace: true };

function mount(initial: string) {
  const [value, setValue] = createSignal(initial);
  const removed: string[] = [];
  const view = render(() => (
    <PhoneField
      value={value()}
      onChange={(t) => setValue(t)}
      opts={() => opts}
      chipLabel={(c) => (c.kind === 'image' ? 'image 1' : undefined)}
      chipThumb={(c) => (c.kind === 'image' ? 'data:image/png;base64,QQ' : undefined)}
      onChipRemoved={(c) => removed.push(c.raw)}
    />
  ));
  const ta = view.container.querySelector('textarea') as HTMLTextAreaElement;
  return { view, ta, value, setValue, removed };
}
const beforeInput = (el: Element, inputType: string, extra: Record<string, unknown> = {}) => {
  const ev = new InputEvent('beforeinput', { inputType, bubbles: true, cancelable: true, ...extra });
  el.dispatchEvent(ev);
  return ev;
};

describe('repairBrokenChips', () => {
  it('a deletion that cut into a chip takes the whole token (and its space)', () => {
    const prev = 'fix $review now';
    expect(repairBrokenChips(prev, 'fix $revie now', opts)).toEqual({ text: 'fix now', caret: 4 });
    // Only the chip's trailing space went (Gboard's non-cancelable backspace): the chip goes whole.
    expect(repairBrokenChips(prev, 'fix $reviewnow', opts)).toEqual({ text: 'fix now', caret: 4 });
    expect(repairBrokenChips(prev, 'fix  now', opts)).toBeUndefined(); // removed whole
    expect(repairBrokenChips(prev, 'fix $rXeview now', opts)).toEqual({ text: 'fix X now', caret: 5 });
    expect(repairBrokenChips('a b', 'a xb', opts)).toBeUndefined();
  });
});

describe('PhoneField chips', () => {
  it('no chips: plain textarea, no mirror (the text renders natively)', () => {
    const { view } = mount('just text');
    expect(view.container.querySelector('[data-testid="phone-field-mirror"]')).toBeNull();
    expect(view.container.querySelector('.ph-field-box')!.hasAttribute('data-chips')).toBe(false);
  });

  it('draws chips in the mirror with the exact token text; image/file chips get a label over it', () => {
    const { view } = mount(`/graph go $review and ${imageToken('i1')} ${attachmentToken('a.pdf', 'f1')} x`);
    const mirror = view.container.querySelector('[data-testid="phone-field-mirror"]')!;
    expect(view.container.querySelector('.ph-field-box')!.hasAttribute('data-chips')).toBe(true);
    const chips = [...mirror.querySelectorAll('.phf-chip')] as HTMLElement[];
    expect(chips.map((c) => c.dataset.kind)).toEqual(['command', 'skill', 'image', 'file']);
    expect(chips[0]!.hasAttribute('data-mode')).toBe(true);
    // Same characters as the textarea, so the layout lines up.
    expect(mirror.textContent!.startsWith('/graph go $review and [image]@img:i1')).toBe(true);
    expect(chips[2]!.querySelector('.phf-chip-text')!.textContent).toBe('image 1');
    expect(chips[2]!.querySelector('img')!.getAttribute('src')).toBe('data:image/png;base64,QQ');
    expect(chips[3]!.querySelector('.phf-chip-text')!.textContent).toBe('a.pdf');
  });

  it('Backspace after a chip (or over its trailing space) removes the whole token', () => {
    const { ta, value, removed } = mount('fix $review now');
    ta.focus();
    ta.setSelectionRange(12, 12); // after "$review "
    expect(beforeInput(ta, 'deleteContentBackward').defaultPrevented).toBe(true);
    expect(value()).toBe('fix now');
    expect(ta.value).toBe('fix now');
    expect(ta.selectionStart).toBe(4);
    expect(removed).toEqual(['$review']);
  });

  it('never intercepts while a composition is open, and plain Backspace is native', () => {
    const { ta, value } = mount('fix $review now');
    ta.focus();
    ta.setSelectionRange(12, 12);
    fireEvent.compositionStart(ta);
    expect(beforeInput(ta, 'deleteContentBackward', { isComposing: true }).defaultPrevented).toBe(false);
    fireEvent.compositionEnd(ta);
    ta.setSelectionRange(2, 2);
    expect(beforeInput(ta, 'deleteContentBackward').defaultPrevented).toBe(false);
    expect(value()).toBe('fix $review now');
  });

  it('an input that still broke a chip is repaired to the whole token', () => {
    const { ta, value } = mount('fix $review now');
    ta.focus();
    ta.value = 'fix $revie now';
    ta.setSelectionRange(10, 10);
    fireEvent.input(ta);
    expect(value()).toBe('fix now');
    expect(ta.value).toBe('fix now');
  });

  it("Gboard's backspace over a chip's trailing space (an input, not a cancelable beforeinput) removes the chip", () => {
    const { ta, value } = mount('fix it with $review and');
    ta.focus();
    ta.value = 'fix it with $reviewand';
    ta.setSelectionRange(19, 19);
    fireEvent.input(ta);
    expect(value()).toBe('fix it with and');
    expect(ta.selectionStart).toBe(12);
  });

  it('a caret landing inside a chip moves to its end', () => {
    const { ta } = mount('fix $review now');
    ta.focus();
    ta.setSelectionRange(6, 6);
    fireEvent.click(ta);
    expect(ta.selectionStart).toBe(11);
  });
});
