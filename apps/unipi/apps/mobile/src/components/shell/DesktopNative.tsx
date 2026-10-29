/**
 * Desktop-app native wiring (UNI-117 part 2), mounted once by the shell:
 * - window title follows the screen and the open chat / terminal / file
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
import { chatVisible, openChatState } from '../../lib/chat/open';
import { activeLive } from '../../lib/liveSessions';
import { getEditorTarget } from '../../lib/editorTarget';
import { needsYou } from '../../lib/needsYou';
import { chatLists, paneStatus } from '../../lib/chat/list';
import { useHosts } from '../../hosts/context';
import { pollNeedsYou } from '../../lib/needsYouPoll';

const NOTIFY_POLL_MS = 20_000;
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
    else if (location.pathname === '/term') detail = activeLive()?.target.label;
    else if (location.pathname === '/files/edit') detail = getEditorTarget()?.path.split('/').pop();
    setWindowTitle(windowTitle([detail, label]));
  });

  // Native notifications for new "needs you" items (seeded on first look: no burst at launch).
  // Its own poll: the bell's poll pauses while a chat/terminal covers it and while the window is
  // hidden — exactly when a notification matters.
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
    onMount(() => {
      const timer = setInterval(() => void pollNeedsYou(hosts.hosts()), NOTIFY_POLL_MS);
      onCleanup(() => clearInterval(timer));
    });
  }

  // Tray status: agents that need you (unread) and agents working, across every PC.
  if (can('tray')) {
    createEffect(() => {
      const store = needsYou();
      const needs = store.list().filter((i) => i.group === 'waiting' && !store.isRead(i.id)).length;
      let running = 0;
      for (const host of hosts.hosts()) {
        const data = chatLists().get(host.id)?.data;
        for (const s of data?.herdr.sessions ?? [])
          for (const w of s.workspaces) for (const t of w.tabs) for (const p of t.panes) if (p.agent && paneStatus(p) === 'working') running += 1;
      }
      setTrayStatus(needs, running);
    });
  }

  onMount(() => {
    if (!can('windowTitle')) return;
    onCleanup(() => setWindowTitle('UniPi'));
  });
  return null;
}
