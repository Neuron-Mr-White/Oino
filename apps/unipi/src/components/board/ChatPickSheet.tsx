/**
 * Board → chat picker (UNI-185): one board project can serve several
 * chats, so going to "the" chat asks which. Chats under the project's
 * root come first; the open one is marked. A single candidate is opened
 * straight away (no sheet).
 */
import { createEffect, createMemo, For, on, Show } from 'solid-js';
import { Check, MessagesSquare, Plus } from 'lucide-solid';
import { BoardSheet } from './BoardSheet';
import { chatGroups, chatLists, STATUS_WORD, type ChatTarget } from '../../lib/session/list';
import { activeChatTargetFor } from '../../lib/session/open';
import { sessions } from '../../lib/session/store';
import { rankChatsForProject } from '../../lib/board/portal';

export function ChatPickSheet(props: {
  open: boolean;
  hostId: string | undefined;
  /** The board project's root (chats under it come first). */
  root?: string;
  /** "Add UNI-12 to…" or "Go to a chat". */
  label: string;
  onPick: (target: ChatTarget) => void;
  /** Starts a new v2 chat (`/new/<host>?cwd=`) when chosen. */
  onNewChat?: (hostId: string, cwd?: string) => void;
  onClose: () => void;
}) {
  const lists = chatLists();
  const entry = () => (props.hostId ? lists.get(props.hostId) : undefined);
  const chats = createMemo(() => {
    if (!props.hostId) return [];
    const items = chatGroups(props.hostId, entry()?.data).flatMap((g) => g.items);
    return rankChatsForProject(items, props.root);
  });
  const isOpen = (t: ChatTarget) => {
    const active = activeChatTargetFor(t.hostId);
    if (active?.kind === 'session' && active.sessionId === t.session) return true;
    return !!sessions().peek(t.hostId, t.session);
  };
  // Fresh list on open; a single candidate needs no question.
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (!open || !props.hostId) return;
        void lists.load(props.hostId).then(() => {
          if (props.open && chats().length === 1) props.onPick(chats()[0]!);
        });
      },
    ),
  );
  const inProject = () => chats().filter((c) => c.inProject);
  const others = () => chats().filter((c) => !c.inProject);
  const Row = (chat: ChatTarget) => (
    <button type="button" class="model-row" data-testid="chat-pick-row" aria-current={isOpen(chat) ? 'true' : undefined} onClick={() => props.onPick(chat)}>
      <MessagesSquare size={18} class="shrink-0 text-text-muted" />
      <span class="min-w-0 flex-1 text-left">
        <span class="block truncate">{chat.title}</span>
        <span class="block truncate model-row-sub">
          {STATUS_WORD[chat.status]}
          {chat.cwd ? ` · ${chat.cwd}` : ''}
          {isOpen(chat) ? ' · open' : ''}
        </span>
      </span>
      <Show when={isOpen(chat)}>
        <Check size={18} class="board-accent shrink-0" />
      </Show>
    </button>
  );
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label={props.label} testId="chat-pick-sheet">
      <Show
        when={chats().length > 0}
        fallback={<p class="sheet-label">{!entry() || entry()?.loading ? 'Loading chats…' : entry()?.error ? `Couldn't list chats: ${entry()!.error}` : 'No chats on this PC'}</p>}
      >
        <Show when={inProject().length > 0}>
          <p class="sheet-label">In this project</p>
          <div class="model-list mb-4">
            <For each={inProject()}>{(chat) => Row(chat)}</For>
          </div>
        </Show>
        <Show when={others().length > 0}>
          <p class="sheet-label">{inProject().length > 0 ? 'Other chats' : 'Chats on this PC'}</p>
          <div class="model-list">
            <For each={others()}>{(chat) => Row(chat)}</For>
          </div>
        </Show>
      </Show>
      <Show when={props.onNewChat && props.hostId && !!entry() && !entry()?.loading}>
        <div class="model-list mt-3">
          <button
            type="button"
            class="model-row"
            data-testid="chat-pick-new"
            onClick={() => props.onNewChat!(props.hostId!, props.root)}
          >
            <Plus size={18} class="shrink-0 text-text-muted" />
            <span class="min-w-0 flex-1 text-left">
              <span class="block truncate">New chat</span>
              <Show when={props.root}>
                <span class="block truncate model-row-sub">{props.root}</span>
              </Show>
            </span>
          </button>
        </div>
      </Show>
    </BoardSheet>
  );
}
