/**
 * Chat tab (UNI-113, DESIGN §3.4): pick a PC (like the Board), then a pi
 * running in one of its herdr workspaces; the conversation opens over the
 * tab (ChatView) and stays live both ways with the TUI.
 */
import { createEffect, createMemo, createSignal, For, on, Show, untrack } from 'solid-js';
import { ChevronRight, MessagesSquare, Plus, RefreshCw } from 'lucide-solid';
import { NewChatSheet, type NewChatStarted } from '../components/chat/NewChatSheet';
import { errorWords, folderName, hideKey, newChatHere, newChatOpen, pruneHidden, readHidden, setNewChatOpen, targetFromNew, unhideChat } from '../lib/chat/newChat';
import { desktopShell } from '../lib/layout';
import { useHosts } from '../hosts/context';
import { useSettings } from '../settings/context';
import { EmptyState, NoHostState, OfflineState } from '../components/EmptyState';
import { PageHeader } from '../components/PageHeader';
import { PcChip } from '../components/PcChip';
import { setAddHostOpen } from '../components/Pairing';
import { hapticTick } from '../lib/haptics';
import { boardPcs } from '../lib/board/pcs';
import { BoardSheet } from '../components/board/BoardSheet';
import { chatGroups, chatLists, needsYouFirst, readLastChatHost, rememberChatHost, STATUS_WORD, withLive, WORKING_WORD, type ChatTarget } from '../lib/chat/list';
import { chatVisible, openChat, openChatState } from '../lib/chat/open';
import { listModeBadge } from '../lib/chat/lhMode';
import { ModeIcon } from '../components/chat/ModeChip';
import { Monitor, Check } from 'lucide-solid';
import './chat.css';

export default function Chat() {
  const hosts = useHosts();
  const [settings] = useSettings();
  const lists = chatLists();
  const pcs = createMemo(() => boardPcs(hosts.hosts()).filter((pc) => !pc.dev || !!(window as { __UNIPI_CHAT__?: unknown }).__UNIPI_CHAT__));
  const [hostId, setHostId] = createSignal<string | undefined>();
  const [pcSheet, setPcSheet] = createSignal(false);
  const pc = () => pcs().find((p) => p.id === hostId());
  const entry = () => (hostId() ? lists.get(hostId()!) : undefined);
  // UNI-212: the open chat's live run state beats a stale polled "Idle".
  const liveOpen = () => {
    const o = openChatState();
    if (!o || o.chat.state.conn !== 'live') return undefined;
    return { hostId: o.target.hostId, pid: o.target.pid!, running: !!o.chat.state.run.running, waiting: o.chat.state.run.waiting, lhMode: o.chat.state.lhMode };
  };
  const allGroups = createMemo(() =>
    hostId()
      ? chatGroups(hostId()!, entry()?.data, entry()?.data?.protected ?? []).map((g) => ({ ...g, items: g.items.map((t) => withLive(t, liveOpen())) }))
      : [],
  );
  // UNI-260: chats hidden on this device ("Just hide from list"), per PC.
  const [showHidden, setShowHidden] = createSignal(false);
  const hidden = createMemo(() => (hostId() ? readHidden(hostId()!) : []));
  const groups = createMemo(() =>
    showHidden()
      ? allGroups()
      : allGroups()
          .map((g) => ({ ...g, items: g.items.filter((t) => !hidden().includes(hideKey(t))) }))
          .filter((g) => g.items.length > 0),
  );
  const hiddenCount = () => allGroups().reduce((n, g) => n + g.items.filter((t) => hidden().includes(hideKey(t))).length, 0);
  // Forget hide marks whose pi is gone (only once the list is really loaded).
  createEffect(() => {
    const id = hostId();
    const data = entry()?.data;
    if (id && data?.herdr.available) pruneHidden(id, allGroups().flatMap((g) => g.items));
  });
  const total = () => groups().reduce((n, g) => n + g.items.length, 0);

  createEffect(() => {
    const list = pcs();
    const current = untrack(hostId);
    if (current && list.some((p) => p.id === current)) return;
    const last = readLastChatHost();
    setHostId(list.find((p) => p.id === last)?.id ?? list[0]?.id);
  });
  createEffect(on(hostId, (id) => id && void lists.load(id)));
  // Keep agent states fresh while the tab is visible.
  createEffect(() => {
    const id = hostId();
    if (!id) return;
    const timer = setInterval(() => {
      if (!document.hidden) void lists.load(id);
    }, 8000);
    return () => clearInterval(timer);
  });

  const haptic = () => void hapticTick(settings.input.haptics, 'tap');
  const pickPc = (id: string) => {
    haptic();
    setPcSheet(false);
    setHostId(id);
    rememberChatHost(id);
  };
  const open = (t: ChatTarget) => {
    haptic();
    openChat(t, pc()?.name ?? '');
  };
  // UNI-260: a chat started from the New chat sheet opens at once; its first
  // message waits in the chat's outbox until the connection is live.
  const onStarted = (started: NewChatStarted) => {
    setNewChatOpen(false);
    if (started.hostId !== hostId()) {
      setHostId(started.hostId);
      rememberChatHost(started.hostId);
    }
    openChat(targetFromNew(started.hostId, started.data, started.title, started.label), started.pcName);
    if (started.first) openChatState()?.chat.send(started.first);
    void lists.load(started.hostId);
  };

  // "+" on a workspace group: a separate pi beside the others, same workspace
  // and folder — no sheet, no folder picking (the sheet stays for new places).
  const [startingIn, setStartingIn] = createSignal<string | null>(null);
  const [groupError, setGroupError] = createSignal<{ key: string; text: string } | null>(null);
  const groupKey = (g: { session: string; workspaceId: string }) => `${g.session}:${g.workspaceId}`;
  const newInGroup = async (group: { session: string; workspaceId: string; label: string; items: ChatTarget[] }) => {
    const first = group.items[0];
    const id = hostId();
    if (!first || !id || startingIn()) return;
    haptic();
    const key = groupKey(group);
    setStartingIn(key);
    setGroupError(null);
    const cwd = first.cwdFull;
    const res = await newChatHere({ ...first, hostId: id }, cwd);
    setStartingIn(null);
    if (!res.ok) {
      setGroupError({ key, text: errorWords(res.error).text });
      return;
    }
    openChat(targetFromNew(id, res.value, folderName(res.value.cwd ?? cwd ?? group.label), group.label), pc()?.name ?? '');
    void lists.load(id);
  };

  return (
    <Show
      when={pcs().length > 0}
      fallback={
        <div class="page-screen">
          <PageHeader title="Chat" />
          <NoHostState what="its pi chats" onAdd={() => setAddHostOpen(true)} />
        </div>
      }
    >
      <div class="chat-tab">
        <PageHeader
          class="chat-tab-head"
          title="Chat"
          lead={<PcChip name={pc()?.name} state={entry()?.error ? 'error' : entry()?.data ? 'ok' : 'checking'} onClick={() => setPcSheet(true)} />}
          tools={
            <>
              <button type="button" class="ghost-icon-button" aria-label="Refresh" onClick={() => hostId() && void lists.load(hostId()!)}>
                <RefreshCw size={20} classList={{ 'animate-spin': !!entry()?.loading }} />
              </button>
              <button type="button" class="ghost-icon-button" aria-label="New chat" data-testid="chat-new" onClick={() => (haptic(), setNewChatOpen(true))}>
                <Plus size={22} />
              </button>
            </>
          }
          meta={entry()?.data ? `${total()} pi ${total() === 1 ? 'session' : 'sessions'}${groups().some((g) => g.mux === 'tmux') ? '' : ' in herdr'}` : entry()?.error ? '\u00a0' : 'Connecting…'}
        />

        <div class="chat-tab-list">
          <Show when={entry()?.error && !entry()?.data}>
            <OfflineState compact={desktopShell()} pc={pc()?.name} message={entry()!.error} hint="Is unipi-host running on the PC?" onRetry={() => void lists.load(hostId()!)} />
          </Show>
          <Show when={entry()?.data && !entry()!.data!.herdr.available && total() === 0}>
            <EmptyState
              compact
              icon={<MessagesSquare size={22} />}
              title="herdr isn't running on this PC"
              body={`Chat follows pi sessions in herdr (and tmux).${entry()!.data!.herdr.reason ? ` (${entry()!.data!.herdr.reason})` : ''}`}
              action={
                <button type="button" class="btn btn-secondary" onClick={() => setNewChatOpen(true)}>
                  <Plus size={18} /> New chat
                </button>
              }
            />
          </Show>
          <Show when={entry()?.data?.herdr.available && total() === 0}>
            <EmptyState
              compact
              icon={<MessagesSquare size={22} />}
              title={`No pi running on ${pc()?.name ?? 'this PC'}`}
              body={hiddenCount() > 0 ? 'Every chat here is hidden on this device.' : 'Start one here, or run pi in a herdr pane on the PC.'}
              action={
                <button type="button" class="btn btn-primary" onClick={() => setNewChatOpen(true)}>
                  <Plus size={18} /> New chat
                </button>
              }
            />
          </Show>
          <For each={groups()}>
            {(group) => (
              <section class="chat-group">
                <h2 class="chat-group-head">
                  <span class="truncate">{group.label}</span>
                  <span class="chat-group-session">{group.session}</span>
                  <button
                    type="button"
                    class="chat-group-new"
                    aria-label={`New chat in ${group.label}`}
                    title={`New chat in ${group.label}`}
                    data-testid="chat-group-new"
                    disabled={startingIn() !== null}
                    onClick={() => void newInGroup(group)}
                  >
                    <Show when={startingIn() === groupKey(group)} fallback={<Plus size={16} />}>
                      <RefreshCw size={14} class="animate-spin" />
                    </Show>
                  </button>
                </h2>
                <Show when={groupError()?.key === groupKey(group)}>
                  <p class="chat-group-error" role="alert">
                    Couldn't start a chat here: {groupError()!.text}
                  </p>
                </Show>
                <div class="chat-group-rows">
                  <For each={needsYouFirst(group.items)}>
                    {(t) => (
                      <button
                        type="button"
                        class="chat-row"
                        data-status={t.status}
                        data-waiting={t.waiting ? '' : undefined}
                        data-open={chatVisible() && openChatState()?.target.hostId === t.hostId && openChatState()?.target.pid === t.pid ? '' : undefined}
                        disabled={t.pid === null}
                        onClick={() => open(t)}
                      >
                        <span class="chat-row-glyph" data-status={t.status}>
                          π
                        </span>
                        <span class="min-w-0 flex-1 text-left">
                          <span class="chat-row-title">
                            <span class="chat-row-title-text">{t.title}</span>
                            <Show when={t.waiting}>
                              <span class="chat-needs-badge">Needs you</span>
                            </Show>
                            <Show when={listModeBadge(t.lhMode, t.lhPaused)}>
                              {(b) => (
                                <span class="chat-mode-badge" data-mode={b().mode} data-paused={b().paused ? '' : undefined} data-testid="chat-mode-badge" title={`Working mode: ${b().label}`}>
                                  <ModeIcon mode={b().mode} size={11} />
                                  {b().label}
                                </span>
                              )}
                            </Show>
                          </span>
                          <span class="chat-row-sub">
                            <Show
                              when={t.pid !== null}
                              fallback={<span class="text-amber">Restart pi with the latest unipi to chat</span>}
                            >
                              <Show
                                when={t.waiting?.title}
                                fallback={
                                  <>
                                    <span class="chat-status" data-status={t.status}>
                                      {t.waiting ? (desktopShell() ? 'Run finished — click to see' : 'Run finished — tap to see') : t.pendingWork && t.status === 'working' ? WORKING_WORD : STATUS_WORD[t.status]}
                                    </span>
                                    <Show when={!t.waiting && t.status === 'working' && t.pendingWork} fallback={<Show when={t.cwd}><span class="truncate">{t.cwd}</span></Show>}>
                                      <span class="chat-row-pending truncate" data-pending-work="">{t.pendingWork}</span>
                                    </Show>
                                  </>
                                }
                              >
                                <span class="chat-row-waiting-text truncate">{t.waiting!.title}</span>
                              </Show>
                            </Show>
                          </span>
                        </span>
                        <Show when={showHidden() && hidden().includes(hideKey(t))}>
                          <span
                            role="button"
                            tabindex="0"
                            class="chat-unhide"
                            onClick={(e) => (e.stopPropagation(), unhideChat(t.hostId, hideKey(t)))}
                            onKeyDown={(e) => e.key === 'Enter' && (e.stopPropagation(), unhideChat(t.hostId, hideKey(t)))}
                          >
                            Unhide
                          </span>
                        </Show>
                        <ChevronRight size={18} class="text-text-dim shrink-0" />
                      </button>
                    )}
                  </For>
                </div>
              </section>
            )}
          </For>
          <Show when={hiddenCount() > 0}>
            <button type="button" class="board-text-button chat-hidden-toggle" data-testid="chat-hidden-toggle" onClick={() => setShowHidden(!showHidden())}>
              {showHidden() ? 'Hide hidden chats' : `${hiddenCount()} hidden · Show`}
            </button>
          </Show>
        </div>
      </div>

      <NewChatSheet open={newChatOpen()} onClose={() => setNewChatOpen(false)} pcs={pcs()} hostId={hostId()} onStarted={onStarted} />

      <BoardSheet open={pcSheet()} onClose={() => setPcSheet(false)} label="Choose a PC" testId="chat-pc-sheet">
        <div class="settings-group rounded-card bg-surface-2/60">
          <For each={pcs()}>
            {(p) => (
              <button type="button" class="picker-row" aria-current={p.id === hostId() ? 'true' : undefined} onClick={() => pickPc(p.id)}>
                <span class="settings-icon">
                  <Monitor size={18} />
                </span>
                <span class="min-w-0 flex-1 text-left">
                  <span class="block truncate text-[16px] font-medium">{p.name}</span>
                </span>
                <Show when={p.id === hostId()}>
                  <Check size={18} class="board-accent shrink-0" />
                </Show>
              </button>
            )}
          </For>
        </div>
      </BoardSheet>
    </Show>
  );
}
