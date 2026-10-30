/**
 * UNI-200: hands the mascot's state + bubbles to the two native mascots
 * outside the page — the Android "over other apps" overlay
 * (window.UniPiOverlay → MascotOverlayService.kt) and the desktop
 * always-on-top mascot window (Tauri event `mascot-feed` → mascotWindow.tsx).
 * Both draw the same rig from the same JSON; this only says what to show.
 */
import type { NeedsYouItem } from './needsYou';
import type { MascotState } from './unicrab';
import { bubbleText, chatLink, mascotWindowSize, type MascotFeed } from './mascot';
import type { NotifySettings } from '../settings/schema';

export type OverlayBridge = {
  /** Settings.canDrawOverlays — "Display over other apps" granted. */
  canDraw?: () => boolean;
  /** Opens the system "Display over other apps" page for UniPi. */
  requestPermission?: () => void;
  /** Starts (or keeps) the foreground service + overlay window. */
  start?: () => void;
  stop?: () => void;
  running?: () => boolean;
  /** JSON MascotFeed. */
  update?: (json: string) => void;
  /** JSON {serverUrl, topic, token} — so the service can poll ntfy while the app is backgrounded (the E2E key is already native: UniPiNotify.setKey). */
  configure?: (json: string) => void;
};

export function overlayBridge(): OverlayBridge | undefined {
  return typeof window === 'undefined' ? undefined : (window as unknown as { UniPiOverlay?: OverlayBridge }).UniPiOverlay;
}

/** The feed for the current state; `bubbleItem` (a fresh item) rides along once. */
export function buildFeed(state: MascotState, count: number, bubbleItem: NeedsYouItem | undefined): MascotFeed {
  return {
    state,
    count,
    bubble: bubbleItem ? { id: bubbleItem.id, text: `${bubbleItem.title}: ${bubbleText(bubbleItem)}`, link: chatLink(bubbleItem) } : null,
    size: mascotWindowSize(),
  };
}

/** What the overlay service needs to poll ntfy itself (never the key — that's already native). */
export function overlayConfig(n: Pick<NotifySettings, 'enabled' | 'serverUrl' | 'topic' | 'token'>): { serverUrl: string; topic: string; token: string } | null {
  if (!n.enabled || !n.serverUrl || !n.topic) return null;
  return { serverUrl: n.serverUrl.replace(/\/$/, ''), topic: n.topic, token: n.token ?? '' };
}
