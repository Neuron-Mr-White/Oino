// @vitest-environment jsdom
/**
 * Thinking row (WP-A8, Z-41): "Thinking…" shimmer while live →
 * "Thought for 3s" settled (duration from the entry pair), unipi's
 * one-line preview, hidden when empty, full text only while open.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import type { SessionEntry } from '../../../lib/session/types';
import { resetSessionRenderCaches } from '../../../lib/session/renderCache';
import { ThinkingRow } from './ThinkingRow';
import { fakeCtx } from './testutil';

afterEach(() => {
  cleanup();
  resetSessionRenderCaches();
});

const T0 = Date.UTC(2026, 9, 30, 9, 0, 0);
const entries = (): SessionEntry[] => [
  { type: 'message', id: 'u1', timestamp: new Date(T0).toISOString(), message: { role: 'user', content: 'go', timestamp: T0 } } as SessionEntry,
  { type: 'message', id: 'a1', timestamp: new Date(T0 + 1000).toISOString(), message: { role: 'assistant', content: [{ type: 'thinking', thinking: 'First look at the test.' }], timestamp: T0 + 1000 } } as SessionEntry,
  { type: 'message', id: 'r1', timestamp: new Date(T0 + 4000).toISOString(), message: { role: 'toolResult', toolCallId: 't', content: [], timestamp: T0 + 4000 } } as SessionEntry,
];

describe('ThinkingRow', () => {
  it('live: "Thinking…" with the shimmer and a preview line', () => {
    const r = render(() => <ThinkingRow item={{ key: 'stream-0', text: 'Working through the failing case.', live: true }} ctx={fakeCtx(entries())} />);
    expect(r.container.querySelector('.tool-name')!.textContent).toBe('Thinking…');
    expect(r.container.querySelector('.tool-name')!.classList.contains('shimmer')).toBe(true);
    expect(r.container.querySelector('.tool-subject')!.textContent).toContain('failing case');
  });

  it('settled: "Thought for 3s" (entry → next entry) plus the preview', () => {
    const r = render(() => <ThinkingRow item={{ key: 'a1-0', text: 'First look at the test.', live: false }} ctx={fakeCtx(entries())} />);
    expect(r.container.querySelector('.tool-name')!.textContent).toBe('Thought for 3s');
    expect(r.container.querySelector('.tool-name')!.classList.contains('shimmer')).toBe(false);
    expect(r.container.querySelector('.tool-subject')!.textContent).toContain('First look at the test');
  });

  it('without a duration source the label is plain "Thought"', () => {
    const r = render(() => <ThinkingRow item={{ key: 'zz-0', text: 'Something.', live: false }} ctx={fakeCtx(entries())} />);
    expect(r.container.querySelector('.tool-name')!.textContent).toBe('Thought');
  });

  it('hidden when settled and empty', () => {
    const r = render(() => <ThinkingRow item={{ key: 'a1-0', text: '  ', live: false }} ctx={fakeCtx(entries())} />);
    expect(r.container.textContent).toBe('');
  });

  it('the full text mounts only while open', async () => {
    const r = render(() => <ThinkingRow item={{ key: 'a1-0', text: 'First look at the test.\n\nThen the reducer.', live: false }} ctx={fakeCtx(entries())} />);
    expect(r.container.querySelector('.think-body')).toBeNull();
    fireEvent.click(r.getByTestId('tl-think').querySelector('button')!);
    await waitFor(() => expect(r.container.querySelector('.think-body')).toBeTruthy());
    expect(r.container.querySelector('.think-body')!.textContent).toContain('Then the reducer.');
  });
});
