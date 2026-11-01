/**
 * The side pane (Z-5, WP-A3): one tabs container right of the conversation
 * for Files, Git + status, Diff, Tree, Side chat, Subagent, Preview,
 * forwarded ports, extension panels (and browser / terminal in wave 3).
 *
 * Modes (RULINGS Z-95): `push` (window ≥ 1280 px, the chat narrows),
 * `overlay` (below that: over the conversation frame with a shadow, a click
 * outside closes it), `full` (Z-6, Cmd/Ctrl+Shift+F: covers the conversation
 * frame; "Chat" becomes the first tab). Closing the last tab hides the pane
 * (and leaves full view).
 */
import { createSignal, Show } from 'solid-js';
import { Maximize2, MessagesSquare, Minimize2, PanelRightClose } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { panes, type PaneScope, type PaneTab, type PaneView } from '../../lib/panes';
import { confirmCloseMany } from './tabs/terminalClose';
import { paneKind, tabTitle } from '../../lib/panes/kinds';
import { modKey } from '../../lib/shell';
import { SidebarReveal } from '../shell/ConversationHeader';
import { PaneBody } from './PaneBody';
import { LauncherList, PaneLauncher, type LaunchSpec } from './PaneLauncher';
import { TabOverview } from './TabOverview';
import { TabStrip } from './TabStrip';
import './ForwardTab';
import '../status/StatusPane';

export type SidePaneMode = 'push' | 'overlay' | 'full';

/** Opens a tab in the side pane and shows the pane. */
export function openSideTab(scope: PaneScope, view: PaneView, spec: LaunchSpec): string {
  const id = panes().open(scope, 'side', view, spec);
  layout().toggleSide(true);
  return id;
}

export function SidePane(props: {
  scope: PaneScope;
  view: PaneView;
  mode: SidePaneMode;
  localHost: boolean;
  /** Full view: the Chat tab is the active one. */
  chatActive: boolean;
  onChat: (active: boolean) => void;
  onNewChat: () => void;
}) {
  const store = panes();
  const tabs = () => store.visible(props.scope, 'side', props.view);
  const active = () => store.active(props.scope, 'side', props.view);
  const [launcher, setLauncher] = createSignal<DOMRect | undefined>();
  const [overview, setOverview] = createSignal<DOMRect | undefined>();
  const full = () => props.mode === 'full';

  const close = (id: string) => {
    const tab = tabs().find((t) => t.id === id);
    if (tab && paneKind(tab.kind)?.canUserClose?.(tab, () => close(id)) === false) return;
    if (tab) paneKind(tab.kind)?.onUserClose?.(tab);
    store.close(props.scope, 'side', props.view, id);
    if (tabs().length === 0) layout().toggleSide(false);
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
  const pick = (spec: LaunchSpec) => {
    store.open(props.scope, 'side', props.view, spec);
    props.onChat(false);
  };
  const activate = (id: string) => {
    store.activate(props.scope, 'side', props.view, id);
    props.onChat(false);
  };

  return (
    <aside class="side-pane" data-mode={props.mode} aria-label="Side pane" data-testid="side-pane">
      <TabStrip
        label="Side pane tabs"
        testId="side-strip"
        tabs={tabs()}
        activeId={active()?.id}
        noneActive={full() && props.chatActive}
        badge={(t) => t.badge}
        onActivate={activate}
        onClose={close}
        onMove={(id, before) => store.move(props.scope, 'side', id, before)}
        onLauncher={setLauncher}
        onOverview={setOverview}
        lead={
          <Show when={full()}>
            <Show when={!layout().sidebarOpen()}>
              <SidebarReveal onNewChat={props.onNewChat} />
            </Show>
            <button type="button" role="tab" class="pane-tab pane-tab-chat" aria-selected={props.chatActive} onClick={() => props.onChat(true)} data-testid="fullview-chat-tab">
              <MessagesSquare size={14} strokeWidth={1.8} class="pane-tab-icon" />
              <span class="pane-tab-title">Chat</span>
            </button>
          </Show>
        }
        tools={
          <>
            <button
              type="button"
              class="shell-icon-btn tab-strip-btn"
              aria-label={full() ? 'Exit full view' : 'Full view'}
              aria-pressed={full()}
              title={`${full() ? 'Exit full view' : 'Full view'} (${modKey()} ⇧ F)`}
              onClick={() => layout().toggleFullView()}
              data-testid="toggle-fullview"
            >
              <Show when={full()} fallback={<Maximize2 size={15} />}>
                <Minimize2 size={15} />
              </Show>
            </button>
            <button type="button" class="shell-icon-btn tab-strip-btn" aria-label="Hide side pane" title={`Hide side pane (${modKey()} Alt B)`} onClick={() => layout().toggleSide(false)} data-testid="hide-side">
              <PanelRightClose size={16} />
            </button>
          </>
        }
      />
      <Show
        when={tabs().length > 0}
        fallback={
          <Show when={!(full() && props.chatActive)}>
            <div class="pane-empty" data-testid="side-empty">
              <p class="pane-empty-title">Open a tab</p>
              <LauncherList inline area="side" hasSession={!!props.view.sessionId} hostId={props.scope.hostId} localHost={props.localHost} onPick={pick} />
            </div>
          </Show>
        }
      >
        <PaneBody
          area="side"
          scope={props.scope}
          sessionId={props.view.sessionId}
          tabs={tabs()}
          activeId={active()?.id}
          hidden={full() && props.chatActive}
          onClose={close}
          onTitle={(id, title) => store.retitle(props.scope, 'side', id, title)}
        />
      </Show>
      <Show when={launcher()}>
        {(a) => (
          <PaneLauncher anchor={a()} area="side" hasSession={!!props.view.sessionId} hostId={props.scope.hostId} localHost={props.localHost} onPick={pick} onDismiss={() => setLauncher(undefined)} />
        )}
      </Show>
      <Show when={overview()}>
        {(a) => (
          <TabOverview
            anchor={a()}
            tabs={tabs()}
            activeId={active()?.id}
            closed={store.recentlyClosed(props.scope, 'side', props.view)}
            onActivate={activate}
            onClose={close}
            onCloseOthers={(id) => closeMany(tabs().filter((t) => t.id !== id), () => store.closeOthers(props.scope, 'side', props.view, id))}
            onCloseAll={() =>
              closeMany(tabs(), () => {
                store.closeAll(props.scope, 'side', props.view);
                layout().toggleSide(false);
              })
            }
            onReopen={(id) => store.reopen(props.scope, 'side', props.view, id)}
            onDismiss={() => setOverview(undefined)}
          />
        )}
      </Show>
    </aside>
  );
}
