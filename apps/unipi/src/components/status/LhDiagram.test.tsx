// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { clampView, openView, LhDiagram, roundedPath, zoomAt } from './LhDiagram';
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

describe('LhDiagram open + pan bounds (owner-fix-6)', () => {
  it('opens fitting the whole graph when that keeps zoom >= 0.6', () => {
    const v = openView(600, 540, 700, 440);
    expect(v.z).toBeGreaterThanOrEqual(0.6);
    expect(540 * v.z + v.y).toBeLessThanOrEqual(440);
  });
  it('else starts at 0.6 top-left, and the pan bounds reach the last row', () => {
    const v = openView(1200, 1500, 500, 400);
    expect(v.z).toBe(0.6);
    expect(v.y).toBe(12);
    const down = clampView({ ...v, y: -99999 }, 1200, 1500, 500, 400);
    expect(down.y + 1500 * v.z).toBe(400 - 12);
    expect(clampView({ ...v, y: 500 }, 1200, 1500, 500, 400).y).toBe(12);
  });
});
