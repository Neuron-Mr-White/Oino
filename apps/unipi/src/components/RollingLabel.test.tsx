// @vitest-environment jsdom
/**
 * Rolling label (WP-A14 acceptance): a changed model/thinking/mode name rolls
 * — the old text leaves while the new one arrives — and with
 * `prefers-reduced-motion: reduce` it swaps instantly with no leaving layer.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { prefersReducedMotion, ROLL_MS, RollingLabel } from './RollingLabel';

const tick = () => new Promise((r) => setTimeout(r, 0));

function stubMotion(reduce: boolean): void {
  vi.stubGlobal(
    'matchMedia',
    (query: string) =>
      ({
        matches: reduce && query.includes('prefers-reduced-motion'),
        media: query,
        onchange: null,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
        addListener: () => undefined,
        removeListener: () => undefined,
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList,
  );
}

afterEach(() => {
  cleanup();
  // restore only matchMedia: unstubAllGlobals would drop test-setup's ResizeObserver
  vi.stubGlobal('matchMedia', undefined);
});

describe('prefersReducedMotion', () => {
  it('is false where matchMedia does not exist (jsdom default)', () => {
    expect(prefersReducedMotion()).toBe(false);
  });
  it('follows the media query', () => {
    stubMotion(true);
    expect(prefersReducedMotion()).toBe(true);
    stubMotion(false);
    expect(prefersReducedMotion()).toBe(false);
  });
});

describe('RollingLabel', () => {
  it('shows the new text and the leaving one while the roll runs', async () => {
    stubMotion(false);
    const [text, setText] = createSignal('Claude Sonnet 4.5');
    render(() => <RollingLabel text={text()} testId="roll" />);
    expect(screen.getByTestId('roll').textContent).toBe('Claude Sonnet 4.5');
    setText('GPT-5');
    await tick();
    const root = screen.getByTestId('roll');
    expect(root.querySelector('.roll-in')?.textContent).toBe('GPT-5');
    expect(root.querySelector('.roll-out')?.textContent).toBe('Claude Sonnet 4.5');
    expect(root.querySelector('.roll-out')?.getAttribute('aria-hidden')).toBe('true');
    await waitFor(() => expect(root.querySelector('.roll-out')).toBeNull(), { timeout: ROLL_MS + 400 });
    expect(root.textContent).toBe('GPT-5');
  });

  it('is marked reduced and keeps no leaving layer with reduced motion', async () => {
    stubMotion(true);
    const [text, setText] = createSignal('Low');
    render(() => <RollingLabel text={text()} testId="roll" />);
    expect(screen.getByTestId('roll').getAttribute('data-reduced')).toBe('');
    setText('High');
    await tick();
    const root = screen.getByTestId('roll');
    expect(root.textContent).toBe('High');
    expect(root.querySelector('.roll-out')).toBeNull();
  });
});
