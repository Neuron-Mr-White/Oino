/**
 * The phone's bottom tab bar (phone spec §1.1): Chats · Board · Cron · Files ·
 * Settings, fixed order, no "More". Board shows while its owner extension is
 * on (`unipi.kanboard/board` reserved gate, live); Files while the platform
 * can browse files. Tapping a tab restores that tab's last location; tapping
 * the active tab pops it to its root, or scrolls to top when already there.
 * Tab switches replace the route (no history, §1.3). The Chats tab carries
 * the needs-you count.
 */
import { createEffect, For, on, Show, type Component } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { CalendarClock, Folder, MessagesSquare, Settings as SettingsIcon, SquareKanban } from 'lucide-solid';
import { reservedGate } from '../../ui/slots/reserved';
import { can } from '../../lib/platform';
import { isTabRoot, TAB_ROOT, tabOf, type PhoneTab } from '../../lib/phoneNav';
import { needsYou } from '../../lib/needsYou';
import { haptic } from '../phone/services';

type TabDef = { id: PhoneTab; label: string; icon: Component<{ size?: number; strokeWidth?: number }>; when?: () => boolean };

export const TABS: readonly TabDef[] = [
  { id: 'chat', label: 'Chats', icon: MessagesSquare },
  { id: 'board', label: 'Board', icon: SquareKanban, when: () => reservedGate('unipi.kanboard/board') },
  { id: 'cron', label: 'Cron', icon: CalendarClock },
  { id: 'files', label: 'Files', icon: Folder, when: () => can('files') },
  { id: 'settings', label: 'Settings', icon: SettingsIcon },
];

/** Each tab's last location (pathname + search), restored when the tab is tapped. */
const lastLocation = new Map<PhoneTab, string>();

/** Pure: where tapping `tab` goes from `current` (its pathname + search). */
export function tabTarget(tab: PhoneTab, current: string, remembered: string | undefined): string | 'top' {
  const [path, search = ''] = current.split('?');
  const here = tabOf(path!);
  if (here === tab) return isTabRoot(path!, search ? `?${search}` : '') ? 'top' : TAB_ROOT[tab];
  // Workspaces (a chat, the editor) are not "a location of the tab" to come back to.
  return remembered ?? TAB_ROOT[tab];
}

/** Scrolls the visible page's scroller back to the top. */
function scrollToTop(): void {
  const scroller = document.querySelector<HTMLElement>('.ph-scroll, .app-content');
  scroller?.scrollTo({ top: 0, behavior: 'smooth' });
}

export function TabBar() {
  const location = useLocation();
  const navigate = useNavigate();
  createEffect(
    on([() => location.pathname, () => location.search], ([path, search]) => {
      const tab = tabOf(path);
      // Remember list locations only (a chat or a draft is reopened from the list, not restored).
      if (tab && !path.startsWith('/s/') && !path.startsWith('/new/') && path !== '/files/edit') lastLocation.set(tab, `${path}${search}`);
    }),
  );
  const active = () => tabOf(location.pathname);
  const count = () => needsYou().unreadCount();
  const go = (tab: PhoneTab) => {
    const target = tabTarget(tab, `${location.pathname}${location.search}`, lastLocation.get(tab));
    if (target === 'top') return scrollToTop();
    if (active() !== tab) haptic('selection');
    if (active() === tab) lastLocation.delete(tab);
    navigate(target, { replace: true });
  };
  return (
    <nav class="bottom-nav ph-tabbar" aria-label="Main navigation" data-testid="tab-bar">
      <For each={TABS.filter((t) => !t.when || t.when())}>
        {(tab) => {
          const Icon = tab.icon;
          return (
            <a
              href={`#${TAB_ROOT[tab.id]}`}
              aria-current={active() === tab.id ? 'page' : undefined}
              data-testid={`tab-${tab.id}`}
              onClick={(e) => {
                e.preventDefault();
                go(tab.id);
              }}
            >
              <span class="ph-tab-icon">
                <Icon size={24} strokeWidth={active() === tab.id ? 2.1 : 1.6} />
                <Show when={tab.id === 'chat' && count() > 0}>
                  <span class="ph-tab-badge" aria-label={`${count()} need you`}>
                    {count() > 9 ? '9+' : count()}
                  </span>
                </Show>
              </span>
              <span>{tab.label}</span>
            </a>
          );
        }}
      </For>
    </nav>
  );
}
