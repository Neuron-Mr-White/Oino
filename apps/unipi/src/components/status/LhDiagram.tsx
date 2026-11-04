/**
 * The long-horizon diagram ([lh-ui]): one HTML card per node (name, status chip, profile and
 * model chips, metrics) over an SVG edge layer, with pan (drag), zoom (wheel / pinch / buttons),
 * Fit and a full-window overlay. Layout is elkjs (`layoutGraphElk`, async, orthogonal edges).
 * Edges and cards live in ONE transformed layer (a single CSS transform), so everything zooms
 * together — no foreignObject (WebKit does not scale HTML inside it reliably).
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Expand, Minus, Plus, ScanSearch, X } from 'lucide-solid';
import { NODE_H, NODE_W, graphShapeKey, layoutGraph, layoutGraphElk, type Layout, type LhWork, type Pt } from '../../lib/status/lhGraph';
import { Metrics, RunChips, StatusChip } from './LhParts';

const MIN_Z = 0.15;
const MAX_Z = 2.5;

export type View = { x: number; y: number; z: number };

/** The view that fits a `w`×`h` drawing into a `vw`×`vh` viewport with `pad` (never zooms past 1). Pure. */
export function fitView(w: number, h: number, vw: number, vh: number, pad = 12): View {
  if (w <= 0 || h <= 0 || vw <= 0 || vh <= 0) return { x: 0, y: 0, z: 1 };
  const z = Math.max(MIN_Z, Math.min(1, (vw - pad * 2) / w, (vh - pad * 2) / h));
  return { z, x: (vw - w * z) / 2, y: Math.max(pad, (vh - h * z) / 2) };
}

/** The opening view: fit the WIDTH (nodes stay readable, never below 0.55), anchored at the top; a tall
 *  graph is then panned / zoomed, "Fit" shows all of it. Pure. */
export function openView(w: number, h: number, vw: number, vh: number, pad = 12): View {
  const all = fitView(w, h, vw, vh, pad);
  if (all.z >= 0.55) return all;
  const z = Math.max(0.55, Math.min(1, (vw - pad * 2) / Math.max(1, w)));
  return { z, x: Math.max(pad, (vw - w * z) / 2), y: pad };
}

/** Zoom by `factor` keeping the viewport point (`px`,`py`) fixed. Pure. */
export function zoomAt(v: View, factor: number, px: number, py: number): View {
  const z = Math.max(MIN_Z, Math.min(MAX_Z, v.z * factor));
  const k = z / v.z;
  return { z, x: px - (px - v.x) * k, y: py - (py - v.y) * k };
}

/** A polyline as an SVG path with rounded corners (radius `r`). Pure. */
export function roundedPath(pts: readonly Pt[], r = 8): string {
  if (pts.length < 2) return '';
  let d = `M${pts[0]!.x},${pts[0]!.y}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const p = pts[i - 1]!;
    const c = pts[i]!;
    const n = pts[i + 1]!;
    const l1 = Math.hypot(c.x - p.x, c.y - p.y);
    const l2 = Math.hypot(n.x - c.x, n.y - c.y);
    const k = Math.min(r, l1 / 2, l2 / 2);
    if (k <= 0) continue;
    d += ` L${c.x + ((p.x - c.x) / l1) * k},${c.y + ((p.y - c.y) / l1) * k} Q${c.x},${c.y} ${c.x + ((n.x - c.x) / l2) * k},${c.y + ((n.y - c.y) / l2) * k}`;
  }
  const last = pts[pts.length - 1]!;
  return `${d} L${last.x},${last.y}`;
}

type Props = { work: readonly LhWork[]; now: number; selected?: string; onSelect: (id: string) => void; height?: number; perRow?: number };

export function LhDiagram(props: Props) {
  // The grid is the instant first frame; elk replaces it when ready (and on every shape change).
  const [layout, setLayout] = createSignal<Layout>(layoutGraph(props.work, props.perRow ?? 2));
  let token = 0;
  createEffect(
    on(
      () => graphShapeKey(props.work),
      () => {
        const mine = ++token;
        const work = [...props.work];
        void layoutGraphElk(work).then((l) => mine === token && setLayout(l));
      },
    ),
  );
  onCleanup(() => token++);
  const [full, setFull] = createSignal(false);
  return (
    <>
      <Canvas {...props} layout={layout()} onFull={() => setFull(true)} />
      <Show when={full()}>
        <Portal>
          <div class="lh-dia-full" role="dialog" aria-modal="true" aria-label="Graph diagram, full window" data-testid="lh-dia-full">
            <Canvas {...props} layout={layout()} full onClose={() => setFull(false)} />
          </div>
        </Portal>
      </Show>
    </>
  );
}

function Canvas(props: Props & { layout: Layout; full?: boolean; onFull?: () => void; onClose?: () => void }) {
  const layout = () => props.layout;
  const byId = createMemo(() => new Map(props.work.map((w) => [w.workId, w])));
  // A stale layout (just after hiding errors) never shows a node / edge that is no longer in `work`.
  const nodes = createMemo(() => layout().nodes.filter((n) => byId().has(n.id)));
  const edges = createMemo(() => layout().edges.filter((e) => byId().has(e.from) && byId().has(e.to)));
  const [view, setView] = createSignal<View>({ x: 0, y: 0, z: 1 });
  const [touched, setTouched] = createSignal(false);
  let host: HTMLDivElement | undefined;
  const size = () => ({ w: host?.clientWidth ?? 0, h: host?.clientHeight ?? 0 });
  const place = (all: boolean) => {
    const s = size();
    setView((all ? fitView : openView)(layout().width, layout().height, s.w, s.h));
    setTouched(false);
  };
  const fit = () => place(true);
  onMount(() => {
    props.full ? fit() : place(false);
    host?.focus({ preventScroll: true });
    if (typeof ResizeObserver === 'undefined' || !host) return;
    const ro = new ResizeObserver(() => !touched() && (props.full ? fit() : place(false)));
    ro.observe(host);
    onCleanup(() => ro.disconnect());
  });
  // Re-place while the user has not panned/zoomed and the drawing changes.
  createEffect(on(() => `${layout().width}:${layout().height}`, () => !touched() && (props.full ? fit() : place(false)), { defer: true }));
  if (props.full) {
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && (e.stopPropagation(), props.onClose?.());
    window.addEventListener('keydown', esc, true);
    onCleanup(() => window.removeEventListener('keydown', esc, true));
  }

  const zoomBy = (f: number) => {
    const s = size();
    setView((v) => zoomAt(v, f, s.w / 2, s.h / 2));
    setTouched(true);
  };
  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const r = host!.getBoundingClientRect();
    setView((v) => zoomAt(v, Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top));
    setTouched(true);
  };
  // Pointer pan + two-pointer pinch.
  const pointers = new Map<number, { x: number; y: number }>();
  let moved = 0;
  let pinch = 0;
  const onDown = (e: PointerEvent) => {
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    moved = 0;
    pinch = pointers.size === 2 ? dist() : 0;
  };
  const dist = () => {
    const [a, b] = [...pointers.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  };
  const onMove = (e: PointerEvent) => {
    const p = pointers.get(e.pointerId);
    if (!p) return;
    const dx = e.clientX - p.x;
    const dy = e.clientY - p.y;
    p.x = e.clientX;
    p.y = e.clientY;
    moved += Math.abs(dx) + Math.abs(dy);
    if (moved > 4 && pointers.size === 1) {
      if (!host?.hasPointerCapture(e.pointerId)) host?.setPointerCapture(e.pointerId);
      setView((v) => ({ ...v, x: v.x + dx, y: v.y + dy }));
      setTouched(true);
    } else if (pointers.size === 2) {
      const d = dist();
      if (pinch > 0 && d > 0) {
        const r = host!.getBoundingClientRect();
        const [a, b] = [...pointers.values()];
        setView((v) => zoomAt(v, d / pinch, (a!.x + b!.x) / 2 - r.left, (a!.y + b!.y) / 2 - r.top));
        setTouched(true);
      }
      pinch = d;
    }
  };
  const onUp = (e: PointerEvent) => {
    pointers.delete(e.pointerId);
    pinch = 0;
  };
  const pick = (id: string) => {
    if (moved <= 4) props.onSelect(id);
  };
  const onKey = (e: KeyboardEvent) => {
    const step = 40;
    if (e.key === '+' || e.key === '=') zoomBy(1.2);
    else if (e.key === '-') zoomBy(1 / 1.2);
    else if (e.key === '0') fit();
    else if (e.key === 'ArrowLeft') (setView((v) => ({ ...v, x: v.x + step })), setTouched(true));
    else if (e.key === 'ArrowRight') (setView((v) => ({ ...v, x: v.x - step })), setTouched(true));
    else if (e.key === 'ArrowUp') (setView((v) => ({ ...v, y: v.y + step })), setTouched(true));
    else if (e.key === 'ArrowDown') (setView((v) => ({ ...v, y: v.y - step })), setTouched(true));
    else return;
    e.preventDefault();
  };

  return (
    <div class="lh-dia" data-testid="lh-diagram" data-full={props.full ? '' : undefined}>
      <div
        ref={host}
        class="lh-dia-view"
        style={props.full ? undefined : { height: `${props.height ?? 320}px` }}
        tabindex={0}
        role="group"
        aria-label="Graph diagram — drag to pan, scroll or pinch to zoom"
        onWheel={onWheel}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onKeyDown={onKey}
      >
        <div
          class="lh-dia-layer"
          data-testid="lh-dia-layer"
          style={{ width: `${layout().width}px`, height: `${layout().height}px`, transform: `translate(${view().x}px, ${view().y}px) scale(${view().z})` }}
        >
          <svg class="lh-dia-svg" width={layout().width} height={layout().height} aria-hidden="true">
            <defs>
              <marker id={props.full ? 'lh-arrow-full' : 'lh-arrow'} viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto">
                <path d="M0,0 L8,4 L0,8 z" class="lh-dia-arrow" />
              </marker>
            </defs>
            <For each={edges()}>
              {(e) => (
                <path
                  class="lh-dia-edge"
                  data-kind={e.kind}
                  data-lit={props.selected === e.from || props.selected === e.to ? '' : undefined}
                  d={roundedPath(e.points)}
                  marker-end={`url(#${props.full ? 'lh-arrow-full' : 'lh-arrow'})`}
                />
              )}
            </For>
          </svg>
          <For each={nodes()}>
            {(n) => {
              const w = () => byId().get(n.id)!;
              return (
                <Show when={byId().has(n.id)}>
                  <div class="lh-dia-fo" style={{ left: `${n.x}px`, top: `${n.y}px`, width: `${NODE_W}px`, height: `${NODE_H}px` }}>
                    <div
                      class="lh-dia-node"
                      data-status={w().status}
                      data-selected={props.selected === n.id ? '' : undefined}
                      data-testid="lh-dia-node"
                      role="button"
                      tabindex={0}
                      aria-label={`${n.id}, ${w().status}`}
                      onClick={() => pick(n.id)}
                      onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), props.onSelect(n.id))}
                    >
                      <div class="lh-dia-top">
                        <span class="lh-dia-name" title={n.id}>{n.id}</span>
                        <StatusChip status={w().status} size="sm" />
                      </div>
                      <div class="lh-dia-chips">
                        <RunChips work={w()} />
                      </div>
                      <div class="lh-dia-foot">
                        <Metrics work={w()} now={props.now} />
                      </div>
                    </div>
                  </div>
                </Show>
              );
            }}
          </For>
        </div>
        <div class="lh-dia-tools" onPointerDown={(e) => e.stopPropagation()}>
          <button type="button" class="lh-icon-btn" aria-label="Zoom in" title="Zoom in" data-testid="lh-dia-zoom-in" onClick={() => zoomBy(1.25)}><Plus size={14} /></button>
          <button type="button" class="lh-icon-btn" aria-label="Zoom out" title="Zoom out" data-testid="lh-dia-zoom-out" onClick={() => zoomBy(0.8)}><Minus size={14} /></button>
          <button type="button" class="lh-icon-btn" aria-label="Fit the graph" title="Fit to view" data-testid="lh-dia-fit" onClick={fit}><ScanSearch size={14} /></button>
          <Show when={!props.full}>
            <button type="button" class="lh-icon-btn" aria-label="Open in full window" title="Full window" data-testid="lh-dia-full-open" onClick={() => props.onFull?.()}><Expand size={14} /></button>
          </Show>
        </div>
        <Show when={props.full}>
          <button type="button" class="lh-dia-close" aria-label="Close full window (Esc)" title="Close (Esc)" data-testid="lh-dia-full-close" onClick={() => props.onClose?.()} onPointerDown={(e) => e.stopPropagation()}><X size={16} /></button>
        </Show>
        <Show when={props.work.length === 0}>
          <p class="lh-dia-empty status-empty">Nothing to draw.</p>
        </Show>
      </div>
    </div>
  );
}
