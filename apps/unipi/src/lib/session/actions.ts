/**
 * Session actions (app spec §3.14, WP-A5; Z-13, Z-15, Z-16, Z-19, Z-20): what
 * the row menu and the Archived list do, as plain functions over the host seam.
 *
 *   setPinned(host, session, true)            → `session_meta_set{pinned}` (Z-14)
 *   renameSession(host, session, "Triage")    → `session_rename` + the title lock (Z-15)
 *   archiveSession(host, session)             → `session_archive` (Z-13)
 *   restoreSession / deleteArchived / emptyTrash → §3.14's archive list (Z-13)
 *   forkSession(host, session, entryId?)      → `session_fork`, then the new chat (Z-16/Z-17)
 *   revealFolder(host, cwd)                   → the Files pane rooted at the project (Z-16)
 *   moveToWorkspace(host, session, cwd)       → `session_move`: a scratch chat moves to a folder (one way)
 *
 * The row menu's open state lives here too (one menu app-wide, anchored to the
 * row that asked) so the sidebar and the phone Chat list share it.
 */
import { createSignal } from 'solid-js';
import { appNavigate } from '../nav';
import { copyText } from '../board/share';
import { setFilesTarget } from '../filesTarget';
import { hostClient } from '../host/factory';
import { desktopShell, layout } from '../layout';
import { panes } from '../panes';
import type { SessionCard } from '../host/client';
import { forkFrom, showActionNotice } from '../timeline/actions';
import { toasts } from '../toasts';
import { hubs } from './hub';
import { sessions } from './store';
import { sessionGroups } from './selectors';
import { sessionHref } from '../../components/shell/Sidebar';
import { currentSessionRoute } from '../../components/shell/AppShell';

/** The row whose context menu is open (right-click / long-press). */
export type RowMenuState = { hostId: string; card: SessionCard; anchor: DOMRect };
const [rowMenu, setRowMenu] = createSignal<RowMenuState | undefined>();
export const openRowMenuState = rowMenu;
export function openRowMenu(hostId: string, card: SessionCard, anchor: DOMRect): void {
  setRowMenu({ hostId, card, anchor });
}
export function closeRowMenu(): void {
  setRowMenu(undefined);
}

const hubOf = (hostId: string) => hubs().ensure(hostId);

/** Reports a failed op in the notice line (the menu has already closed). */
function failed(error: unknown): void {
  showActionNotice(String((error as { message?: unknown } | null)?.message ?? error));
}

export async function setPinned(hostId: string, sessionId: string, pinned: boolean): Promise<void> {
  await hubOf(hostId).setMeta(sessionId, { pinned });
}

/** Z-16 "Mark as unread": the dot comes back (the host clears it again on focus). */
export async function markUnread(hostId: string, sessionId: string): Promise<void> {
  await hubOf(hostId).setMeta(sessionId, { unread: true });
  showActionNotice('Marked as unread');
}

/** Z-15: a manual name wins over auto-rename (`title_locked`; the utility extension skips it). */
export async function renameSession(hostId: string, sessionId: string, name: string, opts?: { notify?: boolean }): Promise<void> {
  const trimmed = name.trim();
  if (!trimmed) return;
  const hub = hubOf(hostId);
  try {
    await hostClient(hostId).request('session_rename', { session: sessionId, name: trimmed });
    hub.patchCard(sessionId, { name: trimmed });
    await hub.setMeta(sessionId, { titleLock: true });
    if (opts?.notify) showActionNotice(`Renamed to “${trimmed.slice(0, 60)}”`);
  } catch (error) {
    if (opts?.notify) failed(error);
    else throw error;
  }
}

/** Z-13: archive moves the session's file to the host's trash; the Archived list restores it. */
export async function archiveSession(hostId: string, sessionId: string): Promise<void> {
  // Leave the shown chat BEFORE closing/archiving it: otherwise the mounted route
  // immediately re-runs `session_open`, hitting `E_TRANSPORT: backend closed` / `E_NOT_FOUND`
  // or re-emitting `SessionChanged` on a file-less chat and resurrecting its sidebar row.
  leaveShownSession(hostId, new Set([sessionId]));
  sessions().close(hostId, sessionId);
  const hub = hubOf(hostId);
  await hub.archive(sessionId);
  if (hub.state.error) {
    failed(hub.state.error);
    return;
  }
  showActionNotice('Archived · restore it from Archived');
}

/**
 * [ui-sessions] "Archive all" (list header / group header, after a confirm): archives every
 * listed chat in ONE host op ([ux-sessions] `sessions_archive` — was one request per chat, four
 * at a time, which lagged for minutes on 2,000 chats and left the Archived count stale). The rows
 * leave the list at once; `hub.state.bulk` carries the host's progress; one toast with **Undo**
 * (`sessions_restore` of exactly those).
 */
export async function archiveMany(hostId: string, sessionIds: readonly string[], what = 'sessions'): Promise<string[]> {
  if (!sessionIds.length) return [];
  leaveShownSession(hostId, new Set(sessionIds));
  for (const id of sessionIds) sessions().close(hostId, id);
  const hub = hubOf(hostId);
  const result = await hub.archiveMany(sessionIds);
  const done = result.done;
  const failedCount = sessionIds.length - done.length;
  const n = done.length;
  const label = `Archived ${n} ${n === 1 ? 'session' : what}${failedCount ? ` · ${failedCount} not archived` : ''}`;
  if (n > 0) {
    toasts().push(label, failedCount ? 'warning' : 'success', {
      label: 'Undo',
      run: async () => {
        const back = await hub.restoreMany(done);
        showActionNotice(back.done.length === n ? `Restored ${n} ${n === 1 ? 'session' : 'sessions'}` : `Restored ${back.done.length} of ${n}`);
      },
    });
  } else {
    showActionNotice(
      result.error
        ? `Could not archive: ${result.error.message}`
        : `Could not archive ${sessionIds.length === 1 ? 'the session' : `${sessionIds.length} sessions`}${result.failed[0] ? ` (${result.failed[0].message})` : ''}`,
    );
  }
  return done;
}

export async function restoreSession(hostId: string, sessionId: string): Promise<void> {
  sessions().close(hostId, sessionId);
  await hubOf(hostId).restore(sessionId);
  showActionNotice('Restored');
}

export async function deleteArchived(hostId: string, sessionId: string): Promise<void> {
  await hubOf(hostId).deleteArchived(sessionId);
  showActionNotice('Deleted');
}

export async function emptyTrash(hostId: string): Promise<void> {
  const deleted = await hubOf(hostId).emptyTrash();
  showActionNotice(deleted > 0 ? `Deleted ${deleted} archived ${deleted === 1 ? 'session' : 'sessions'}` : 'Nothing to delete');
}

/**
 * [owner-fix-8] The chat to show when the chat on screen is deleted: the next
 * one in the sidebar order (the neighbour below, else the one above), on this
 * PC first and then on any other PC with chats. `undefined` when no chat is
 * left anywhere — the empty state is then the only place to go.
 */
export function nextChatAfter(
  hostId: string,
  sessionId: string,
  exclude?: ReadonlySet<string>,
): { hostId: string; sessionId: string } | undefined {
  const skip = (id: string) => id === sessionId || (exclude?.has(id) ?? false);
  const of = (id: string): { hostId: string; sessionId: string } | undefined => {
    const hub = hubs().peek(id);
    if (!hub) return undefined;
    const groups = sessionGroups(hub.state.cards, hub.state.meta);
    const cards = [...groups.pinned, ...groups.projects.flatMap((group) => group.cards)];
    const at = cards.findIndex((card) => card.session_id === sessionId);
    const after = at >= 0 ? cards.slice(at + 1).find((c) => !skip(c.session_id)) : undefined;
    const before =
      at > 0
        ? cards
            .slice(0, at)
            .reverse()
            .find((c) => !skip(c.session_id))
        : undefined;
    const next = after ?? before ?? cards.find((card) => !skip(card.session_id));
    return next ? { hostId: id, sessionId: next.session_id } : undefined;
  };
  return of(hostId) ?? hubs().keys().filter((id) => id !== hostId).map(of).find(Boolean);
}

function leaveShownSession(hostId: string, removing: ReadonlySet<string>): void {
  const shown = currentSessionRoute();
  if (!shown || shown.hostId !== hostId || !removing.has(shown.sessionId)) return;
  const next = nextChatAfter(hostId, shown.sessionId, removing);
  if (next) appNavigate(sessionHref(next.hostId, next.sessionId));
  else appNavigate(`/new/${encodeURIComponent(hostId)}`);
}

/**
 * Z-20: Delete from a live row. The host only unlinks trash entries, so a live
 * session is archived first — the same move Archive does — and then removed.
 *
 * [owner-fix-8] Deleting the chat that is on screen leaves the app on the next
 * chat: the route still pointed at the deleted one, so the "this chat is gone"
 * handler took over and opened a new chat. Deleting any other chat changes
 * nothing.
 */
export async function deleteSession(hostId: string, sessionId: string): Promise<void> {
  // Leave the deleted chat before it is gone: the route must not point at it.
  leaveShownSession(hostId, new Set([sessionId]));
  sessions().close(hostId, sessionId);
  const hub = hubOf(hostId);
  await hub.archive(sessionId);
  if (hub.state.error) {
    failed(hub.state.error);
    return;
  }
  await hub.deleteArchived(sessionId);
  showActionNotice('Deleted');
}

/** Z-16 "Copy id" / "Copy path" / "Copy log path". */
export async function copyFromRow(what: 'id' | 'path' | 'log', card: SessionCard): Promise<void> {
  const text = what === 'id' ? card.session_id : what === 'path' ? (card.cwd ?? '') : (card.file ?? '');
  const ok = await copyText(text);
  showActionNotice(ok ? (what === 'id' ? 'Session id copied' : what === 'path' ? 'Path copied' : 'Log path copied') : 'Could not copy');
}

/**
 * Z-16 "Fork": `session_fork` opens the fork as a live stream (host.md §4.2);
 * `forkFrom` takes its id from the first snapshot, hangs up, and the app shows
 * the new chat.
 */
export async function forkSession(hostId: string, sessionId: string, entryId?: string): Promise<void> {
  try {
    await forkFrom(hostId, sessionId, entryId, (id) => {
      showActionNotice('Forked');
      appNavigate(sessionHref(hostId, id));
    });
  } catch (error) {
    failed(error);
  }
}

/** Z-16 "Reveal folder": the Files surface rooted at the session's project —
 *  the desktop's side-pane Files tab (its scope), the phone's `/files` layer
 *  (both consume the same `{hostId, root}` target). */
export function revealFolder(hostId: string, cwd: string | undefined): void {
  if (!cwd) return;
  if (desktopShell()) {
    panes().open({ hostId, projectKey: cwd }, 'side', { sessionId: undefined }, { kind: 'files', params: { hostId, root: cwd } });
    layout().toggleSide(true);
    showActionNotice('Revealed in Files');
    return;
  }
  setFilesTarget({ hostId, path: cwd });
  appNavigate('/files');
}

/**
 * "Move to a workspace…": a No-workspace chat (its private scratch folder) runs in `cwd` from its
 * next turn on — one way only (the host refuses anything else). The card the host answers with
 * regroups the chat in the sidebar at once; the hub stream confirms it.
 */
export async function moveToWorkspace(
  hostId: string,
  sessionId: string,
  cwd: string,
  opts: { copyFiles?: boolean } = {},
): Promise<{ cwd: string; copied: number; skipped: number }> {
  const reply = (await hostClient(hostId).request('session_move', { session: sessionId, cwd, copyFiles: !!opts.copyFiles })) as
    | { card?: Partial<SessionCard>; copied?: number; skipped?: number }
    | null
    | undefined;
  const moved = typeof reply?.card?.cwd === 'string' && reply.card.cwd ? reply.card.cwd : cwd;
  hubOf(hostId).patchCard(sessionId, { cwd: moved });
  return { cwd: moved, copied: reply?.copied ?? 0, skipped: reply?.skipped ?? 0 };
}

export { sessionHref };
