/**
 * UNI-161 §2/§3: mounted once in the shell (main.tsx) — pushes the
 * stored E2E key to the native bridge, listens for `unipi://chat` deep
 * links (notification taps while the app runs), and replays one pending
 * action queued while the app was closed.
 */
import { can, isTauri } from '../lib/platform';
import { createEffect, onMount, onCleanup } from 'solid-js';
import { useHosts } from '../hosts/context';
import { useSettings } from '../settings/context';
import { syncNotifyKeyToNative, openChatFromDeepLink } from '../lib/notifyInit';
import { sessions } from '../lib/session/store';
import { syncPushEndpoint } from '../lib/pushRegister';
import { handlePendingAction, hasLatchedPendingAction, startRunningNotification } from '../lib/runningNotification';
import { sessionUi } from '../ui/registries';
import { MascotFeedRunner } from './MascotFeedRunner';

export function NotifyInitRunner() {
  const hosts = useHosts();
  const [settings] = useSettings();

  // UNI-286: the settings load asynchronously (plugin-store / localStorage),
  // so a one-shot push at mount only ever saw the defaults: the stored E2E key
  // was never handed to native (so background UnifiedPush messages could not
  // be decrypted) or, worse, '' was pushed and wiped the key the native side
  // still had. Follow the key instead — but never push the pre-load empty
  // default, which is not "the user cleared it".
  let lastKey: string | undefined;
  createEffect(() => {
    const key = settings.notify.key ?? '';
    if (key === lastKey) return;
    lastKey = key;
    // An empty key is never pushed: the empty value is also what the app has
    // BEFORE the settings load (and after the user clears the key), and
    // wiping native's copy there would kill background delivery for good.
    // A key left behind is inert — nothing publishes to its endpoint.
    if (key) syncNotifyKeyToNative(key);
  });

  onMount(() => {
    // Background push: hand the UnifiedPush endpoint to every paired host
    // (now, on resume, and a little after a Register when it arrives).
    const syncPush = () => {
      if (settings.notify.enabled) void syncPushEndpoint(hosts.hosts().map((h) => h.id));
    };
    syncPush();
    const pushTimer = setInterval(syncPush, 30_000);
    const onVisible = () => document.visibilityState === 'visible' && syncPush();
    document.addEventListener('visibilitychange', onVisible);
    onCleanup(() => {
      clearInterval(pushTimer);
      document.removeEventListener('visibilitychange', onVisible);
    });

    // The endpoint also has to reach the hosts once the settings have loaded
    // (`enabled` is false until then), not only on the next 30 s tick.
    createEffect(() => {
      if (settings.notify.enabled) void syncPushEndpoint(hosts.hosts().map((h) => h.id));
    });

    // Ongoing "Running … · Stop" notification while a v2 session works.
    onCleanup(startRunningNotification(() => settings.notify.enabled));

    // Replay one pending notification action (Stop or ask_user/approval answer
    // tapped while the app was closed): latch on mount, then dispatch as soon
    // as the target v2 session stream is live and its dialogs arrive.
    handlePendingAction();
    createEffect(() => {
      const keys = sessions().keys();
      for (const key of keys) {
        const slash = key.indexOf('/');
        if (slash <= 0) continue;
        const hostId = key.slice(0, slash);
        const sessionId = key.slice(slash + 1);
        const handle = sessions().peek(hostId, sessionId);
        void handle?.state.conn;
        void handle?.state.run.running;
        void sessionUi(hostId, sessionId).dialogs().length;
      }
      if (hasLatchedPendingAction()) handlePendingAction();
    });

    if (!isTauri() || !can('deepLinks')) return;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const { getCurrent, onOpenUrl } = await import('@tauri-apps/plugin-deep-link');
        const current = await getCurrent();
        if (current?.[0]) openChatFromDeepLink(hosts.hosts(), current[0]);
        unlisten = await onOpenUrl((urls) => {
          if (urls[0]) openChatFromDeepLink(hosts.hosts(), urls[0]);
        });
      } catch {
        // deep-link plugin unavailable (desktop/web dev): no-op
      }
    })();
    onCleanup(() => unlisten?.());
  });

  // UNI-200: the Android overlay + desktop mascot window follow the in-app crab.
  return <MascotFeedRunner />;
}
