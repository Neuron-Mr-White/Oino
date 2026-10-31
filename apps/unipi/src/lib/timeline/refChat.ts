/**
 * `RefChat` adapter for the v2 session store (WP-A6): file refs, image
 * chips and markdown hydration were built against the v1 `Chat`; the
 * plumbing now takes the structural `RefChat` and the timeline hands it
 * this adapter over a `SessionHandle`.
 *
 * - `statPaths` verifies paths with the host `fs_stat` op (the v1 bridge's
 *   `paths_stat` batch op has no session-protocol op yet; resolves relative
 *   paths against the session cwd — enough for the timeline's mentions and
 *   prose refs until A7/A8 revisit batching).
 * - `media`/`shareFile` are best-effort session commands (`media`,
 *   `file_share`); images with inline `data` never call `media`, and a
 *   share that the host doesn't know simply fails the preview retry.
 *
 * One adapter per session: `pathStat` caches answers per *object identity*
 * (WeakMap), so keep it stable for the session's lifetime.
 */
import type { PathStatItem } from '../chat/store';
import type { RefChat } from '../chat/pathStat';
import { hostClient } from '../host/factory';
import { resolvePathToken } from '../media/pathTokens';
import type { SessionHandle } from '../session/store';

export function sessionRefChat(handle: SessionHandle, cwd: () => string | undefined): RefChat {
  const client = () => hostClient(handle.hostId);
  return {
    hostId: handle.hostId,
    get session() {
      return handle.sessionId();
    },
    get state() {
      return { conn: handle.state.conn };
    },
    cwd: () => cwd() ?? '',
    async statPaths(paths) {
      if (!paths.length) return [];
      const base = cwd() || undefined;
      const items = await Promise.all(
        paths.map(async (path): Promise<PathStatItem | undefined> => {
          const resolved = resolvePathToken(path.replace(/^@/, ''), base);
          try {
            const stat = await client().request<{ kind?: string } | null>('fs_stat', { path: resolved });
            const kind = stat?.kind === 'dir' ? 'dir' : 'file';
            return { path, resolved, kind };
          } catch {
            return { path, resolved, kind: 'missing' };
          }
        }),
      );
      return items.filter((i): i is PathStatItem => !!i);
    },
    async media(mediaRef, mime) {
      const reply = await handle.cmd<{ mime?: string; dataUrl?: string }>({ type: 'media', mediaRef });
      if (!reply?.dataUrl) throw new Error('No image data');
      return { mime: reply.mime ?? mime, dataUrl: reply.dataUrl };
    },
    async shareFile(path) {
      await client().request('file_share', { path });
    },
  };
}
