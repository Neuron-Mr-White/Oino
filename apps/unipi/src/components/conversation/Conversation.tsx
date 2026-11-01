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
import { createEffect, createMemo, createSignal, on, onCleanup, Show, type JSX } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { ArchiveRestore, CircleAlert, RefreshCw } from 'lucide-solid';
import { hubs } from '../../lib/session/hub';
import { createArchivedSession } from '../../lib/session/archivedSession';
import { restoreSession } from '../../lib/session/actions';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { connBanner, sessionCwd } from '../../lib/session/selectors';
import { sessionRefChat } from '../../lib/timeline/refChat';
import { layout } from '../../lib/layout';
import type { RefChat } from '../../lib/chat/pathStat';
import { useHostsOptional } from '../../hosts/context';
import { dialogResponse } from '../../ui/dialogs/adapter';
import { installUiSinks, sessionUi } from '../../ui/registries';
import { installConversationSlots } from '../../ui/slots';
import { ComposerTray } from '../../ui/slots/ComposerTray';
import { HeaderBand } from '../../ui/slots/HeaderBand';
import { PhonePaneLayer } from '../../ui/slots/paneTabs';
import { SlotModal } from '../../ui/slots/SlotModal';
import { WidgetStack } from '../../ui/slots/WidgetStack';
import { installUiEffects } from '../../ui/slots/effects';
import { reservedGate } from '../../ui/slots/reserved';
import { DraftView } from '../draft/DraftView';
import { takeFirstPrompt } from '../../lib/session/firstPrompt';
import { TURN_FILES_ENTRY, fileSummaryCapability } from '../../lib/fileSummary';
import { parseExtShortcutSpecs, setExtensionShortcuts } from '../../lib/shortcuts/registry';
import { commandNames, type TimelineCtx } from '../timeline/context';
import { Timeline } from '../timeline/Timeline';
import { FileRefMenu } from '../chat/FileRefMenu';
import { EmptyChat } from '../EmptyChat';
import { ConversationBoundary } from '../ErrorBoundaries';
import { BottomDock } from '../dock/BottomDock';
import { Composer } from '../composer/Composer';
import { ReadOnlyBanner, isTakenOverReadOnly } from './ReadOnlyBanner';
import '../timeline/timeline.css';

// The session store's ui sink must be wired before any stream opens, or
// `extension_ui_request`s (the dock's dialogs, Z-46) never reach the registry.
installUiSinks();
// U10's app-wide effect handlers (notify → toasts, set_editor_text, …).
installUiEffects();

/** The session's banners: connection loss (Z-50), open error, takeover (Z-51), host crashes/reloads. */
function Banners(props: { session: SessionHandle }) {
  const conn = () => {
    const st = props.session.state;
    const base = connBanner(st);
    if (base) return base;
    if (st.conn === 'closed' && st.connError && st.entries.length > 0) {
      return { text: st.connError.message || 'Disconnected from the session', action: 'retry' as const };
    }
    return undefined;
  };
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
  const hosts = useHostsOptional();
  /** A paired PC's name for the empty state's hint (Z-87); This computer has none. */
  const hostName = () => {
    const h = hosts?.hosts().find((x) => x.id === props.hostId);
    return h && h.transport !== 'local' ? h.name : undefined;
  };
  const hub = createMemo(() => hubs().ensure(props.hostId));
  const liveCard = () => (props.sessionId ? hub().state.cards[props.sessionId] : undefined);
  const archivedCard = createMemo(() => {
    const id = props.sessionId;
    if (!id || liveCard()) return undefined;
    return hub().state.archived.find((c) => c.session_id === id);
  });
  const archivedHandle = createMemo<SessionHandle | undefined>(() => {
    const card = archivedCard();
    if (!card) return undefined;
    const h = createArchivedSession(props.hostId, card);
    onCleanup(() => h.close());
    return h;
  });
  const isArchived = () => !!archivedCard();
  const session = createMemo<SessionHandle | undefined>(() => {
    if (!props.sessionId) return undefined;
    const arch = archivedHandle();
    if (arch) return arch;
    return sessions().ensure(props.hostId, props.sessionId);
  });
  // Pin the session against LRU eviction while shown; release on change/unmount.
  createMemo(() => {
    const s = session();
    const release = s?.acquire();
    onCleanup(() => release?.());
  });
  const cwd = () => {
    const s = session();
    return props.sessionId ? sessionCwd(liveCard() ?? archivedCard(), s?.state) : props.cwd;
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
      // Z-28 "Also undo file changes": A27's file-summary capability. The gate is
      // read in the handler (not tracked), so disabling the owner hides the option
      // without rebuilding the context; the summaries come from the mounted footers.
      undoFiles: (() => {
        const files = fileSummaryCapability(s);
        return {
          canUndo: (turnId: string) => reservedGate(TURN_FILES_ENTRY) && files.canUndo(turnId),
          undoFrom: (turnId: string) => files.undoFrom(turnId),
        };
      })(),
    };
  });
  const [colW, setColW] = createSignal(0);

  // ── dialogs (Z-46): pending pi/Oino dialogs, answered over the session ──
  const registry = createMemo(() => (props.sessionId ? sessionUi(props.hostId, props.sessionId) : undefined));
  // The session's extension shortcuts feed the registry (G-7): `setShortcuts`
  // ui requests arrive on the stream; hosts that never push them are asked once.
  createEffect(() => {
    const meta = registry()?.state.meta as { shortcuts?: unknown } | undefined;
    setExtensionShortcuts(parseExtShortcutSpecs(meta?.shortcuts));
  });
  // Ask once the stream is live: a command sent while it connects is rejected
  // and would leave the registry without the extension chords (G-7).
  const askedShortcuts = new Set<string>();
  createEffect(() => {
    const id = props.sessionId;
    const s = session();
    if (!id || !s || isArchived() || s.state.conn !== 'live' || askedShortcuts.has(id)) return;
    askedShortcuts.add(id);
    void s
      .cmd<unknown>({ type: '_oino/get_shortcuts' })
      .then((reply) => setExtensionShortcuts(parseExtShortcutSpecs(reply)))
      .catch(() => askedShortcuts.delete(id));
  });
  // Z-19 (WP-A5): a draft's first prompt created this session — send it the
  // moment its stream is live (taken, so a reconnect can never resend it).
  const sentFirst = new Set<string>();
  createEffect(() => {
    const s = session();
    const id = s?.sessionId();
    if (!s || !id || s.state.conn !== 'live' || sentFirst.has(id)) return;
    const first = takeFirstPrompt(props.hostId, id);
    if (!first) return;
    sentFirst.add(id);
    s.send(first.text, first.images?.length ? { images: first.images } : undefined);
  });
  onCleanup(() => setExtensionShortcuts(undefined));
  const dialogs = () => registry()?.dialogs() ?? [];
  const answer = (id: string, value: unknown) => {
    const s = session();
    if (!s) return;
    // First answer wins: drop it locally at once, core confirms with `dismiss`.
    registry()?.dismissDialog(id);
    void s.cmd(dialogResponse(id, value)).catch((error: unknown) => console.warn('[dialog] answer failed', id, error));
  };

  // ── U10 slot feeds + hosts (ui-protocol §6.2) ─────────────────────
  const target = () => ({ scope: 'session' as const, hostId: props.hostId, sessionId: props.sessionId });
  const widgets = () => {
    const reg = registry();
    const phone = !layout().desktop();
    const list = phone
      ? [...reg?.entriesFor('widgetBelow')() ?? [], ...(reg?.entriesFor('widget')() ?? [])] // §6.2: phone folds belowEditor into the stack
      : (reg?.entriesFor('widget')() ?? []);
    return list;
  };
  const widgetBelow = () => (layout().desktop() ? (registry()?.entriesFor('widgetBelow')() ?? []) : []);
  const overlayStack = () => registry()?.stack('overlay')() ?? [];
  const trayStack = () => registry()?.stack('editorReplace')() ?? [];
  // Suppress the TUI-only ASCII Unicrab startup splash (`unipi.info-screen/setHeader:splash`) —
  // the GUI already renders native Unicrab/EmptyChat and the 14-line ASCII banner causes a 280 px jump on open.
  const headerEntries = () => (registry()?.entriesFor('header')() ?? []).filter((e) => e.id !== 'unipi.info-screen/setHeader:splash');
  installConversationSlots({ hostId: props.hostId, sessionId: props.sessionId ?? '', cwd, registry, session });

  return (
    <div class="conv-tl" data-testid="conversation">
      <Show
        when={props.sessionId}
        fallback={
          <div class="tl-empty" data-testid="draft-empty">
            <DraftView hostId={props.hostId} cwd={props.cwd} />
          </div>
        }
      >
        <Show when={session()}>
          {(s) => (
            <ConversationBoundary>
              <Show when={isArchived()}>
                <div class="chat-banner tl-banner tl-readonly" data-kind="archived" data-testid="tl-archived-banner" role="status">
                  <ArchiveRestore size={18} class="shrink-0" />
                  <span class="min-w-0 flex-1">Archived chat · Read-only. Restore it to continue the conversation.</span>
                  <button
                    type="button"
                    class="chat-banner-action"
                    data-testid="tl-restore-archived"
                    onClick={() => props.sessionId && void restoreSession(props.hostId, props.sessionId)}
                  >
                    <ArchiveRestore size={14} /> Restore
                  </button>
                </div>
              </Show>
              <Banners session={s()} />
              <Show when={ctx()}>
                {(c) => (
                  <TimelineCtxWidth ctx={c()} onWidth={setColW}>
                    {/* remount per session file: fresh scroll memory + height cache */}
                    <Show when={s().sessionId()} keyed>
                      {(_sid) => (
                        <Timeline
                          ctx={c()}
                          empty={<EmptyChat cwd={cwd()} hostName={hostName()} />}
                          header={<HeaderBand entries={headerEntries()} target={target()} />}
                          overlay={<SlotModal stack={overlayStack()} target={target()} phone={!layout().desktop()} />}
                          dock={
                            <BottomDock
                              session={s()}
                              dialogs={dialogs()}
                              onAnswer={answer}
                              readOnly={isArchived() || isTakenOverReadOnly(s().state)}
                              phone={!layout().desktop()}
                              widgets={<WidgetStack entries={widgets()} target={target()} phone={!layout().desktop()} />}
                              widgetBelow={<WidgetStack entries={widgetBelow()} target={target()} />}
                              composerTray={trayStack().length ? <ComposerTray stack={trayStack()} target={target()} /> : undefined}
                              composer={
                                <Composer
                                  session={s()}
                                  cwd={cwd}
                                  readOnly={isArchived() || isTakenOverReadOnly(s().state)}
                                  placeholder={isArchived() ? 'Archived chat — restore to reply' : undefined}
                                  phone={!layout().desktop()}
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
              <Show when={!layout().desktop() && registry()}>
                {(reg) => <PhonePaneLayer registry={reg()} target={target()} />}
              </Show>
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
