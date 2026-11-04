import { describe, expect, it } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { useVirtual } from './virtual';

describe('useVirtual with a late-mounted scroller', () => {
  it('attaches the scroller once it is connected (observers on, rect + offset read)', async () => {
    const el = document.createElement('div'); // detached when the signal is set, as a Solid ref inside <Show>
    const [scroller, setScroller] = createSignal<HTMLElement>();
    const virt = createRoot(() => useVirtual({ count: () => 1000, scroller, estimate: () => 24, key: (i) => i, fixed: true }));
    setScroller(el);
    document.body.appendChild(el);
    await Promise.resolve();
    await Promise.resolve();
    expect(virt.scrollElement).toBe(el);
    expect(virt.targetWindow).toBe(window);
    el.remove();
  });
});
