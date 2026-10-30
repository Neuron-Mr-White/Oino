/**
 * One install for the conversation's slot feeds (WP-U10): status strip,
 * pane tabs, extension pages, palette entries, the `setTitle` window title
 * and the `setWorkingMessage` working line — everything that mirrors the
 * session registry into app surfaces, torn down with the conversation.
 */
import { createEffect, onCleanup } from 'solid-js';
import { setWindowTitle } from '../../lib/desktop';
import './slots.css';
import type { SessionHandle } from '../../lib/session/store';
import { installUiEffects, publishWorkingMessage } from './effects';
import { feedStatusStrip } from './statusFeed';
import { installExtensionPageFeed } from './pages';
import { feedPaletteEntries } from './paletteFeed';
import { reconcilePaneTabs } from './paneTabs';
import type { UiRegistry } from '../registry';

export { installUiEffects };
export { footerRev, publishWorkingMessage, toastKindOf, workingMessageOf } from './effects';

/**
 * Feeds every conversation surface from the session registry. Runs in the
 * caller's reactive scope (Conversation); re-runs when the registry, session
 * or project folder changes.
 */
export function installConversationSlots(opts: {
  hostId: string;
  sessionId: string;
  cwd: () => string | undefined;
  registry: () => UiRegistry | undefined;
  session: () => SessionHandle | undefined;
}): void {
  installUiEffects();
  // Every feed is installed once and takes identity *accessors*, so a route or
  // project change re-points it from inside its own effect (no nested
  // computations accumulating per re-run, no double-owned disposal).
  feedStatusStrip({ scope: () => `${opts.hostId}/${opts.sessionId}`, registry: opts.registry });
  reconcilePaneTabs({ hostId: opts.hostId, sessionId: opts.sessionId, cwd: opts.cwd, registry: opts.registry });
  feedPaletteEntries({ hostId: opts.hostId, sessionId: opts.sessionId, cwd: opts.cwd, registry: opts.registry });
  // The rail's `fullscreen` pages are fed by the shell (`installExtensionPageFeed`, Rail).

  // §3.2 `setTitle` / `setWorkingMessage` registry meta.
  createEffect(() => {
    const session = opts.session();
    const meta = (opts.registry()?.state.meta ?? {}) as { title?: unknown; workingMessage?: unknown; workingVisible?: unknown };
    const title = typeof meta.title === 'string' && meta.title ? meta.title : undefined;
    if (title) setWindowTitle(title);
    if (!session) return;
    const working = meta.workingVisible !== false && typeof meta.workingMessage === 'string' && meta.workingMessage ? meta.workingMessage : undefined;
    onCleanup(publishWorkingMessage(session.key, working));
  });
}
