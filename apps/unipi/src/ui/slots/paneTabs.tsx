/**
 * `sidePane` / `bottomPane` slot hosts (WP-U10, G-1; ui-protocol §6.2):
 * registry entries become tabs of the Z-5 tabs container (A3's pane store,
 * kind `ext`, project scope of the session's folder). The reconciler mirrors
 * registry → tabs: a new entry opens a tab, `meta` updates retitle / badge
 * it, `focus` activates it (§6.5), a deleted entry closes its tab (§6.6) and
 * the tab's ✕ sends the panel `$close` (via the kind's `onUserClose`).
 * A user-closed tab is not re-opened while the entry lives on (`id:seq`
 * pairs; a reopen assigns a new `seq`, §6.4).
 *
 * Phone (§13.6 row 6): there is no side pane — the entries list in the chat
 * ⋯ "Panes" group and open as full-screen layers (`PhonePaneLayer`).
 */
import { batch, createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { ArrowLeft, Puzzle } from 'lucide-solid';
import { pushBack } from '../../lib/back';
import { layout } from '../../lib/layout';
import { panes, type PaneScope, type PaneTab, type PaneView } from '../../lib/panes';
import { paneKind, registerPaneKind, type PaneTabProps } from '../../lib/panes/kinds';
import { projectKeyOf } from '../../components/shell/Workspace';
import { MenuRow, MenuSeparator } from '../../components/board/AnchoredMenu';
import { EntryView } from '../NodeView';
import { ICONS } from '../nodes/icons';
import { plainLine } from '../nodes/Text';
import { uiTransport } from '../actions';
import { findUiEntry } from '../registries';
import type { UiRegistry } from '../registry';
import type { UiTarget } from '../actions';
import type { Slot, UiEntry } from '../types';

/** Tabs the user closed while their entry lives on (`id:seq` pairs). */
const userClosed = new Set<string>();
export const resetPaneClosures = () => userClosed.clear();

export const extTargetOf = (tab: PaneTab): UiTarget | undefined => {
  const p = tab.params ?? {};
  const hostId = typeof p.hostId === 'string' ? p.hostId : undefined;
  const sessionId = typeof p.sessionId === 'string' ? p.sessionId : undefined;
  return hostId ? { scope: 'session', hostId, sessionId } : undefined;
};

const entryOf = (target: UiTarget, id: string): UiEntry | undefined => findUiEntry(target, id);

/** The `ext` pane tab: one registry entry's view in the tabs container. */
export function ExtPaneTab(props: PaneTabProps) {
  const target = () => extTargetOf(props.tab);
  const entry = () => {
    const t = target();
    const id = typeof props.tab.params?.entryId === 'string' ? props.tab.params.entryId : '';
    return t && id ? entryOf(t, id) : undefined;
  };
  return (
    <Show when={entry()} keyed fallback={<div class="ui-pane-missing">This panel closed.</div>}>
      {(e) => <EntryView entry={e} target={{ scope: 'session', hostId: props.tab.params?.hostId as string, sessionId: props.tab.params?.sessionId as string | undefined }} mode="auto" />}
    </Show>
  );
}

/** Registers the `ext` kind's view + close hook (idempotent). */
let kindWired = false;
export function wireExtPaneKind(): void {
  if (kindWired) return;
  kindWired = true;
  const def = paneKind('ext');
  if (!def) return;
  registerPaneKind({ ...def, component: ExtPaneTab, onUserClose: extPaneUserClosed });
}

/**
 * Mirrors the registry's `sidePane`/`bottomPane` entries into tabs. Runs in
 * the caller's reactive scope (Conversation); converges idempotently.
 */
export function reconcilePaneTabs(opts: { hostId: string; sessionId: string; cwd: () => string | undefined; registry: () => UiRegistry | undefined }) {
  wireExtPaneKind();
  const view: PaneView = { sessionId: opts.sessionId };
  const store = panes();

  for (const area of ['side', 'bottom'] as const) {
    const slot: Slot = area === 'side' ? 'sidePane' : 'bottomPane';
    // The focus entry shown last (a new one re-shows the pane once, §6.5).
    let lastFocus = '';
    createEffect(() => {
      const registry = opts.registry();
      const scope: PaneScope = { hostId: opts.hostId, projectKey: projectKeyOf(opts.cwd()) };
      const entries = registry ? registry.entriesFor(slot)() : [];
      const tabs = store.visible(scope, area, view).filter((t) => t.kind === 'ext' && t.params?.hostId === opts.hostId && t.params?.sessionId === opts.sessionId);
      const byEntry = new Map(tabs.map((t) => [String(t.params?.entryId ?? ''), t]));
      let opened = false;
      batch(() => {
        for (const e of entries) {
          if (userClosed.has(`${e.id}:${e.seq}`)) continue;
          const title = e.title ?? e.extensionId;
          const badge = plainLine(e.badge) || undefined;
          const tab = byEntry.get(e.id);
          if (!tab) {
            store.open(scope, area, view, { kind: 'ext', params: { entryId: e.id, hostId: opts.hostId, sessionId: opts.sessionId }, title });
            opened = true;
          } else {
            if (tab.title !== title) store.retitle(scope, area, tab.id, title);
            if ((tab.badge ?? undefined) !== badge) store.badge(scope, area, tab.id, badge);
            if (e.focus && store.active(scope, area, view)?.id !== tab.id) store.activate(scope, area, view, tab.id);
          }
        }
        for (const [id, tab] of byEntry) {
          if (!entries.some((e) => e.id === id)) store.close(scope, area, view, tab.id);
        }
      });
      // Show the pane once for a new tab or a newly focused entry — never on
      // every effect run (the layout writes re-trigger this effect).
      const focus = entries.find((e) => e.focus && !userClosed.has(`${e.id}:${e.seq}`));
      const focusKey = focus ? `${focus.id}:${focus.seq}` : '';
      if (opened || (focus && focusKey !== lastFocus)) {
        lastFocus = focusKey;
        if (area === 'side' && layout().desktop()) layout().toggleSide(true);
        if (area === 'bottom') layout().toggleBottom(true);
      }
    });
  }
}

/** The kind's user-close hook: remember it, and tell the extension `$close`. */
export function extPaneUserClosed(tab: PaneTab): void {
  const target = extTargetOf(tab);
  const id = typeof tab.params?.entryId === 'string' ? tab.params.entryId : '';
  const entry = target && id ? entryOf(target, id) : undefined;
  if (entry) {
    userClosed.add(`${entry.id}:${entry.seq}`);
    if (target) uiTransport().action(target, { entryId: entry.id, actionId: '$close' }).catch(() => {});
  }
}

// ── phone: full-screen pane layers from the chat ⋯ "Panes" group ─────────

const [openPane, setOpenPane] = createSignal<{ entryId: string; slot: 'sidePane' | 'bottomPane' } | undefined>();
export const phonePaneOpen = openPane;

/** The phone ⋯ "Panes" rows: the session's sidePane + bottomPane entries. */
export function phonePaneEntries(registry: UiRegistry): UiEntry[] {
  return [...registry.entriesFor('sidePane')(), ...registry.entriesFor('bottomPane')()];
}

export function openPhonePane(entry: UiEntry): void {
  setOpenPane({ entryId: entry.id, slot: entry.slot === 'bottomPane' ? 'bottomPane' : 'sidePane' });
}

/** The phone's full-screen layer over the conversation (§13.6 row 6). */
export function PhonePaneLayer(props: { registry: UiRegistry; target: UiTarget }) {
  // Android Back (and the browser's Back in the web build) closes the layer.
  createEffect(() => {
    if (openPane()) onCleanup(pushBack(() => setOpenPane(undefined)));
  });
  return (
    <Show when={openPane()} keyed>
      {(open) => {
        const entry = () => props.registry.entry(open.entryId);
        const close = () => setOpenPane(undefined);
        return (
          <Show when={entry()} keyed>
            {(e) => (
              <div class="ui-phone-pane" data-testid="ui-phone-pane" data-entry={e.id}>
                <header class="ui-phone-pane-head">
                  <button type="button" class="ghost-icon-button" aria-label="Back to chat" onClick={close}>
                    <ArrowLeft size={20} />
                  </button>
                  <span class="ui-phone-pane-title">{e.title ?? e.id}</span>
                  <span class="flex-1" />
                </header>
                <div class="ui-phone-pane-body">
                  <EntryView entry={e} target={props.target} mode="auto" />
                </div>
              </div>
            )}
          </Show>
        );
      }}
    </Show>
  );
}

/** The ⋯ menu's "Panes" group (phone): every sidePane/bottomPane entry as a row. */
export function PhonePanesGroup(props: { registry: UiRegistry; onOpen: () => void }) {
  const entries = () => phonePaneEntries(props.registry);
  return (
    <Show when={entries().length > 0}>
      <MenuSeparator />
      <For each={entries()}>
        {(e) => {
          const Icon = (e.icon && ICONS[e.icon]) || Puzzle;
          return (
            <MenuRow
              icon={<Icon size={17} />}
              label={e.title ?? e.id}
              onSelect={() => {
                openPhonePane(e);
                props.onOpen();
              }}
            />
          );
        }}
      </For>
    </Show>
  );
}
