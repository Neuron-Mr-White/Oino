// @vitest-environment jsdom
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { MiddleText, splitMiddle } from './MiddleText';

afterEach(cleanup);

describe('MiddleText', () => {
  it('a file name keeps its extension and a few characters before it in the tail', () => {
    const [head, tail] = splitMiddle('composer-mobile-390-pdf-fallback-chip.png', { file: true });
    expect(tail).toBe('chip.png');
    expect(head + tail).toBe('composer-mobile-390-pdf-fallback-chip.png');
  });

  it('a name without an extension (a folder) keeps its last 12 characters; an explicit tail wins', () => {
    expect(splitMiddle('a-project-with-an-extremely-long-name')[1]).toBe('emely-long-name'.slice(-12));
    expect(splitMiddle('~/Projects/a-very-long-folder-name', { tail: 28 })[1]).toBe('~/Projects/a-very-long-folder-name'.slice(-28));
  });

  it('a short text is not cut at all', () => {
    expect(splitMiddle('shot.png', { file: true })).toEqual(['shot.png', '']);
    expect(splitMiddle('notes')).toEqual(['notes', '']);
  });

  it('renders the whole text, as two spans', () => {
    const { container } = render(() => <MiddleText text="mobile-390-pdf-card.png" file />);
    expect(container.textContent).toBe('mobile-390-pdf-card.png');
    expect(container.querySelector('.mid-head')!.textContent).toBe('mobile-390-pdf-');
    expect(container.querySelector('.mid-tail')!.textContent).toBe('card.png');
  });
});
