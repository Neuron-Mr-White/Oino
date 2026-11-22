// @vitest-environment jsdom
/**
 * r8 T6: the first paint of the model list renders one screen of rows; the rest follows once that
 * frame has painted (a search shows every match at once).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import type { ModelInfo } from '../../lib/model/picker';
import { FIRST_ROWS, ModelList, type ModelView } from './ModelList';

afterEach(cleanup);

const models: ModelInfo[] = Array.from({ length: 200 }, (_, i) => ({ provider: 'p', id: `m${i}`, name: `Model ${i}` }));
const view = (rest: ModelInfo[]): ModelView => ({ usable: rest, unavailable: [], suggested: [], rest });
const rows = (root: HTMLElement) => root.querySelectorAll('[data-testid=model-picker-list] .mp-row').length;

describe('ModelList renders progressively (r8 T6)', () => {
  it('first one screen of rows, then the whole catalogue', async () => {
    const { container } = render(() => <ModelList view={view(models)} query="" onSelect={() => undefined} />);
    expect(rows(container)).toBe(FIRST_ROWS);
    await waitFor(() => expect(rows(container)).toBe(200), { timeout: 2000 });
  });

  it('the first scroll or key renders the rest at once, before the key reaches the list', () => {
    const { container } = render(() => <ModelList view={view(models)} query="" onSelect={() => undefined} />);
    expect(rows(container)).toBe(FIRST_ROWS);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(rows(container)).toBe(200);
  });

  it('a list closed before anything happened never renders the rest', async () => {
    const { container, unmount } = render(() => <ModelList view={view(models)} query="" onSelect={() => undefined} />);
    expect(rows(container)).toBe(FIRST_ROWS);
    unmount();
    await new Promise((r) => setTimeout(r, 500));
    expect(rows(container)).toBe(0);
  });

  it('a search shows every match at once', () => {
    const [query, setQuery] = createSignal('');
    const { container } = render(() => <ModelList view={view(models)} query={query()} onSelect={() => undefined} />);
    expect(rows(container)).toBe(FIRST_ROWS);
    setQuery('Model');
    expect(rows(container)).toBe(200);
  });

  it('a short list is whole from the first paint', () => {
    const { container } = render(() => <ModelList view={view(models.slice(0, 10))} query="" onSelect={() => undefined} />);
    expect(rows(container)).toBe(10);
  });
});
