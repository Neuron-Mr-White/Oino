// @vitest-environment jsdom
import { cleanup, fireEvent, screen, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const copyText = vi.fn(async (_text: string) => true);
vi.mock('../../lib/board/share', () => ({ copyText: (t: string) => copyText(t), saveFile: async () => 'unsupported' }));

import { MermaidViewer, openMermaidViewer } from './MermaidViewer';

type Bridge = { saveDownload: (n: string, m: string, b: string) => string };
const w = window as unknown as { UniPiFiles?: Bridge };

afterEach(() => {
  cleanup();
  delete w.UniPiFiles;
  vi.clearAllMocks();
});

function open() {
  render(() => <MermaidViewer />);
  openMermaidViewer({ svg: '<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>', source: 'graph TD; A-->B' });
}

describe('MermaidViewer (UNI-182)', () => {
  it('the zoomable stage sits inside a clipping body between header and footer', () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage')!;
    expect(stage.parentElement!.classList.contains('mmd-viewer-body')).toBe(true);
    const order = [...document.querySelector('.mmd-viewer')!.children].map((el) => el.tagName.toLowerCase() + '.' + el.classList[0]);
    expect(order).toEqual(['header.mmd-viewer-head', 'div.mmd-viewer-body', 'footer.mmd-viewer-foot']);
  });

  it('Save as SVG goes through the native bridge and says so', async () => {
    const seen: string[] = [];
    w.UniPiFiles = { saveDownload: (n, m, b) => (seen.push(`${n}|${m}|${atob(b)}`), '{"ok":true,"uri":"content://d/1"}') };
    open();
    fireEvent.click(screen.getByLabelText('Save as SVG'));
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('Saved diagram.svg'));
    expect(seen[0]).toMatch(/^diagram\.svg\|image\/svg\+xml\|<svg/);
  });

  it('without the bridge or Web Share it reports instead of failing silently', async () => {
    open();
    fireEvent.click(screen.getByLabelText('Save as SVG'));
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Saving needs the latest app build'));
  });

  it('Copy source confirms', async () => {
    open();
    fireEvent.click(screen.getByLabelText('Copy source'));
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Diagram source copied'));
    expect(copyText).toHaveBeenCalledWith('graph TD; A-->B');
  });

  it("UNI-306: double-click zooms once — the manual double-tap timer and the browser's own dblclick used to both fire off the same two clicks and cancel out", async () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage') as HTMLElement;
    // Two clicks under 300ms = one double-tap, toggling zoom on.
    fireEvent.click(stage);
    fireEvent.click(stage);
    expect(stage.style.transform).toContain('scale(2.5)');
    // Past the double-tap window: a lone click here must not toggle again
    // (a real dblclick firing a *separate* handler on the same two clicks
    // used to undo the toggle right back to scale(1) before this check).
    await new Promise((r) => setTimeout(r, 400));
    fireEvent.click(stage);
    expect(stage.style.transform).toContain('scale(2.5)');
  });

  it('UNI-306: wheel zooms the stage in and out, clamped to 1–5', () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage') as HTMLElement;
    fireEvent.wheel(stage, { deltaY: -100 });
    expect(stage.style.transform).toContain('scale(1.35)');
    fireEvent.wheel(stage, { deltaY: 100 });
    expect(stage.style.transform).toContain('scale(1)');
    // Clamped at the bottom: wheeling further in while already at 1 doesn't go below it.
    fireEvent.wheel(stage, { deltaY: 100 });
    expect(stage.style.transform).toContain('scale(1)');
  });

  it('UNI-306: the +/− buttons zoom like wheel/pinch do', () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage') as HTMLElement;
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(stage.style.transform).toContain('scale(1.5)');
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(stage.style.transform).toContain('scale(2)');
    fireEvent.click(screen.getByLabelText('Zoom out'));
    expect(stage.style.transform).toContain('scale(1.5)');
  });

  it('UNI-306: Fit resets zoom and pan after zooming via any path', () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage') as HTMLElement;
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(stage.style.transform).toContain('scale(1.5)');
    fireEvent.click(screen.getByLabelText('Fit'));
    expect(stage.style.transform).toBe('translate(0px, 0px) scale(1)');
  });

  it('UNI-306: stays clickable when opened from inside a corvu drawer (e.g. the progress chart) — ' +
    'corvu sets `body { pointer-events: none }` while its drawer is open, and since this viewer is a ' +
    'Portal appended as a sibling of that drawer, it would otherwise inherit `none` and swallow every tap', () => {
    const css = readFileSync(resolve(__dirname, '../../style.css'), 'utf8');
    const rule = /\.mmd-viewer\s*\{[^}]*\}/.exec(css)?.[0] ?? '';
    expect(rule).toMatch(/pointer-events:\s*auto/);
  });

  it('progress chart: a mouse drag at fit size zooms in and moves the chart (it used to do nothing), and does not count as a click', () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage') as HTMLElement;
    stage.getBoundingClientRect = () => ({ width: 400, height: 300, top: 0, left: 0, right: 400, bottom: 300, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    const ptr = (type: string, x: number, y: number) =>
      stage.dispatchEvent(Object.assign(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button: 0 }), { pointerId: 1, pointerType: 'mouse' }));
    ptr('pointerdown', 100, 100);
    ptr('pointermove', 160, 130);
    ptr('pointerup', 160, 130);
    fireEvent.click(stage);
    expect(stage.style.transform).toBe('translate(60px, 30px) scale(2)');
  });

  it('progress chart: a one-finger drag at fit size zooms in and pans too', () => {
    open();
    const stage = document.querySelector('.mmd-viewer-stage') as HTMLElement;
    stage.getBoundingClientRect = () => ({ width: 400, height: 300, top: 0, left: 0, right: 400, bottom: 300, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    fireEvent.touchStart(stage, { touches: [{ clientX: 100, clientY: 100 }] });
    fireEvent.touchMove(stage, { touches: [{ clientX: 60, clientY: 80 }] });
    fireEvent.touchEnd(stage, { touches: [] });
    expect(stage.style.transform).toBe('translate(-40px, -20px) scale(2)');
  });
});
