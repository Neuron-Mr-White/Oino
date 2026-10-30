/**
 * Command-palette entries (WP-U10; ui-protocol §14 U10 row): one
 * "Open <title>" command per `sidePane` / `bottomPane` / `fullscreen` entry
 * of the open session, in A18's command registry (`lib/shell.ts`).
 * sidePane → its side-pane tab (pane opens); bottomPane → its bottom-panel
 * tab; fullscreen → the `/x/<entryId>` page. Phone: the pane entries open
 * the full-screen pane layer instead (§13.6 row 6).
 */
import { createEffect, onCleanup } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { layout } from '../../lib/layout';
import { panes } from '../../lib/panes';
import { registerCommands } from '../../lib/shell';
import { projectKeyOf } from '../../components/shell/Workspace';
import { feedExtensionPages } from './pages';
import { openPhonePane, wireExtPaneKind } from './paneTabs';
import type { UiRegistry } from '../registry';
import type { UiEntry } from '../types';

export function feedPaletteEntries(opts: { hostId: string; sessionId: string; cwd: () => string | undefined; registry: () => UiRegistry | undefined }): void {
  const navigate = useNavigate();
  wireExtPaneKind();
  createEffect(() => {
    const registry = opts.registry();
    const scope = { hostId: opts.hostId, projectKey: projectKeyOf(opts.cwd()) };
    const view = { sessionId: opts.sessionId };
    const entries = registry ? [...registry.entriesFor('sidePane')(), ...registry.entriesFor('bottomPane')(), ...registry.entriesFor('fullscreen')()] : [];
    const desktop = layout().desktop();
    const offs = entries.map((entry) =>
      registerCommands([
        {
          id: `open-entry:${entry.id}`,
          title: `Open ${entry.title ?? entry.id}`,
          group: 'Panels',
          keywords: `panel open ${entry.extensionId}`,
          run: () => openEntry(entry, { desktop, navigate: () => navigate(`/x/${encodeURIComponent(entry.id)}`), scope, view }),
        },
      ]),
    );
    onCleanup(() => offs.forEach((off) => off()));
  });
}

/** Where one entry's "Open" goes. */
export function openEntry(entry: UiEntry, ctx: { desktop: boolean; navigate: () => void; scope: { hostId: string; projectKey: string }; view: { sessionId?: string } }): void {
  if (entry.slot === 'fullscreen') {
    ctx.navigate();
    return;
  }
  if (!ctx.desktop) {
    openPhonePane(entry);
    return;
  }
  const area = entry.slot === 'bottomPane' ? ('bottom' as const) : ('side' as const);
  const store = panes();
  const title = entry.title ?? entry.extensionId;
  store.open(ctx.scope, area, ctx.view, { kind: 'ext', params: { entryId: entry.id, hostId: ctx.scope.hostId, sessionId: ctx.view.sessionId }, title });
  if (area === 'side') layout().toggleSide(true);
  else layout().toggleBottom(true);
}

/** Re-exported for the conversation's install (one call feeds pages too). */
export { feedExtensionPages };
