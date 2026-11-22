import { batch, createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, untrack } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Bot, Copy, ExternalLink, Eye, MessageSquare, Play, RotateCcw, Shuffle, Square, TriangleAlert } from 'lucide-solid';
import { toAppError } from '@unipi/protocol';
import { layout as appLayout } from '../../lib/layout';
import { appNavigate } from '../../lib/nav';
import { boardStore } from '../../lib/board/store';
import { copyText } from '../../lib/board/share';
import { hubs } from '../../lib/session/hub';
import { openRowMenu } from '../../lib/session/actions';
import { showActionNotice } from '../../lib/timeline/actions';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { ActionSheet, confirmSheet, SheetFrame, type ActionItem } from '../phone/Sheet';
import { HostModelDialog } from '../model/HostModelPicker';
import { isUnsupportedOp, superagentApi, type LeadAction } from './api';
import type { SuperagentState } from './state';
import {
  buildTree, clock, filterActive, filterTree, fitView, fleetCounts, intersects, layoutFleet, lodOf, LOD_DOTS, navOf, nextNeedingYou, nodeTitle, openTarget, panToReveal, RESET_VIEW, searchMatches, DEFAULT_FILTER, STATUS_LABEL, STATUSES, worldRect, zoomAt,
  type CanvasFilter, type Layout, type PlacedEdge, type PlacedNode, type Rect, type View,
} from './fleet';
import { canvasCollapsed, canvasDensity, canvasFilter, canvasPeekWidth, canvasView, setCanvasDensity, setCanvasFilter, setCanvasPeekWidth, setCanvasView, toggleCanvasGroup, PEEK_MAX, PEEK_MIN } from './canvasStore';
import { FleetDetail } from './FleetDetail';
import { Edges } from './canvas/Edges';
import { GroupView } from './canvas/GroupView';
import { NodeView, type NodeCtx } from './canvas/NodeView';
import { FilterPanel, Toolbar, ZoomDock } from './canvas/Toolbar';
import './canvas.css';

const PAN_STEP = 64;
const ZOOM_STEP = 1.2;
const CULL_ABOVE = 150;
const ANIM_BUDGET = 60;
const STALE_AFTER = 3;
/** A hung host call holds a poll for up to 12 s, so "3 missed polls" is also judged by the clock: 3 x 5 s. */
const STALE_MS = 15_000;
const DRAG_PX = 4;
const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), [tabindex]:not([tabindex="-1"])';
const sameList = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);
const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
const reduced = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

/** What a lead action's refusal reads like. An older host answers "unknown method": say so, never pretend. */
function actionError(error: { code?: unknown; message?: string }): string {
  const text = error.message ?? '';
  if (isUnsupportedOp(error.code) || /unknown (method|op)/i.test(text)) return `This host can't do that yet (${text || 'unknown method'}). Update Unipi on the PC.`;
  return text || 'The host refused.';
}

type MenuState = { id: string; anchor: DOMRect };

/**
 * The agent canvas (OIN-77): every non-archived agent grouped by project, with filters, working agents
 * animated, and the picked agent's session in a side pane. One pure `layoutFleet` places nodes and edges;
 * this component only draws it, and pan/zoom writes touch nothing but the world's transform.
 */
export function CanvasView(props: { hostId?: string; sa: SuperagentState; onOpen: (sessionId: string) => void; onClose: () => void }) {
  const sa = props.sa;
  const hostId = () => props.hostId ?? '';
  const phone = () => !appLayout().desktop();
  const density = () => (phone() ? 'compact' as const : canvasDensity());
  const [size, setSize] = createSignal({ width: 0, height: 0 });
  const [bodyWidth, setBodyWidth] = createSignal(0);
  const [expanded, setExpanded] = createSignal<ReadonlySet<string>>(new Set<string>());
  const [selected, setSelected] = createSignal<string>();
  const [hovered, setHovered] = createSignal<string>();
  const [query, setQuery] = createSignal('');
  const [matchAt, setMatchAt] = createSignal(0);
  const [menu, setMenu] = createSignal<MenuState>();
  const [filtersOpen, setFiltersOpen] = createSignal(false);
  const [pickModel, setPickModel] = createSignal<string>();
  const [busy, setBusy] = createSignal<ReadonlySet<string>>(new Set<string>());
  const [errors, setErrors] = createSignal<Record<string, string>>({});

  const tree = createMemo(() => buildTree(sa.fleet()));
  const counts = createMemo(() => fleetCounts(sa.fleet().nodes));
  const filtered = createMemo(() => filterTree(tree(), canvasFilter()));
  /** Row width for the shelf packing: from the whole canvas (not the stage), so opening the peek never re-lays-out. */
  const wrap = () => (phone() ? Math.max(320, bodyWidth() || 390) : Math.min(2800, Math.max(1200, (bodyWidth() || 1200) / 0.7)));

  // The layout of the same fleet is reused while its `rev` and the inputs are unchanged: text still refreshes, geometry does not.
  let kept: { key: string; rev: string; layout: Layout } | undefined;
  const layout = createMemo<Layout>(() => {
    const t = tree(); const f = canvasFilter(); const d = density(); const ex = expanded(); const col = canvasCollapsed(); const w = Math.round(wrap());
    const rev = sa.fleet().rev ?? '';
    const key = JSON.stringify([f, d, [...ex], [...col], w]);
    if (rev && kept && kept.rev === rev && kept.key === key) return kept.layout;
    const next = layoutFleet(t, f, d, ex, w, col, filtered());
    kept = { key, rev, layout: next };
    return next;
  });
  const placed = createMemo(() => new Map(layout().nodes.map((n) => [n.id, n])));
  const nav = createMemo(() => navOf(layout()));
  const groupOf = createMemo(() => new Map(layout().groups.map((g) => [g.key, g])));
  const groupKeys = createMemo(() => layout().groups.map((g) => g.key), [], { equals: sameList });
  const edgeOf = createMemo(() => new Map<string, PlacedEdge>(layout().edges.map((e) => [e.id, e])));

  // ---- view -----------------------------------------------------------------------------------------
  const initialView = untrack(canvasView); // read once: reading it reactively recreated the stage on every pan
  const [view, setViewSignal] = createSignal<View>(initialView ?? RESET_VIEW);
  const [panning, setPanning] = createSignal(false);
  const [cull, setCull] = createSignal<Rect | null>(null);
  let root!: HTMLDivElement; let stage!: HTMLDivElement; let world!: HTMLDivElement; let search: HTMLInputElement | undefined;
  /** Fit until the owner moves the view themselves (and again when the filter or density changes). */
  let autoFit = initialView === null;
  const opener = document.activeElement as HTMLElement | null;
  const move = (next: View, manual = true) => { if (manual) autoFit = false; setViewSignal(next); setCanvasView(next); };
  const fit = () => move(fitView(layout().bounds, size()), false);
  const centre = () => ({ x: size().width / 2, y: size().height / 2 });
  const zoomBy = (factor: number, at = centre()) => move(zoomAt(view(), factor, at.x, at.y));
  /** Pan with a short ease (instant under reduced motion). */
  const glide = (next: View) => {
    if (!reduced()) { world.dataset.glide = ''; setTimeout(() => delete world.dataset.glide, 240); }
    move(next);
  };
  const reveal = (id: string, centreIt = false) => {
    const p = placed().get(id); if (!p || !stage.clientWidth) return;
    const vp = { width: stage.clientWidth, height: stage.clientHeight };
    const next = centreIt ? { ...view(), x: vp.width / 2 - (p.x + p.w / 2) * view().scale, y: vp.height / 2 - (p.y + p.h / 2) * view().scale } : panToReveal(view(), p, vp);
    if (next) glide(next);
  };
  const setFilter = (f: CanvasFilter) => { autoFit = true; setCanvasFilter(f); };
  const setDensity = (d: 'compact' | 'detailed') => { if (d !== canvasDensity()) { autoFit = true; setCanvasDensity(d); } };

  createEffect(() => {
    const v = view();
    if (!world) return;
    world.style.transform = `translate(${v.x}px, ${v.y}px) scale(${v.scale})`;
    // Dots and group names keep their screen size below the dots threshold (the world is scaled down, so scale them back up).
    world.style.setProperty('--inv', v.scale < LOD_DOTS ? String(1 / v.scale) : '1');
  });
  createEffect(() => { const l = layout(); const s = size(); if (autoFit && l.nodes.length && s.width) { autoFit = false; untrack(fit); } });
  const lod = createMemo(() => lodOf(view().scale));

  // Culling: above 150 nodes only what is near the viewport is drawn, recomputed once per animation frame.
  let cullFrame = 0;
  createEffect(() => {
    const v = view(); const s = size(); const big = layout().nodes.length > CULL_ABOVE;
    if (!big || !s.width) { setCull(null); return; }
    cancelAnimationFrame(cullFrame);
    cullFrame = requestAnimationFrame(() => setCull(worldRect(v, s)));
  });
  onCleanup(() => cancelAnimationFrame(cullFrame));
  const shown = createMemo<PlacedNode[]>(() => { const c = cull(); const l = layout().nodes; return c ? l.filter((n) => intersects(n, c)) : l; });
  const nodeIds = createMemo(() => shown().map((n) => n.id), [], { equals: sameList });
  const edgeIds = createMemo(() => { const c = cull(); return layout().edges.filter((e) => !c || intersects(e.box.w || e.box.h ? { x: e.box.x - 1, y: e.box.y - 1, w: e.box.w + 2, h: e.box.h + 2 } : e.box, c)).map((e) => e.id); }, [], { equals: sameList });

  // ---- freshness ------------------------------------------------------------------------------------------
  const stale = () => { const at = sa.updatedAt(); return at !== null && (sa.failedPolls() >= STALE_AFTER || sa.now() - at >= STALE_MS); };
  const loading = () => !sa.fleetLoaded() && !sa.fleetError();
  const firstError = () => (!sa.fleetLoaded() ? sa.fleetError() : undefined);
  /** At most 60 working nodes animate; the rest keep a static green ring. A stale canvas animates nothing: it has no evidence. */
  const animated = createMemo<ReadonlySet<string>>((prev) => {
    if (stale()) return prev && prev.size === 0 ? prev : new Set<string>();
    const ids = shown().filter((n) => !n.pseudo && tree().nodes.get(n.id)?.status === 'working').slice(0, ANIM_BUDGET).map((n) => n.id);
    return prev && prev.size === ids.length && ids.every((id) => prev.has(id)) ? prev : new Set(ids);
  });

  // ---- search -----------------------------------------------------------------------------------------------
  const matches = createMemo<ReadonlySet<string> | null>(() => (query().trim() ? searchMatches(tree(), query()) : null));
  const matchList = createMemo(() => { const m = matches(); return m ? layout().nodes.filter((n) => !n.pseudo && m.has(n.id)).map((n) => n.id) : []; });
  const current = () => (matches() ? matchList()[matchAt() % Math.max(1, matchList().length)] : undefined);
  createEffect(() => { query(); setMatchAt(-1); });
  const nextMatch = () => { const list = matchList(); if (!list.length) return; const i = (matchAt() + 1) % list.length; setMatchAt(i); focusNode(list[i]!, true); };

  // ---- selection and focus ------------------------------------------------------------------------------
  const selectedNode = () => { const id = selected(); return id ? tree().nodes.get(id) : undefined; };
  const tabbable = () => selected() ?? layout().nodes.find((n) => !n.pseudo)?.id;
  const focusNode = (id: string, centreIt = false) => {
    reveal(id, centreIt);
    requestAnimationFrame(() => requestAnimationFrame(() => root.querySelector<HTMLElement>(`[data-id="${CSS.escape(id)}"]`)?.focus({ preventScroll: true })));
  };
  const select = (id: string) => { setSelected(id); if (!phone()) requestAnimationFrame(() => requestAnimationFrame(() => reveal(id))); };
  const open = (id: string) => props.onOpen(openTarget(tree(), id));
  const toggleExpanded = (owner: string) => setExpanded((old) => { const next = new Set(old); if (next.has(owner)) next.delete(owner); else next.add(owner); return next; });
  const board = (key: string) => { void boardStore().open(hostId(), key); appNavigate('/board'); };

  const nodeKey = (e: KeyboardEvent, id: string) => {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); open(id); return; }
    const n = nav(); const p = n.parent.get(id);
    let to: string | undefined;
    if (e.key === 'ArrowUp') to = p ?? undefined;
    else if (e.key === 'ArrowDown') to = n.kids.get(id)?.[0];
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      const at = placed().get(id);
      const list = p ? n.kids.get(p) ?? [] : layout().nodes.filter((x) => !x.parent && x.group === at?.group).map((x) => x.id);
      const i = list.indexOf(id); to = list[i + (e.key === 'ArrowRight' ? 1 : -1)];
    } else return;
    e.preventDefault(); e.stopPropagation();
    if (to) focusNode(to);
  };

  // The peek opening shrinks the stage; a node that ends up under it is brought back (and nothing else moves).
  createEffect(() => { const id = selected(); if (id && !untrack(phone)) requestAnimationFrame(() => requestAnimationFrame(() => untrack(() => reveal(id)))); });
  // A node that left the fleet closes its own peek.
  createEffect(() => { const id = selected(); if (id && sa.fleetLoaded() && !tree().nodes.has(id)) setSelected(undefined); });

  // ---- lead actions ------------------------------------------------------------------------------------------
  async function act(id: string, action: LeadAction, model?: string) {
    if (busy().has(id)) return;
    if (action === 'stop' && !(await confirmSheet({ title: 'Stop this lead?', body: 'It stops working on its tickets.', confirm: 'Stop lead', tone: 'danger' }))) return;
    setBusy((old) => new Set(old).add(id)); setErrors((e) => ({ ...e, [id]: '' }));
    try {
      const r = await superagentApi.leadAction(hostId(), id, action, model);
      if (r.ok) { sa.flash(action === 'stop' ? 'Lead stopped.' : action === 'switch_model' ? 'Restarted on the new model.' : action === 'wake' ? 'Waking the lead.' : 'Retrying.'); await sa.refresh(); }
      else { setErrors((e) => ({ ...e, [id]: actionError(r.error) })); setSelected(id); }
    } catch (error) { setErrors((e) => ({ ...e, [id]: actionError(toAppError(error)) })); setSelected(id); }
    finally { setBusy((old) => { const next = new Set(old); next.delete(id); return next; }); }
  }

  // ---- context menu ---------------------------------------------------------------------------------------------
  type Row = { id: string; label: string; icon: typeof Eye; tone?: 'danger'; run: () => void };
  const menuRows = (id: string): Row[] => {
    const n = tree().nodes.get(id); if (!n) return [];
    const target = openTarget(tree(), id);
    const rows: Row[] = [];
    if (n.role === 'subagent' || n.role === 'worker') {
      rows.push({ id: 'parent', label: 'Open parent chat', icon: ExternalLink, run: () => open(id) }, { id: 'copy', label: 'Copy id', icon: Copy, run: () => { void copyText(id).then((ok) => showActionNotice(ok ? 'Copied the id' : 'Could not copy')); } });
      return rows;
    }
    rows.push({ id: 'open', label: 'Open chat', icon: ExternalLink, run: () => props.onOpen(target) }, { id: 'peek', label: 'Peek', icon: Eye, run: () => select(id) });
    if (n.role === 'lead') {
      rows.push({ id: 'message', label: 'Message lead', icon: MessageSquare, run: () => props.onOpen(target) });
      if (n.status === 'idle') rows.push({ id: 'wake', label: 'Wake', icon: Play, run: () => void act(id, 'wake') });
      if (n.status === 'stalled') rows.push({ id: 'retry', label: 'Retry', icon: RotateCcw, run: () => void act(id, 'retry') }, { id: 'switch', label: 'Switch model', icon: Shuffle, run: () => { select(id); setPickModel(id); } });
      if (n.status !== 'ended') rows.push({ id: 'stop', label: 'Stop', icon: Square, tone: 'danger', run: () => void act(id, 'stop') });
    }
    return rows;
  };
  const openMenu = (id: string, anchor: DOMRect) => {
    const n = tree().nodes.get(id);
    if (n?.role === 'chat') {
      const card = hubs().ensure(hostId()).state.cards[id];
      if (card) { openRowMenu(hostId(), card, anchor); return; }
    }
    setMenu({ id, anchor });
  };
  const sheetRows = (): ActionItem[][] => { const m = menu(); return m ? [menuRows(m.id).map((r) => ({ id: r.id, label: r.label, icon: r.icon, tone: r.tone, onSelect: r.run }) as ActionItem)] : []; };

  // ---- pointer: pan, pinch, wheel ---------------------------------------------------------------------------
  const pointers = new Map<number, { x: number; y: number }>();
  let gesture: { x: number; y: number; view: View; active: boolean } | undefined;
  let pinch: { d: number; view: View } | undefined;
  let dragged = false;
  const down = (e: PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('.sac-legend, .sac-zoom, .sac-footer, .sac-empty, .sac-banner, .cg-head button, .sac-chip')) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    dragged = false;
    if (pointers.size === 1) gesture = { x: e.clientX, y: e.clientY, view: view(), active: false };
    if (pointers.size === 2) { const [a, b] = [...pointers.values()] as [{ x: number; y: number }, { x: number; y: number }]; pinch = { d: Math.hypot(a.x - b.x, a.y - b.y) || 1, view: view() }; gesture = undefined; dragged = true; }
  };
  const dragMove = (e: PointerEvent) => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pinch && pointers.size >= 2) {
      const [a, b] = [...pointers.values()] as [{ x: number; y: number }, { x: number; y: number }];
      const rect = stage.getBoundingClientRect();
      const factor = Math.hypot(a.x - b.x, a.y - b.y) / pinch.d;
      move(zoomAt(pinch.view, factor, (a.x + b.x) / 2 - rect.left, (a.y + b.y) / 2 - rect.top));
      setPanning(true);
      return;
    }
    if (!gesture) return;
    // A press only becomes a pan after a few pixels, so a click on a node still reaches the node.
    if (!gesture.active) { if (Math.hypot(e.clientX - gesture.x, e.clientY - gesture.y) < DRAG_PX) return; gesture.active = true; dragged = true; stage.setPointerCapture?.(e.pointerId); setPanning(true); }
    move({ ...gesture.view, x: gesture.view.x + e.clientX - gesture.x, y: gesture.view.y + e.clientY - gesture.y });
  };
  const up = (e: PointerEvent) => {
    if (!pointers.delete(e.pointerId)) return;
    if (pointers.size < 2) pinch = undefined;
    if (pointers.size === 0) { gesture = undefined; setPanning(false); }
    stage.releasePointerCapture?.(e.pointerId);
  };
  /** A node's click, unless it ended a drag. */
  const clickOk = (e: MouseEvent) => { void e; const ok = !dragged; dragged = false; return ok; };

  onMount(() => {
    root.focus();
    const measure = () => { setSize((o) => (o.width === stage.clientWidth && o.height === stage.clientHeight ? o : { width: stage.clientWidth, height: stage.clientHeight })); setBodyWidth(stage.parentElement?.clientWidth ?? 0); };
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? undefined : new ResizeObserver(measure);
    observer?.observe(stage); if (stage.parentElement) observer?.observe(stage.parentElement);
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = stage.getBoundingClientRect();
      const lines = e.deltaMode === 1 ? 16 : 1;
      zoomBy(Math.exp(-e.deltaY * lines * (e.ctrlKey || e.metaKey ? 0.01 : 0.0015)), { x: e.clientX - rect.left, y: e.clientY - rect.top });
    };
    stage.addEventListener('wheel', wheel, { passive: false });
    onCleanup(() => { observer?.disconnect(); stage.removeEventListener('wheel', wheel); if (opener?.isConnected) opener.focus?.(); });
  });

  const worldKeys = (e: KeyboardEvent) => {
    if (e.target !== world || e.altKey || e.ctrlKey || e.metaKey) return;
    const v = view();
    const pan: Record<string, [number, number]> = { ArrowLeft: [PAN_STEP, 0], ArrowRight: [-PAN_STEP, 0], ArrowUp: [0, PAN_STEP], ArrowDown: [0, -PAN_STEP] };
    const by = pan[e.key];
    if (by) move({ ...v, x: v.x + by[0], y: v.y + by[1] });
    else if (e.key === '+' || e.key === '=') zoomBy(ZOOM_STEP);
    else if (e.key === '-') zoomBy(1 / ZOOM_STEP);
    else if (e.key === '0') move(RESET_VIEW);
    else if (e.key.toLowerCase() === 'f') fit();
    else return;
    e.preventDefault();
  };
  const keys = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault(); e.stopPropagation();
      if (selected()) setSelected(undefined); else props.onClose();
      return;
    }
    if (!e.altKey && !e.ctrlKey && !e.metaKey && !typing(e.target)) {
      const k = e.key.toLowerCase();
      if (e.key === '/') { e.preventDefault(); search?.focus(); return; }
      if (k === 'd' && !phone()) { e.preventDefault(); setDensity(canvasDensity() === 'detailed' ? 'compact' : 'detailed'); return; }
      if (k === 'n') { const id = nextNeedingYou(tree(), layout(), selected()); if (id) { e.preventDefault(); select(id); focusNode(id, true); } return; }
    }
    if (e.key !== 'Tab') return;
    const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !el.closest('[inert]'));
    const first = items[0]; const last = items[items.length - 1];
    if (!first || !last) return;
    if (e.shiftKey && (document.activeElement === first || document.activeElement === root)) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  };

  // ---- peek resize -------------------------------------------------------------------------------------------------
  const [dragW, setDragW] = createSignal<number>();
  const peekW = () => dragW() ?? canvasPeekWidth();
  const resizeDown = (e: PointerEvent & { currentTarget: HTMLElement }) => {
    e.preventDefault(); e.stopPropagation();
    const startX = e.clientX; const start = canvasPeekWidth(); const el = e.currentTarget; el.setPointerCapture(e.pointerId);
    const clampW = (n: number) => Math.min(PEEK_MAX, Math.max(PEEK_MIN, Math.round(n)));
    const moveTo = (ev: PointerEvent) => setDragW(clampW(start - (ev.clientX - startX)));
    const end = () => { el.removeEventListener('pointermove', moveTo); el.removeEventListener('pointerup', end); el.removeEventListener('pointercancel', end); const w = dragW(); if (w) setCanvasPeekWidth(w); setDragW(undefined); };
    el.addEventListener('pointermove', moveTo); el.addEventListener('pointerup', end); el.addEventListener('pointercancel', end);
  };

  // ---- derived words and sets ---------------------------------------------------------------------------------------
  const empty = () => tree().nodes.size === 0;
  const filterEmpty = () => !empty() && layout().matches === 0;
  const hl = createMemo<ReadonlySet<string> | undefined>(() => {
    const id = hovered(); if (!id) return undefined;
    const out = new Set<string>();
    for (let cur: string | null | undefined = id; cur; cur = tree().parent.get(cur)) out.add(cur);
    const p = placed().get(id); if (p?.pseudo && p.owner) out.add(p.owner);
    return out;
  });
  const ctx: NodeCtx = {
    tree, now: sa.now, lod, density, dimmed: () => filtered().dimmed, match: matches, current, animated, selected, tabbable,
    select, open, hover: setHovered, menu: openMenu, key: nodeKey, toggle: toggleExpanded, click: clickOk,
  };
  const groupsForMenu = () => tree().groups;
  const clearFilters = () => { batch(() => { setFilter({ ...DEFAULT_FILTER, statuses: [], hidden: [] }); setQuery(''); }); };
  const retry = () => void sa.refresh();
  const peekProps = (n: NonNullable<ReturnType<typeof selectedNode>>) => ({
    hostId: hostId(), sa, node: n, tree: tree(), busy: busy().has(n.id), error: errors()[n.id] || undefined,
    onAction: (action: LeadAction, model?: string) => void act(n.id, action, model), onOpen: () => open(n.id), onClose: () => setSelected(undefined),
  });

  return <Portal><div class="sac" role="dialog" aria-modal="true" aria-label="Agent canvas" tabIndex={-1} ref={root} onKeyDown={keys} data-testid="agent-canvas" data-stale={stale() ? '' : undefined}>
    <Toolbar phone={phone()} filter={canvasFilter} set={setFilter} counts={counts} groups={groupsForMenu} density={density} setDensity={setDensity} query={query} setQuery={setQuery} onEnter={nextMatch}
      fit={fit} onClose={props.onClose} openFilters={() => setFiltersOpen(true)} searchRef={(el) => (search = el)} />
    <div class="sac-body">
      <div class="sac-stage" ref={stage} data-panning={panning() ? '' : undefined} data-lod={lod()} onPointerDown={down} onPointerMove={dragMove} onPointerUp={up} onPointerCancel={up}>
        <div class="sac-world" ref={world} tabIndex={0} role="group" aria-label="Agents by project. Arrow keys pan, plus and minus zoom, F fits." onKeyDown={worldKeys}>
          <For each={groupKeys()}>{(key) => <GroupView g={() => groupOf().get(key)} lod={lod} toggle={toggleCanvasGroup} board={board} />}</For>
          <Edges ids={edgeIds} byId={edgeOf} width={layout().width} height={layout().height} hl={hl} animated={animated} />
          <For each={nodeIds()}>{(id) => <NodeView id={id} at={() => placed().get(id)} ctx={ctx} />}</For>
        </div>
        <Show when={loading()}><div class="sac-skel" aria-busy="true" role="status" aria-label="Loading agents"><i /><i /><i /></div></Show>
        <Show when={firstError()}>{(err) => <div class="sac-banner" role="alert" data-tone="error"><TriangleAlert size={16} aria-hidden="true" /><span>Couldn't load agents ({err().code})</span><button type="button" class="sac-btn" onClick={retry}>Retry</button></div>}</Show>
        <Show when={stale()}><div class="sac-banner sac-stale" role="status" data-tone="warn" data-testid="stale-chip"><TriangleAlert size={14} aria-hidden="true" /><span>Updated {clock(sa.now() - (sa.updatedAt() ?? sa.now()))} ago</span><button type="button" class="sac-link" onClick={retry}>Retry</button></div></Show>
        <Show when={empty() && sa.fleetLoaded()}><div class="sac-empty" role="status"><Bot size={48} aria-hidden="true" /><p>No agents yet</p><button type="button" class="sac-btn" onClick={props.onClose}>Back to Superagent</button></div></Show>
        <Show when={filterEmpty()}><div class="sac-empty" role="status"><p>Nothing matches</p><button type="button" class="sac-btn" onClick={clearFilters}>Clear filters</button></div></Show>
        <Show when={layout().hiddenGroups > 0 && filterActive(canvasFilter())}><div class="sac-footer" role="status"><span>{layout().hiddenGroups} project{layout().hiddenGroups === 1 ? '' : 's'} hidden</span><button type="button" class="sac-link" onClick={clearFilters}>Clear filters</button></div></Show>
        <Show when={!phone()}><ZoomDock scale={() => view().scale} zoom={zoomBy} /></Show>
        <Show when={!phone()}>
          <div class="sac-legend" aria-label="Legend">
            <div><For each={STATUSES}>{(s) => <span class="sac-legend-item" data-status={s}><i class="sac-legend-dot" />{STATUS_LABEL[s]}</span>}</For></div>
            <Show when={!tree().modern && !tree().hasEdges && !empty()}><p class="sac-note" data-testid="older-host-note">This host can't show subagents</p></Show>
          </div>
        </Show>
        <Show when={phone() && !tree().modern && !tree().hasEdges && !empty()}><p class="sac-note sac-note-phone" data-testid="older-host-note">This host can't show subagents</p></Show>
      </div>
      <Show when={!phone() && selectedNode()}>{(n) => <>
        <div class="sac-peek-resize" role="separator" aria-orientation="vertical" aria-label="Resize details" aria-valuemin={PEEK_MIN} aria-valuemax={PEEK_MAX} aria-valuenow={peekW()} tabIndex={0} data-dragging={dragW() ? '' : undefined}
          onPointerDown={resizeDown} onKeyDown={(e) => { const d = e.key === 'ArrowLeft' ? 16 : e.key === 'ArrowRight' ? -16 : 0; if (d) { e.preventDefault(); e.stopPropagation(); setCanvasPeekWidth(canvasPeekWidth() + d); } }} />
        <div class="sac-peek" style={{ width: `${peekW()}px` }}><FleetDetail {...peekProps(n())} /></div>
      </>}</Show>
    </div>
    <Show when={phone()}>
      <SheetFrame open={!!selectedNode()} onClose={() => setSelected(undefined)} label="Agent details" title={false} detents={['half', 'full']} initialDetent="half" testId="peek-sheet">
        <Show when={selectedNode()}>{(n) => <FleetDetail {...peekProps(n())} phone />}</Show>
      </SheetFrame>
      <SheetFrame open={filtersOpen()} onClose={() => setFiltersOpen(false)} label="Filters" title="Filters" testId="filters-sheet">
        <FilterPanel filter={canvasFilter} set={setFilter} counts={counts} groups={groupsForMenu} onClear={clearFilters} />
      </SheetFrame>
    </Show>
    <Show when={menu()}>{(m) => <Show when={!phone()} fallback={<ActionSheet open onClose={() => setMenu(undefined)} title={tree().nodes.get(m().id) ? nodeTitle(tree().nodes.get(m().id)!) : 'Agent'} sections={sheetRows()} />}>
      <AnchoredMenu anchor={m().anchor} label="Agent actions" width={220} onDismiss={() => setMenu(undefined)}>
        <For each={menuRows(m().id)}>{(r, i) => { const Icon = r.icon; return <>
          <Show when={r.tone === 'danger' && i() > 0}><MenuSeparator /></Show>
          <MenuRow icon={<Icon size={15} />} label={r.label} tone={r.tone} onSelect={() => { setMenu(undefined); r.run(); }} /></>; }}</For>
      </AnchoredMenu></Show>}</Show>
    <Show when={pickModel()}>{(id) => <HostModelDialog hostId={hostId()} title="Switch model" testId="menu-switch-model" onClose={() => setPickModel(undefined)} value={tree().nodes.get(id())?.model ?? ''} fallback="Lead model" onPick={(key) => { if (key) void act(id(), 'switch_model', key); }} />}</Show>
    <span class="sac-sr" aria-live="polite">{filtered().matches} of {tree().nodes.size} agents shown</span>
  </div></Portal>;
}
