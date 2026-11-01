import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { measureRow } from './Timeline';

/** A virtualizer whose `measureElement` measures nothing (it skips while `isScrolling`). */
function fakeVirt() {
  const sizes = new Map<number, number>();
  return {
    sizes,
    measureElement: vi.fn(() => {}),
    resizeItem: vi.fn((index: number, size: number) => void sizes.set(index, size)),
  };
}

function row(index: number, height: number): HTMLDivElement {
  const el = document.createElement('div');
  el.dataset.index = String(index);
  Object.defineProperty(el, 'offsetHeight', { configurable: true, get: () => height });
  document.body.append(el);
  return el;
}

describe('bubble overlap regression: a row re-keyed while the list scrolls keeps its real height', () => {
  it('writes the measured height under the current key even when measureElement skips (isScrolling)', () => {
    const virt = fakeVirt();
    const el = row(3, 334);
    measureRow(virt, el);
    expect(virt.measureElement).toHaveBeenCalledWith(el);
    expect(virt.sizes.get(3)).toBe(334);
    el.remove();
  });

  it('leaves the estimate for a row that is not laid out or not connected', () => {
    const virt = fakeVirt();
    const hidden = row(1, 0);
    measureRow(virt, hidden);
    expect(virt.resizeItem).not.toHaveBeenCalled();
    hidden.remove();
    const gone = row(2, 120);
    gone.remove();
    measureRow(virt, gone);
    expect(virt.measureElement).toHaveBeenCalledTimes(1);
    expect(virt.resizeItem).not.toHaveBeenCalled();
  });
});
