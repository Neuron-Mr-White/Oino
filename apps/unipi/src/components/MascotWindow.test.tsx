// @vitest-environment jsdom
/**
 * UNI-255: the desktop mascot window shows only the crab (no tile/button
 * chrome), at the size the feed carries; drag still moves the window and a
 * click still opens UniPi.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MascotWindow } from '../mascotWindow';
import type { MascotFeed } from '../lib/mascot';

afterEach(() => cleanup());
const feed = (f: Partial<MascotFeed>) => (window as unknown as { __unipiMascotFeed: (f: MascotFeed) => void }).__unipiMascotFeed({ state: 'idle', count: 0, bubble: null, ...f });

describe('MascotWindow (UNI-255)', () => {
  it('crab only, default 96 px, resized by the feed; badge kept', () => {
    const { container } = render(() => <MascotWindow emit={() => {}} startDrag={() => {}} />);
    const button = container.querySelector('.mascot-button')!;
    expect(button.classList.contains('mascot-crab-only')).toBe(true);
    expect(container.querySelector('svg.unicrab')!.getAttribute('width')).toBe('96');
    feed({ size: 128, count: 3 });
    expect(container.querySelector('svg.unicrab')!.getAttribute('width')).toBe('128');
    expect(container.querySelector('.mascot-badge')!.textContent).toBe('3');
    feed({ size: 5 }); // out of range: ignored
    expect(container.querySelector('svg.unicrab')!.getAttribute('width')).toBe('128');
  });
  it('drag moves the window, a click opens Unipi, the bubble opens its chat', () => {
    const emit = vi.fn();
    const startDrag = vi.fn();
    const { container, getByTestId } = render(() => <MascotWindow emit={emit} startDrag={startDrag} />);
    const button = container.querySelector('.mascot-button')!;
    fireEvent.mouseDown(button, { clientX: 10, clientY: 10 });
    fireEvent.mouseMove(button, { clientX: 30, clientY: 10 });
    expect(startDrag).toHaveBeenCalledTimes(1);
    fireEvent.mouseUp(button);
    expect(emit).not.toHaveBeenCalled();
    fireEvent.mouseDown(button, { clientX: 10, clientY: 10 });
    fireEvent.mouseUp(button);
    expect(emit).toHaveBeenCalledWith('mascot-open', {});
    feed({ bubble: { id: 'b1', text: 'auth: done', link: 'unipi://chat?host=h&pid=3' } });
    fireEvent.click(getByTestId('mascot-bubble'));
    expect(emit).toHaveBeenCalledWith('mascot-open', { link: 'unipi://chat?host=h&pid=3' });
  });
});
