// @vitest-environment jsdom
/**
 * The image editor's ellipse tool (WP-A13 acceptance, D-5): the tool exists in
 * the picker, a drag draws one, Shift makes it a circle, grabbing an existing
 * ellipse's outline drags it instead of drawing, undo removes it, and Done
 * flattens the ellipse into the output canvas (the PNG that gets attached).
 *
 * jsdom has no canvas or image decoding, so both are stubbed with recorders.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ImageEditor } from './ImageEditor';
import { fitBox } from '../../lib/composer/imageEdit';

/** The 800×600 image fitted into the 800×600 stage with 16 px padding. */
const BOX = fitBox({ x: 0, y: 0, w: 800, h: 600 }, 800, 600, 16);
/** Image pixels → client coordinates on the canvas (the editor maps back). */
const client = (x: number, y: number): [number, number] => [BOX.left + x * BOX.scale, BOX.top + y * BOX.scale];

type Call = { fn: string; args: number[] };

function fakeCtx(): { ctx: CanvasRenderingContext2D; calls: Call[] } {
  const calls: Call[] = [];
  const record = (fn: string) => (...args: unknown[]) => {
    calls.push({ fn, args: args.filter((a): a is number => typeof a === 'number') });
  };
  const ctx = {
    setTransform: record('setTransform'),
    clearRect: record('clearRect'),
    drawImage: record('drawImage'),
    save: record('save'),
    restore: record('restore'),
    translate: record('translate'),
    beginPath: record('beginPath'),
    closePath: record('closePath'),
    rect: record('rect'),
    clip: record('clip'),
    moveTo: record('moveTo'),
    lineTo: record('lineTo'),
    ellipse: record('ellipse'),
    stroke: record('stroke'),
    fill: record('fill'),
    fillRect: record('fillRect'),
    strokeRect: record('strokeRect'),
    strokeStyle: '',
    fillStyle: '',
    lineWidth: 0,
    lineCap: 'butt',
    lineJoin: 'miter',
  } as unknown as CanvasRenderingContext2D;
  return { ctx, calls };
}

const ellipses = (calls: Call[]) => calls.filter((c) => c.fn === 'ellipse');

beforeEach(() => {
  class FakeImage {
    naturalWidth = 800;
    naturalHeight = 600;
    onload?: () => void;
    set src(_value: string) {
      queueMicrotask(() => this.onload?.());
    }
    get src() {
      return 'blob:fake';
    }
  }
  vi.stubGlobal('Image', FakeImage);
  // 800×600 stage, so the fitted image box is real.
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 800 });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => 600 });
  Object.defineProperty(Element.prototype, 'setPointerCapture', { configurable: true, value: () => undefined });
  Object.defineProperty(HTMLElement.prototype, 'getBoundingClientRect', {
    configurable: true,
    value() {
      return { left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    },
  });
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(private cb: () => void) {}
      observe() {
        this.cb();
      }
      unobserve() {}
      disconnect() {}
    },
  );
  const { ctx, calls } = fakeCtx();
  (HTMLCanvasElement.prototype as unknown as { getContext: () => CanvasRenderingContext2D }).getContext = () => ctx;
  (HTMLCanvasElement.prototype as unknown as { toBlob: (cb: (b: Blob | null) => void, type?: string) => void }).toBlob = (cb) => cb(new Blob([new Uint8Array(4)], { type: 'image/png' }));
  (globalThis as unknown as { __calls: Call[] }).__calls = calls;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  delete (Element.prototype as unknown as Record<string, unknown>).setPointerCapture;
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).getBoundingClientRect;
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientWidth;
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
});

const calls = () => (globalThis as unknown as { __calls: Call[] }).__calls;

/** A pointer drag on the editor canvas (jsdom has no PointerEvent). */
async function drag(canvas: HTMLCanvasElement, from: [number, number], to: [number, number], shiftKey = false) {
  // The stubbed Image decodes on a microtask; let the editor see it first.
  await new Promise((r) => setTimeout(r, 0));
  const fire = (type: string, [x, y]: [number, number], shift = false) => {
    const event = new MouseEvent(type, { clientX: x, clientY: y, bubbles: true, cancelable: true, shiftKey: shift, button: 0 });
    Object.defineProperty(event, 'pointerId', { value: 1 });
    Object.defineProperty(event, 'pointerType', { value: 'mouse' });
    canvas.dispatchEvent(event);
  };
  fire('pointerdown', from, shiftKey);
  fire('pointermove', [Math.round((from[0] + to[0]) / 2), Math.round((from[1] + to[1]) / 2)], shiftKey);
  fire('pointermove', to, shiftKey);
  fire('pointerup', to, shiftKey);
}

/** The editor portals over the whole window, so queries go to the document. */
function mount(onDone = vi.fn<(blob: Blob) => void>(), onCancel = vi.fn<() => void>()) {
  render(() => <ImageEditor src="blob:fake" mime="image/png" name="shot.png" onCancel={onCancel} onDone={onDone} />);
  const canvas = document.querySelector('.img-edit-canvas') as HTMLCanvasElement;
  const testId = (id: string) => document.querySelector(`[data-testid="${id}"]`) as HTMLElement;
  const button = (name: string | RegExp) => [...document.querySelectorAll('.img-edit button')].find((b) => (typeof name === 'string' ? b.getAttribute('aria-label') === name : name.test(b.textContent ?? ''))) as HTMLElement;
  return { canvas, testId, button, onDone, onCancel };
}

describe('ellipse tool (D-5)', () => {
  it('the picker offers Draw, Arrow, Ellipse and Crop', async () => {
    const { testId } = mount();
    await waitFor(() => expect(testId('tool-ellipse')).not.toBeNull());
    const tools = [...document.querySelectorAll('.img-edit-tool')].map((b) => b.textContent?.trim());
    expect(tools).toEqual(['Draw', 'Arrow', 'Ellipse', 'Crop']);
  });

  it('a drag draws an ellipse in image pixels', async () => {
    const { canvas, testId } = mount();
    fireEvent.click(testId('tool-ellipse'));
    calls().length = 0;
    await drag(canvas, client(100, 100), client(400, 300));
    await waitFor(() => expect(ellipses(calls()).length).toBeGreaterThan(0));
    const [cx, cy, rx, ry] = ellipses(calls()).at(-1)!.args;
    // The preview paints in the fitted box's scale, around the drag's middle.
    expect(cx).toBeCloseTo(250 * BOX.scale, 0);
    expect(cy).toBeCloseTo(200 * BOX.scale, 0);
    expect(rx).toBeCloseTo(150 * BOX.scale, 0);
    expect(ry).toBeCloseTo(100 * BOX.scale, 0);
  });

  it('Shift constrains it to a circle', async () => {
    const { canvas, testId } = mount();
    fireEvent.click(testId('tool-ellipse'));
    calls().length = 0;
    await drag(canvas, client(100, 100), client(400, 300), true);
    await waitFor(() => expect(ellipses(calls()).length).toBeGreaterThan(0));
    const [, , rx, ry] = ellipses(calls()).at(-1)!.args;
    expect(rx).toBeCloseTo(ry!, 5);
  });

  it('Done flattens the ellipse into the output canvas', async () => {
    const { canvas, testId, button, onDone } = mount();
    fireEvent.click(testId('tool-ellipse'));
    await drag(canvas, client(100, 100), client(400, 300));
    await waitFor(() => expect(ellipses(calls()).length).toBeGreaterThan(0));
    const painted = ellipses(calls()).length;
    fireEvent.click(button(/Done/));
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(onDone.mock.calls[0]![0]).toBeInstanceOf(Blob);
    // The flatten pass draws the same ellipse at scale 1 on the output canvas:
    // the drag's own image pixels (no crop, so the canvas is the whole image).
    const flat = ellipses(calls()).slice(painted).at(-1)!;
    expect(flat.args).toHaveLength(7);
    expect(flat.args[0]).toBeCloseTo(250, 0);
    expect(flat.args[1]).toBeCloseTo(200, 0);
    expect(flat.args[2]).toBeCloseTo(150, 0);
    expect(flat.args[3]).toBeCloseTo(100, 0);
  });

  it('undo removes it; with nothing left Done just cancels', async () => {
    const { canvas, testId, button, onDone, onCancel } = mount();
    fireEvent.click(testId('tool-ellipse'));
    await drag(canvas, client(100, 100), client(400, 300));
    await waitFor(() => expect(ellipses(calls()).length).toBeGreaterThan(0));
    fireEvent.click(button('Undo'));
    fireEvent.click(button(/Done/));
    await waitFor(() => expect(onCancel).toHaveBeenCalled());
    expect(onDone).not.toHaveBeenCalled();
  });

  it('dragging an existing ellipse moves it instead of drawing another', async () => {
    const { canvas, testId } = mount();
    fireEvent.click(testId('tool-ellipse'));
    await drag(canvas, client(100, 100), client(400, 300));
    await waitFor(() => expect(ellipses(calls()).length).toBeGreaterThan(0));
    calls().length = 0;
    // Grab the outline at its left vertex (image px 100,200) and drag right.
    await drag(canvas, client(100, 200), client(160, 200));
    await waitFor(() => expect(ellipses(calls()).length).toBeGreaterThan(0));
    const moved = ellipses(calls()).at(-1)!.args;
    expect(moved[0]!).toBeCloseTo(310 * BOX.scale, 0); // 250 + 60
    expect(moved[2]!).toBeCloseTo(150 * BOX.scale, 0); // same shape
    expect(moved[3]!).toBeCloseTo(100 * BOX.scale, 0);
  });
});
