/**
 * App-wide idle guards (perf-app):
 * - `html[data-app-hidden]` while the document is hidden (minimized, other
 *   workspace, tray): style.css pauses every CSS animation under it, so a
 *   spinner left on screen costs nothing while nobody can see it;
 * - `window.__UNIPI_IDLE__()`: what still runs (shared clocks, running CSS
 *   animations, DOM size) for the perf harness (`scripts/perf/app.py`).
 */
import { clockStats } from './clock';

let installed = false;

export function installIdleGuards(): void {
  if (installed || typeof document === 'undefined') return;
  installed = true;
  const sync = () => {
    if (document.hidden) document.documentElement.setAttribute('data-app-hidden', '');
    else document.documentElement.removeAttribute('data-app-hidden');
  };
  sync();
  document.addEventListener('visibilitychange', sync);
  (window as unknown as { __UNIPI_IDLE__?: () => unknown }).__UNIPI_IDLE__ = () => {
    let anims = 0;
    try {
      anims = document.getAnimations().filter((a) => a.playState === 'running').length;
    } catch {
      anims = -1;
    }
    return { clocks: clockStats(), anims, dom: document.getElementsByTagName('*').length, hidden: document.hidden };
  };
}
