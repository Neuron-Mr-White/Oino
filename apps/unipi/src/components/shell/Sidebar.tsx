/**
 * The desktop sidebar frame (§3.7, WP-A3): brand + collapse (Cmd/Ctrl+B),
 * one top nav group (New chat, Search, Board, Cron, extension pages —
 * [ui-shell] #2), the sessions area, and at the bottom the global host row
 * with the Settings gear beside it ([ui-shell] #1). Width 264 px
 * (220–420) via the resize handle on its right edge (AppShell).
 *
 * WP-A4: the sessions area is `components/sidebar/SessionList` — hosts →
 * projects → sessions with ordering, state words, badges, paging and the
 * Pinned group — the same component the phone Chat tab shows full screen.
 */
import { Show } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { PanelLeftClose, Search, SquarePen } from 'lucide-solid';
import crab from '../../assets/unicrab-pixel.png';
import { useHostsOptional } from '../../hosts/context';
import { can, isWeb } from '../../lib/platform';
import { ShellNeedsYouBell } from '../NeedsYouBell';
import { setNeedsYouOpen } from '../../lib/needsYouOpen';
import { layout } from '../../lib/layout';
import { modKey, setPaletteOpen } from '../../lib/shell';
import { SessionList } from '../sidebar/SessionList';
import '../sidebar/sidebar.css';
import { Rail, SettingsLink } from './Rail';
import { HostRow } from './HostSwitcher';

/** `/s/:host/:session` → its parts. */
export function routeSession(pathname: string): { hostId: string; sessionId: string } | undefined {
  const m = /^\/s\/([^/]+)\/([^/?#]+)/.exec(pathname);
  return m ? { hostId: decodeURIComponent(m[1]!), sessionId: decodeURIComponent(m[2]!) } : undefined;
}

/** `/new/:host` → the host of a draft. */
export function routeDraftHost(pathname: string): string | undefined {
  const m = /^\/new\/([^/?#]+)/.exec(pathname);
  return m ? decodeURIComponent(m[1]!) : undefined;
}

export const sessionHref = (hostId: string, sessionId: string): string => `/s/${encodeURIComponent(hostId)}/${encodeURIComponent(sessionId)}`;

/** The host a "New chat" starts on: the globally selected PC (sidebar host row — it follows the shown
 *  chat, see HostSwitcher), else the shown session's / draft's host, else This computer, else the first PC. */
export function useNewChatHost(): () => string {
  const location = useLocation();
  const hosts = useHostsOptional();
  return () => {
    const all = hosts?.hosts() ?? [];
    return all.find((h) => h.id === hosts?.selectedId())?.id ?? routeSession(location.pathname)?.hostId ?? routeDraftHost(location.pathname) ?? all.find((h) => h.transport === 'local')?.id ?? all[0]?.id ?? 'local';
  };
}

export function Sidebar() {
  const navigate = useNavigate();
  const newChatHost = useNewChatHost();
  return (
    <aside class="sb" aria-label="Sidebar" data-testid="sidebar">
      <div class="sb-top">
        <img src={crab} alt="" class="sb-crab" />
        <span class="sb-word">Unipi</span>
        <Show when={isWeb()}>
          <span class="sb-badge" title="Running in a browser: only the board is available">web</span>
        </Show>
        <Show when={can('hosts')}>
          <span class="sb-bell">
            <ShellNeedsYouBell onOpen={() => setNeedsYouOpen(true)} />
          </span>
        </Show>
        <button type="button" class="shell-icon-btn sb-collapse" aria-label="Hide sidebar" title={`Hide sidebar (${modKey()} B)`} onClick={() => layout().toggleSidebar(false)}>
          <PanelLeftClose size={17} />
        </button>
      </div>
      <nav class="sb-actions" aria-label="Pages">
        <button type="button" class="sb-action" onClick={() => navigate(`/new/${encodeURIComponent(newChatHost())}`)} data-testid="sb-new-chat">
          <SquarePen size={16} />
          <span>New chat</span>
        </button>
        <button type="button" class="sb-action" onClick={() => setPaletteOpen(true, 'sessions')} aria-label="Search sessions" data-testid="sb-search">
          <Search size={16} />
          <span>Search</span>
          <span class="sb-keys">
            <kbd class="kbd">{modKey()}</kbd>
            <kbd class="kbd">K</kbd>
          </span>
        </button>
        <Rail />
      </nav>
      <div class="sb-scroll">
        <SessionList variant="sidebar" />
      </div>
      <div class="sb-foot">
        <Show when={can('hosts')} fallback={<SettingsLink row />}>
          <HostRow />
          <SettingsLink />
        </Show>
      </div>
    </aside>
  );
}
