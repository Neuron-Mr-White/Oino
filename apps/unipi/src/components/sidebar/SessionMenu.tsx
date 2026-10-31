/**
 * The session row's context menu (Z-16, WP-A5): right-click (desktop) or
 * long-press (phone) on any row of the sessions list — sidebar or the phone
 * Chat tab, both render `SessionRow`.
 *
 * One menu app-wide (`lib/session/actions.ts` holds whose row is open, so both
 * shells share it and tests drive it directly): Pin, Rename…, Archive, Mark as
 * unread, Fork, Copy id / path / log path, Reveal folder, Export…, Delete.
 * Archive is instant (Z-13's Archived list restores it); Delete is a two-step
 * row — confirm first, then the archive+unlink move (Z-20). The ⋯ header keeps
 * chat-scoped rows only; this menu owns the row-scoped ones.
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { Archive, ArchiveRestore, Copy, FileText, Folder, FolderOpen, GitFork, Pencil, Pin, PinOff, Share, Trash2 } from 'lucide-solid';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { exportSession } from '../../lib/session/exportSession';
import { openRenameDialog, type RenameTarget } from '../session/RenameDialog';
import { archiveSession, closeRowMenu, copyFromRow, deleteSession, forkSession, markUnread, revealFolder, setPinned, openRowMenuState } from '../../lib/session/actions';
import { hubs } from '../../lib/session/hub';
import { sessions } from '../../lib/session/store';

export function SessionMenu() {
  const state = openRowMenuState;
  /** Z-20: Delete asks once (the row turns into a confirm pair). */
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const reset = () => setConfirmDelete(false);
  const pinned = createMemo(() => !!(state() && hubs().peek(state()!.hostId)?.state.meta[state()!.card.session_id]?.pinned));
  const run = (fn: () => unknown) => () => {
    closeRowMenu();
    reset();
    void fn();
  };
  return (
    <Show when={state()}>
      {(s) => {
        const { hostId, card } = s();
        const archiveLabel = () => (pinned() ? 'Archive (pinned)' : 'Archive');
        return (
          <AnchoredMenu anchor={s().anchor} width={248} label={`Session ${card.name || card.session_id}`} onDismiss={() => (closeRowMenu(), reset())}>
            <MenuRow
              icon={pinned() ? <PinOff size={17} /> : <Pin size={17} />}
              label={pinned() ? 'Unpin' : 'Pin'}
              onSelect={() => {
                // Capture before the menu unmounts: `pinned()` reads the row
                // menu's state, gone once `run` has closed it.
                const next = !pinned();
                run(() => setPinned(hostId, card.session_id, next))();
              }}
            />
            <MenuRow
              icon={<Pencil size={17} />}
              label="Rename…"
              onSelect={run(() => openRenameDialog({ hostId, sessionId: card.session_id, name: card.name ?? '' } satisfies RenameTarget))}
            />
            <MenuRow icon={<Archive size={17} />} label={archiveLabel()} hint="Archived list" onSelect={run(() => archiveSession(hostId, card.session_id))} />
            <MenuRow icon={<ArchiveRestore size={17} />} label="Mark as unread" onSelect={run(() => markUnread(hostId, card.session_id))} />
            <MenuSeparator />
            <MenuRow icon={<GitFork size={17} />} label="Fork" onSelect={run(() => forkSession(hostId, card.session_id))} />
            <MenuRow icon={<Copy size={17} />} label="Copy session id" onSelect={run(() => copyFromRow('id', card))} />
            <MenuRow icon={<Folder size={17} />} label="Copy path" disabled={!card.cwd} onSelect={run(() => copyFromRow('path', card))} />
            <MenuRow icon={<FileText size={17} />} label="Copy log path" disabled={!card.file} onSelect={run(() => copyFromRow('log', card))} />
            <MenuRow icon={<FolderOpen size={17} />} label="Reveal folder" disabled={!card.cwd} onSelect={run(() => revealFolder(hostId, card.cwd))} />
            <MenuRow icon={<Share size={17} />} label="Export…" onSelect={run(() => { sessions().ensure(hostId, card.session_id); return exportSession(hostId, card.session_id); })} />
            <MenuSeparator />
            <Show
              when={!confirmDelete()}
              fallback={
                <MenuRow icon={<Trash2 size={17} />} label={`Delete “${rowTitle(card)}”? Yes, delete`} tone="danger" onSelect={run(() => deleteSession(hostId, card.session_id))} />
              }
            >
              <MenuRow icon={<Trash2 size={17} />} label="Delete" tone="danger" onSelect={() => setConfirmDelete(true)} />
            </Show>
          </AnchoredMenu>
        );
      }}
    </Show>
  );
}

const rowTitle = (card: { name?: string | null; cwd?: string | null; session_id: string }): string =>
  card.name?.trim() || (card.cwd ? card.cwd.split(/[\\/]/).filter(Boolean).pop() ?? card.session_id : card.session_id);
