// @vitest-environment jsdom
/** Long-press on a file ref opens our menu and drops the WebView's own word selection (seen on the Redmi). */
import { afterEach, describe, expect, it, vi } from 'vitest';

const openMenu = vi.fn();
vi.mock('./fileRefOpen', () => ({ openFileRef: vi.fn(async () => {}), openFileRefMenu: (...a: unknown[]) => openMenu(...a) }));

import { attachFileRefGestures } from './fileRefGestures';

afterEach(() => {
  vi.useRealTimers();
  openMenu.mockReset();
  document.body.textContent = '';
});

function touch(type: string, el: Element, x = 5, y = 5) {
  const ev = new Event(type, { bubbles: true }) as Event & { touches: unknown[] };
  Object.defineProperty(ev, 'touches', { value: type === 'touchend' ? [] : [{ clientX: x, clientY: y }] });
  el.dispatchEvent(ev);
}

describe('attachFileRefGestures', () => {
  it('long-press opens the menu and clears the native selection the WebView started', () => {
    vi.useFakeTimers();
    const root = document.createElement('div');
    root.innerHTML = 'see <span class="file-ref" data-file-ref="/tmp/a/README.md" data-file-kind="file">README.md</span> now';
    document.body.append(root);
    const off = attachFileRefGestures(root, () => ({}) as never);
    const ref = root.querySelector('.file-ref')!;
    touch('touchstart', ref);
    vi.advanceTimersByTime(500);
    expect(openMenu).toHaveBeenCalledTimes(1);
    expect(openMenu.mock.calls[0]![1]).toMatchObject({ resolved: '/tmp/a/README.md', kind: 'file' });
    // The WebView selects the word after our timer fired.
    const range = document.createRange();
    range.selectNodeContents(ref);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(range);
    vi.advanceTimersByTime(450);
    expect(document.getSelection()!.isCollapsed).toBe(true);
    off();
  });

  it('a short tap does not open the menu', () => {
    vi.useFakeTimers();
    const root = document.createElement('div');
    root.innerHTML = '<span class="file-ref" data-file-ref="/x/a.ts" data-file-kind="file">a.ts</span>';
    document.body.append(root);
    attachFileRefGestures(root, () => ({}) as never);
    touch('touchstart', root.firstElementChild!);
    vi.advanceTimersByTime(200);
    touch('touchend', root.firstElementChild!);
    vi.advanceTimersByTime(600);
    expect(openMenu).not.toHaveBeenCalled();
  });
});
