// @vitest-environment jsdom
/**
 * AnchoredMenu placement: measured by layout size (offsetHeight), not the
 * transformed box — the popover-in animation scales it to 0.94 on mount,
 * which made a right-click near the bottom edge open a menu that ran past
 * the window (desktop QA).
 */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AnchoredMenu } from './AnchoredMenu';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function mount(anchor: DOMRect, menuHeight: number, scaledHeight: number) {
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(menuHeight);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 240, scaledHeight));
  Object.defineProperty(window, 'innerHeight', { configurable: true, value: 900 });
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
  render(() => (
    <AnchoredMenu anchor={anchor} onDismiss={() => {}} label="Task menu">
      <button type="button">Open</button>
    </AnchoredMenu>
  ));
  return screen.getByRole('menu');
}

describe('AnchoredMenu', () => {
  it('flips above a pointer near the bottom using the unscaled height', () => {
    // 474px menu, mid-animation it measures 0.94 × 474 = 446. Below the pointer at y=424 only 468px fit.
    const menu = mount(new DOMRect(1430, 424, 0, 0), 474, 446);
    const top = parseFloat(menu.style.top);
    expect(top + 474).toBeLessThanOrEqual(900 - 8);
  });

  it('opens below when it fits and clamps to the right edge', () => {
    const menu = mount(new DOMRect(1430, 100, 0, 0), 300, 282);
    expect(menu.style.top).toBe('108px');
    expect(parseFloat(menu.style.left) + 240).toBeLessThanOrEqual(1440 - 8);
  });
});
