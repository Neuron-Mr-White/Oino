import { createSignal } from 'solid-js';
import { DEFAULT_FILTER, STATUSES, type CanvasFilter, type Density, type FleetStatus, type View } from './fleet';

/**
 * What the agent canvas remembers: the rail's collapsed state, the filter, the density, the
 * collapsed groups and the peek width (localStorage), the zoom/pan (sessionStorage + a module
 * signal, so "Back to canvas" lands where the owner left), and whether a session was opened
 * from the canvas.
 */
export const RAIL_KEY = 'unipi.superagent.railCollapsed';
export const FILTER_KEY = 'unipi.superagent.canvas.filter.v2';
export const DENSITY_KEY = 'unipi.superagent.canvas.density';
export const COLLAPSED_KEY = 'unipi.superagent.canvas.collapsed';
export const PEEK_WIDTH_KEY = 'unipi.superagent.canvas.peekWidth';
export const VIEW_KEY = 'unipi.superagent.canvas.view';
export const PEEK_MIN = 360;
export const PEEK_MAX = 720;
export const PEEK_DEFAULT = 440;

const read = (store: 'local' | 'session', key: string): string | null => {
  try { return (store === 'local' ? localStorage : sessionStorage).getItem(key); } catch { return null; }
};
const write = (store: 'local' | 'session', key: string, value: string) => {
  try { (store === 'local' ? localStorage : sessionStorage).setItem(key, value); } catch { /* Private storage: the choice lasts for this window. */ }
};
const json = <T>(key: string): T | null => { try { return JSON.parse(read('local', key) ?? 'null') as T | null; } catch { return null; } };

export const readRailCollapsed = () => read('local', RAIL_KEY) === '1';
export const writeRailCollapsed = (collapsed: boolean) => write('local', RAIL_KEY, collapsed ? '1' : '0');

function readFilter(): CanvasFilter {
  const v = json<Partial<CanvasFilter>>(FILTER_KEY);
  if (!v || typeof v !== 'object') return { ...DEFAULT_FILTER, statuses: [], hidden: [] };
  return {
    statuses: Array.isArray(v.statuses) ? v.statuses.filter((s): s is FleetStatus => STATUSES.includes(s as FleetStatus)) : [],
    chats: v.chats !== false,
    hidden: Array.isArray(v.hidden) ? v.hidden.filter((k): k is string => typeof k === 'string') : [],
  };
}
const readDensity = (): Density => (read('local', DENSITY_KEY) === 'compact' ? 'compact' : 'detailed');
function readCollapsed(): ReadonlySet<string> {
  const v = json<unknown>(COLLAPSED_KEY);
  return new Set(Array.isArray(v) ? v.filter((k): k is string => typeof k === 'string') : []);
}
const clampPeek = (n: number) => Math.min(PEEK_MAX, Math.max(PEEK_MIN, Math.round(n)));
function readPeek(): number {
  const n = Number(read('local', PEEK_WIDTH_KEY));
  return Number.isFinite(n) && n > 0 ? clampPeek(n) : PEEK_DEFAULT;
}
function readView(): View | null {
  try {
    const v = JSON.parse(read('session', VIEW_KEY) ?? 'null') as Partial<View> | null;
    return v && [v.x, v.y, v.scale].every((n) => typeof n === 'number' && Number.isFinite(n)) ? (v as View) : null;
  } catch { return null; }
}

const [filter, setFilterSignal] = createSignal<CanvasFilter>(readFilter());
const [density, setDensitySignal] = createSignal<Density>(readDensity());
const [collapsed, setCollapsedSignal] = createSignal<ReadonlySet<string>>(readCollapsed());
const [peekWidth, setPeekWidthSignal] = createSignal<number>(readPeek());
const [view, setViewSignal] = createSignal<View | null>(readView());
const [returnable, setReturnable] = createSignal(false);

export const canvasFilter = filter;
export const canvasDensity = density;
export const canvasCollapsed = collapsed;
export const canvasPeekWidth = peekWidth;
export const canvasView = view;
/** True after a session was opened from the canvas: the header offers "Back to canvas". */
export const canvasReturn = returnable;
export const setCanvasReturn = setReturnable;

export function setCanvasFilter(next: CanvasFilter) { setFilterSignal(next); write('local', FILTER_KEY, JSON.stringify(next)); }
export function setCanvasDensity(next: Density) { setDensitySignal(next); write('local', DENSITY_KEY, next); }
export function toggleCanvasGroup(key: string) {
  const next = new Set(collapsed());
  if (next.has(key)) next.delete(key); else next.add(key);
  setCollapsedSignal(next);
  write('local', COLLAPSED_KEY, JSON.stringify([...next]));
}
export function setCanvasPeekWidth(next: number) { const w = clampPeek(next); setPeekWidthSignal(w); write('local', PEEK_WIDTH_KEY, String(w)); }
export function setCanvasView(next: View | null) { setViewSignal(next); if (next) write('session', VIEW_KEY, JSON.stringify(next)); }

/** Tests: forget everything remembered. */
export function resetCanvasStore() {
  for (const key of [RAIL_KEY, FILTER_KEY, DENSITY_KEY, COLLAPSED_KEY, PEEK_WIDTH_KEY]) { try { localStorage.removeItem(key); } catch { /* ignore */ } }
  try { sessionStorage.removeItem(VIEW_KEY); } catch { /* ignore */ }
  setFilterSignal({ ...DEFAULT_FILTER, statuses: [], hidden: [] }); setDensitySignal('detailed'); setCollapsedSignal(new Set<string>()); setPeekWidthSignal(PEEK_DEFAULT); setViewSignal(null); setReturnable(false);
}
