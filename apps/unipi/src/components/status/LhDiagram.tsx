/**
 * The long-horizon diagram ([lh-ui]): one HTML card per node (name, status chip, profile and
 * model chips, metrics) on an SVG edge layer, with pan (drag), zoom (wheel / pinch / buttons)
 * and Fit. Layout is the pure layered grid of `layoutGraph`. Nodes are foreignObjects so they
 * share the list rows' chip styles; at 40 nodes that is 40 small subtrees, no per-node timers.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Maximize, Minus, Plus } from 'lucide-solid';
import { NODE_H, NODE_W, layoutGraph, type LhWork } from '../../lib/status/lhGraph';
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

const edgePath = (ax: number, ay: number, bx: number, by: number) => {
  const dy = Math.max(20, Math.abs(by - ay) / 2);
  return `M${ax},${ay} C${ax},${ay + dy} ${bx},${by - dy} ${bx},${by}`;
};

export function LhDiagram(props: { work: readonly LhWork[]; now: number; selected?: string; onSelect: (id: string) => void; perRow?: number; height?: number }) {
  const layout = createMemo(() => layoutGraph(props.work, props.perRow));
  const byId = createMemo(() => new Map(props.work.map((w) => [w.workId, w])));
  const pos = createMemo(() => new Map(layout().nodes.map((n) => [n.id, n])));
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
    place(false);
    if (typeof ResizeObserver === 'undefined' || !host) return;
    const ro = new ResizeObserver(() => !touched() && place(false));
    ro.observe(host);
    onCleanup(() => ro.disconnect());
  });
  // Re-fit while the user has not panned/zoomed and the shape of the graph changes.
  createEffect(on(() => `${layout().nodes.length}:${layout().width}:${layout().height}`, () => !touched() && place(false), { defer: true }));

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
    <div class="lh-dia" data-testid="lh-diagram">
      <div
        ref={host}
        class="lh-dia-view"
        style={{ height: `${props.height ?? 320}px` }}
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
        <svg class="lh-dia-svg" width="100%" height="100%">
          <defs>
            <marker id="lh-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
              <path d="M0,0 L8,4 L0,8 z" class="lh-dia-arrow" />
            </marker>
          </defs>
          <g transform={`translate(${view().x} ${view().y}) scale(${view().z})`}>
            <For each={layout().edges}>
              {(e) => {
                const a = () => pos().get(e.from);
                const b = () => pos().get(e.to);
                return (
                  <Show when={a() && b()}>
                    <path
                      class="lh-dia-edge"
                      data-kind={e.kind}
                      data-lit={props.selected === e.from || props.selected === e.to ? '' : undefined}
                      d={edgePath(a()!.x + NODE_W / 2, a()!.y + NODE_H, b()!.x + NODE_W / 2, b()!.y)}
                      marker-end="url(#lh-arrow)"
                    />
                  </Show>
                );
              }}
            </For>
            <For each={layout().nodes}>
              {(n) => {
                const w = () => byId().get(n.id)!;
                return (
                  <foreignObject x={n.x} y={n.y} width={NODE_W} height={NODE_H} class="lh-dia-fo">
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
                  </foreignObject>
                );
              }}
            </For>
          </g>
        </svg>
        <div class="lh-dia-tools" onPointerDown={(e) => e.stopPropagation()}>
          <button type="button" class="lh-icon-btn" aria-label="Zoom in" title="Zoom in" onClick={() => zoomBy(1.25)}><Plus size={14} /></button>
          <button type="button" class="lh-icon-btn" aria-label="Zoom out" title="Zoom out" onClick={() => zoomBy(0.8)}><Minus size={14} /></button>
          <button type="button" class="lh-icon-btn" aria-label="Fit the graph" title="Fit" data-testid="lh-dia-fit" onClick={fit}><Maximize size={14} /></button>
        </div>
        <Show when={props.work.length === 0}>
          <p class="lh-dia-empty status-empty">Nothing to draw.</p>
        </Show>
      </div>
    </div>
  );
}
