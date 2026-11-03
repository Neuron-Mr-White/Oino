/**
 * The phone Chats tab (phone spec §3.3) and its Archived page (`/chat/archived`).
 *
 * Chats: tab page with the PC chip; tools = the needs-you crab (badge = count,
 * opens the Needs-you sheet) and ⋯ (Archived chats · Select · Sort). Large
 * title "Chats", a local search under it, FAB "New chat" → `/new/<PC>`. The
 * body is the selected PC's chats (`components/phone/ChatsList`): no per-host
 * groups (the PC is the chip), no Archived footer, no "More" (PH-16) — Cron and
 * Files are tabs, Ports and extension pages live in Settings.
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Archive, ArchiveRestore, ArrowDownAZ, CheckSquare, Clock, Ellipsis, MessageSquare, SquarePen, Trash2, WifiOff } from 'lucide-solid';
import { can } from '../lib/platform';
import { useHostsOptional } from '../hosts/context';
import { hubs } from '../lib/session/hub';
import { isInternalSession } from '../lib/session/internal';
import { listView, setListView } from '../lib/session/listView';
import { archiveMany, deleteArchived, restoreSession, sessionHref } from '../lib/session/actions';
import { sessionTitle } from '../lib/session/selectors';
import { needsYou } from '../lib/needsYou';
import { setNeedsYouOpen } from '../components/FloatingBell';
import { setAddHostOpen } from '../components/Pairing';
import { EmptyState, NoHostState } from '../components/EmptyState';
import { Unicrab } from '../components/Unicrab';
import { mascotLive } from '../lib/mascotLive';
import { hostStatus, useSelectedHost } from '../components/shell/HostSwitcher';
import { ActionSheet, Button, ListGroup, ListRow, PageScaffold, confirmSheet, type ActionItem } from '../components/phone';
import { archiveWithUndo, ChatsList, projectName, shortTime } from '../components/phone/ChatsList';
import { haptic } from '../components/phone/services';
import './chat.css';

/** The needs-you button of the Chats top bar: the crab with the unread count (replaces the floating crab, PH-12). */
function NeedsYouButton() {
  const hosts = useHostsOptional();
  const live = mascotLive(() => hosts?.hosts() ?? []);
  const count = () => needsYou().unreadCount();
  return (
    <button
      type="button"
      class="ph-icon-btn ph-needs-you"
      aria-label={count() > 0 ? `Needs you, ${count()} waiting` : 'Needs you'}
      data-testid="chats-needs-you"
      data-state={live.state()}
      onClick={() => (live.poke(), setNeedsYouOpen(true))}
    >
      <Unicrab state={live.state()} size={30} title="" />
      <Show when={count() > 0}>
        <span class="ph-badge" aria-hidden="true">
          {count() > 9 ? '9+' : count()}
        </span>
      </Show>
    </button>
  );
}

export default function Chat() {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const selected = useSelectedHost();
  const [query, setQuery] = createSignal('');
  const [menu, setMenu] = createSignal(false);
  const [selecting, setSelecting] = createSignal<Set<string> | undefined>();
  const hostId = () => selected()?.id;
  const noHosts = () => !!hosts && !hosts.loading() && hosts.hosts().length === 0;
  const hub = () => (hostId() ? hubs().ensure(hostId()!) : undefined);
  const offline = () => !!hostId() && hostStatus(hostId()!) === 'offline';
  const total = () => (hub() ? Object.keys(hub()!.state.cards).filter((id) => !isInternalSession(id)).length : 0);
  // [ux-sessions] "No chats yet" only on a settled list: never while the first list, a re-list of
  // an empty store, or a batch archive is in flight.
  const loading = () => !!hub() && !offline() && (!hub()!.state.loaded || !!hub()!.state.bulk || (hub()!.state.loading && total() === 0));
  const sort = () => (hostId() ? listView(hostId()!).sort : 'updated');
  const newChat = () => navigate(`/new/${encodeURIComponent(hostId() ?? 'local')}`);

  const toggleSelect = (id: string) =>
    setSelecting((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const archiveSelected = async () => {
    const ids = [...(selecting() ?? [])];
    if (!ids.length || !hostId()) return;
    const ok = await confirmSheet({ title: `Archive ${ids.length} chat${ids.length === 1 ? '' : 's'}?`, body: 'You can restore them from Archived chats.', confirm: 'Archive' });
    if (!ok) return;
    setSelecting(undefined);
    // [ux-sessions] one batched host op + one Undo toast (was one request and one toast per chat).
    if (ids.length === 1) await archiveWithUndo(hostId()!, ids[0]!);
    else await archiveMany(hostId()!, ids, 'chats');
  };

  const menuSections = (): ActionItem[][] => [
    [
      { id: 'archived', label: 'Archived chats', icon: Archive, onSelect: () => navigate('/chat/archived') },
      { id: 'select', label: 'Select', icon: CheckSquare, disabled: total() === 0, onSelect: () => setSelecting(new Set<string>()) },
    ],
    [
      { id: 'sort-recent', label: 'Sort: Recent', icon: Clock, checked: sort() !== 'name', onSelect: () => hostId() && setListView(hostId()!, { sort: 'updated' }) },
      { id: 'sort-name', label: 'Sort: Name', icon: ArrowDownAZ, checked: sort() === 'name', onSelect: () => hostId() && setListView(hostId()!, { sort: 'name' }) },
    ],
  ];

  return (
    <div class="chat-tab ph-chats-tab">
      <PageScaffold
        title="Chats"
        testId="chats-page"
        pcChip={can('hosts')}
        tools={
          <Show
            when={!selecting()}
            fallback={
              <button type="button" class="ph-text-btn" data-testid="chats-select-done" onClick={() => setSelecting(undefined)}>
                Done
              </button>
            }
          >
            <Show when={can('hosts')}>
              <NeedsYouButton />
            </Show>
            <button type="button" class="ph-icon-btn" aria-label="More options" data-testid="chats-menu" onClick={() => setMenu(true)}>
              <Ellipsis size={24} />
            </button>
          </Show>
        }
        search={!noHosts() && total() > 0 ? { value: query(), onInput: setQuery, placeholder: 'Search chats' } : undefined}
        fab={!noHosts() && !selecting() ? { icon: SquarePen, label: 'New chat', onClick: newChat } : undefined}
        onRefresh={hub() ? () => hub()!.reload().catch(() => undefined) : undefined}
        footer={
          selecting() ? (
            <div class="ph-select-bar">
              <span class="ph-select-count">{selecting()!.size} selected</span>
              <Button kind="primary" disabled={selecting()!.size === 0} onClick={() => void archiveSelected()} testId="chats-archive-selected">
                Archive
              </Button>
            </div>
          ) : undefined
        }
      >
        <Show when={!noHosts()} fallback={<NoHostState what="its chats" onAdd={() => setAddHostOpen(true)} />}>
          <Show when={hostId()}>
            {(id) => (
              <Show
                when={total() > 0 || loading()}
                fallback={
                  <Show
                    when={!offline()}
                    fallback={<EmptyState icon={<WifiOff size={22} />} title={`Can't reach ${selected()?.name ?? 'your PC'}`} body="Its chats show up here once it is back." compact testId="chats-offline" />}
                  >
                    <EmptyState
                      title={`No chats on ${selected()?.name ?? 'this PC'} yet`}
                      body="Start one and it shows up here."
                      testId="chats-empty"
                      action={
                        <Button kind="primary" onClick={newChat}>
                          New chat
                        </Button>
                      }
                    />
                  </Show>
                }
              >
                <Show when={!loading()} fallback={<ChatsSkeleton />}>
                  <ChatsList hostId={id()} query={query()} offline={offline()} selecting={selecting()} onToggleSelect={toggleSelect} />
                </Show>
              </Show>
            )}
          </Show>
        </Show>
      </PageScaffold>
      <ActionSheet open={menu()} onClose={() => setMenu(false)} title="Chats" sections={menuSections()} testId="chats-menu-sheet" />
    </div>
  );
}

function ChatsSkeleton() {
  return (
    <div class="ph-chats-skeleton" aria-busy="true" aria-label="Loading chats">
      <For each={[0, 1, 2, 3, 4]}>{() => <div class="ph-skel-row"><span class="ph-skel-dot" /><span class="ph-skel-lines"><span /><span /></span></div>}</For>
    </div>
  );
}

/** `/chat/archived` (pushed page, §1.2): the selected PC's trash — open, restore, delete. */
export function ArchivedChats() {
  const navigate = useNavigate();
  const selected = useSelectedHost();
  const hostId = () => selected()?.id;
  const hub = createMemo(() => (hostId() ? hubs().ensure(hostId()!) : undefined));
  const [loaded, setLoaded] = createSignal(false);
  const load = async () => {
    await hub()?.loadArchived(true).catch(() => undefined);
    setLoaded(true);
  };
  void load();
  const list = () => hub()?.state.archived ?? [];
  const [menuFor, setMenuFor] = createSignal<string | undefined>();
  const card = () => list().find((c) => c.session_id === menuFor());
  const restore = (id: string) => {
    haptic('selection');
    void restoreSession(hostId()!, id);
  };
  const remove = async (id: string, title: string) => {
    haptic('warning');
    if (await confirmSheet({ title: `Delete “${title}” for good?`, body: 'This removes the chat from your PC.', confirm: 'Delete', tone: 'danger' })) void deleteArchived(hostId()!, id);
  };
  return (
    <PageScaffold title="Archived chats" subtitle={selected()?.name} back="/chat" testId="archived-page" onRefresh={load}>
      <Show
        when={list().length > 0}
        fallback={<Show when={loaded()}><EmptyState icon={<Archive size={22} />} title="No archived chats" body="Chats you archive land here." testId="archived-empty" /></Show>}
      >
        <ListGroup>
          <For each={list()}>
            {(c) => {
              const title = () => sessionTitle(c, undefined, 'New chat');
              return (
                <ListRow
                  testId={`archived-row-${c.session_id}`}
                  leading={<span class="chat-glyph" data-state="idle"><MessageSquare size={20} /></span>}
                  title={title()}
                  subtitle={projectName(c.cwd)}
                  caption={shortTime(c.last_activity ?? c.updated_at)}
                  onClick={() => navigate(sessionHref(hostId()!, c.session_id))}
                  onLongPress={() => setMenuFor(c.session_id)}
                  swipeStart={{ label: 'Restore', icon: ArchiveRestore, tone: 'accent', onTrigger: () => restore(c.session_id) }}
                  swipeEnd={{ label: 'Delete', icon: Trash2, tone: 'danger', onTrigger: () => void remove(c.session_id, title()) }}
                />
              );
            }}
          </For>
        </ListGroup>
      </Show>
      <Show when={card()} keyed>
        {(c) => (
          <ActionSheet
            open
            onClose={() => setMenuFor(undefined)}
            title={sessionTitle(c, undefined, 'New chat')}
            sections={[
              [{ id: 'restore', label: 'Restore', icon: ArchiveRestore, onSelect: () => restore(c.session_id) }],
              [{ id: 'delete', label: 'Delete for good', icon: Trash2, tone: 'danger', confirm: 'Delete this chat for good?', onSelect: () => void deleteArchived(hostId()!, c.session_id) }],
            ]}
          />
        )}
      </Show>
    </PageScaffold>
  );
}
