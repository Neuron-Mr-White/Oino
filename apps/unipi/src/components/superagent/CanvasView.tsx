import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Maximize2, Minus, Network, Plus, RotateCcw, SlidersHorizontal, X } from 'lucide-solid';
import { modelDisplayName } from '../../lib/model/displayName';
import type { SuperagentState } from './state';
import { buildTree, fitView, filterTree, fleetCounts, FILTERS, FILTER_LABEL, layoutFleet, MODE_BADGE, openTarget, RESET_VIEW, STATUS_LABEL, WRAP_WIDTH, zoomAt, type FleetFilter, type FleetStatus, type View } from './fleet';
import { canvasDisplay, canvasFilter, canvasView, setCanvasDisplay, setCanvasFilter, setCanvasView, type CanvasDisplay } from './canvasStore';
import './canvas.css';

const STATUSES: readonly FleetStatus[] = ['working', 'waiting', 'idle', 'ended'];
const MIN_WRAP = 520;
const PAN_STEP = 64;
const ZOOM_STEP = 1.2;
const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])';

/**
 * Full-screen map of every agent: one dashed region per project, the lead on top, its
 * subagents below. Reads `sa.fleet()` reactively, so the shared 5 s poll adds, removes and
 * updates nodes. Clicking a node hands its session to the page (`onOpen`) and the page closes the canvas.
 */
export function CanvasView(props: { sa: SuperagentState; onOpen: (sessionId: string) => void; onClose: () => void }) {
  const [size, setSize] = createSignal({ width: 0, height: 0 });
  const tree = createMemo(() => buildTree(props.sa.fleet()));
  const counts = createMemo(() => fleetCounts(props.sa.fleet().nodes));
  const filtered = createMemo(() => filterTree(tree(), canvasFilter()));
  /** Regions wrap to the screen: on a phone they stack instead of running off to a 25% zoom. */
  const wrap = () => (size().width ? Math.min(WRAP_WIDTH, Math.max(MIN_WRAP, size().width / 0.65)) : WRAP_WIDTH);
  const layout = createMemo(() => layoutFleet(tree(), filtered(), wrap()));
  const placed = createMemo(() => new Map(layout().nodes.map((n) => [n.id, n])));
  const nodeIds = createMemo(() => layout().nodes.map((n) => n.id));
  const regionKeys = createMemo(() => layout().regions.map((r) => r.key));
  const edgeKeys = createMemo(() => layout().edges.map((e) => `${e.from}>${e.to}`));
  const edgeOf = createMemo(() => new Map(layout().edges.map((e) => [`${e.from}>${e.to}`, e])));
  const regionOf = createMemo(() => new Map(layout().regions.map((r) => [r.key, r])));

  const [view, setViewSignal] = createSignal<View>(canvasView() ?? RESET_VIEW);
    const [menu, setMenu] = createSignal(false);
  const [panning, setPanning] = createSignal(false);
  let root!: HTMLDivElement;
  let stage!: HTMLDivElement;
  let world!: HTMLDivElement;
  let displayBtn!: HTMLButtonElement;
  let menuEl: HTMLDivElement | undefined;
  /** Fit until the owner moves the view themselves (and again when the filter changes). */
  let autoFit = canvasView() === null;
  const opener = document.activeElement as HTMLElement | null;

  const move = (next: View, manual = true) => { if (manual) autoFit = false; setViewSignal(next); setCanvasView(next); };
  const fit = () => move(fitView(layout(), size()), false);
  const centre = () => ({ x: size().width / 2, y: size().height / 2 });
  const zoomBy = (factor: number, at = centre()) => move(zoomAt(view(), factor, at.x, at.y));

  createEffect(on(canvasFilter, () => { autoFit = true; }, { defer: true }));
  createEffect(() => {
    const l = layout(); size();
    if (autoFit && l.nodes.length && size().width) { fit(); autoFit = false; }
  });

  onMount(() => {
    root.focus();
    const measure = () => setSize({ width: stage.clientWidth, height: stage.clientHeight });
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(measure);
    observer?.observe(stage);
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = stage.getBoundingClientRect();
      const lines = e.deltaMode === 1 ? 16 : 1;
      zoomBy(Math.exp(-e.deltaY * lines * (e.ctrlKey || e.metaKey ? 0.01 : 0.0015)), { x: e.clientX - rect.left, y: e.clientY - rect.top });
    };
    stage.addEventListener('wheel', wheel, { passive: false });
    const outside = (e: PointerEvent) => { if (menu() && !menuEl?.contains(e.target as Node) && !displayBtn.contains(e.target as Node)) setMenu(false); };
    document.addEventListener('pointerdown', outside);
    onCleanup(() => { observer?.disconnect(); stage.removeEventListener('wheel', wheel); document.removeEventListener('pointerdown', outside); if (opener?.isConnected) opener.focus?.(); });
  });

  let drag: { id: number; x: number; y: number; view: View } | undefined;
  const down = (e: PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('.sac-node, .sac-legend, .sac-empty')) return;
    drag = { id: e.pointerId, x: e.clientX, y: e.clientY, view: view() };
    stage.setPointerCapture?.(e.pointerId);
    setPanning(true);
  };
  const dragMove = (e: PointerEvent) => { if (drag?.id === e.pointerId) move({ ...drag.view, x: drag.view.x + e.clientX - drag.x, y: drag.view.y + e.clientY - drag.y }); };
  const up = (e: PointerEvent) => { if (drag?.id === e.pointerId) { drag = undefined; setPanning(false); stage.releasePointerCapture?.(e.pointerId); } };

  const worldKeys = (e: KeyboardEvent) => {
    if (e.target !== world || e.altKey || e.ctrlKey || e.metaKey) return;
    const v = view();
    const pan: Record<string, [number, number]> = { ArrowLeft: [PAN_STEP, 0], ArrowRight: [-PAN_STEP, 0], ArrowUp: [0, PAN_STEP], ArrowDown: [0, -PAN_STEP] };
    const by = pan[e.key];
    if (by) move({ ...v, x: v.x + by[0], y: v.y + by[1] });
    else if (e.key === '+' || e.key === '=') zoomBy(ZOOM_STEP);
    else if (e.key === '-') zoomBy(1 / ZOOM_STEP);
    else if (e.key === '0') move(RESET_VIEW);
    else return;
    e.preventDefault();
  };

  const keys = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault(); e.stopPropagation();
      if (menu()) { setMenu(false); displayBtn.focus(); } else props.onClose();
      return;
    }
    if (e.key !== 'Tab') return;
    const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const first = items[0]; const last = items[items.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && (document.activeElement === first || document.activeElement === root)) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  };

  const setDisplay = (key: keyof CanvasDisplay, value: boolean) => setCanvasDisplay({ ...canvasDisplay(), [key]: value });
  const empty = () => layout().nodes.length === 0;
  const emptyText = () => (canvasFilter() === 'working' ? 'No agents working' : canvasFilter() === 'all' ? 'No agents yet' : 'No agents match this filter');
  const open = (id: string) => props.onOpen(openTarget(tree(), id));

  return <Portal><div class="sac" role="dialog" aria-modal="true" aria-label="Agent canvas" tabIndex={-1} ref={root} onKeyDown={keys} data-testid="agent-canvas">
    <header class="sac-bar">
      <div class="sac-title"><Network size={18} aria-hidden="true" /><h2>Agent canvas</h2></div>
      <div class="sac-chips" role="group" aria-label="Filter agents">
        <For each={FILTERS}>{(f: FleetFilter) => <button type="button" class="sac-chip" aria-pressed={canvasFilter() === f} data-filter={f} onClick={() => setCanvasFilter(f)}>{FILTER_LABEL[f]}<span class="sac-chip-count" aria-label={`${counts()[f]} agents`}>{counts()[f]}</span></button>}</For>
      </div>
      <div class="sac-tools">
        <div class="sac-menu-anchor">
          <button type="button" class="sac-btn" ref={displayBtn} aria-label="Display" aria-haspopup="dialog" aria-expanded={menu()} onClick={() => setMenu(!menu())}><SlidersHorizontal size={15} aria-hidden="true" />Display</button>
          <Show when={menu()}><div class="sac-menu" role="dialog" aria-label="Display" ref={menuEl}>
            <label class="sac-check"><input type="checkbox" checked={canvasDisplay().name} onChange={(e) => setDisplay('name', e.currentTarget.checked)} />Agent name</label>
            <label class="sac-check"><input type="checkbox" checked={canvasDisplay().model} onChange={(e) => setDisplay('model', e.currentTarget.checked)} />Model name</label>
            <label class="sac-check"><input type="checkbox" checked={canvasDisplay().status} onChange={(e) => setDisplay('status', e.currentTarget.checked)} />Status</label>
          </div></Show>
        </div>
        <div class="sac-zoom" role="group" aria-label="Zoom">
          <button type="button" class="sac-icon" aria-label="Zoom out" title="Zoom out" onClick={() => zoomBy(1 / ZOOM_STEP)}><Minus size={16} /></button>
          <output class="sac-scale" aria-label="Zoom level">{Math.round(view().scale * 100)}%</output>
          <button type="button" class="sac-icon" aria-label="Zoom in" title="Zoom in" onClick={() => zoomBy(ZOOM_STEP)}><Plus size={16} /></button>
        </div>
        <button type="button" class="sac-btn" aria-label="Fit to screen" title="Fit all visible projects" onClick={fit}><Maximize2 size={15} aria-hidden="true" /><span class="sac-lbl">Fit</span></button>
        <button type="button" class="sac-btn" aria-label="Reset view" title="Scale 100%" onClick={() => move(RESET_VIEW)}><RotateCcw size={15} aria-hidden="true" /><span class="sac-lbl">Reset</span></button>
      </div>
      <button type="button" class="sac-icon sac-close" aria-label="Close canvas" title="Close (Esc)" onClick={props.onClose}><X size={18} /></button>
    </header>
    <div class="sac-stage" ref={stage} data-panning={panning() ? '' : undefined} onPointerDown={down} onPointerMove={dragMove} onPointerUp={up} onPointerCancel={up}>
      <div class="sac-world" ref={world} tabIndex={0} role="group" aria-label="Agents by project. Arrow keys pan, plus and minus zoom." onKeyDown={worldKeys} style={{ transform: `translate(${view().x}px, ${view().y}px) scale(${view().scale})` }}>
        <svg class="sac-edges" width={layout().width} height={layout().height} aria-hidden="true">
          <For each={edgeKeys()}>{(key) => <path d={edgeOf().get(key)?.d} data-dim={edgeOf().get(key)?.dimmed ? '' : undefined} />}</For>
        </svg>
        <For each={regionKeys()}>{(key) => <div class="sac-region" data-ungrouped={regionOf().get(key)?.ungrouped ? '' : undefined} data-testid={`fleet-region-${key}`} style={{ left: `${regionOf().get(key)?.x}px`, top: `${regionOf().get(key)?.y}px`, width: `${regionOf().get(key)?.w}px`, height: `${regionOf().get(key)?.h}px` }}><span class="sac-region-name" title={regionOf().get(key)?.name}>{regionOf().get(key)?.name}</span></div>}</For>
        <For each={nodeIds()}>{(id) => {
          const node = () => tree().nodes.get(id);
          const at = () => placed().get(id);
          const dim = () => filtered().dimmed.has(id);
          return <button type="button" class="sac-node" data-testid={`fleet-node-${id}`} data-status={node()?.status} data-role={node()?.role} data-dim={dim() ? '' : undefined}
            aria-label={`${node()?.title || id}, ${STATUS_LABEL[node()?.status ?? 'idle']}${node()?.model ? `, ${modelDisplayName({ id: node()!.model! })}` : ''}`}
            style={{ left: `${at()?.x}px`, top: `${at()?.y}px`, width: `${at()?.w}px`, height: `${at()?.h}px` }} onClick={() => open(id)}>
            <span class="sac-dot" aria-hidden="true" />
            <span class="sac-node-main">
              <span class="sac-node-top">
                <Show when={canvasDisplay().name}><strong class="sac-node-name" title={node()?.title}>{node()?.title || id}</strong></Show>
                <Show when={node()?.role === 'lead'}><span class="sac-mode" data-mode={node()?.mode}>{MODE_BADGE[node()?.mode ?? 'plain']}</span></Show>
              </span>
              <Show when={canvasDisplay().model && node()?.model}><span class="sac-node-model">{modelDisplayName({ id: node()!.model! })}</span></Show>
              <Show when={canvasDisplay().status}><span class="sac-node-status">{STATUS_LABEL[node()?.status ?? 'idle']}</span></Show>
            </span>
          </button>;
        }}</For>
      </div>
      <Show when={empty()}><div class="sac-empty" role="status"><p>{emptyText()}</p><Show when={canvasFilter() !== 'all'}><button type="button" class="sac-btn" onClick={() => setCanvasFilter('all')}>Show all</button></Show></div></Show>
      <div class="sac-legend" aria-label="Legend">
        <div><For each={STATUSES}>{(s) => <span class="sac-legend-item" data-status={s}><i class="sac-dot" />{s === 'waiting' ? 'Waiting' : STATUS_LABEL[s]}</span>}</For></div>
        <div><For each={['plain', 'graph', 'swarm'] as const}>{(m) => <span class="sac-legend-item"><span class="sac-mode" data-mode={m}>{MODE_BADGE[m]}</span></span>}</For></div>
      </div>
    </div>
  </div></Portal>;
}
