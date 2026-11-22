/** Shared haptic feedback (toolbar keys, arrow pads). */
import { isTauri } from './platform';
export type HapticKind = 'tap' | 'tick';
let hapticWarned = false;

/**
 * Haptic feedback when the setting is on. Prefers the native bridge
 * (MainActivity's `UniPiHaptics`: View.performHapticFeedback, crisp and
 * follows the system haptics setting); falls back to the haptics
 * plugin (a 50 ms vibration).
 */
export async function hapticTick(enabled: boolean, kind: HapticKind = 'tap'): Promise<void> {
  if (!enabled) return;
  const bridge = (window as Window & { UniPiHaptics?: { tick: (kind: string) => void } }).UniPiHaptics;
  if (bridge) {
    try {
      bridge.tick(kind);
      return;
    } catch {
      // fall through to the plugin
    }
  }
  try {
    const haptics = await import('@tauri-apps/plugin-haptics');
    await haptics.impactFeedback(kind === 'tick' ? 'light' : 'medium');
  } catch (error) {
    if (!hapticWarned && isTauri()) {
      hapticWarned = true;
      console.warn('[haptics] unavailable', error);
    }
  }
}
