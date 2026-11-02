/**
 * The phone Chats list (phone spec §3.3): the selected PC's chats as 72 px
 * rows — an optional Pinned group, then everything by last activity (the
 * host's Properties sort / filters apply, never a per-project grouping).
 * Swipe left = Archive (undo toast), swipe right = Pin / Unpin, long-press =
 * an action sheet without desktop-only rows (PH-19). The selected PC offline:
 * rows dimmed, still openable (cached, read-only, PH-15).
 */
import { createMemo, createSignal, For, Match, Show, Switch } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Archive, ArchiveRestore, Circle, CircleCheck, GitFork, MessageSquare, Pencil, Pin, PinOff, Trash2 } from 'lucide-solid';
import type { SessionCard } from '../../lib/host/client';
import { hubs } from '../../lib/session/hub';
import { isInternalSession } from '../../lib/session/internal';
import { sessions } from '../../lib/session/store';
import { deleteSession, forkSession, markUnread, restoreSession, sessionHref } from '../../lib/session/actions';
import { isHomeCwd, sessionTitle } from '../../lib/session/selectors';
import { listView, viewGroups, type CardFacts } from '../../lib/session/listView';
import { cronRunLabel } from '../../lib/cronRunLabels';
import { toasts } from '../../lib/toasts';
import { liveRowState } from '../sidebar/SessionRow';
import { openRenameDialog } from '../session/RenameDialog';
import { ActionSheet, type ActionItem } from './Sheet';
import { ListGroup, ListRow } from './lists';
import { haptic } from './services';
import './chats.css';

/** Pure (§3.3 caption): "now", "5m", "11:47" (today), "Mon" (this week), "3 Feb". */
export function shortTime(ms: number | undefined, now = Date.now()): string {
  if (!ms) return '';
  const diff = Math.max(0, now - ms);
  if (diff < 60_000) return 'now';
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m`;
  const d = new Date(ms);
  const today = new Date(now);
  if (d.toDateString() === today.toDateString()) return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
  if (diff < 6 * 86_400_000) return d.toLocaleDateString([], { weekday: 'short' });
  return d.toLocaleDateString([], { day: 'numeric', month: 'short' });
}

/** Pure: the project basename a row shows (never a path; home / scratch → none). */
export function projectName(cwd: string | undefined): string | undefined {
  if (!cwd || isHomeCwd(cwd)) return undefined;
  const base = cwd.replace(/[\\/]+$/, '').split(/[\\/]/).pop();
  if (!base || /^(scratch|tmp)$/i.test(base) || /^\.?unipi-scratch/i.test(base)) return undefined;
  return base;
}

/** Pure: matches a chat against the Chats search (title + project, case-insensitive). */
export function matchesQuery(title: string, project: string | undefined, query: string): boolean {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return true;
  return title.toLocaleLowerCase().includes(q) || (project ?? '').toLocaleLowerCase().includes(q);
}

type RowState = ReturnType<typeof liveRowState>;

function StatusGlyph(p: { state: RowState; unread?: boolean }) {
  return (
    <span class="chat-glyph" data-state={p.state} data-unread={p.unread ? '' : undefined} aria-hidden="true">
      <Switch fallback={<MessageSquare size={20} />}>
        <Match when={p.state === 'working'}>
          <span class="chat-glyph-ring" />
        </Match>
        <Match when={p.state === 'needs-you'}>
          <span class="chat-glyph-dot">!</span>
        </Match>
      </Switch>
      <Show when={p.unread && p.state !== 'needs-you'}>
        <span class="chat-glyph-unread" />
      </Show>
    </span>
  );
}

/** Archive with an Undo toast (§3.3 swipe). */
export async function archiveWithUndo(hostId: string, sessionId: string): Promise<void> {
  sessions().close(hostId, sessionId);
  const hub = hubs().ensure(hostId);
  await hub.archive(sessionId);
  if (hub.state.error) {
    toasts().push("Couldn't archive the chat", 'error');
    return;
  }
  toasts().push('Chat archived', 'info', { label: 'Undo', run: () => restoreSession(hostId, sessionId) });
}

export function ChatsList(p: { hostId: string; query: string; offline?: boolean; selecting?: Set<string>; onToggleSelect?: (id: string) => void }) {
  const navigate = useNavigate();
  const hub = createMemo(() => hubs().ensure(p.hostId));
  const cards = createMemo(() => Object.values(hub().state.cards).filter((c) => !isInternalSession(c.session_id)));
  const factsOf = (card: SessionCard): CardFacts => ({
    state: liveRowState(card, sessions().peek(p.hostId, card.session_id)),
    ...(card.source === 'cron' ? { cronStatus: cronRunLabel(p.hostId, card.session_id)?.status } : {}),
    archived: false,
  });
  const titleOf = (card: SessionCard) => {
    const open = sessions().peek(p.hostId, card.session_id);
    const named = open?.state.name?.trim() || card.name?.trim();
    const run = card.source === 'cron' ? cronRunLabel(p.hostId, card.session_id) : undefined;
    return !named && run ? run.jobName : sessionTitle(card, open?.state, 'New chat');
  };
  const view = createMemo(() => {
    const v = listView(p.hostId);
    // The phone never groups by project (the PC is the chip; §3.3) and lists the trash on its own page.
    return { ...v, group: 'none' as const, filters: { ...v.filters, archived: ['unarchived' as const] } };
  });
  const groups = createMemo(() => viewGroups(cards(), hub().state.meta, view(), factsOf));
  const filtered = (list: SessionCard[]) => list.filter((c) => matchesQuery(titleOf(c), projectName(c.cwd), p.query));
  const pinned = createMemo(() => filtered(groups().pinned));
  const rest = createMemo(() => filtered(groups().groups.flatMap((g) => g.cards)));
  const [menuFor, setMenuFor] = createSignal<SessionCard | undefined>();

  const setPinned = (card: SessionCard, on: boolean) => {
    haptic('selection');
    void hub().setMeta(card.session_id, { pinned: on });
  };

  const menuSections = (card: SessionCard): ActionItem[][] => {
    const isPinned = !!hub().state.meta[card.session_id]?.pinned;
    const title = titleOf(card);
    return [
      [
        { id: 'pin', label: isPinned ? 'Unpin' : 'Pin', icon: isPinned ? PinOff : Pin, onSelect: () => setPinned(card, !isPinned) },
        {
          id: 'rename',
          label: 'Rename',
          icon: Pencil,
          onSelect: () => openRenameDialog({ hostId: p.hostId, sessionId: card.session_id, name: sessions().peek(p.hostId, card.session_id)?.state.name ?? card.name ?? '' }),
        },
        { id: 'unread', label: 'Mark as unread', icon: ArchiveRestore, onSelect: () => void markUnread(p.hostId, card.session_id) },
        { id: 'fork', label: 'Fork', icon: GitFork, onSelect: () => void forkSession(p.hostId, card.session_id) },
        { id: 'archive', label: 'Archive', icon: Archive, onSelect: () => void archiveWithUndo(p.hostId, card.session_id) },
      ],
      [{ id: 'delete', label: 'Delete', icon: Trash2, tone: 'danger', confirm: `Delete “${title}”?`, onSelect: () => void deleteSession(p.hostId, card.session_id) }],
    ];
  };

  const row = (card: SessionCard) => {
    const state = () => factsOf(card).state;
    const meta = () => hub().state.meta[card.session_id];
    const selecting = () => !!p.selecting;
    const picked = () => !!p.selecting?.has(card.session_id);
    const subtitle = () => {
      const project = projectName(card.cwd);
      const what =
        state() === 'working'
          ? 'Working…'
          : state() === 'needs-you'
            ? 'Needs you'
            : state() === 'crashed'
              ? 'Crashed'
              : card.message_count
                ? `${card.message_count} message${card.message_count === 1 ? '' : 's'}`
                : undefined;
      return [project, what].filter(Boolean).join(' · ') || undefined;
    };
    return (
      <ListRow
        testId={`chat-row-${card.session_id}`}
        leading={
          selecting() ? (
            <span class="chat-select" data-on={picked() ? '' : undefined}>{picked() ? <CircleCheck size={24} /> : <Circle size={24} />}</span>
          ) : (
            <StatusGlyph state={state()} unread={meta()?.unread ?? card.unread} />
          )
        }
        title={<span class="chat-row-title" data-unread={(meta()?.unread ?? card.unread) ? '' : undefined}>{titleOf(card)}</span>}
        subtitle={<span class="chat-row-sub" data-state={state()}>{subtitle()}</span>}
        caption={shortTime(card.last_activity ?? card.updated_at)}
        trailing={meta()?.pinned && !selecting() ? <Pin size={16} class="chat-row-pin" aria-label="Pinned" /> : undefined}
        dim={p.offline}
        selected={picked()}
        onClick={() => (selecting() ? p.onToggleSelect?.(card.session_id) : navigate(sessionHref(p.hostId, card.session_id)))}
        onLongPress={selecting() ? undefined : () => setMenuFor(card)}
        swipeStart={selecting() ? undefined : { label: meta()?.pinned ? 'Unpin' : 'Pin', icon: meta()?.pinned ? PinOff : Pin, tone: 'accent', onTrigger: () => setPinned(card, !meta()?.pinned) }}
        swipeEnd={selecting() || state() === 'working' ? undefined : { label: 'Archive', icon: Archive, tone: 'neutral', onTrigger: () => void archiveWithUndo(p.hostId, card.session_id) }}
      />
    );
  };

  return (
    <div class="ph-chats" data-testid="phone-chats">
      <Show when={pinned().length > 0}>
        <ListGroup title="Pinned">
          <For each={pinned()}>{row}</For>
        </ListGroup>
      </Show>
      <Show when={rest().length > 0}>
        <ListGroup title={pinned().length > 0 ? 'Recent' : undefined}>
          <For each={rest()}>{row}</For>
        </ListGroup>
      </Show>
      <Show when={p.query.trim() && pinned().length === 0 && rest().length === 0}>
        <p class="ph-chats-none" data-testid="chats-no-match">No chats match “{p.query.trim()}”.</p>
      </Show>
      <Show when={menuFor()} keyed>
        {(card) => <ActionSheet open onClose={() => setMenuFor(undefined)} title={titleOf(card)} subtitle={projectName(card.cwd)} sections={menuSections(card)} testId="chat-row-sheet" />}
      </Show>
    </div>
  );
}

/** The selected PC's chats, as ids, in list order (for Select › Archive). */
export function chatIds(hostId: string): string[] {
  return Object.values(hubs().ensure(hostId).state.cards)
    .filter((c) => !isInternalSession(c.session_id))
    .map((c) => c.session_id);
}

export { sessionHref };
