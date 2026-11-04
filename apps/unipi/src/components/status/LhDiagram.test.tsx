// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { LhDiagram, roundedPath, zoomAt } from './LhDiagram';
import { asStatus, type LhWork } from '../../lib/status/lhGraph';

const w = (id: string, status: string, extra: Partial<LhWork> = {}): LhWork => ({
  workId: id, status: asStatus(status), instruction: '', inputIds: [], attempts: 0, autoContinues: 0, continues: 0, toolCalls: 0, canRetry: false, hasOutput: false, events: [], eventsDropped: 0, ...extra,
});
const work = [w('a', 'completed'), w('b', 'running', { inputIds: ['r-a'] }), w('c', 'failed', { inputIds: ['r-a'], supersededBy: 'd' }), w('d', 'queued', { inputIds: ['r-b'] })];
afterEach(cleanup);

describe('LhDiagram', () => {
  it('one transformed layer holds edges and cards; zoom buttons change its scale', async () => {
    render(() => <LhDiagram work={work} now={0} onSelect={vi.fn()} />);
    await waitFor(() => expect(document.querySelectorAll('.lh-dia-edge').length).toBe(4));
    const layer = screen.getByTestId('lh-dia-layer');
    expect(layer.querySelectorAll('[data-testid="lh-dia-node"]')).toHaveLength(4);
    expect(layer.querySelector('svg')).toBeTruthy();
    const before = layer.style.transform;
    fireEvent.click(screen.getByTestId('lh-dia-zoom-in'));
    expect(layer.style.transform).not.toBe(before);
    expect(layer.style.transform).toMatch(/scale\(/);
  });
  it('hiding a node drops its cards and edges at once', async () => {
    const { unmount } = render(() => <LhDiagram work={work.filter((x) => x.workId !== 'c')} now={0} onSelect={vi.fn()} />);
    await waitFor(() => expect(document.querySelectorAll('.lh-dia-edge[data-kind="replaces"]').length).toBe(0));
    expect(document.querySelectorAll('[data-testid="lh-dia-node"]')).toHaveLength(3);
    unmount();
  });
  it('Full window opens an overlay with the same diagram; Esc and the close button leave', async () => {
    render(() => <LhDiagram work={work} now={0} onSelect={vi.fn()} />);
    fireEvent.click(screen.getByTestId('lh-dia-full-open'));
    expect(screen.getByTestId('lh-dia-full')).toBeTruthy();
    expect(screen.getByTestId('lh-dia-full').querySelectorAll('[data-testid="lh-dia-node"]')).toHaveLength(4);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByTestId('lh-dia-full')).toBeNull();
    fireEvent.click(screen.getByTestId('lh-dia-full-open'));
    fireEvent.click(screen.getByTestId('lh-dia-full-close'));
    expect(screen.queryByTestId('lh-dia-full')).toBeNull();
  });
  it('pure helpers', () => {
    expect(zoomAt({ x: 0, y: 0, z: 1 }, 2, 10, 10)).toEqual({ z: 2, x: -10, y: -10 });
    expect(roundedPath([{ x: 0, y: 0 }, { x: 0, y: 20 }, { x: 20, y: 20 }, { x: 20, y: 40 }])).toMatch(/^M0,0 L0,12 Q0,20 8,20/);
  });
});
