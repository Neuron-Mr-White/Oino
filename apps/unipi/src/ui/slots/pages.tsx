/**
 * `fullscreen` slot host (WP-U10, G-1; ui-protocol §6.2): one sidebar page
 * per entry — a rail nav item under "Extensions" (`registerRailPage`, A3's
 * seam; the phone Chat tab's "More" menu lists the same items) at
 * `/x/<entryId>`, rendering the entry's tree full page. Entries appear and
 * disappear live with the registry (a closed page falls back to the "not
 * available" state); the view is found across the open registries so the
 * page survives navigation inside the app. Cap: one `fullscreen` page per
 * extension (§16.1 default) — the registry rejects more; the feed keeps the
 * first if several arrive.
 */
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import { Puzzle } from 'lucide-solid';
import { registerRailPage, setExtPageView } from '../../components/shell/Rail';
import { PageHeader } from '../../components/PageHeader';
import { EmptyState } from '../../components/EmptyState';
import { EntryView } from '../NodeView';
import { ICONS } from '../nodes/icons';
import { findUiEntryAnywhere, sessionUi } from '../registries';
import { sessions } from '../../lib/session/store';
import type { UiRegistry } from '../registry';
import type { UiEntry } from '../types';

let viewWired = false;

/** The `/x/:entryId` page (plugged into A3's route seam once). */
export function ExtensionPage(props: { entryId: string }) {
  const found = () => findUiEntryAnywhere(props.entryId);
  return (
    <div class="ui-extpage" data-testid="ui-extpage" data-entry={props.entryId}>
      <Show
        when={found()}
        fallback={<EmptyState icon={<Puzzle size={22} />} title="This page isn't available" body="The extension that adds it is off or not installed." testId="ext-page-missing" />}
        keyed
      >
        {({ entry, target }) => (
          <>
            <PageHeader title={entry.title ?? entry.id} data-testid="ui-extpage-title" />
            <div class="ui-extpage-body">
              <EntryView entry={entry} target={target} mode="auto" />
            </div>
          </>
        )}
      </Show>
    </div>
  );
}

/** Registers the `/x/:entryId` view once (the route seam A3 left open). */
function wireExtPageView(): void {
  if (viewWired) return;
  viewWired = true;
  // Solid signals treat a bare function as a thunk — store it wrapped.
  setExtPageView(() => ExtensionPage);
}

/** Registers one rail page per entry (one `fullscreen` page per extension, §16.1). Returns the removers. */
function railPagesOf(entries: UiEntry[]): Array<() => void> {
  const seen = new Set<string>();
  const offs: Array<() => void> = [];
  for (const e of entries) {
    if (seen.has(e.extensionId)) continue;
    seen.add(e.extensionId);
    const Icon = (e.icon && ICONS[e.icon]) || Puzzle;
    offs.push(registerRailPage({ entryId: e.id, label: e.title ?? e.extensionId, icon: Icon, order: e.order }));
  }
  return offs;
}

/** Feeds rail pages from a registry accessor (installed once, reactive to the registry). */
export function feedExtensionPages(registry: () => UiRegistry | undefined): void {
  wireExtPageView();
  createEffect(() => {
    const reg = registry();
    const offs = railPagesOf(reg ? reg.entriesFor('fullscreen')() : []);
    onCleanup(() => offs.forEach((off) => off()));
  });
}

/** The last conversation route seen (the rail outlives the conversation view). */
const [lastSession, setLastSession] = createSignal<{ hostId: string; sessionId: string } | undefined>();

/**
 * The shell-level page feed: a `fullscreen` entry's rail item belongs to its
 * session, so it must stay in the rail while the user is on Settings or a
 * board — the conversation unmounts, the entry does not (§6.2 `fullscreen`,
 * §6.6 lifetime). Fed from the current route, kept while the session is open,
 * dropped when the entry is deleted or the session closes.
 */
export function installExtensionPageFeed(pathname: () => string): void {
  wireExtPageView();
  createEffect(() => {
    const m = /^\/s\/([^/]+)\/([^/]+)/.exec(pathname());
    if (m) setLastSession({ hostId: decodeURIComponent(m[1]!), sessionId: decodeURIComponent(m[2]!) });
  });
  createEffect(() => {
    const target = lastSession();
    // `keys()` is reactive: closing the session takes its pages with it.
    const open = !!target && sessions().keys().includes(`${target.hostId}/${target.sessionId}`);
    const entries = target && open ? sessionUi(target.hostId, target.sessionId).entriesFor('fullscreen')() : [];
    const offs = railPagesOf(entries);
    onCleanup(() => offs.forEach((off) => off()));
  });
}
