/**
 * UNI-200: the native mascots' on/off switches — the Android overlay
 * (permission + service, window.UniPiOverlay) and the desktop mascot window
 * (`mascot_window` command). Used by Settings → Unicrab and MascotFeedRunner.
 */
import { createSignal } from 'solid-js';
import { z } from 'zod';
import { call } from './ipc';
import { overlayBridge } from './mascotFeed';
import { mascotWindowSize, setMascotWindowOn, setOverlayOn } from './mascot';

const [overlayRunning, setOverlayRunningSignal] = createSignal(false);
export { overlayRunning };
export const setOverlayRunning = (on: boolean) => setOverlayRunningSignal(on);

export type OverlayStep = 'unsupported' | 'needs-permission' | 'on' | 'off';

/** Turns the Android overlay on/off. Without "Display over other apps" it
 *  opens the system page and answers `needs-permission` (the switch stays
 *  off; Settings re-checks on resume). */
export function setOverlay(on: boolean): OverlayStep {
  const bridge = overlayBridge();
  if (!bridge?.start) return 'unsupported';
  if (!on) {
    bridge.stop?.();
    setOverlayOn(false);
    setOverlayRunning(false);
    return 'off';
  }
  if (!bridge.canDraw?.()) {
    bridge.requestPermission?.();
    return 'needs-permission';
  }
  bridge.start();
  setOverlayOn(true);
  setOverlayRunning(true);
  return 'on';
}

/** Desktop: shows/hides the always-on-top mascot window (UNI-255: sized for
 *  the chosen crab size; calling it again while shown resizes it). */
export async function syncMascotWindow(show: boolean): Promise<boolean> {
  setMascotWindowOn(show);
  const res = await call('mascot_window', { show, size: mascotWindowSize() }, z.boolean());
  // The window keeps its page: re-send the feed so the crab takes the new size.
  if (show && res.ok) void import('@tauri-apps/api/event').then(({ emit }) => emit('mascot-size-changed', {})).catch(() => undefined);
  return res.ok ? res.value : false;
}
