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
import { createMemo, createSignal, Show } from 'solid-js';
import { Archive, ArchiveRestore, Copy, FileText, Folder, FolderOpen, GitFork, Pencil, Pin, PinOff, Share, Trash2 } from 'lucide-solid';
import { AnchoredMenu, MenuRow, MenuSeparator } from '../board/AnchoredMenu';
import { desktopShell } from '../../lib/layout';
import { exportSession } from '../../lib/session/exportSession';
import { openRenameDialog, type RenameTarget } from '../session/RenameDialog';
import { archiveSession, closeRowMenu, copyFromRow, deleteSession, forkSession, markUnread, revealFolder, setPinned, openRowMenuState } from '../../lib/session/actions';
import { hubs } from '../../lib/session/hub';
import { sessionTitle } from '../../lib/session/selectors';
import { sessions } from '../../lib/session/store';

export function SessionMenu() {
  const state = openRowMenuState;
  /** Z-20: Delete asks once (the row expands into an unclipped confirm pair, D-04). */
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const reset = () => setConfirmDelete(false);
  const pinned = createMemo(() => !!(state() && hubs().peek(state()!.hostId)?.state.meta[state()!.card.session_id]?.pinned));
  const iconSize = () => (desktopShell() ? 15 : 17);
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
        const liveName = () => sessions().peek(hostId, card.session_id)?.state.name ?? card.name ?? '';
        const title = () => sessionTitle(card, sessions().peek(hostId, card.session_id)?.state, card.session_id);
        return (
          <AnchoredMenu
            anchor={s().anchor}
            width={desktopShell() ? 220 : 248}
            class="srow-menu"
            label={`Session ${title()}`}
            onDismiss={() => (closeRowMenu(), reset())}
          >
            <MenuRow
              icon={pinned() ? <PinOff size={iconSize()} /> : <Pin size={iconSize()} />}
              label={pinned() ? 'Unpin' : 'Pin'}
              onSelect={() => {
                // Capture before the menu unmounts: `pinned()` reads the row
                // menu's state, gone once `run` has closed it.
                const next = !pinned();
                run(() => setPinned(hostId, card.session_id, next))();
              }}
            />
            <MenuRow
              icon={<Pencil size={iconSize()} />}
              label="Rename…"
              onSelect={run(() => openRenameDialog({ hostId, sessionId: card.session_id, name: liveName() } satisfies RenameTarget))}
            />
            <MenuRow icon={<Archive size={iconSize()} />} label={archiveLabel()} onSelect={run(() => archiveSession(hostId, card.session_id))} />
            <MenuRow icon={<ArchiveRestore size={iconSize()} />} label="Mark as unread" onSelect={run(() => markUnread(hostId, card.session_id))} />
            <MenuSeparator />
            <MenuRow icon={<GitFork size={iconSize()} />} label="Fork" onSelect={run(() => forkSession(hostId, card.session_id))} />
            <MenuRow icon={<Copy size={iconSize()} />} label="Copy session id" onSelect={run(() => copyFromRow('id', card))} />
            <MenuRow icon={<Folder size={iconSize()} />} label="Copy path" disabled={!card.cwd} onSelect={run(() => copyFromRow('path', card))} />
            <MenuRow icon={<FileText size={iconSize()} />} label="Copy log path" disabled={!card.file} onSelect={run(() => copyFromRow('log', card))} />
            <MenuRow icon={<FolderOpen size={iconSize()} />} label="Reveal folder" disabled={!card.cwd} onSelect={run(() => revealFolder(hostId, card.cwd))} />
            <MenuRow icon={<Share size={iconSize()} />} label="Export…" onSelect={run(() => { sessions().ensure(hostId, card.session_id); return exportSession(hostId, card.session_id); })} />
            <MenuSeparator />
            <Show
              when={!confirmDelete()}
              fallback={
                <div class="srow-confirm" role="group" aria-label={`Confirm deleting ${title()}`} data-testid="srow-delete-confirm">
                  <p class="srow-confirm-prompt">Delete “{title()}”?</p>
                  <div class="srow-confirm-actions">
                    <button type="button" role="menuitem" class="srow-confirm-cancel" onClick={() => setConfirmDelete(false)}>
                      Cancel
                    </button>
                    <button
                      type="button"
                      role="menuitem"
                      class="srow-confirm-yes"
                      onClick={run(() => deleteSession(hostId, card.session_id))}
                    >
                      <Trash2 size={14} />
                      Yes, delete
                    </button>
                  </div>
                </div>
              }
            >
              <MenuRow icon={<Trash2 size={iconSize()} />} label="Delete" tone="danger" onSelect={() => setConfirmDelete(true)} />
            </Show>
          </AnchoredMenu>
        );
      }}
    </Show>
  );
}
