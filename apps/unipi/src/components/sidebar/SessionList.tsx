/**
 * The sessions list (WP-A4): hosts → projects → sessions, the one component
 * the desktop sidebar (`variant="sidebar"`) and the phone Chat tab
 * (`variant="page"`, §3.9) share. This computer first, then the paired PCs
 * (Z-8; the PcChip stays only on Board/Cron). Rows: Z-12 content, ordering
 * Z-9/Z-10, paging Z-11, the Pinned group Z-14.
 */
import { createMemo, For, Show, type JSX } from 'solid-js';
import { useLocation } from '@solidjs/router';
import { useHostsOptional } from '../../hosts/context';
import { setAddHostOpen } from '../Pairing';
import { NoHostState } from '../EmptyState';
import type { ModeBadge } from './SessionRow';
import { HostGroup } from './HostGroup';
import type { RowVariant } from './SessionRow';

export function SessionList(props: { variant: RowVariant }): JSX.Element {
  const hosts = useHostsOptional();
  const location = useLocation();
  /** This computer first, then the paired PCs (the host list's pairing order, Z-8). */
  const list = createMemo(() => {
    const all = hosts?.hosts() ?? [];
    return [...all].sort((a, b) => Number(b.transport === 'local') - Number(a.transport === 'local'));
  });
  const noHosts = () => !!hosts && !hosts.loading() && hosts.hosts().length === 0;
  /** The shown conversation (row highlight; `/s/:host/:session`). */
  const activeSession = createMemo(() => {
    const m = /^\/s\/([^/]+)\/([^/?#]+)/.exec(location.pathname);
    return m ? { hostId: decodeURIComponent(m[1]!), sessionId: decodeURIComponent(m[2]!) } : undefined;
  });
  /**
   * The row's working-mode badge (Z-12 keeps unipi's): a seam — the card carries
   * no mode yet; A16's status pane (reserved `unipi.long-horizon/visualize`)
   * exposes the live mode, and this is where it plugs in.
   */
  const modeOf = (_key: { hostId: string; session_id: string }): ModeBadge | undefined => undefined;

  return (
    <div class="sb-sessions" data-variant={props.variant} data-testid="sidebar-sessions">
      <Show when={!noHosts()} fallback={<NoHostState what="its sessions" onAdd={() => setAddHostOpen(true)} />}>
        <For each={list()}>{(host) => <HostGroup host={host} variant={props.variant} activeSession={activeSession()} modeOf={modeOf} />}</For>
      </Show>
    </div>
  );
}
