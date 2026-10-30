/**
 * The desktop sidebar frame (§3.7, WP-A3): brand + collapse (Cmd/Ctrl+B),
 * New chat + search, the sessions area and the rail (Z-1). Width 264 px
 * (220–420) via the resize handle on its right edge (AppShell).
 *
 * The sessions area here is the minimal frame list (hosts → projects →
 * sessions, 5 per project, links to `/s/:host/:session`); WP-A4 replaces
 * `SidebarSessions` with the full list (ordering, state words, badges, menus,
 * show more, pinned) shared with the phone Chat tab.
 */
import { createMemo, For, Show } from 'solid-js';
import { A, useLocation, useNavigate } from '@solidjs/router';
import { Monitor, PanelLeftClose, Search, SquarePen } from 'lucide-solid';
import crab from '../../assets/unicrab-pixel.png';
import { useHostsOptional } from '../../hosts/context';
import { isWeb } from '../../lib/platform';
import { layout } from '../../lib/layout';
import { modKey, setPaletteOpen } from '../../lib/shell';
import { hubs } from '../../lib/session/hub';
import { projectsOf, sessionTitle } from '../../lib/session/selectors';
import { Rail } from './Rail';

export const SIDEBAR_ROWS_PER_PROJECT = 5;

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

function HostSessions(props: { hostId: string; name: string; local: boolean }) {
  const location = useLocation();
  const hub = hubs().ensure(props.hostId);
  const projects = createMemo(() => projectsOf(hub.state.cards, hub.state.meta));
  const current = () => routeSession(location.pathname);
  return (
    <section class="sb-host" data-testid={`sb-host-${props.hostId}`}>
      <div class="sb-host-head">
        <Monitor size={14} />
        <span class="sb-host-name">{props.local ? 'This computer' : props.name}</span>
      </div>
      <Show when={hub.state.loaded && projects().length === 0}>
        <p class="sb-empty">No sessions yet</p>
      </Show>
      <For each={projects()}>
        {(project) => (
          <div class="sb-project">
            <div class="sb-project-head" title={project.cwd}>
              {project.label || 'No folder'}
            </div>
            <For each={project.cards.slice(0, SIDEBAR_ROWS_PER_PROJECT)}>
              {(card) => (
                <A
                  href={sessionHref(props.hostId, card.session_id)}
                  class="sb-row"
                  aria-current={current()?.hostId === props.hostId && current()?.sessionId === card.session_id ? 'page' : undefined}
                  data-state={card.needs_you ? 'needs-you' : card.run_state}
                  title={sessionTitle(card)}
                >
                  <span class="sb-row-dot" aria-hidden="true" />
                  <span class="sb-row-title">{sessionTitle(card)}</span>
                </A>
              )}
            </For>
          </div>
        )}
      </For>
    </section>
  );
}

/** The sessions area (frame list until WP-A4). */
export function SidebarSessions() {
  const hosts = useHostsOptional();
  const list = createMemo(() => {
    const all = hosts?.hosts() ?? [];
    return [...all].sort((a, b) => Number(b.transport === 'local') - Number(a.transport === 'local'));
  });
  return (
    <div class="sb-sessions" data-testid="sidebar-sessions">
      <For each={list()}>{(h) => <HostSessions hostId={h.id} name={h.name} local={h.transport === 'local'} />}</For>
    </div>
  );
}

/** The host a "New chat" starts on: the shown session's / draft's host, else This computer, else the first PC. */
export function useNewChatHost(): () => string {
  const location = useLocation();
  const hosts = useHostsOptional();
  return () => {
    const all = hosts?.hosts() ?? [];
    return routeSession(location.pathname)?.hostId ?? routeDraftHost(location.pathname) ?? all.find((h) => h.transport === 'local')?.id ?? all[0]?.id ?? 'local';
  };
}

export function Sidebar() {
  const navigate = useNavigate();
  const newChatHost = useNewChatHost();
  return (
    <aside class="sb" aria-label="Sidebar" data-testid="sidebar">
      <div class="sb-top">
        <img src={crab} alt="" class="sb-crab" />
        <span class="sb-word">UniPi</span>
        <Show when={isWeb()}>
          <span class="sb-badge" title="Running in a browser: only the board is available">web</span>
        </Show>
        <button type="button" class="shell-icon-btn sb-collapse" aria-label="Hide sidebar" title={`Hide sidebar (${modKey()} B)`} onClick={() => layout().toggleSidebar(false)}>
          <PanelLeftClose size={17} />
        </button>
      </div>
      <div class="sb-actions">
        <button type="button" class="sb-action" onClick={() => navigate(`/new/${encodeURIComponent(newChatHost())}`)} data-testid="sb-new-chat">
          <SquarePen size={16} />
          <span>New chat</span>
        </button>
        <button type="button" class="sb-action" onClick={() => setPaletteOpen(true)} aria-label="Search" data-testid="sb-search">
          <Search size={16} />
          <span>Search</span>
          <span class="sb-keys">
            <kbd class="kbd">{modKey()}</kbd>
            <kbd class="kbd">K</kbd>
          </span>
        </button>
      </div>
      <div class="sb-scroll">
        <SidebarSessions />
      </div>
      <Rail />
    </aside>
  );
}
