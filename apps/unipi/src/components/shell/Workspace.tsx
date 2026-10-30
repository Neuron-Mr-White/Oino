/**
 * The conversation frame with its side pane and bottom panel (§3.7–3.8,
 * WP-A3) for `/s/:host/:session` and the draft `/new/:host?cwd=`.
 *
 *   ┌ conv ─────────────────────────────┬─┬ side pane ───────┐
 *   │ header 56 (Z-2)                   │ │ tab strip · +     │
 *   │ body (timeline: A6)               │ │ active tab body   │
 *   │ dock slot (A9)                    │ │                   │
 *   │ status strip 24 (§3.10)           │ │                   │
 *   │ ─ bottom panel (Cmd/Ctrl+J) ───── │ │                   │
 *   └───────────────────────────────────┴─┴───────────────────┘
 *
 * The side pane pushes the chat at ≥ 1280 px, overlays it below (Z-95) and
 * covers it in full view (Z-6, the conversation stays mounted under its
 * "Chat" tab). Split panes (Z-7) are deferred: exactly one conversation.
 */
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useHostsOptional } from '../../hosts/context';
import { backDepth } from '../../lib/back';
import { layout, SIDE_DEFAULT, BOTTOM_DEFAULT } from '../../lib/layout';
import { panes, type PaneScope, type PaneView } from '../../lib/panes';
import { hubs } from '../../lib/session/hub';
import { sessionCwd, sessionTitle, shortCwd } from '../../lib/session/selectors';
import { Conversation } from '../conversation/Conversation';
import { BottomPanel } from '../panes/BottomPanel';
import { SidePane } from '../panes/SidePane';
import { Capsule, capsuleVisible } from '../status/Capsule';
import { ConversationHeader } from './ConversationHeader';
import { PhonePanesGroup } from '../../ui/slots/paneTabs';
import { sessionUi } from '../../ui/registries';
import { ResizeHandle } from './ResizeHandle';
import { StatusStrip } from './StatusStrip';

const LAST_KEY = 'unipi.shell.last';

/** The last shown session (`/` reopens it). */
export function readLastSession(): { hostId: string; sessionId: string } | undefined {
  try {
    const v = JSON.parse(localStorage.getItem(LAST_KEY) ?? 'null') as { hostId?: unknown; sessionId?: unknown } | null;
    return v && typeof v.hostId === 'string' && typeof v.sessionId === 'string' ? { hostId: v.hostId, sessionId: v.sessionId } : undefined;
  } catch {
    return undefined;
  }
}
function writeLastSession(hostId: string, sessionId: string): void {
  try {
    localStorage.setItem(LAST_KEY, JSON.stringify({ hostId, sessionId }));
  } catch {
    // not remembered
  }
}

/** The project key panes are scoped to: the session's folder, `~` when it has none. */
export const projectKeyOf = (cwd: string | undefined): string => cwd || '~';

/** The scope + view of the shown conversation (for app-wide keys like Cmd/Ctrl+Shift+T). */
const [current, setCurrent] = createSignal<{ scope: PaneScope; view: PaneView } | undefined>();
export const currentWorkspace = current;

export function Workspace(props: { hostId: string; sessionId?: string; cwd?: string; body?: JSX.Element }) {
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  const L = layout();
  const hub = createMemo(() => hubs().ensure(props.hostId));
  const card = () => (props.sessionId ? hub().state.cards[props.sessionId] : undefined);
  const host = () => hosts?.hosts().find((h) => h.id === props.hostId);
  const localHost = () => host()?.transport === 'local';
  const cwd = () => (props.sessionId ? sessionCwd(card()) : (props.cwd ?? ''));
  const scope = createMemo<PaneScope>((prev) => {
    const next = { hostId: props.hostId, projectKey: projectKeyOf(cwd()) };
    return prev && prev.hostId === next.hostId && prev.projectKey === next.projectKey ? prev : next;
  });
  const view = createMemo<PaneView>((prev) => (prev && prev.sessionId === props.sessionId ? prev : { sessionId: props.sessionId }));
  const title = () => (props.sessionId ? (card() ? sessionTitle(card()) : 'Session') : 'New chat');
  const newChat = () => navigate(`/new/${encodeURIComponent(props.hostId)}`);
  const desktop = () => L.desktop();
  const mode = () => (L.fullView() ? 'full' : L.sideMode());
  // Tabs are scoped to the session's project: wait for its card (or the loaded list) first.
  const scopeReady = () => !props.sessionId || !!card() || hub().state.loaded;
  const sideShown = () => desktop() && L.sideOpen() && scopeReady();

  // Full view: entering keeps the current pane tab (ZCode); the Chat tab shows the conversation.
  const [chatActive, setChatActive] = createSignal(false);
  createEffect(on(L.fullView, (full) => setChatActive(full && panes().visible(scope(), 'side', view()).length === 0)));

  createEffect(() => {
    if (props.sessionId) writeLastSession(props.hostId, props.sessionId);
  });
  createEffect(() => setCurrent({ scope: scope(), view: view() }));
  onCleanup(() => setCurrent(undefined));

  // §3.14 (Z-86): the hub connection reports which session this workspace shows;
  // leaving (or the draft) reports none. The host clears the session's `unread`.
  createEffect(() => {
    const hub = hubs().ensure(props.hostId);
    hub.focus(props.sessionId ?? null);
    onCleanup(() => hub.focus(null));
  });

  // Esc leaves full view (after any open menu / dialog took its Esc).
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || !L.fullView() || backDepth() > 0 || e.defaultPrevented) return;
      e.preventDefault();
      L.toggleFullView(false);
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  let frame: HTMLDivElement | undefined;
  const subline = () => (cwd() ? shortCwd(cwd()) : undefined);
  const ctx = () => (props.sessionId ? { hostId: props.hostId, sessionId: props.sessionId, title: title(), cwd: cwd() || undefined, file: card()?.file || undefined } : undefined);

  return (
    <div
      ref={frame}
      class="ws"
      data-side={sideShown() ? mode() : 'closed'}
      data-chat={L.fullView() && chatActive() ? '' : undefined}
      data-shell-frame={desktop() ? 'desktop' : 'phone'}
      style={{ '--side-w': `${L.sideWidth()}px`, '--bottom-h': `${L.bottomHeight()}px` }}
      data-testid="workspace"
    >
      <section class="conv" aria-label="Conversation" data-testid="conv">
        <ConversationHeader
          title={title()}
          cwd={cwd() || undefined}
          project={subline()}
          hostName={localHost() ? undefined : host()?.name}
          ctx={ctx()}
          desktop={desktop()}
          extraMenu={
            !desktop() && props.sessionId
              ? (close) => <PhonePanesGroup registry={sessionUi(props.hostId, props.sessionId!)} onOpen={close} />
              : undefined
          }
          onBack={desktop() ? undefined : () => navigate('/chat')}
          onNewChat={newChat}
        />
        {/* §3.10: the collapsed-pane capsule — top-right of the timeline, visible while the
            pane is closed or overlaying (a pushing pane owns the status). */}
        <Show when={capsuleVisible(desktop(), sideShown(), mode())}>
          <Capsule hostId={props.hostId} sessionId={props.sessionId} cwd={cwd() || undefined} />
        </Show>
        {/* §3.10: on the phone the strip sits under the header (≤ 4 segments); on the desktop it is the bottom row. */}
        <Show when={!desktop()}>
          <StatusStrip scope={`${props.hostId}/${props.sessionId ?? 'draft'}`} />
        </Show>
        <div class="conv-body" data-testid="conv-body">
          {props.body ?? <Conversation hostId={props.hostId} sessionId={props.sessionId} cwd={props.cwd} />}
        </div>
        <Show when={desktop()}>
          <StatusStrip scope={`${props.hostId}/${props.sessionId ?? 'draft'}`} />
        </Show>
        <Show when={desktop() && L.bottomOpen() && scopeReady()}>
          <ResizeHandle
            axis="y"
            dir={-1}
            label="Resize panel"
            class="bottom-handle"
            size={L.bottomHeight()}
            onSize={(h) => L.setBottomHeight(h, frame?.clientHeight ?? window.innerHeight)}
            onReset={() => L.setBottomHeight(BOTTOM_DEFAULT, frame?.clientHeight ?? window.innerHeight)}
            testId="bottom-handle"
          />
          <BottomPanel scope={scope()} view={view()} localHost={localHost()} />
        </Show>
      </section>
      <Show when={sideShown()}>
        <Show when={mode() === 'overlay'}>
          <div class="side-scrim" aria-hidden="true" onClick={() => L.toggleSide(false)} data-testid="side-scrim" />
        </Show>
        <Show when={mode() !== 'full'}>
          <ResizeHandle
            axis="x"
            dir={-1}
            label="Resize side pane"
            class="side-handle"
            size={L.sideWidth()}
            onSize={(w) => L.setSideWidth(w)}
            onReset={() => L.setSideWidth(SIDE_DEFAULT)}
            testId="side-handle"
          />
        </Show>
        <SidePane scope={scope()} view={view()} mode={mode()} localHost={localHost()} chatActive={chatActive()} onChat={setChatActive} onNewChat={newChat} />
      </Show>
    </div>
  );
}

