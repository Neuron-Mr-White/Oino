/**
 * Duplicate a chat (WP-A25, G-15 `/clone`): a fork **at the leaf** is pi's
 * clone (`session_fork` without `entry_id`, host.md §4.2) — the new session
 * keeps the whole transcript and continues from its end. The ⋯ menu's
 * "Duplicate" row, the `/clone` command and the palette all land here, so the
 * app has one duplicate surface (DIRECTION §4.2.2).
 *
 * `session_fork` answers by keeping the fork as the stream's session: its id
 * arrives in the first snapshot, the stream is dropped, and the app opens it
 * like any other chat.
 */
import { hostClient } from '../host/factory';
import { appNavigate } from '../nav';
import { sessions } from './store';
import { showActionNotice } from '../timeline/actions';
import { chatHref } from '../settings/manage';

/** The fork's id from the first snapshot (or undefined when the host refuses). */
export function forkAtLeaf(hostId: string, sessionId: string): Promise<string | undefined> {
  return new Promise((resolve) => {
    let settled = false;
    const done = (value: string | undefined) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      stream.close();
      resolve(value);
    };
    const stream = hostClient(hostId).openSession(
      { kind: 'fork', session: sessionId },
      {
        snapshot: (snap) => done(snap.session_id),
        events: () => {},
        closed: () => done(undefined),
      },
    );
    const timer = setTimeout(() => done(undefined), 30_000);
  });
}

/** "Duplicate": forks the chat at its leaf and shows the copy. */
export async function duplicateSession(hostId: string, sessionId: string): Promise<boolean> {
  const id = await forkAtLeaf(hostId, sessionId);
  if (!id) {
    showActionNotice('Could not duplicate the chat');
    return false;
  }
  showActionNotice('Duplicated');
  sessions().ensure(hostId, id);
  appNavigate(chatHref(hostId, id));
  return true;
}
