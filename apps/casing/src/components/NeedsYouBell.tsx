/**
 * The "needs you" bell + drawer (UNI-161 §3): a bell in the app header,
 * all screens, with an unread badge; tapping it opens a bottom sheet
 * grouped Waiting / Finished / Other. Tapping an item opens that chat;
 * swipe/✕ dismisses one, header actions mark-all-read / clear everything.
 */
import { createMemo, For, Show } from 'solid-js';
import { X } from 'lucide-solid';
import { Unicrab } from './Unicrab';
import { mascotLive } from '../lib/mascotLive';
import './mascot.css';
import { BoardSheet } from './board/BoardSheet';
import { groupNeedsYou, needsYou, type NeedsYouGroup, type NeedsYouItem } from '../lib/needsYou';
import { openChat } from '../lib/chat/open';
import { useHosts, useHostsOptional } from '../hosts/context';
import './needsYou.css';

const GROUP_LABEL: Record<NeedsYouGroup, string> = { waiting: 'Waiting', finished: 'Finished', other: 'Other' };
const GROUP_ORDER: NeedsYouGroup[] = ['waiting', 'finished', 'other'];

function relativeTime(ms: number, now = Date.now()): string {
  const diff = Math.max(0, now - ms);
  if (diff < 60_000) return 'just now';
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function Row(props: { item: NeedsYouItem; onOpen: () => void; onDismiss: () => void }) {
  return (
    <div class="ny-row" data-read={needsYou().isRead(props.item.id) ? '' : undefined}>
      <button type="button" class="ny-row-main" onClick={props.onOpen}>
        <span class="ny-row-top">
          <span class="ny-row-host">{props.item.hostName}</span>
          <span class="ny-row-time">{relativeTime(props.item.since)}</span>
        </span>
        <span class="ny-row-title">{props.item.title}</span>
        <Show when={props.item.what}>
          <span class="ny-row-what truncate">{props.item.what}</span>
        </Show>
      </button>
      <button type="button" class="ny-row-x" aria-label="Dismiss" onClick={props.onDismiss}>
        <X size={16} />
      </button>
    </div>
  );
}

export function NeedsYouDrawer(props: { open: boolean; onClose: () => void }) {
  const hosts = useHosts();
  const store = needsYou();
  const grouped = createMemo(() => groupNeedsYou(store.list()));

  const open = (item: NeedsYouItem) => {
    store.markRead(item.id);
    const host = hosts.hosts().find((h) => h.id === item.hostId);
    if (item.pid === null || !host) {
      props.onClose();
      return;
    }
    openChat(
      { hostId: item.hostId, session: item.session, workspaceId: '', workspaceLabel: item.session, tabId: '', paneId: '', pid: item.pid, title: item.title, status: 'unknown', focused: false },
      host.name,
    );
    props.onClose();
  };

  return (
    <BoardSheet
      open={props.open}
      onClose={props.onClose}
      label="Needs you"
      headerAction={
        <div class="ny-actions">
          <button type="button" class="ny-action btn btn-ghost btn-sm" onClick={() => store.markAllRead()}>
            Mark all read
          </button>
          <button type="button" class="ny-action btn btn-ghost btn-sm" onClick={() => store.clearAll()}>
            Clear
          </button>
          <button type="button" class="ghost-icon-button board-sheet-close" aria-label="Close" onClick={props.onClose}>
            <X size={20} />
          </button>
        </div>
      }
      testId="needs-you-drawer"
    >
      <Show when={store.list().length > 0} fallback={<p class="ny-empty">Nothing needs you right now.</p>}>
        <For each={GROUP_ORDER}>
          {(group) => (
            <Show when={grouped()[group].length > 0}>
              <section class="ny-group">
                <h3 class="ny-group-head">{GROUP_LABEL[group]}</h3>
                <div class="ny-group-rows">
                  <For each={grouped()[group]}>
                    {(item) => <Row item={item} onOpen={() => open(item)} onDismiss={() => store.dismiss(item.id)} />}
                  </For>
                </div>
              </section>
            </Show>
          )}
        </For>
      </Show>
    </BoardSheet>
  );
}

/** The "needs you" button: the animated Unicrab (UNI-200) in place of a bell icon, badge count kept. */
export function NeedsYouBell(props: { onOpen: () => void }) {
  const hosts = useHostsOptional();
  const live = mascotLive(() => hosts?.hosts() ?? []);
  const unread = () => needsYou().unreadCount();
  return (
    <button type="button" class="ghost-icon-button ny-bell" aria-label="Needs you" data-state={live.state()} onClick={() => (live.poke(), props.onOpen())}>
      <Unicrab state={live.state()} size={30} />
      <Show when={unread() > 0}>
        <span class="ny-badge">{unread() > 9 ? '9+' : unread()}</span>
      </Show>
    </button>
  );
}
