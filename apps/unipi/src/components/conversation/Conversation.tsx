/**
 * The conversation view (WP-A6, extended by WP-A9): an open session's
 * timeline on the host session protocol, its banners (connection loss Z-50,
 * takeover Z-51, host `crashed`/`reloaded`), the bottom dock (dialogs above
 * the composer, Z-46) and the crash boundary around the whole region (Z-89).
 *
 *   banner → timeline (virtual turns + live tail + dock)
 *
 * Draft (`/new/:host`) renders its empty state until A5's draft composer.
 */
import { createMemo, createSignal, onCleanup, Show, type JSX } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { CircleAlert, RefreshCw } from 'lucide-solid';
import { hubs } from '../../lib/session/hub';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { connBanner, sessionCwd } from '../../lib/session/selectors';
import { sessionRefChat } from '../../lib/timeline/refChat';
import { layout } from '../../lib/layout';
import type { RefChat } from '../../lib/chat/pathStat';
import { dialogResponse } from '../../ui/dialogs/adapter';
import { installUiSinks, sessionUi } from '../../ui/registries';
import { commandNames, type TimelineCtx } from '../timeline/context';
import { Timeline } from '../timeline/Timeline';
import { FileRefMenu } from '../chat/FileRefMenu';
import { EmptyState } from '../EmptyState';
import { ConversationBoundary } from '../ErrorBoundaries';
import { BottomDock } from '../dock/BottomDock';
import { ComposerBridge } from './ComposerBridge';
import { ReadOnlyBanner, isTakenOverReadOnly } from './ReadOnlyBanner';
import '../timeline/timeline.css';

// The session store's ui sink must be wired before any stream opens, or
// `extension_ui_request`s (the dock's dialogs, Z-46) never reach the registry.
installUiSinks();

/** The session's banners: connection loss (Z-50), takeover (Z-51), host crashes/reloads. */
function Banners(props: { session: SessionHandle }) {
  const conn = () => connBanner(props.session.state);
  const banner = () => props.session.state.banner;
  return (
    <>
      <Show when={conn()}>
        <div class="chat-banner tl-banner" role="alert" data-testid="tl-conn-banner">
          <CircleAlert size={18} class="shrink-0" />
          <span class="min-w-0 flex-1">{conn()!.text}</span>
          <button type="button" class="chat-banner-action" onClick={() => props.session.retry()}>
            <RefreshCw size={14} /> Retry
          </button>
        </div>
      </Show>
      <Show when={banner()}>
        {(b) => (
          <Show
            when={b().kind === 'taken_over'}
            fallback={
              <div class="chat-banner tl-banner" data-kind={b().kind} role="alert" data-testid="tl-host-banner">
                <CircleAlert size={18} class="shrink-0" />
                <span class="min-w-0 flex-1">{b().text}</span>
              </div>
            }
          >
            <ReadOnlyBanner banner={b()} onTakeBack={() => props.session.takeBack()} />
          </Show>
        )}
      </Show>
    </>
  );
}

export function Conversation(props: { hostId: string; sessionId?: string; cwd?: string }) {
  const navigate = useNavigate();
  const hub = createMemo(() => hubs().ensure(props.hostId));
  const session = createMemo(() => (props.sessionId ? sessions().ensure(props.hostId, props.sessionId) : undefined));
  // Pin the session against LRU eviction while shown; release on change/unmount.
  createMemo(() => {
    const s = session();
    const release = s?.acquire();
    onCleanup(() => release?.());
  });
  const cwd = () => {
    const s = session();
    return props.sessionId ? sessionCwd(props.sessionId ? hub().state.cards[props.sessionId] : undefined, s?.state) : props.cwd;
  };
  const refs = createMemo<RefChat | undefined>(() => {
    const s = session();
    return s ? sessionRefChat(s, cwd) : undefined;
  });
  const ctx = createMemo<TimelineCtx | undefined>(() => {
    const s = session();
    const r = refs();
    if (!s || !r) return undefined;
    return {
      session: s,
      hostId: props.hostId,
      sessionId: s.sessionId,
      cwd,
      refs: r,
      commands: () => commandNames(s.state.commands),
      columnWidth: () => colW() || 720,
      // A7 Fork: the host streams the fork's new id; route the workspace there.
      openSession: (id: string) => navigate(`/s/${encodeURIComponent(props.hostId)}/${encodeURIComponent(id)}`),
    };
  });
  const [colW, setColW] = createSignal(0);

  // ── dialogs (Z-46): pending pi/Oino dialogs, answered over the session ──
  const registry = createMemo(() => (props.sessionId ? sessionUi(props.hostId, props.sessionId) : undefined));
  const dialogs = () => registry()?.dialogs() ?? [];
  const answer = (id: string, value: unknown) => {
    const s = session();
    if (!s) return;
    // First answer wins: drop it locally at once, core confirms with `dismiss`.
    registry()?.dismissDialog(id);
    void s.cmd(dialogResponse(id, value)).catch((error: unknown) => console.warn('[dialog] answer failed', id, error));
  };

  return (
    <div class="conv-tl" data-testid="conversation">
      <Show
        when={props.sessionId}
        fallback={<EmptyState title="New chat" body="Start a chat in a project folder on this PC." testId="draft-empty" />}
      >
        <Show when={session()}>
          {(s) => (
            <ConversationBoundary>
              <Banners session={s()} />
              <Show when={ctx()}>
                {(c) => (
                  <TimelineCtxWidth ctx={c()} onWidth={setColW}>
                    {/* remount per session file: fresh scroll memory + height cache */}
                    <Show when={s().sessionId()} keyed>
                      {(_sid) => (
                        <Timeline
                          ctx={c()}
                          dock={
                            <BottomDock
                              session={s()}
                              dialogs={dialogs()}
                              onAnswer={answer}
                              readOnly={isTakenOverReadOnly(s().state)}
                              phone={!layout().desktop()}
                              composer={
                                <ComposerBridge
                                  session={s()}
                                  readOnly={isTakenOverReadOnly(s().state)}
                                />
                              }
                            />
                          }
                        />
                      )}
                    </Show>
                  </TimelineCtxWidth>
                )}
              </Show>
              <FileRefMenu />
            </ConversationBoundary>
          )}
        </Show>
      </Show>
    </div>
  );
}

/** Measures the timeline column width for the markdown cache's width class. */
function TimelineCtxWidth(props: { ctx: TimelineCtx; onWidth: (w: number) => void; children: JSX.Element }) {
  return (
    <div
      class="tl-frame"
      ref={(el) => {
        if (typeof ResizeObserver === 'undefined') return;
        props.onWidth(el.clientWidth);
        const ro = new ResizeObserver(() => props.onWidth(el.clientWidth));
        ro.observe(el);
        onCleanup(() => ro.disconnect());
      }}
    >
      {props.children}
    </div>
  );
}
