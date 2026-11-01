/**
 * The phone Chat tab (§3.9, WP-A4): the shared sessions list full screen
 * (`components/sidebar/SessionList`, hosts → projects → sessions) — the same
 * component the desktop sidebar shows. Header: New chat, and "More" with Cron
 * and the extension pages (§3.9's phone row). The PcChip is gone here (Z-8:
 * hosts are the list's groups; the chip stays on Board/Cron). Selecting a
 * session routes to `/s/:host/:session` (the conversation layer).
 */
import { createSignal, For, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Ellipsis, FolderOpen, Plug, SquarePen } from 'lucide-solid';
import { can } from '../lib/platform';
import { PageHeader } from '../components/PageHeader';
import { AnchoredMenu, MenuRow } from '../components/board/AnchoredMenu';
import { SessionList } from '../components/sidebar/SessionList';
import { useNewChatHost } from '../components/shell/Sidebar';
import { railItems } from '../components/shell/Rail';
import './chat.css';

/** The "More" pages of the Chat header menu (§3.9): Cron, extension pages, and the
 *  phone's Files and Ports screens (P1-05: reachable without opening a chat first). */
export function moreItems() {
  const phonePages = [
    ...(can('files') ? [{ id: 'files', label: 'Files', href: '/files', icon: FolderOpen, order: 30 }] : []),
    ...(can('portForward') ? [{ id: 'preview', label: 'Ports', href: '/preview', icon: Plug, order: 31 }] : []),
  ];
  return [...railItems().filter((item) => item.id !== 'board'), ...phonePages];
}

export default function Chat() {
  const navigate = useNavigate();
  const newChatHost = useNewChatHost();
  const [moreAnchor, setMoreAnchor] = createSignal<DOMRect | undefined>();

  return (
    <div class="chat-tab">
      <PageHeader
        class="chat-tab-head"
        title="Chat"
        tools={
          <>
            <button
              type="button"
              class="ghost-icon-button"
              aria-label="New chat"
              data-tip="New chat"
              data-testid="chat-new"
              onClick={() => navigate(`/new/${encodeURIComponent(newChatHost())}`)}
            >
              <SquarePen size={21} />
            </button>
            <Show when={moreItems().length > 0}>
              <button
                type="button"
                class="ghost-icon-button"
                aria-label="More"
                data-tip="Cron, Files, Ports and extension pages"
                data-testid="chat-more"
                onClick={(e) => setMoreAnchor((e.currentTarget as HTMLElement).getBoundingClientRect())}
              >
                <Ellipsis size={22} />
              </button>
            </Show>
          </>
        }
      />
      <div class="chat-tab-list chat-tab-page">
        <SessionList variant="page" />
      </div>
      <Show when={moreAnchor()} keyed>
        {(anchor) => (
          <AnchoredMenu anchor={anchor} onDismiss={() => setMoreAnchor(undefined)} label="More" alignEnd>
            <For each={moreItems()}>
              {(item) => {
                const Icon = item.icon;
                return (
                  <MenuRow icon={<Icon size={17} />} label={item.label} onSelect={() => (setMoreAnchor(undefined), navigate(item.href))} />
                );
              }}
            </For>
          </AnchoredMenu>
        )}
      </Show>
    </div>
  );
}
