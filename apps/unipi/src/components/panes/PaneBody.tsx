/**
 * The tab bodies of a pane (WP-A3): a tab mounts the first time it is shown
 * and stays mounted while it is in the view (browsers, terminals and editors
 * keep their state when you switch tabs); each body sits in its own
 * `PaneTabBoundary` (Z-89) so one crashing tab never takes the shell down.
 */
import { createEffect, createSignal, For, Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import type { PaneArea, PaneScope, PaneTab } from '../../lib/panes';
import { paneKind, tabTitle } from '../../lib/panes/kinds';
import { EmptyState } from '../EmptyState';
import { PaneTabBoundary } from '../ErrorBoundaries';

export function PaneBody(props: {
  area: PaneArea;
  scope: PaneScope;
  sessionId?: string;
  tabs: PaneTab[];
  activeId?: string;
  /** Hide every body (full view shows the chat instead). */
  hidden?: boolean;
  onClose: (id: string) => void;
  onTitle: (id: string, title: string | undefined) => void;
}) {
  // Tabs shown at least once in this view (lazy mount, then kept).
  const [seen, setSeen] = createSignal<ReadonlySet<string>>(new Set());
  createEffect(() => {
    const id = props.activeId;
    if (id && !seen().has(id)) setSeen((s) => new Set([...s, id]));
  });
  // Keyed by tab id: a retitle (a browser tab following its page title) replaces the tab
  // object, and must not remount the view (that would drop its stream / terminal / editor).
  const mounted = () => props.tabs.filter((t) => seen().has(t.id) || t.id === props.activeId).map((t) => t.id);
  const byId = (id: string): PaneTab | undefined => props.tabs.find((t) => t.id === id);
  return (
    <div class="pane-body" data-hidden={props.hidden ? '' : undefined}>
      <For each={mounted()}>
        {(id) => {
          // The live tab (the last known one while a closing tab is being dropped).
          let last = byId(id)!;
          const snap = (): PaneTab => (last = byId(id) ?? last);
          const tab: PaneTab = {
            id,
            get kind() {
              return snap().kind;
            },
            get params() {
              return snap().params;
            },
            get title() {
              return snap().title;
            },
            get badge() {
              return snap().badge;
            },
            get sessionId() {
              return snap().sessionId;
            },
          };
          const kind = () => paneKind(tab.kind);
          return (
            <div class="pane-tab-body" role="tabpanel" aria-label={tabTitle(tab)} hidden={tab.id !== props.activeId} data-kind={tab.kind}>
              <PaneTabBoundary title={tabTitle(tab)}>
                <Show
                  when={kind()?.component}
                  fallback={<EmptyState compact title={`${tabTitle(tab)} isn't available`} body="This build can't show this tab. Close it, or update Unipi." testId="tab-unavailable" />}
                >
                  {(component) => (
                    <Dynamic
                      component={component()}
                      tab={tab}
                      area={props.area}
                      hostId={props.scope.hostId}
                      projectKey={props.scope.projectKey}
                      sessionId={props.sessionId}
                      close={() => props.onClose(tab.id)}
                      setTitle={(title: string | undefined) => props.onTitle(tab.id, title)}
                    />
                  )}
                </Show>
              </PaneTabBoundary>
            </div>
          );
        }}
      </For>
    </div>
  );
}
