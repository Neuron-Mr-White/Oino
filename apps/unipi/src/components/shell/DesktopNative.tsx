/**
 * Desktop-app native wiring (UNI-117 part 2), mounted once by the shell:
 * - window title follows the page and the shown session
 *   (`windowTitle` cap: desktop app + web);
 * - the tray's status line ("2 agents need you · 1 working", `tray`).
 *
 * OS notifications are event-driven since WP-A19 (Z-84): `NeedsYouFeed`
 * routes hub `session_changed` cards through `lib/notifications/route` —
 * nothing polls here anymore. Renders nothing; on the phone every cap is
 * off, so it does nothing.
 */
import { createEffect, createMemo, onCleanup, onMount } from 'solid-js';
import { useLocation } from '@solidjs/router';
import { can } from '../../lib/platform';
import { hubs } from '../../lib/session/hub';
import { sessionTitle } from '../../lib/session/selectors';
import { needsYou } from '../../lib/needsYou';
import { useHosts } from '../../hosts/context';
import { runningAgents } from '../../lib/mascotLive';
import { setTrayStatus, setWindowTitle, windowTitle } from '../../lib/desktop';
import { routeSession } from './Sidebar';

/** Pure: the window title's page label for a route (WP-A3 routes). */
export function pageLabel(pathname: string): string | undefined {
  const first = pathname.split('/').filter(Boolean)[0] ?? '';
  const labels: Record<string, string> = { board: 'Board', cron: 'Cron', settings: 'Settings', setup: 'Setup', new: 'New chat', chat: 'Chat', files: 'Files', preview: 'Preview' };
  return labels[first];
}

export function DesktopNative() {
  const location = useLocation();
  const hosts = useHosts();
  const shown = createMemo(() => routeSession(location.pathname));

  createEffect(() => {
    if (!can('windowTitle')) return;
    const s = shown();
    const card = s ? hubs().peek(s.hostId)?.state.cards[s.sessionId] : undefined;
    const detail = s ? (card ? sessionTitle(card) : undefined) : undefined;
    setWindowTitle(windowTitle([detail, pageLabel(location.pathname)]));
  });

  // Tray status: agents that need you (unread) and agents working, across every PC.
  if (can('tray')) {
    createEffect(() => {
      const store = needsYou();
      const needs = store.list().filter((i) => i.group === 'waiting' && !store.isRead(i.id)).length;
      setTrayStatus(needs, runningAgents(hosts.hosts()));
    });
  }

  onMount(() => {
    if (!can('windowTitle')) return;
    onCleanup(() => setWindowTitle('Unipi'));
  });
  return null;
}
