/**
 * Phone shell services (phone spec §2.10, §2.11, §2.12): the one keyboard
 * variable `--kb`, haptics that follow the Haptics setting, the tab bar's
 * hide reasons and the open-sheet count (FAB / floating crab hide under sheets).
 *
 * Everything here is a module-level singleton (created lazily in its own root)
 * so B and C can call it from anywhere without a provider.
 */
import { createRoot, createSignal, type Accessor } from 'solid-js';
import { hapticTick, type HapticKind } from '../../lib/haptics';
import { isTauriMobile } from '../../lib/platform';

// ── Keyboard (--kb) ───────────────────────────────────────────────────────

const kb = createRoot(() => {
  const [height, setHeight] = createSignal(0);
  return { height, setHeight };
});

/**
 * Pure: how much of the soft keyboard still overlaps the layout viewport.
 * `ime` is the native IME inset (MainActivity, 0 in a browser), `fullHeight`
 * the tallest innerHeight seen in this orientation, `innerHeight` the current
 * one (Android may have resized the WebView by part of the keyboard already),
 * `panned` the visual-viewport pan a browser reports instead.
 */
export function keyboardOverlap(ime: number, fullHeight: number, innerHeight: number, panned: number): number {
  const resized = Math.max(0, fullHeight - innerHeight);
  const native = Math.max(0, ime - resized);
  const px = Math.max(native, panned);
  // Sub-60 px "keyboards" are toolbars / rounding noise (a pinned suggestion strip is ≥ 40 but
  // never alone); a real IME is always taller.
  return px < 60 ? 0 : Math.round(px);
}

/** The keyboard as the phone frame sees it (§2.10). */
export function useKeyboard(): { height: Accessor<number>; open: Accessor<boolean> } {
  return { height: kb.height, open: () => kb.height() > 0 };
}

let tracking = false;
/**
 * Starts the one keyboard tracker (main.tsx, once). Sets `--kb`,
 * `html[data-kb=open]`, and keeps the legacy `--vvh` / `--vv-bottom` aliases
 * for CSS that has not migrated yet.
 */
export function startKeyboardTracking(): void {
  if (tracking || typeof window === 'undefined') return;
  tracking = true;
  // Tallest innerHeight per orientation (portrait/landscape differ).
  const tallest = new Map<string, number>();
  const set = () => {
    const vv = window.visualViewport;
    const ime = (window as unknown as { unipiImeInset?: number }).unipiImeInset ?? 0;
    const key = window.innerWidth > window.innerHeight ? 'land' : 'port';
    // A window resized while the keyboard is down is a real resize (split screen, desktop window):
    // only grow the baseline, or reset it while no keyboard is reported.
    const prev = tallest.get(key) ?? 0;
    const full = ime > 0 ? Math.max(prev, window.innerHeight) : window.innerHeight;
    tallest.set(key, full);
    const panned = vv ? Math.max(0, window.innerHeight - (vv.offsetTop + vv.height)) : 0;
    const px = keyboardOverlap(ime, full, window.innerHeight, panned);
    kb.setHeight(px);
    const root = document.documentElement;
    root.style.setProperty('--kb', `${px}px`);
    if (px > 0) root.dataset.kb = 'open';
    else delete root.dataset.kb;
    // Legacy aliases (sheets, composer) until B/C migrate to --kb.
    const h = Math.min(vv?.height ?? window.innerHeight, window.innerHeight - Math.max(0, ime - Math.max(0, full - window.innerHeight)));
    root.style.setProperty('--vvh', `${Math.round(h)}px`);
    root.style.setProperty('--vv-bottom', `${Math.round(Math.max(panned, ime))}px`);
    // Keep a focused field in view once the frame has resized (§2.10).
    const active = document.activeElement;
    if (px > 0 && active instanceof HTMLElement && active.matches('input, textarea, [contenteditable=true]')) {
      requestAnimationFrame(() => active.scrollIntoView({ block: active.closest('.sheet') ? 'center' : 'nearest' }));
    }
  };
  set();
  window.visualViewport?.addEventListener('resize', set);
  window.visualViewport?.addEventListener('scroll', set);
  window.addEventListener('resize', set);
  window.addEventListener('unipi-ime', set);
}

// ── Haptics ──────────────────────────────────────────────────────────────

let hapticsOn = true;
/** The shell mirrors Settings › Input › Haptics here (PhoneServices in AppShell). */
export function setHapticsEnabled(on: boolean): void {
  hapticsOn = on;
}

/** Pure: our four semantic kinds → the bridge's two (tick = light, tap = firm). */
export function hapticKind(kind: 'selection' | 'impact' | 'success' | 'warning'): HapticKind {
  return kind === 'selection' ? 'tick' : 'tap';
}

/**
 * Semantic haptics (§2.11): `selection` on tab/segment/picker change and
 * detent snap; `impact` on long-press menus / swipe threshold; `success` on
 * send / pairing; `warning` before a destructive confirm. Phone app only.
 */
export function haptic(kind: 'selection' | 'impact' | 'success' | 'warning'): void {
  if (!isTauriMobile()) return;
  void hapticTick(hapticsOn, hapticKind(kind));
}

// ── Tab bar + sheets ─────────────────────────────────────────────────────

const chrome = createRoot(() => {
  const [hidden, setHidden] = createSignal<ReadonlySet<string>>(new Set());
  const [sheets, setSheets] = createSignal(0);
  return { hidden, setHidden, sheets, setSheets };
});

/** State-driven workspace layers (task detail, viewers) hide the tab bar while they are up. */
export function setTabBarHidden(reason: string, hidden: boolean): void {
  chrome.setHidden((prev) => {
    if (prev.has(reason) === hidden) return prev;
    const next = new Set(prev);
    if (hidden) next.add(reason);
    else next.delete(reason);
    return next;
  });
}

/** Any reason holding the tab bar hidden right now? */
export function tabBarHiddenByLayer(): boolean {
  return chrome.hidden().size > 0;
}

/** Sheets register while open (FAB, floating crab and toasts' anchor read it). */
export function trackSheetOpen(): () => void {
  chrome.setSheets((n) => n + 1);
  if (typeof document !== 'undefined') document.documentElement.dataset.sheet = 'open';
  let done = false;
  return () => {
    if (done) return;
    done = true;
    chrome.setSheets((n) => {
      const next = Math.max(0, n - 1);
      if (next === 0 && typeof document !== 'undefined') delete document.documentElement.dataset.sheet;
      return next;
    });
  };
}

export function sheetOpen(): boolean {
  return chrome.sheets() > 0;
}
