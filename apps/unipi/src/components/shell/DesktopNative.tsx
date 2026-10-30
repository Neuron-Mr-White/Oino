/**
 * Desktop-app native wiring (UNI-117 part 2), mounted once by the shell:
 * - window title follows the screen and the open chat / file
 *   (`windowTitle` cap: desktop app + web);
 * - native OS notifications for new "needs you" / finished items while the
 *   window isn't focused (`nativeNotifications`, desktop app only; the
 *   phone keeps its ntfy/UnifiedPush path);
 * - the tray's status line ("2 agents need you · 1 working", `tray`).
 * Renders nothing. On the phone every cap is off, so it does nothing.
 */
import { createEffect, createMemo, onCleanup, onMount } from 'solid-js';
import { useLocation } from '@solidjs/router';
import { can } from '../../lib/platform';
import { screenOf } from '../../lib/shell';
import { chatVisible, openChatState } from '../../lib/session/open';
import { getEditorTarget } from '../../lib/editorTarget';
import { needsYou } from '../../lib/needsYou';
import { useHosts } from '../../hosts/context';
import { runningAgents } from '../../lib/mascotLive';
import { desktopNotifyOn, nativeNotify, notifyPlan, notifyText, setTrayStatus, setWindowTitle, windowTitle } from '../../lib/desktop';
import { NAV } from './DesktopShell';

export function DesktopNative() {
  const location = useLocation();
  const hosts = useHosts();
  const screen = createMemo(() => screenOf(location.pathname));

  createEffect(() => {
    if (!can('windowTitle')) return;
    const label = NAV.find((n) => n.screen === screen())?.label;
    let detail: string | undefined;
    const chat = openChatState();
    if (screen() === 'chat' && chatVisible() && chat) detail = chat.chat.state.session?.name || chat.target.title;
    else if (location.pathname === '/files/edit') detail = getEditorTarget()?.path.split('/').pop();
    setWindowTitle(windowTitle([detail, label]));
  });

  // Native notifications for new "needs you" items (seeded on first look: no burst at launch).
  // The items come from the needs-you store (hub events after WP-A19); no polling here (Z-84).
  if (can('nativeNotifications')) {
    let seen: Set<string> | undefined;
    createEffect(() => {
      const store = needsYou();
      const items = store.list();
      const fresh = notifyPlan(seen, items, { focused: document.hasFocus() && !document.hidden, isRead: store.isRead });
      seen = new Set(items.map((i) => i.id));
      if (!desktopNotifyOn()) return;
      for (const item of fresh.slice(0, 3)) {
        const { title, body } = notifyText(item);
        void nativeNotify(title, body);
      }
    });
  }

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
    onCleanup(() => setWindowTitle('UniPi'));
  });
  return null;
}
