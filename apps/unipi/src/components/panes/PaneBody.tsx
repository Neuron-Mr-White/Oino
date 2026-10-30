/**
 * The tab bodies of a pane (WP-A3): a tab mounts the first time it is shown
 * and stays mounted while it is in the view (browsers, terminals and editors
 * keep their state when you switch tabs); each body sits in its own
 * `ErrorBoundary` (Z-89) so one crashing tab never takes the shell down.
 */
import { createEffect, createSignal, ErrorBoundary, For, Show } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { TriangleAlert } from 'lucide-solid';
import type { PaneArea, PaneScope, PaneTab } from '../../lib/panes';
import { paneKind, tabTitle } from '../../lib/panes/kinds';
import { EmptyState } from '../EmptyState';

export function TabCrashed(props: { title: string; error: unknown; reset: () => void }) {
  const message = () => (props.error instanceof Error ? props.error.message : String(props.error));
  return (
    <EmptyState
      compact
      tone="error"
      icon={<TriangleAlert size={22} />}
      title={`${props.title} stopped working`}
      body={message()}
      testId="tab-crashed"
      action={
        <button type="button" class="btn btn-secondary btn-sm" onClick={() => props.reset()}>
          Reload tab
        </button>
      }
    />
  );
}

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
  const mounted = () => props.tabs.filter((t) => seen().has(t.id) || t.id === props.activeId);
  return (
    <div class="pane-body" data-hidden={props.hidden ? '' : undefined}>
      <For each={mounted()}>
        {(tab) => {
          const kind = () => paneKind(tab.kind);
          return (
            <div class="pane-tab-body" role="tabpanel" aria-label={tabTitle(tab)} hidden={tab.id !== props.activeId} data-kind={tab.kind}>
              <ErrorBoundary fallback={(error, reset) => <TabCrashed title={tabTitle(tab)} error={error} reset={reset} />}>
                <Show
                  when={kind()?.component}
                  fallback={<EmptyState compact title={`${tabTitle(tab)} isn't available`} body="This build can't show this tab. Close it, or update UniPi." testId="tab-unavailable" />}
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
              </ErrorBoundary>
            </div>
          );
        }}
      </For>
    </div>
  );
}
