/**
 * SessionModal (WP-A17, D-7): the native component of the reserved
 * `unipi.info-screen/dashboard` entry — Context / Running / Info tabs, the
 * only full Running view (RULINGS Z-95). Desktop: centred modal dialog
 * (ui-protocol §6.2 `overlay`). Phone: the standard bottom sheet.
 *
 * Opened from:
 * - the header ⋯ "Session info" row (`registerChatAction('info', …)`,
 *   the slot A3 left for this WP);
 * - `openSessionModal({tab})` — the composer context ring (A14, Z-73), the
 *   status pane's Running summary (A16) and Shift+↓ (A18) plug in there;
 * - the reserved overlay entry itself: when a session's registry gains
 * `unipi.info-screen/dashboard` in the overlay stack (`/info …`), the modal
 * opens bound to that session, and dismissing it sends the entry `$close`.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { Gauge, Info as InfoIcon, ListChecks, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { registerChatAction } from '../shell/ConversationHeader';
import { layout } from '../../lib/layout';
import { wrapTab } from '../../lib/focusTrap';
import { sessions, type SessionHandle } from '../../lib/session/store';
import { sessionTitle } from '../../lib/session/selectors';
import { hubs } from '../../lib/session/hub';
import { sessionUi, installUiSinks } from '../../ui/registries';
import { uiTransport } from '../../ui/actions';
import type { UiEntry } from '../../ui/types';
import { ContextTab } from './ContextTab';
import { InfoTab } from './InfoTab';
import { RunningTab } from './RunningTab';
import { openSessionModal, sessionModal, setSessionModalTarget, type InfoPage, type SessionModalTab, type SessionModalTarget } from './modalTarget';
import './session-modal.css';

// The first app consumer of session-scope registry entries: point the store's ui
// sink at the U9 registries now, so a session that opens before this layer mounts
// keeps its first ui snapshot (a cold `/info` load). Idempotent (U10's shell root
// keeps the call too).
installUiSinks();

/** The reserved entry this modal renders (ui-protocol §8). */
export const DASHBOARD_ID = 'unipi.info-screen/dashboard';

/** The app-side entry points all funnel here (A14's ring, A16's summary, A18). */
export { openSessionModal, sessionModal };
const target = sessionModal;
export type { InfoPage, SessionModalTab, SessionModalTarget };

const setTarget = setSessionModalTarget;
/** Entries the user dismissed while the host still lists them (the `$close`
 * round-trip may lag): don't reopen them until they leave the stack. */
const dismissed = new Set<string>();

/** Closes it (tests, and a session going away). */
export function closeSessionModal(): void {
  dismissed.clear();
  setTarget(undefined);
}

/** `/info <page>` → the Info sub-page; unknown pages fall back to Session. */
const PAGE_OF: Record<string, InfoPage> = { session: 'session', usage: 'usage', tools: 'tools', skills: 'skills', modules: 'modules' };
const pageParam = (entry: UiEntry): InfoPage | undefined => {
  const params = (entry.view as { params?: unknown } | undefined)?.params;
  const page = params && typeof params === 'object' ? (params as Record<string, unknown>).page : undefined;
  return typeof page === 'string' ? PAGE_OF[page] : undefined;
};

const TABS: Array<{ id: SessionModalTab; label: string; icon: typeof Gauge }> = [
  { id: 'context', label: 'Context', icon: Gauge },
  { id: 'running', label: 'Running', icon: ListChecks },
  { id: 'info', label: 'Info', icon: InfoIcon },
];

/** The shared body: tabs + the active tab's content (both shells). */
export function SessionModalBody(props: { target: SessionModalTarget }) {
  // ensure() subscribes; acquire() pins the session against the LRU while shown.
  const s = sessions().ensure(props.target.hostId, props.target.sessionId);
  onCleanup(s.acquire());
  const [tab, setTab] = createSignal<SessionModalTab>(props.target.tab);
  createEffect(on(() => props.target, (t) => t && setTab(t.tab), { defer: true }));
  const running = () => s.state.run.running || s.state.run.phase === 'compacting' || s.state.run.phase === 'retrying';

  return (
    <>
      <div class="sm-tabs" role="tablist" aria-label="Session sections">
        <For each={TABS}>
          {(t) => (
            <button type="button" role="tab" aria-selected={tab() === t.id} class="sm-tab" data-testid={`sm-tab-${t.id}`} onClick={() => setTab(t.id)}>
              <t.icon size={15} />
              {t.label}
              <Show when={t.id === 'running' && running()}>
                <span class="sm-badge" aria-label="run active">•</span>
              </Show>
            </button>
          )}
        </For>
      </div>
      <div class="sm-body" data-testid="sm-body">
        <Show when={tab() === 'context'}>
          <ContextTab s={s} />
        </Show>
        <Show when={tab() === 'running'}>
          <RunningTab s={s} />
        </Show>
        <Show when={tab() === 'info'}>
          <InfoTab s={s} initialPage={props.target.infoPage} />
        </Show>
      </div>
    </>
  );
}

/** The open modal: desktop centred dialog, phone bottom sheet (corvu). */
export function SessionModal(props: { onClose: () => void }) {
  const t = createMemo(() => target());
  const desktop = () => layout().desktop();
  const title = (cur: SessionModalTarget): string => {
    const hub = hubs().ensure(cur.hostId);
    return sessionTitle(hub.state.cards[cur.sessionId], sessions().peek(cur.hostId, cur.sessionId)?.state);
  };
  const close = () => {
    const cur = t();
    // P1-03: the Info pages may have asked the extension (`/info usage`) — its
    // dashboard entry then exists even when the modal opened from ⋯; close it too.
    const entryId =
      cur?.entryId ?? (cur && sessionUi(cur.hostId, cur.sessionId).stack('overlay')().some((e) => e.id === DASHBOARD_ID) ? DASHBOARD_ID : undefined);
    // Remember before closing: the reopen effect must see the dismissal.
    if (entryId) dismissed.add(entryId);
    props.onClose();
    if (cur && entryId) {
      uiTransport().action({ scope: 'session', hostId: cur.hostId, sessionId: cur.sessionId }, { entryId, actionId: '$close' }).catch(() => {});
    }
  };
  onMount(() => {
    if (!desktop()) return; // the sheet's pushBack already owns Esc/Back on the phone
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && target() && !e.defaultPrevented) {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener('keydown', onKey, true);
    onCleanup(() => window.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show when={t()} keyed>
      {(cur) =>
        desktop() ? (
          <div class="sm-overlay" onClick={(e) => e.target === e.currentTarget && close()} data-testid="sm-overlay">
            <div
              role="dialog"
              aria-modal="true"
              aria-label={title(cur)}
              class="sm-dialog"
              tabindex="-1"
              ref={(el) => queueMicrotask(() => el.focus({ preventScroll: true }))}
              onKeyDown={(e: KeyboardEvent) => {
                if (wrapTab(e, e.currentTarget as HTMLElement)) e.stopPropagation();
              }}
            >
              <div class="sm-head">
                <span class="sm-title">
                  <h2>{title(cur)}</h2>
                </span>
                <button type="button" class="ghost-icon-button" aria-label="Close" onClick={close} data-testid="sm-close">
                  <X size={20} />
                </button>
              </div>
              <SessionModalBody target={cur} />
            </div>
          </div>
        ) : (
          <BoardSheet open onClose={close} label="Session" title={title(cur)} testId="session-modal-sheet" class="session-sheet">
            <SessionModalBody target={cur} />
          </BoardSheet>
        )
      }
    </Show>
  );
}

/**
 * Mounted once by the shell: renders the modal, plugs the ⋯ "Session info"
 * action in, and opens on the reserved overlay entry (`/info`) of any open
 * session — the only reserved component this WP owns.
 */
export function SessionModalLayer() {
  // The first app consumer of session-scope registry entries (`/info`): point the
  // session store's ui sink at the U9 registries (idempotent; U10 owns the rest).
  installUiSinks();
  onCleanup(
    registerChatAction('info', {
      run: (ctx) => openSessionModal({ hostId: ctx.hostId, sessionId: ctx.sessionId, tab: 'info' }),
    }),
  );

  // `/info …`: the reserved entry appears in a session's overlay stack → open.
  createEffect(() => {
    for (const key of sessions().keys()) {
      const [hostId, sessionId] = key.split('/');
      if (!sessionId) continue;
      const entry = sessionUi(hostId!, sessionId).stack('overlay')().find((e) => e.id === DASHBOARD_ID);
      if (!entry) continue;
      if (dismissed.has(entry.id)) return;
      const cur = target();
      if (cur?.entryId === entry.id || cur?.sessionId === sessionId) return;
      setTarget({ hostId: hostId!, sessionId, tab: 'info', infoPage: pageParam(entry), entryId: entry.id });
      return;
    }
  });
  // The entry went away (core closed it): the modal follows.
  createEffect(() => {
    const t = target();
    if (!t?.entryId) return;
    const present = sessionUi(t.hostId, t.sessionId).stack('overlay')().some((e) => e.id === t.entryId);
    if (present) return;
    dismissed.delete(t.entryId);
    setTarget(undefined);
  });

  return (
    <Show when={target()}>
      <SessionModal onClose={() => setTarget(undefined)} />
    </Show>
  );
}
