/**
 * UNI-200: mounted once (by NotifyInitRunner) — keeps the Android overlay
 * and the desktop mascot window in step with the in-app crab: state,
 * unread count and one bubble per fresh "needs you"/finished item. Also
 * restores the overlay / mascot window the user turned on, and follows the
 * tray's "Mascot window" toggle. Renders nothing.
 */
import { createEffect, onCleanup, onMount } from 'solid-js';
import { useHosts } from '../hosts/context';
import { useSettings } from '../settings/context';
import { needsYou } from '../lib/needsYou';
import { mascotLive } from '../lib/mascotLive';
import { freshBubbles, mascotWindowOn, overlayOn, setMascotWindowOn, type MascotFeed } from '../lib/mascot';
import { buildFeed, overlayBridge, overlayConfig } from '../lib/mascotFeed';
import { setOverlayRunning, syncMascotWindow } from '../lib/mascotNative';
import { can, isTauri } from '../lib/platform';
import { openChatFromDeepLink } from '../lib/notifyInit';
import { call } from '../lib/ipc';
import { z } from 'zod';
import { setNeedsYouOpen } from './FloatingBell';

export function MascotFeedRunner() {
  const hosts = useHosts();
  const [settings] = useSettings();
  const live = mascotLive(() => hosts.hosts());
  const store = needsYou();

  onMount(() => {
    const overlay = overlayBridge();
    if (overlay && overlayOn() && overlay.canDraw?.()) {
      overlay.start?.();
      setOverlayRunning(true);
    }
    if (can('tray') && mascotWindowOn()) void syncMascotWindow(true);
  });

  createEffect(() => {
    const cfg = overlayConfig(settings.notify);
    overlayBridge()?.configure?.(JSON.stringify(cfg ?? {}));
  });

  let seen: Set<string> | undefined;
  createEffect(() => {
    const items = store.list();
    const fresh = freshBubbles(seen, items, store.isRead);
    seen = new Set(items.map((i) => i.id));
    send(buildFeed(live.state(), live.count(), fresh[fresh.length - 1]));
  });

  // Desktop: the mascot window's clicks and the tray's toggle.
  onMount(() => {
    if (!isTauri() || !can('tray')) return;
    const offs: (() => void)[] = [];
    void (async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        offs.push(
          await listen<{ link?: string }>('mascot-open', (e) => {
            void call('mascot_focus_main', {}, z.null());
            if (e.payload.link) openChatFromDeepLink(hosts.hosts(), e.payload.link);
            else setNeedsYouOpen(true);
          }),
          await listen<boolean>('mascot-window-changed', (e) => setMascotWindowOn(!!e.payload)),
          await listen('mascot-ready', () => send(buildFeed(live.state(), live.count(), undefined))),
          // UNI-255: Settings changed the crab size — tell the window at once.
          await listen('mascot-size-changed', () => send(buildFeed(live.state(), live.count(), undefined))),
        );
      } catch {
        // No Tauri IPC (tests, plain web): there is no mascot window to listen to.
      }
    })();
    onCleanup(() => offs.forEach((off) => off()));
  });

  return null;
}

function send(feed: MascotFeed): void {
  overlayBridge()?.update?.(JSON.stringify(feed));
  if (isTauri() && can('tray') && mascotWindowOn()) {
    void import('@tauri-apps/api/event').then(({ emitTo }) => emitTo('mascot', 'mascot-feed', feed)).catch(() => undefined);
  }
}
