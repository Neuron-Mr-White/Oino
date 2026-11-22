/**
 * The bottom panel (§3.8, WP-A3): the same tab model as the side pane for
 * `bottomPane` slot entries (U10) and terminal tabs (wave 3, A31), under the
 * conversation. Cmd/Ctrl+J toggles it; closing the last tab hides it; its
 * height is remembered (resize handle on its top edge, in AppShell).
 */
import { createComputed, createRoot, createSignal, on, onMount, Show } from 'solid-js';
import { PanelBottomClose } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { panes, type PaneScope, type PaneTab, type PaneView } from '../../lib/panes';
import { confirmCloseMany } from './tabs/terminalClose';
import { launchableKinds, paneKind, tabTitle } from '../../lib/panes/kinds';
import { modKey } from '../../lib/shell';
import { PaneBody } from './PaneBody';
import { LauncherList, PaneLauncher, type LaunchSpec } from './PaneLauncher';
import { TabOverview } from './TabOverview';
import { TabStrip } from './TabStrip';

/** When the user last opened the panel (Cmd/Ctrl+J, the header's Terminal button). */
let openedAt = -Infinity;
createRoot(() => createComputed(on(() => layout().bottomOpen(), (open, prev) => {
  if (open && prev === false) openedAt = performance.now();
})));

export function BottomPanel(props: { scope: PaneScope; view: PaneView; localHost: boolean }) {
  const store = panes();
  const tabs = () => store.visible(props.scope, 'bottom', props.view);
  const active = () => store.active(props.scope, 'bottom', props.view);
  const [launcher, setLauncher] = createSignal<DOMRect | undefined>();
  const [overview, setOverview] = createSignal<DOMRect | undefined>();
  const close = (id: string) => {
    const tab = tabs().find((t) => t.id === id);
    if (tab && paneKind(tab.kind)?.canUserClose?.(tab, () => close(id)) === false) return;
    if (tab) paneKind(tab.kind)?.onUserClose?.(tab);
    store.close(props.scope, 'bottom', props.view, id);
    if (tabs().length === 0) layout().toggleBottom(false);
  };
  /** Close others / Close all: one ask for every busy terminal (host-reported), then each closed tab's own end (a terminal's shell). */
  const closeMany = (victims: PaneTab[], done: () => void) =>
    void confirmCloseMany(
      victims.map((tab) => ({ tab, title: tabTitle(tab) })),
      () => {
        for (const tab of victims) paneKind(tab.kind)?.onUserClose?.(tab);
        done();
      },
    );
  const pick = (spec: LaunchSpec) => void store.open(props.scope, 'bottom', props.view, spec);
  // Cmd/Ctrl+J or the header's Terminal button on an empty panel: a terminal in the session's folder
  // (only when the user just opened it — not when a remembered-open panel mounts on navigation).
  onMount(() => {
    if (performance.now() - openedAt < 1000 && tabs().length === 0 && launchableKinds('bottom', !!props.view.sessionId).some((k) => k.kind === 'terminal')) pick({ kind: 'terminal' });
  });
  return (
    <section class="bottom-panel" aria-label="Bottom panel" data-testid="bottom-panel">
      <TabStrip
        label="Bottom panel tabs"
        testId="bottom-strip"
        tabs={tabs()}
        activeId={active()?.id}
        badge={(t) => t.badge}
        onActivate={(id) => store.activate(props.scope, 'bottom', props.view, id)}
        onClose={close}
        onMove={(id, before) => store.move(props.scope, 'bottom', id, before)}
        onLauncher={setLauncher}
        onOverview={setOverview}
        tools={
          <button type="button" class="shell-icon-btn tab-strip-btn" aria-label="Hide panel" title={`Hide panel (${modKey()} J)`} onClick={() => layout().toggleBottom(false)} data-testid="hide-bottom">
            <PanelBottomClose size={16} />
          </button>
        }
      />
      <Show
        when={tabs().length > 0}
        fallback={
          <div class="pane-empty" data-testid="bottom-empty">
            <LauncherList inline area="bottom" emptyText="Terminals and extension panels open here." hasSession={!!props.view.sessionId} localHost={props.localHost} onPick={pick} />
          </div>
        }
      >
        <PaneBody
          area="bottom"
          scope={props.scope}
          sessionId={props.view.sessionId}
          tabs={tabs()}
          activeId={active()?.id}
          onClose={close}
          onTitle={(id, title) => store.retitle(props.scope, 'bottom', id, title)}
        />
      </Show>
      <Show when={launcher()}>
        {(a) => <PaneLauncher anchor={a()} area="bottom" hasSession={!!props.view.sessionId} localHost={props.localHost} onPick={pick} onDismiss={() => setLauncher(undefined)} />}
      </Show>
      <Show when={overview()}>
        {(a) => (
          <TabOverview
            anchor={a()}
            tabs={tabs()}
            activeId={active()?.id}
            closed={store.recentlyClosed(props.scope, 'bottom', props.view)}
            onActivate={(id) => store.activate(props.scope, 'bottom', props.view, id)}
            onClose={close}
            onCloseOthers={(id) => closeMany(tabs().filter((t) => t.id !== id), () => store.closeOthers(props.scope, 'bottom', props.view, id))}
            onCloseAll={() =>
              closeMany(tabs(), () => {
                store.closeAll(props.scope, 'bottom', props.view);
                layout().toggleBottom(false);
              })
            }
            onReopen={(id) => store.reopen(props.scope, 'bottom', props.view, id)}
            onDismiss={() => setOverview(undefined)}
          />
        )}
      </Show>
    </section>
  );
}
