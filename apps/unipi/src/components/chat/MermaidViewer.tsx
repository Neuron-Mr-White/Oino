/**
 * Full-screen mermaid diagram viewer (UNI-156): tap a diagram in chat or
 * board markdown to open it full-screen — pinch zoom, drag pan, wheel zoom,
 * +/− buttons, double-tap/double-click to toggle fit/zoom, a Fit button,
 * Back (lib/back.ts pushBack) closes, copy source, save as SVG/PNG. The
 * gesture code mirrors components/board/Viewers.tsx's ImageViewer.
 *
 * UNI-306: double-tap and the browser's native dblclick both used to call
 * toggleZoom() off the same two clicks, cancelling each other out — one
 * path now (onTap's own timestamp check); dblclick is not handled
 * separately. Wheel and +/− zoom are new; there was no desktop zoom path
 * at all before.
 */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { ArrowLeft, Copy, Download, Maximize2, ZoomIn, ZoomOut } from 'lucide-solid';
import { pushBack } from '../../lib/back';
import { copyText } from '../../lib/board/share';
import { saveBytesToDevice, type TransferResult } from '../../lib/filesTransfer';

export type MermaidViewerRequest = { svg: string; source: string };

const [request, setRequest] = createSignal<MermaidViewerRequest | undefined>(undefined);
export function openMermaidViewer(req: MermaidViewerRequest): void {
  setRequest(req);
}

export function MermaidViewer() {
  return (
    <Show when={request()}>
      {(req) => <ViewerLayer request={req()} onClose={() => setRequest(undefined)} />}
    </Show>
  );
}

type Xform = { scale: number; x: number; y: number };
const MAX_SCALE = 5;
const clampN = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

function ViewerLayer(props: { request: MermaidViewerRequest; onClose: () => void }) {
  const [xf, setXf] = createSignal<Xform>({ scale: 1, x: 0, y: 0 });
  let stage: HTMLDivElement | undefined;

  onMount(() => onCleanup(pushBack(props.onClose)));

  const bounds = (scale: number) => {
    const rect = stage?.getBoundingClientRect();
    const w = rect?.width ?? window.innerWidth;
    const h = rect?.height ?? window.innerHeight;
    return { x: ((scale - 1) * w) / 2, y: ((scale - 1) * h) / 2 };
  };
  const clamp = (next: Xform): Xform => {
    const b = bounds(next.scale);
    return { scale: next.scale, x: clampN(next.x, -b.x, b.x), y: clampN(next.y, -b.y, b.y) };
  };

  type Gesture = { kind: 'pinch'; dist: number; start: Xform } | { kind: 'pan'; x0: number; y0: number; start: Xform } | { kind: 'none' };
  let gesture: Gesture = { kind: 'none' };
  let lastTap = 0;
  const dist = (touches: TouchList) => Math.hypot(touches[0]!.clientX - touches[1]!.clientX, touches[0]!.clientY - touches[1]!.clientY);

  const onTouchStart = (event: TouchEvent) => {
    if (event.touches.length === 2) gesture = { kind: 'pinch', dist: dist(event.touches), start: xf() };
    else if (event.touches.length === 1) gesture = { kind: 'pan', x0: event.touches[0]!.clientX, y0: event.touches[0]!.clientY, start: xf() };
  };
  const onTouchMove = (event: TouchEvent) => {
    if (gesture.kind === 'pinch' && event.touches.length === 2) {
      event.preventDefault();
      const scale = clampN((gesture.start.scale * dist(event.touches)) / gesture.dist, 1, MAX_SCALE);
      setXf(clamp({ ...gesture.start, scale }));
    } else if (gesture.kind === 'pan' && event.touches.length === 1) {
      const dx = event.touches[0]!.clientX - gesture.x0;
      const dy = event.touches[0]!.clientY - gesture.y0;
      if (gesture.start.scale <= 1.01) {
        // At fit scale a drag used to do nothing (clamped to 0). Zoom in on
        // the first real move so the chart can be grabbed and moved.
        if (Math.hypot(dx, dy) < 8) return;
        gesture = { ...gesture, start: { scale: 2, x: 0, y: 0 } };
      }
      event.preventDefault();
      setXf(clamp({ ...gesture.start, x: gesture.start.x + dx, y: gesture.start.y + dy }));
    }
  };
  const onTouchEnd = () => {
    gesture = { kind: 'none' };
  };
  // Mouse/pen drag (desktop): touch keeps its own pinch/pan above. A drag
  // that moved must not count as a tap (it would toggle zoom on release).
  let drag: { id: number; x0: number; y0: number; start: Xform; moved: boolean } | undefined;
  let suppressTap = false;
  const onPointerDown = (event: PointerEvent) => {
    if (event.pointerType === 'touch' || event.button !== 0) return;
    drag = { id: event.pointerId, x0: event.clientX, y0: event.clientY, start: xf(), moved: false };
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent) => {
    if (!drag || event.pointerId !== drag.id) return;
    const dx = event.clientX - drag.x0;
    const dy = event.clientY - drag.y0;
    if (!drag.moved && Math.hypot(dx, dy) < 4) return;
    drag.moved = true;
    // At fit scale there is nowhere to move: drag zooms in first, so a
    // big graph can be grabbed straight away instead of looking frozen.
    const start = drag.start.scale > 1.01 ? drag.start : { scale: 2, x: 0, y: 0 };
    if (drag.start.scale <= 1.01) drag.start = start;
    setXf(clamp({ ...start, x: start.x + dx, y: start.y + dy }));
  };
  const onPointerUp = (event: PointerEvent) => {
    if (!drag || event.pointerId !== drag.id) return;
    suppressTap = drag.moved;
    drag = undefined;
  };
  const toggleZoom = () => setXf(xf().scale > 1.01 ? { scale: 1, x: 0, y: 0 } : { scale: 2.5, x: 0, y: 0 });
  // UNI-306: one double-tap/double-click path, not two. A real double-click
  // used to fire both this timestamp check (two `click`s under 300ms) AND
  // the browser's native `dblclick` — each toggled zoom, so together they
  // cancelled out and the diagram never visibly zoomed on desktop.
  const onTap = () => {
    if (suppressTap) {
      suppressTap = false;
      return;
    }
    const now = Date.now();
    if (now - lastTap < 300) toggleZoom();
    lastTap = now;
  };
  // UNI-306: wheel zoom (desktop) — any wheel over the stage, not just
  // ctrl+wheel, since this view has nothing else to scroll.
  const onWheel = (event: WheelEvent) => {
    event.preventDefault();
    const step = event.deltaY > 0 ? -0.35 : 0.35;
    setXf((prev) => clamp({ ...prev, scale: clampN(prev.scale + step, 1, MAX_SCALE) }));
  };
  // UNI-306: +/− buttons for pointer devices with no wheel/pinch (or a user
  // who just prefers buttons).
  const zoomStep = (delta: number) => setXf((prev) => clamp({ ...prev, scale: clampN(prev.scale + delta, 1, MAX_SCALE) }));

  // Every action says what happened (UNI-182: it used to fail silently).
  const [toast, setToast] = createSignal<string | null>(null);
  const flash = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2200);
  };
  const report = (result: TransferResult, name: string) =>
    flash(
      result.outcome === 'saved'
        ? `Saved ${name} to Download/unipi`
        : result.outcome === 'unsupported'
          ? 'Saving needs the latest app build'
          : (result.detail ?? "Couldn't save"),
    );
  async function copySource() {
    flash((await copyText(props.request.source)) ? 'Diagram source copied' : "Couldn't copy");
  }
  async function saveAsSvg() {
    report(await saveBytesToDevice('diagram.svg', 'image/svg+xml', new TextEncoder().encode(props.request.svg)), 'diagram.svg');
  }
  async function saveAsPng() {
    const img = new Image();
    const svgBlob = new Blob([props.request.svg], { type: 'image/svg+xml' });
    const url = URL.createObjectURL(svgBlob);
    await new Promise<void>((resolve, reject) => {
      img.onload = () => resolve();
      img.onerror = () => reject(new Error('svg decode failed'));
      img.src = url;
    }).catch(() => undefined);
    const canvas = document.createElement('canvas');
    canvas.width = img.naturalWidth || 800;
    canvas.height = img.naturalHeight || 600;
    const ctx = canvas.getContext('2d');
    if (ctx) {
      ctx.fillStyle = '#141211';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.drawImage(img, 0, 0);
    }
    URL.revokeObjectURL(url);
    const blob: Blob | null = await new Promise((resolve) => canvas.toBlob((b) => resolve(b), 'image/png'));
    if (!blob) return flash("Couldn't draw the PNG");
    report(await saveBytesToDevice('diagram.png', 'image/png', new Uint8Array(await blob.arrayBuffer())), 'diagram.png');
  }

  return (
    <Portal>
      <div class="mmd-viewer" role="dialog" aria-label="Diagram">
        <header class="mmd-viewer-head detail-head">
          <button type="button" class="ghost-icon-button" aria-label="Close diagram" onClick={props.onClose}>
            <ArrowLeft size={22} />
          </button>
          <div class="flex-1" />
          <button type="button" class="ghost-icon-button" aria-label="Zoom out" onClick={() => zoomStep(-0.5)}>
            <ZoomOut size={18} />
          </button>
          <button type="button" class="ghost-icon-button" aria-label="Zoom in" onClick={() => zoomStep(0.5)}>
            <ZoomIn size={18} />
          </button>
          <button type="button" class="ghost-icon-button" aria-label="Fit" onClick={() => setXf({ scale: 1, x: 0, y: 0 })}>
            <Maximize2 size={18} />
          </button>
          <button type="button" class="ghost-icon-button" aria-label="Copy source" onClick={() => void copySource()}>
            <Copy size={18} />
          </button>
          <button type="button" class="ghost-icon-button" aria-label="Save as SVG" onClick={() => void saveAsSvg()}>
            <Download size={18} />
          </button>
        </header>
        <div class="mmd-viewer-body">
        <div
          ref={stage}
          class="mmd-viewer-stage"
          innerHTML={props.request.svg}
          style={{ transform: `translate(${xf().x}px, ${xf().y}px) scale(${xf().scale})` }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onTouchStart={onTouchStart}
          onTouchMove={onTouchMove}
          onTouchEnd={onTouchEnd}
          onTouchCancel={onTouchEnd}
          onWheel={onWheel}
          onClick={onTap}
        />
        </div>
        <footer class="mmd-viewer-foot">
          <Show when={toast()}>
            <div class="mmd-viewer-toast" role="status">{toast()}</div>
          </Show>
          <button type="button" class="report-button" onClick={() => void saveAsPng()}>
            Save as PNG
          </button>
        </footer>
      </div>
    </Portal>
  );
}
