// @vitest-environment jsdom
/**
 * The composer toolbar's layout ownership (WP-A13 acceptance, Z-64): the bar
 * measures its items, picks the smallest of the seven compression steps that
 * fits, publishes it as `data-step` plus the per-control `data-c-*` flags the
 * stylesheet folds on, restores as it widens, and keeps the DOM order
 * `+ … controls … Send`. jsdom has no layout, so the item/bar widths come
 * from `data-w` / `data-cw` through stubbed measurement.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { Toolbar, ToolbarSlot } from './Toolbar';
import { compressionStep, type ToolbarItem } from '../../lib/composer/compression';

/** A `ToolbarSlot`'s box is its child's width (jsdom has no layout). */
const rect = (el: HTMLElement) => Number(el.dataset.w ?? (el.firstElementChild as HTMLElement | null)?.dataset.w ?? 0);

beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value(this: HTMLElement) {
      const w = rect(this);
      return { width: w, height: 28, top: 0, left: 0, right: w, bottom: 28, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    },
  });
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get(this: HTMLElement) {
      return Number(this.dataset.cw ?? 0);
    },
  });
});

afterAll(() => {
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).getBoundingClientRect;
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientWidth;
});

afterEach(() => cleanup());

/** The same item set at every width: + / permission / plan / think / model / send. */
const widths: ToolbarItem[] = [
  { width: 28 },
  { slot: 'permission', width: 84 },
  { slot: 'plan', width: 76 },
  { slot: 'think', width: 92 },
  { slot: 'model', width: 148 },
  { width: 40 },
];

function mount(available: number) {
  const view = render(() => (
    <Toolbar lead={<span data-w="28">+</span>} end={<span data-w="40">send</span>}>
      <ToolbarSlot slot="permission">
        <div data-w="84">permission</div>
      </ToolbarSlot>
      <ToolbarSlot slot="plan">
        <div data-w="76">plan</div>
      </ToolbarSlot>
      <ToolbarSlot slot="think">
        <div data-w="92">think</div>
      </ToolbarSlot>
      <ToolbarSlot slot="model">
        <div data-w="148">model</div>
      </ToolbarSlot>
    </Toolbar>
  ));
  const bar = view.container.querySelector('.comp-bar') as HTMLElement;
  bar.dataset.cw = String(available);
  // Re-run the decision the way a resize would.
  fireEvent(window, new Event('resize'));
  return { view, bar };
}

describe('toolbar compression (Z-64)', () => {
  it('keeps the DOM order: + , permission, plan, think, model, Send', () => {
    const { view } = mount(2000);
    const slots = [...view.container.querySelectorAll('[data-tb-item]')].map((el) => el.getAttribute('data-slot') ?? el.textContent?.trim());
    expect(slots).toEqual(['+', 'permission', 'plan', 'think', 'model', 'send']);
  });

  it('full width stays at step 0; each narrower width takes one more step', () => {
    // Widths straight from the pure decision, so this pins the wiring, not the numbers.
    const full = compressionStep(2000, widths);
    expect(full).toBe(0);
    const w2 = compressionStep(460, widths);
    expect(w2).toBe(2);
    const w4 = compressionStep(390, widths);
    expect(w4).toBe(4);
    const w7 = compressionStep(260, widths);
    expect(w7).toBe(7);

    const fullBar = mount(2000).bar;
    expect(fullBar.dataset.step).toBe('0');
    expect(fullBar.hasAttribute('data-c-permission')).toBe(false);
    expect(fullBar.hasAttribute('data-c-model')).toBe(false);

    const two = mount(460).bar;
    expect(two.dataset.step).toBe('2');
    expect(two.hasAttribute('data-c-permission')).toBe(true);
    expect(two.hasAttribute('data-c-plan')).toBe(false);

    const four = mount(390).bar;
    expect(four.dataset.step).toBe('4');
    expect(four.dataset.cThink).toBe('lite');
    expect(four.hasAttribute('data-c-plan')).toBe(true);

    const seven = mount(260).bar;
    expect(seven.dataset.step).toBe('7');
    expect(seven.dataset.cThink).toBe('square');
    expect(seven.hasAttribute('data-c-model')).toBe(true);
  });

  it('restores in reverse when the bar widens again', () => {
    const { bar } = mount(260);
    expect(bar.dataset.step).toBe('7');
    bar.dataset.cw = '390';
    fireEvent(window, new Event('resize'));
    expect(bar.dataset.step).toBe('4');
    bar.dataset.cw = '2000';
    fireEvent(window, new Event('resize'));
    expect(bar.dataset.step).toBe('0');
    expect(bar.hasAttribute('data-c-permission')).toBe(false);
  });

  it('a control the bar does not render contributes no step', () => {
    const view = render(() => (
      <Toolbar lead={<span data-w="28">+</span>} end={<span data-w="40">send</span>}>
        <ToolbarSlot slot="permission">
          <div data-w="84">permission</div>
        </ToolbarSlot>
      </Toolbar>
    ));
    const bar = view.container.querySelector('.comp-bar') as HTMLElement;
    bar.dataset.cw = '100';
    fireEvent(window, new Event('resize'));
    // only permission folds (84 → 28): 2 items + lead + end + gaps = 28+28+40+24 = 120 > 100 → last effective step 2
    expect(bar.dataset.step).toBe('2');
    expect(bar.dataset.cThink).toBeUndefined();
  });
});
