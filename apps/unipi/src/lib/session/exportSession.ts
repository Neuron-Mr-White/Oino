/**
 * Export a chat (WP-A25, Z-20 + G-15 `/export`): `_oino/export_jsonl` on the
 * session writes the branch as pi's session JSONL and answers the path; the
 * app says where it went. One surface for the ⋯ row, `/export` and the
 * palette (DIRECTION §4.2.2) — a richer export dialog can replace the ⋯
 * action later without touching the command.
 */
import { sessions } from './store';
import { showActionNotice } from '../timeline/actions';

/** `_oino/export_jsonl`'s reply (`{path}`). */
export type ExportReply = { path?: string };

/** Writes the chat's JSONL export; false when the chat is gone or the host refused. */
export async function exportSession(hostId: string, sessionId: string): Promise<boolean> {
  const handle = sessions().peek(hostId, sessionId);
  if (!handle) {
    showActionNotice('Open the chat first');
    return false;
  }
  try {
    const reply = (await handle.cmd({ type: '_oino/export_jsonl' })) as ExportReply | undefined;
    const path = reply?.path;
    showActionNotice(path ? `Exported to ${path}` : 'Exported the session');
    return true;
  } catch (error) {
    showActionNotice(`Could not export: ${String((error as { message?: unknown })?.message ?? error)}`);
    return false;
  }
}
