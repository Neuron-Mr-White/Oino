/**
 * Cell units (ui-protocol §4.3): 1 column = `--ui-col` (0.5rem), 1 row =
 * `--ui-row` (1.25rem). One shared `ResizeObserver` (callbacks batched per
 * animation frame, §13.3) measures definite boxes; the shell signal tracks
 * `html[data-shell]` (§13.6 phone rules).
 */
import { createSignal, type Accessor } from 'solid-js';

const COL_REM = 0.5;
const ROW_REM = 1.25;

const [rootPx, setRootPx] = createSignal(16);
let rootWatched = false;

function readRoot(): number {
  if (typeof document === 'undefined') return 16;
  const px = parseFloat(getComputedStyle(document.documentElement).fontSize);
  return Number.isFinite(px) && px > 0 ? px : 16;
}

function watchRoot(): void {
  if (rootWatched || typeof window === 'undefined') return;
  rootWatched = true;
  setRootPx(readRoot());
  // The UI font size (Z-114) changes the root size; re-read on resize too.
  window.addEventListener('resize', () => setRootPx(readRoot()));
}

/** px per column. */
export function colPx(): number {
  watchRoot();
  return rootPx() * COL_REM;
}

/** px per row. */
export function rowPx(): number {
  watchRoot();
  return rootPx() * ROW_REM;
}

/** Re-reads the root font size (after a UI font-size change). */
export function refreshUnits(): void {
  setRootPx(readRoot());
}

export type Size = { width: number; height: number };
type Listener = (size: Size) => void;

const listeners = new Map<Element, Listener>();
const pending = new Map<Element, Size>();
let observer: ResizeObserver | undefined;
let frame = 0;

function flush(): void {
  frame = 0;
  const batch = [...pending];
  pending.clear();
  for (const [el, size] of batch) listeners.get(el)?.(size);
}

function ensureObserver(): ResizeObserver | undefined {
  if (observer) return observer;
  if (typeof ResizeObserver === 'undefined') return undefined;
  observer = new ResizeObserver((entries) => {
    for (const entry of entries) {
      const box = entry.contentRect;
      pending.set(entry.target, { width: box.width, height: box.height });
    }
    if (!frame) frame = requestAnimationFrame(flush);
  });
  return observer;
}

/** Observes `el`'s content box; returns the unobserve. No-op without ResizeObserver (jsdom). */
export function observeSize(el: Element, listener: Listener): () => void {
  const ro = ensureObserver();
  if (!ro) return () => {};
  listeners.set(el, listener);
  ro.observe(el);
  return () => {
    listeners.delete(el);
    pending.delete(el);
    ro.unobserve(el);
  };
}

// ── Shell (desktop / phone) ─────────────────────────────────────────────

const [shellSig, setShellSig] = createSignal<'desktop' | 'phone'>('phone');
let shellWatched = false;

function readShell(): 'desktop' | 'phone' {
  return typeof document !== 'undefined' && document.documentElement.dataset.shell === 'desktop' ? 'desktop' : 'phone';
}

/** `desktop` when `html[data-shell=desktop]`, else `phone` (§13.6 selector). */
export const shell: Accessor<'desktop' | 'phone'> = () => {
  if (!shellWatched && typeof document !== 'undefined') {
    shellWatched = true;
    setShellSig(readShell());
    if (typeof MutationObserver !== 'undefined')
      new MutationObserver(() => setShellSig(readShell())).observe(document.documentElement, { attributes: true, attributeFilter: ['data-shell'] });
  }
  return shellSig();
};

/** Re-reads `data-shell` synchronously (tests switch shells between renders). */
export function refreshShell(): void {
  setShellSig(readShell());
}
