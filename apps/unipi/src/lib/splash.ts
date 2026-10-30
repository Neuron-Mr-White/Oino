/**
 * In-app animated splash (index.html `#splash`). The native Android
 * 12+ splash (dark bg + Unicrab, themes.xml) hands over to this HTML
 * splash on the WebView's first paint; it stays until the app is
 * ready (fonts + the first `list_hosts`, so Home never flashes
 * "No hosts yet") and at least `MIN_MS` since launch, then fades out.
 */
import { platform } from './platform';

export const SPLASH_MIN_MS = 900;
/** Desktop windows (≥ 900px, not the phone app): a short splash, keys pressed early aren't lost for long (UNI-218 F15). */
export const SPLASH_MIN_MS_DESKTOP = 250;

/** Pure: the minimum splash time for this platform/window. */
export function splashMin(mode: string, width: number): number {
  return mode !== 'tauri-mobile' && width >= 900 ? SPLASH_MIN_MS_DESKTOP : SPLASH_MIN_MS;
}
export const SPLASH_MAX_MS = 3000;
const FADE_MS = 320;

let ready = false;
let hidden = false;
let readyResolve: (() => void) | undefined;
const readyPromise = new Promise<void>((resolve) => {
  readyResolve = resolve;
});

/** Called once the first screen has real data. Idempotent. */
export function markAppReady(): void {
  if (ready) return;
  ready = true;
  readyResolve?.();
}

/** Remaining wait before the splash may hide. Pure, for tests. */
export function splashDelay(elapsedMs: number, minMs = SPLASH_MIN_MS): number {
  return Math.max(0, minMs - elapsedMs);
}

function reducedMotion(): boolean {
  return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

/** Waits for readiness (bounded), then fades the splash out. */
export async function dismissSplashWhenReady(): Promise<void> {
  const el = typeof document !== 'undefined' ? document.getElementById('splash') : null;
  if (!el || hidden) return;
  const fonts = (document as Document & { fonts?: { ready: Promise<unknown> } }).fonts?.ready ?? Promise.resolve();
  await Promise.race([
    Promise.all([readyPromise, fonts]),
    new Promise((resolve) => setTimeout(resolve, SPLASH_MAX_MS)),
  ]);
  const min = Math.min(reducedMotion() ? 300 : Infinity, splashMin(platform().mode, window.innerWidth));
  // Measured from when the splash first painted (index.html stamps it).
  const shownAt = (window as Window & { __splashShownAt?: number }).__splashShownAt ?? 0;
  await new Promise((resolve) => setTimeout(resolve, splashDelay(performance.now() - shownAt, min)));
  hidden = true;
  el.classList.add('splash-out');
  setTimeout(() => el.remove(), reducedMotion() ? 0 : FADE_MS);
}
