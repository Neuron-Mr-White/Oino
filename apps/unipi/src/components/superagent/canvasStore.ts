import { createSignal } from 'solid-js';
import { DEFAULT_FILTER, FILTERS, type FleetFilter, type View } from './fleet';

/**
 * What the agent canvas remembers: the rail's collapsed state, the filter and the
 * Display choices (localStorage), the zoom/pan (sessionStorage + a module signal, so
 * "Back to canvas" lands where the owner left), and whether a session was opened from
 * the canvas.
 */
export const RAIL_KEY = 'unipi.superagent.railCollapsed';
export const FILTER_KEY = 'unipi.superagent.canvas.filter';
export const DISPLAY_KEY = 'unipi.superagent.canvas.display';
export const VIEW_KEY = 'unipi.superagent.canvas.view';

const read = (store: 'local' | 'session', key: string): string | null => {
  try { return (store === 'local' ? localStorage : sessionStorage).getItem(key); } catch { return null; }
};
const write = (store: 'local' | 'session', key: string, value: string) => {
  try { (store === 'local' ? localStorage : sessionStorage).setItem(key, value); } catch { /* Private storage: the choice lasts for this window. */ }
};

export const readRailCollapsed = () => read('local', RAIL_KEY) === '1';
export const writeRailCollapsed = (collapsed: boolean) => write('local', RAIL_KEY, collapsed ? '1' : '0');

export type CanvasDisplay = { name: boolean; model: boolean; status: boolean };
export const DEFAULT_DISPLAY: CanvasDisplay = { name: true, model: true, status: true };

function readDisplay(): CanvasDisplay {
  try {
    const v = JSON.parse(read('local', DISPLAY_KEY) ?? 'null') as Partial<CanvasDisplay> | null;
    return { name: v?.name !== false, model: v?.model !== false, status: v?.status !== false };
  } catch { return { ...DEFAULT_DISPLAY }; }
}
function readFilter(): FleetFilter {
  const v = read('local', FILTER_KEY);
  return FILTERS.includes(v as FleetFilter) ? (v as FleetFilter) : DEFAULT_FILTER;
}
function readView(): View | null {
  try {
    const v = JSON.parse(read('session', VIEW_KEY) ?? 'null') as Partial<View> | null;
    return v && [v.x, v.y, v.scale].every((n) => typeof n === 'number' && Number.isFinite(n)) ? (v as View) : null;
  } catch { return null; }
}

const [filter, setFilterSignal] = createSignal<FleetFilter>(readFilter());
const [display, setDisplaySignal] = createSignal<CanvasDisplay>(readDisplay());
const [view, setViewSignal] = createSignal<View | null>(readView());
const [returnable, setReturnable] = createSignal(false);

export const canvasFilter = filter;
export const canvasDisplay = display;
export const canvasView = view;
/** True after a session was opened from the canvas: the header offers "Back to canvas". */
export const canvasReturn = returnable;
export const setCanvasReturn = setReturnable;

export function setCanvasFilter(next: FleetFilter) { setFilterSignal(next); write('local', FILTER_KEY, next); }
export function setCanvasDisplay(next: CanvasDisplay) { setDisplaySignal(next); write('local', DISPLAY_KEY, JSON.stringify(next)); }
export function setCanvasView(next: View | null) { setViewSignal(next); if (next) write('session', VIEW_KEY, JSON.stringify(next)); }

/** Tests: forget everything remembered. */
export function resetCanvasStore() {
  for (const key of [RAIL_KEY, FILTER_KEY, DISPLAY_KEY]) { try { localStorage.removeItem(key); } catch { /* ignore */ } }
  try { sessionStorage.removeItem(VIEW_KEY); } catch { /* ignore */ }
  setFilterSignal(DEFAULT_FILTER); setDisplaySignal({ ...DEFAULT_DISPLAY }); setViewSignal(null); setReturnable(false);
}
