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
import { sessionHref } from '../../components/shell/Sidebar';

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
  sessions().close(hostId, sessionId);
  const hub = hubOf(hostId);
  await hub.archive(sessionId);
  if (hub.state.error) {
    failed(hub.state.error);
    return;
  }
  showActionNotice('Archived · restore it from Archived');
}

/** Runs `fn` over `items`, `limit` at a time; resolves the ones that did not throw. */
async function pool<T>(items: readonly T[], limit: number, fn: (item: T) => Promise<boolean>): Promise<T[]> {
  const ok: T[] = [];
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const item = items[next++]!;
      try {
        if (await fn(item)) ok.push(item);
      } catch {
        // counted as not done
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return ok;
}

/**
 * [ui-sessions] "Archive all" (list header / group header, after a confirm): archives every
 * listed chat, four at a time, then one toast with **Undo** (restores exactly those).
 */
export async function archiveMany(hostId: string, sessionIds: readonly string[], what = 'sessions'): Promise<string[]> {
  if (!sessionIds.length) return [];
  const client = hostClient(hostId);
  const done = await pool(sessionIds, 4, async (id) => {
    sessions().close(hostId, id);
    await client.request('session_archive', { session: id });
    return true;
  });
  const hub = hubOf(hostId);
  // The host announces each removal on the hub stream; a reload settles anything missed.
  void hub.reload();
  const failedCount = sessionIds.length - done.length;
  const n = done.length;
  const label = `Archived ${n} ${n === 1 ? 'session' : what}${failedCount ? ` · ${failedCount} failed` : ''}`;
  if (n > 0) {
    toasts().push(label, failedCount ? 'warning' : 'success', {
      label: 'Undo',
      run: async () => {
        const back = await pool(done, 4, async (id) => {
          await client.request('session_restore', { session: id });
          return true;
        });
        void hub.reload();
        void hub.loadArchived(true);
        showActionNotice(back.length === n ? `Restored ${n} ${n === 1 ? 'session' : 'sessions'}` : `Restored ${back.length} of ${n}`);
      },
    });
  } else {
    showActionNotice(`Could not archive ${sessionIds.length === 1 ? 'the session' : `${sessionIds.length} sessions`}`);
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
 * Z-20: Delete from a live row. The host only unlinks trash entries, so a live
 * session is archived first — the same move Archive does — and then removed.
 */
export async function deleteSession(hostId: string, sessionId: string): Promise<void> {
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
