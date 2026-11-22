/**
 * Edit an image before sending (UNI-227, moved to the composer by WP-A13) —
 * draw (colour + thickness), arrows, **ellipses/circles** (D-5) and crop.
 * Full screen over the chat; works with a finger (one-finger strokes, big
 * targets, bottom toolbar in thumb reach) and a mouse (pointer events, Esc
 * cancels, Ctrl/⌘+Z undoes, Enter applies a crop).
 *
 * Ellipse (D-5): drag a bounding box, Shift constrains it to a circle, it
 * uses the same stroke colour/width as the other shapes and undo pops it like
 * any other op. Dragging from an existing ellipse's stroke moves it (the hit
 * test is `ellipseAt`); a tap inside a big one starts a new shape.
 *
 * Model: lib/composer/imageEdit.ts (ops in original-image pixels; crop
 * applied last). Done renders the result at full resolution (cropped) to a
 * PNG/JPEG blob.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { ArrowUpRight, Check, Circle, Crop as CropIcon, PenLine, Undo2, X } from 'lucide-solid';
import {
  clampRect,
  COLORS,
  currentCrop,
  drawMarks,
  ellipseAt,
  ellipseRect,
  fitBox,
  hasEdits,
  toImage,
  WIDTHS,
  type EditOp,
  type Ellipse,
  type Point,
  type Rect,
} from '../../lib/composer/imageEdit';

type Tool = 'draw' | 'arrow' | 'ellipse' | 'crop';

/** How close (screen px) a pointer must be to an ellipse's outline to grab it. */
const ELLIPSE_GRAB = 14;

/** A drag the editor is currently running. */
type Drag =
  | { id: number; start: Point; mode: 'draw' | 'arrow' | 'ellipse' | 'crop-new' | 'crop-move' | 'crop-corner'; corner?: [0 | 1, 0 | 1]; base?: Rect }
  | { id: number; start: Point; mode: 'ellipse-move'; index: number; base: Ellipse };

export function ImageEditor(props: { src: string; mime: string; name: string; onCancel: () => void; onDone: (blob: Blob) => void }) {
  const [img, setImg] = createSignal<HTMLImageElement | undefined>();
  const [ops, setOps] = createSignal<EditOp[]>([]);
  const [tool, setTool] = createSignal<Tool>('draw');
  const [color, setColor] = createSignal<string>(COLORS[0]);
  const [widthIdx, setWidthIdx] = createSignal(1);
  /** The crop being dragged (image px), shown over the uncropped image while the Crop tool is on. */
  const [draftCrop, setDraftCrop] = createSignal<Rect | undefined>();
  const [saving, setSaving] = createSignal(false);
  const [size, setSize] = createSignal({ w: 0, h: 0 });
  let wrap: HTMLDivElement | undefined;
  let canvas: HTMLCanvasElement | undefined;

  onMount(() => {
    const el = new Image();
    el.onload = () => setImg(el);
    el.src = props.src;
    // The stage's size drives the fitted box; measure at once (a resize
    // observation can be missed while the portal is still being laid out, as
    // happens in the phone shell) and then on every resize.
    const measure = () => wrap && setSize({ w: wrap.clientWidth, h: wrap.clientHeight });
    const ro = new ResizeObserver(measure);
    if (wrap) ro.observe(wrap);
    measure();
    const frame = requestAnimationFrame(measure);
    onCleanup(() => {
      ro.disconnect();
      cancelAnimationFrame(frame);
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        if (tool() === 'crop' && draftCrop()) setTool('draw');
        else props.onCancel();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        undo();
      } else if (e.key === 'Enter' && tool() === 'crop') {
        e.preventDefault();
        applyCrop();
      }
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  const natural = () => ({ w: img()?.naturalWidth ?? 1, h: img()?.naturalHeight ?? 1 });
  /** Shown region: the whole image while cropping (so you can widen it again), else the crop. */
  const view = (): Rect => (tool() === 'crop' ? { x: 0, y: 0, ...natural() } : currentCrop(ops(), natural().w, natural().h));
  const box = () => fitBox(view(), size().w, size().h, 16);
  const strokeWidth = () => Math.max(2, Math.round(Math.min(natural().w, natural().h) * WIDTHS[widthIdx()]!));

  // Paint: image region + marks (+ the crop frame while cropping).
  createEffect(() => {
    const el = img();
    const c = canvas;
    const { w, h } = size();
    if (!el || !c || !w || !h) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    c.style.width = `${w}px`;
    c.style.height = `${h}px`;
    const ctx = c.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const v = view();
    const b = box();
    ctx.drawImage(el, v.x, v.y, v.w, v.h, b.left, b.top, b.w, b.h);
    ctx.save();
    ctx.beginPath();
    ctx.rect(b.left, b.top, b.w, b.h);
    ctx.clip();
    ctx.translate(b.left, b.top);
    drawMarks(ctx, ops(), { x: v.x, y: v.y }, b.scale);
    ctx.restore();
    if (tool() === 'crop') {
      const r = draftCrop() ?? currentCrop(ops(), natural().w, natural().h);
      const x = b.left + (r.x - v.x) * b.scale;
      const y = b.top + (r.y - v.y) * b.scale;
      const rw = r.w * b.scale;
      const rh = r.h * b.scale;
      ctx.fillStyle = 'rgb(0 0 0 / 55%)';
      ctx.fillRect(b.left, b.top, b.w, y - b.top);
      ctx.fillRect(b.left, y + rh, b.w, b.top + b.h - y - rh);
      ctx.fillRect(b.left, y, x - b.left, rh);
      ctx.fillRect(x + rw, y, b.left + b.w - x - rw, rh);
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 1.5;
      ctx.strokeRect(x, y, rw, rh);
      // Thirds + corner handles.
      ctx.strokeStyle = 'rgb(255 255 255 / 35%)';
      ctx.lineWidth = 1;
      for (const f of [1 / 3, 2 / 3]) {
        ctx.beginPath();
        ctx.moveTo(x + rw * f, y);
        ctx.lineTo(x + rw * f, y + rh);
        ctx.moveTo(x, y + rh * f);
        ctx.lineTo(x + rw, y + rh * f);
        ctx.stroke();
      }
      ctx.fillStyle = '#fff';
      for (const [cx, cy] of [[x, y], [x + rw, y], [x, y + rh], [x + rw, y + rh]] as const) ctx.fillRect(cx - 5, cy - 5, 10, 10);
    }
  });

  // ── pointer input ────────────────────────────────────────────────
  let active: Drag | undefined;
  const local = (e: PointerEvent): Point => {
    const r = canvas!.getBoundingClientRect();
    return toImage({ x: e.clientX - r.left, y: e.clientY - r.top }, box(), view());
  };
  const HANDLE = 28; // px on screen
  const onDown = (e: PointerEvent) => {
    if (active || !img()) return;
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    canvas!.setPointerCapture(e.pointerId);
    const p = local(e);
    const t = tool();
    if (t === 'draw') {
      setOps([...ops(), { kind: 'draw', color: color(), width: strokeWidth(), points: [p] }]);
      active = { id: e.pointerId, start: p, mode: 'draw' };
    } else if (t === 'arrow') {
      setOps([...ops(), { kind: 'arrow', color: color(), width: strokeWidth(), from: p, to: p }]);
      active = { id: e.pointerId, start: p, mode: 'arrow' };
    } else if (t === 'ellipse') {
      // Grabbing an existing outline moves it; anywhere else starts a new shape.
      const hit = ellipseAt(ops(), p, ELLIPSE_GRAB / box().scale);
      if (hit !== undefined) {
        active = { id: e.pointerId, start: p, mode: 'ellipse-move', index: hit, base: ops()[hit] as Ellipse };
        return;
      }
      setOps([...ops(), { kind: 'ellipse', color: color(), width: strokeWidth(), from: p, to: p }]);
      active = { id: e.pointerId, start: p, mode: 'ellipse' };
    } else {
      const r = draftCrop() ?? currentCrop(ops(), natural().w, natural().h);
      const tol = HANDLE / box().scale;
      const near = (a: number, b: number) => Math.abs(a - b) <= tol;
      const cx = near(p.x, r.x) ? 0 : near(p.x, r.x + r.w) ? 1 : -1;
      const cy = near(p.y, r.y) ? 0 : near(p.y, r.y + r.h) ? 1 : -1;
      if (cx >= 0 && cy >= 0) active = { id: e.pointerId, start: p, mode: 'crop-corner', corner: [cx as 0 | 1, cy as 0 | 1], base: r };
      else if (p.x > r.x && p.x < r.x + r.w && p.y > r.y && p.y < r.y + r.h) active = { id: e.pointerId, start: p, mode: 'crop-move', base: r };
      else active = { id: e.pointerId, start: p, mode: 'crop-new' };
    }
  };
  const onMove = (e: PointerEvent) => {
    if (!active || e.pointerId !== active.id) return;
    const p = local(e);
    const { w, h } = natural();
    if (active.mode === 'draw' || active.mode === 'arrow') {
      const list = ops().slice();
      const last = list[list.length - 1]!;
      if (last.kind === 'draw') list[list.length - 1] = { ...last, points: [...last.points, p] };
      else if (last.kind === 'arrow') list[list.length - 1] = { ...last, to: p };
      setOps(list);
    } else if (active.mode === 'ellipse') {
      // The drag started at `from`; `to` is the box's opposite corner. Shift
      // squares the box (D-5), keeping the direction the pointer went.
      const list = ops().slice();
      const last = list[list.length - 1]!;
      if (last.kind === 'ellipse') {
        const from = last.from;
        const raw = { x: Math.max(0, Math.min(w, p.x)), y: Math.max(0, Math.min(h, p.y)) };
        const r = ellipseRect(from, raw, e.shiftKey);
        const to = { x: from.x <= raw.x ? r.x + r.w : r.x, y: from.y <= raw.y ? r.y + r.h : r.y };
        list[list.length - 1] = { ...last, to };
        setOps(list);
      }
    } else if (active.mode === 'ellipse-move') {
      // Drag the whole shape; the box stops at the image edges.
      const base = active.base;
      const rect = ellipseRect(base.from, base.to);
      const dx = Math.max(-rect.x, Math.min(w - rect.x - rect.w, p.x - active.start.x));
      const dy = Math.max(-rect.y, Math.min(h - rect.y - rect.h, p.y - active.start.y));
      const list = ops().slice();
      list[active.index] = { ...base, from: { x: base.from.x + dx, y: base.from.y + dy }, to: { x: base.to.x + dx, y: base.to.y + dy } };
      setOps(list);
    } else if (active.mode === 'crop-new') {
      setDraftCrop(clampRect({ x: active.start.x, y: active.start.y, w: p.x - active.start.x, h: p.y - active.start.y }, w, h));
    } else if (active.mode === 'crop-move' && active.base) {
      const b = active.base;
      const nx = Math.max(0, Math.min(w - b.w, b.x + p.x - active.start.x));
      const ny = Math.max(0, Math.min(h - b.h, b.y + p.y - active.start.y));
      setDraftCrop({ x: Math.round(nx), y: Math.round(ny), w: b.w, h: b.h });
    } else if (active.mode === 'crop-corner' && active.base && active.corner) {
      const b = active.base;
      const [cx, cy] = active.corner;
      const ax = cx === 0 ? b.x + b.w : b.x; // anchor = opposite corner
      const ay = cy === 0 ? b.y + b.h : b.y;
      setDraftCrop(clampRect({ x: ax, y: ay, w: p.x - ax, h: p.y - ay }, w, h));
    }
  };
  const onUp = (e: PointerEvent) => {
    if (!active || e.pointerId !== active.id) return;
    // An arrow tapped without dragging isn't an arrow; nor is a dot of an ellipse.
    if (active.mode === 'arrow') {
      const last = ops()[ops().length - 1];
      if (last?.kind === 'arrow' && Math.hypot(last.to.x - last.from.x, last.to.y - last.from.y) < 4 / box().scale) setOps(ops().slice(0, -1));
    }
    if (active.mode === 'ellipse') {
      const last = ops()[ops().length - 1];
      if (last?.kind === 'ellipse') {
        const rect = ellipseRect(last.from, last.to);
        if (rect.w < 4 / box().scale || rect.h < 4 / box().scale) setOps(ops().slice(0, -1));
      }
    }
    active = undefined;
  };

  const undo = () => {
    if (tool() === 'crop' && draftCrop()) return setDraftCrop(undefined);
    setOps(ops().slice(0, -1));
  };
  const pickTool = (t: Tool) => {
    if (tool() === 'crop' && t !== 'crop') applyCrop(false);
    setTool(t);
  };
  /** Commit the dragged crop as an op (undo-able). */
  const applyCrop = (thenDraw = true) => {
    const d = draftCrop();
    if (d) {
      const cur = currentCrop(ops(), natural().w, natural().h);
      if (d.x !== cur.x || d.y !== cur.y || d.w !== cur.w || d.h !== cur.h) setOps([...ops(), { kind: 'crop', rect: d }]);
    }
    setDraftCrop(undefined);
    if (thenDraw) setTool('draw');
  };
  const resetCrop = () => {
    setDraftCrop({ x: 0, y: 0, ...natural() });
  };

  const done = async () => {
    if (tool() === 'crop') applyCrop(false);
    const el = img();
    if (!el) return;
    if (!hasEdits(ops(), natural().w, natural().h)) return props.onCancel();
    setSaving(true);
    try {
      const crop = currentCrop(ops(), natural().w, natural().h);
      const out = document.createElement('canvas');
      out.width = crop.w;
      out.height = crop.h;
      const ctx = out.getContext('2d')!;
      ctx.drawImage(el, crop.x, crop.y, crop.w, crop.h, 0, 0, crop.w, crop.h);
      drawMarks(ctx, ops(), { x: crop.x, y: crop.y }, 1);
      const type = props.mime === 'image/jpeg' ? 'image/jpeg' : 'image/png';
      const blob = await new Promise<Blob | null>((resolve) => out.toBlob(resolve, type, 0.92));
      if (blob) props.onDone(blob);
    } finally {
      setSaving(false);
    }
  };

  return (
    <Portal>
      <div class="img-edit" role="dialog" aria-label="Edit image" data-testid="image-editor">
        <header class="img-edit-head">
          <button type="button" class="viewer-icon" aria-label="Cancel" onClick={props.onCancel}>
            <X size={22} />
          </button>
          <span class="img-edit-title">{tool() === 'crop' ? 'Crop' : 'Edit image'}</span>
          <button type="button" class="viewer-icon" aria-label="Undo" disabled={!ops().length && !draftCrop()} onClick={undo}>
            <Undo2 size={20} />
          </button>
          <button type="button" class="img-edit-done" disabled={saving() || !img()} onClick={() => void done()}>
            <Check size={18} /> {saving() ? 'Saving…' : 'Done'}
          </button>
        </header>
        <div class="img-edit-stage" ref={wrap}>
          <canvas
            ref={canvas}
            class="img-edit-canvas"
            data-tool={tool()}
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerCancel={onUp}
          />
          <Show when={!img()}>
            <p class="img-edit-loading">Loading…</p>
          </Show>
        </div>
        <footer class="img-edit-foot">
          <Show
            when={tool() !== 'crop'}
            fallback={
              <div class="img-edit-row">
                <button type="button" class="img-edit-pill" onClick={resetCrop}>Reset</button>
                <span class="img-edit-hint">Drag a corner or the frame</span>
                <button type="button" class="img-edit-pill primary" onClick={() => applyCrop()}>Apply crop</button>
              </div>
            }
          >
            <div class="img-edit-row" role="radiogroup" aria-label="Colour">
              <For each={COLORS}>
                {(c) => (
                  <button type="button" role="radio" class="img-edit-swatch" aria-label={`Colour ${c}`} aria-checked={color() === c} style={{ '--swatch': c }} onClick={() => setColor(c)} />
                )}
              </For>
              <label class="img-edit-swatch img-edit-custom" aria-label="Pick any colour" style={{ '--swatch': color() }}>
                <input type="color" value={color()} onInput={(e) => setColor(e.currentTarget.value)} />
              </label>
            </div>
            <div class="img-edit-row" role="radiogroup" aria-label="Thickness">
              <For each={WIDTHS}>
                {(_, i) => (
                  <button type="button" role="radio" class="img-edit-width" aria-label={['Thin', 'Medium', 'Thick'][i()]} aria-checked={widthIdx() === i()} onClick={() => setWidthIdx(i())}>
                    <span style={{ width: `${6 + i() * 6}px`, height: `${6 + i() * 6}px`, background: color() }} />
                  </button>
                )}
              </For>
            </div>
          </Show>
          <div class="img-edit-tools" role="radiogroup" aria-label="Tool">
            <button type="button" role="radio" class="img-edit-tool" aria-checked={tool() === 'draw'} onClick={() => pickTool('draw')}>
              <PenLine size={20} />
              <span>Draw</span>
            </button>
            <button type="button" role="radio" class="img-edit-tool" aria-checked={tool() === 'arrow'} onClick={() => pickTool('arrow')}>
              <ArrowUpRight size={20} />
              <span>Arrow</span>
            </button>
            <button type="button" role="radio" class="img-edit-tool" aria-checked={tool() === 'ellipse'} onClick={() => pickTool('ellipse')} data-testid="tool-ellipse">
              <Circle size={20} />
              <span>Ellipse</span>
            </button>
            <button type="button" role="radio" class="img-edit-tool" aria-checked={tool() === 'crop'} onClick={() => pickTool('crop')}>
              <CropIcon size={20} />
              <span>Crop</span>
            </button>
          </div>
        </footer>
      </div>
    </Portal>
  );
}
