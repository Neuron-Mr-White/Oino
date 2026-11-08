/**
 * Hub store (app spec §3.3 `hub`, WP-A2): per host, the session cards from
 * `sessions_list` kept current by the `events_open` stream
 * (`session_changed` / `session_removed`), host notices (`host_notice`: the
 * safe-mode banner, updater prompts) and per-session meta (pin / unread /
 * title lock, read from the card once the A4 host delta adds them).
 * Host-scope ui-protocol traffic (`ui`) goes to the U9 registry sink.
 *
 *   const h = hubs().ensure(hostId); h.state.cards · h.reload() · h.dismissNotice(id)
 */
import { batch, createRoot, createSignal } from 'solid-js';
import { createStore, produce, reconcile } from 'solid-js/store';
import { toAppError, type AppError } from '@unipi/protocol';
import { hostClient } from '../host/factory';
import type { Closable, EventsStream, HostClient, HubEvent, SessionCard } from '../host/client';

/** Visible host notices kept at once (oldest drop first). */
export const MAX_NOTICES = 5;
/** [ux-sessions] Settle time before an open Archived list re-reads after batch events. */
const ARCHIVED_REFRESH_MS = 400;
/** Re-open the hub event stream after it drops. */
const EVENTS_RETRY_MS = [1_000, 2_000, 5_000, 10_000] as const;

export type HubNotice = { id: string; level: 'info' | 'warning'; text: string; at: number };
export type SessionMeta = { pinned?: boolean; unread?: boolean; titleLock?: boolean };

export type HubState = {
  hostId: string;
  cards: Record<string, SessionCard>;
  /** The archived (trashed) sessions, loaded lazily when the Archived list opens (Z-13). */
  archived: SessionCard[];
  archivedLoaded: boolean;
  archivedLoading: boolean;
  /**
   * [ux-sessions] A batch archive / restore in flight (`sessions_archive` / `sessions_restore`):
   * `done` counts the host's `sessions_removed` / `sessions_changed` chunks — the progress line.
   */
  bulk?: { kind: 'archive' | 'restore'; total: number; done: number };
  notices: HubNotice[];
  meta: Record<string, SessionMeta>;
  /** Repo root → change counter (WP-A15): bumped by every `git_changed`. */
  git: Record<string, number>;
  loaded: boolean;
  loading: boolean;
  error?: AppError;
  /** The `events_open` stream is up. */
  live: boolean;
};

/** Where host-scope `ui` hub events go (U9). */
export type HubUiSink = (hostId: string, event: Extract<HubEvent, { kind: 'ui' }>) => void;
let uiSink: HubUiSink = () => {};
export function setHubUiSink(sink: HubUiSink | undefined): void {
  uiSink = sink ?? (() => {});
}

/** `sessions_list` replies: a bare array (host.md §7.2), or `{sessions}` from older mocks. */
export function cardsOf(data: unknown): SessionCard[] {
  const list = Array.isArray(data) ? data : Array.isArray((data as { sessions?: unknown } | null)?.sessions) ? (data as { sessions: unknown[] }).sessions : [];
  return list.filter((c): c is SessionCard => !!c && typeof c === 'object' && typeof (c as SessionCard).session_id === 'string');
}

/** The meta flags a card carries (A4 host delta: `pinned`, `unread`, `title_locked`). */
export function metaOf(card: SessionCard): SessionMeta {
  const c = card as SessionCard & { pinned?: unknown; unread?: unknown; title_locked?: unknown };
  const meta: SessionMeta = {};
  if (c.pinned === true) meta.pinned = true;
  if (c.unread === true) meta.unread = true;
  if (c.title_locked === true) meta.titleLock = true;
  return meta;
}

let noticeSeq = 0;

/** Pure: one hub event onto the state (tests drive it directly). */
export function applyHubEvent(s: HubState, event: HubEvent, now = Date.now()): void {
  switch (event.kind) {
    case 'session_changed': {
      const card = { ...event.card };
      s.cards[card.session_id] = card;
      s.meta[card.session_id] = metaOf(card);
      return;
    }
    case 'session_removed':
      delete s.cards[event.session];
      delete s.meta[event.session];
      // An archived session restored then deleted again leaves the list too (Z-13).
      s.archived = s.archived.filter((card) => card.session_id !== event.session);
      return;
    // [ux-sessions] one chunk of a batch archive (the app removed them optimistically already).
    case 'sessions_removed': {
      for (const id of event.sessions) {
        delete s.cards[id];
        delete s.meta[id];
      }
      if (s.bulk?.kind === 'archive') s.bulk.done = Math.min(s.bulk.total, s.bulk.done + event.sessions.length);
      return;
    }
    // [ux-sessions] one chunk of a batch restore.
    case 'sessions_changed': {
      const back = new Set<string>();
      for (const c of event.cards) {
        s.cards[c.session_id] = { ...c };
        s.meta[c.session_id] = metaOf(c);
        back.add(c.session_id);
      }
      if (s.archived.length) s.archived = s.archived.filter((card) => !back.has(card.session_id));
      if (s.bulk?.kind === 'restore') s.bulk.done = Math.min(s.bulk.total, s.bulk.done + event.cards.length);
      return;
    }
    case 'host_notice': {
      if (s.notices.some((n) => n.text === event.text && n.level === event.level)) return;
      noticeSeq += 1;
      s.notices.push({ id: `n${noticeSeq}`, level: event.level, text: event.text, at: now });
      if (s.notices.length > MAX_NOTICES) s.notices.splice(0, s.notices.length - MAX_NOTICES);
      return;
    }
    // WP-A15 (app.md §3.14): one tick per repo change; the Git pane refreshes
    // on it (Z-98 — no polling).
    case 'git_changed':
      s.git[event.root] = (s.git[event.root] ?? 0) + 1;
      return;
    default:
      return;
  }
}

export type HubHandle = {
  readonly hostId: string;
  readonly state: HubState;
  /** Re-runs `sessions_list` (the events stream keeps it current after that). */
  reload(): Promise<void>;
  /** Patches a known card in memory (e.g. live `session_info_changed` name before the next hub sync, D-03). */
  patchCard(sessionId: string, patch: Partial<SessionCard>): void;
  /** `session_meta_set` (§3.14): pin / unread / title lock — optimistic here, persisted by the host. */
  setMeta(sessionId: string, patch: { pinned?: boolean; unread?: boolean; titleLock?: boolean }): Promise<void>;
  /** `sessions_list{archived:true}` (§3.14, Z-13): the archived list of this host (once, unless `force`). */
  loadArchived(force?: boolean): Promise<void>;
  /**
   * [ux-sessions] `sessions_archive`: archive many chats in ONE host op. The rows leave the list
   * at once (and join an open Archived list), the host's chunks drive `state.bulk`, failures come
   * back; afterwards the Archived list is re-read so its count and rows are exact.
   */
  archiveMany(sessionIds: readonly string[]): Promise<BatchResult>;
  /** [ux-sessions] `sessions_restore`: put many archived chats back in one host op (Archive all's Undo). */
  restoreMany(sessionIds: readonly string[]): Promise<BatchResult>;
  /** `session_archive` (§4.2, Z-13): move the session's file to the host's trash. */
  archive(sessionId: string): Promise<void>;
  /** `session_restore` (§3.14, Z-13): put an archived session back into its project. */
  restore(sessionId: string): Promise<void>;
  /** `session_delete` (§3.14, Z-13/Z-20): delete an archived session file for good. */
  deleteArchived(sessionId: string): Promise<void>;
  /** `trash_empty` (§3.14, Z-13): delete every archived session; answers how many went. */
  emptyTrash(): Promise<number>;
  /** Reports the focused session on this hub's events connection (`null` = none, §3.14); the host clears its `unread` (Z-82). */
  focus(session: string | null): void;
  dismissNotice(id: string): void;
  close(): void;
};

/** What a batch op did: the ids that went and the ones that did not (with the host's reason). */
export type BatchResult = { done: string[]; failed: { session: string; message: string }[]; error?: AppError };

/** `sessions_archive` / `sessions_restore` reply → a {@link BatchResult} (tolerant of odd shapes). */
export function batchResultOf(data: unknown): BatchResult {
  const d = (data ?? {}) as { done?: unknown; failed?: unknown };
  const done = Array.isArray(d.done) ? d.done.filter((x): x is string => typeof x === 'string') : [];
  const failed = Array.isArray(d.failed)
    ? d.failed
        .filter((f): f is { session: string; message?: unknown } => !!f && typeof (f as { session?: unknown }).session === 'string')
        .map((f) => ({ session: f.session, message: typeof f.message === 'string' ? f.message : 'failed' }))
    : [];
  return { done, failed };
}

export type CreateHubOptions = {
  client?: HostClient;
  setTimer?: (ms: number, cb: () => void) => unknown;
  clearTimer?: (handle: unknown) => void;
  /**
   * perf-app: the frame scheduler hub events are applied on — one store
   * write per animation frame for every card change that arrived (a 200-child
   * graph flips hundreds of cards a second). `null` applies each event at once
   * (the unit-test default, so `emit` then `expect` keeps working).
   */
  schedule?: ((flush: () => void) => void) | null;
};

const frameSchedule = (flush: () => void) => {
  // Hidden documents get no animation frames: a short timer still drains the queue.
  if (typeof requestAnimationFrame === 'function' && !(typeof document !== 'undefined' && document.hidden)) requestAnimationFrame(() => flush());
  else setTimeout(flush, 50);
};

/** One host's hub (call inside a reactive root). */
/**
 * chat-vanish: "the host answers again" listeners (a list reload succeeded after the hub's
 * events stream dropped — a host restart, the frozen-host watchdog, a network blip). Session
 * streams give up after 2 min of trouble and wait for Retry; this is what wakes them.
 */
type HostBackListener = (hostId: string) => void;
const hostBackListeners = new Set<HostBackListener>();
export function onHostBack(fn: HostBackListener): () => void {
  hostBackListeners.add(fn);
  return () => hostBackListeners.delete(fn);
}

export function createHub(hostId: string, opts: CreateHubOptions = {}): HubHandle {
  const client = opts.client ?? hostClient(hostId);
  const setTimer = opts.setTimer ?? ((ms: number, cb: () => void) => setTimeout(cb, ms));
  const clearTimer = opts.clearTimer ?? ((h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>));
  const [state, setState] = createStore<HubState>({
    hostId,
    cards: {},
    archived: [],
    archivedLoaded: false,
    archivedLoading: false,
    notices: [],
    meta: {},
    git: {},
    loaded: false,
    loading: false,
    live: false,
  });
  let events: Closable | undefined;
  let eventsFocus: ((session: string | null) => void) | undefined;
  let retry: unknown;
  let attempt = 0;
  let closed = false;
  let inflight: Promise<void> | undefined;
  let archivedInflight: Promise<void> | undefined;
  /** Sessions with an in-flight `session_archive`: a raced `sessions_list` snapshot must not resurrect them. */
  const archiving = new Set<string>();

  /** The events stream dropped since the last good list: the next good one is "host back". */
  let droppedSinceList = false;
  const reload = (): Promise<void> => {
    if (inflight) return inflight;
    setState('loading', true);
    inflight = client
      .request<unknown>('sessions_list')
      .then((data) => {
        const cards = cardsOf(data);
        const byId: Record<string, SessionCard> = {};
        const meta: Record<string, SessionMeta> = {};
        for (const c of cards) {
          if (archiving.has(c.session_id)) continue;
          byId[c.session_id] = { ...c };
          meta[c.session_id] = metaOf(c);
        }
        batch(() => {
          setState('cards', reconcile(byId));
          setState('meta', reconcile(meta));
          setState({ loaded: true, loading: false, error: undefined });
        });
        if (droppedSinceList) {
          droppedSinceList = false;
          for (const fn of hostBackListeners) fn(hostId);
        }
      })
      .catch((error: unknown) => {
        setState({ loading: false, error: toAppError(error) });
      })
      .finally(() => {
        inflight = undefined;
      });
    return inflight;
  };

  const schedule = opts.schedule === undefined ? (import.meta.env?.MODE === 'test' ? null : frameSchedule) : opts.schedule;
  /** Hub events since the last frame (bounded by one frame's arrivals). */
  let pending: HubEvent[] = [];
  const flushPending = () => {
    const batchOf = pending;
    pending = [];
    if (!batchOf.length || closed) return;
    setState(
      produce((s) => {
        for (const event of batchOf) applyHubEvent(s, event);
      }),
    );
  };

  const openEvents = () => {
    if (closed) return;
    events = client.openEvents(
      (event) => {
        if (event.kind === 'ui') {
          uiSink(hostId, event);
          return;
        }
        // [ux-sessions] the host dropped events for this connection: re-list, never stay stale.
        if (event.kind === 'resync') {
          void reload();
          if (state.archivedLoaded) refreshArchivedSoon();
          return;
        }
        // Another client archived / restored a batch: an open Archived list is stale.
        if (((event.kind === 'sessions_removed' && event.archived) || event.kind === 'sessions_changed') && !state.bulk && state.archivedLoaded) refreshArchivedSoon();
        if (!schedule) {
          setState(produce((s) => applyHubEvent(s, event)));
          return;
        }
        pending.push(event);
        if (pending.length === 1) schedule(flushPending);
      },
      (error) => {
        // The stream dropped: retry with backoff, then resync the list (events may be lost).
        events = undefined;
        eventsFocus = undefined;
        setState('live', false);
        if (closed) return;
        droppedSinceList = true;
        if (error) setState('error', toAppError(error));
        const delay = EVENTS_RETRY_MS[Math.min(attempt, EVENTS_RETRY_MS.length - 1)]!;
        attempt += 1;
        retry = setTimer(delay, () => {
          retry = undefined;
          openEvents();
          void reload();
        });
      },
    );
    eventsFocus = (session) => (events as EventsStream | undefined)?.focus?.(session);
    attempt = 0;
    setState('live', true);
  };

  /** Re-reads an open Archived list once a burst of batch events has settled. */
  let archivedTimer: unknown;
  const refreshArchivedSoon = () => {
    if (archivedTimer !== undefined) clearTimer(archivedTimer);
    archivedTimer = setTimer(ARCHIVED_REFRESH_MS, () => {
      archivedTimer = undefined;
      if (!closed && state.archivedLoaded) void loadArchived(true);
    });
  };

  const loadArchived = (force = false): Promise<void> => {
    if (archivedInflight) return force ? archivedInflight.then(() => loadArchived(true)) : archivedInflight;
    if (state.archivedLoaded && !force) return Promise.resolve();
    setState('archivedLoading', true);
    archivedInflight = client
      .request<unknown>('sessions_list', { archived: true })
      .then((data) => {
        setState('archived', reconcile(cardsOf(data), { key: 'session_id' }));
        setState({ archivedLoaded: true, archivedLoading: false });
      })
      .catch((error: unknown) => {
        setState({ archivedLoading: false, error: toAppError(error) });
      })
      .finally(() => {
        archivedInflight = undefined;
      });
    return archivedInflight;
  };

  const runBatch = async (kind: 'archive' | 'restore', ids: readonly string[]): Promise<BatchResult> => {
    const unique = [...new Set(ids)];
    if (!unique.length) return { done: [], failed: [] };
    // Archive: the rows leave the list now (one store write), an open Archived list gains them.
    const taken = kind === 'archive' ? unique.map((id) => state.cards[id]).filter((c): c is SessionCard => !!c) : [];
    const takenMeta = new Map(taken.map((c) => [c.session_id, state.meta[c.session_id]]));
    batch(() => {
      setState('bulk', { kind, total: unique.length, done: 0 });
      if (kind === 'archive')
        setState(
          produce((s) => {
            for (const c of taken) {
              delete s.cards[c.session_id];
              delete s.meta[c.session_id];
            }
            if (s.archivedLoaded) s.archived = [...taken.map((c) => ({ ...c, run_state: 'idle' as const })), ...s.archived];
          }),
        );
    });
    const putBack = (ids: Set<string>) =>
      setState(
        produce((s) => {
          for (const c of taken) {
            if (!ids.has(c.session_id)) continue;
            s.cards[c.session_id] = c;
            s.meta[c.session_id] = takenMeta.get(c.session_id) ?? {};
          }
          if (s.archivedLoaded) s.archived = s.archived.filter((c) => !ids.has(c.session_id) || !takenMeta.has(c.session_id));
        }),
      );
    try {
      const result = batchResultOf(await client.request<unknown>(kind === 'archive' ? 'sessions_archive' : 'sessions_restore', { sessions: unique }));
      const done = new Set(result.done);
      batch(() => {
        if (kind === 'archive') putBack(new Set(unique.filter((id) => !done.has(id))));
        setState('bulk', undefined);
      });
      // The trash now holds the files under their real paths: re-read it (exact count + rows).
      if (state.archivedLoaded || kind === 'restore') void loadArchived(true);
      if (kind === 'restore') void reload();
      return result;
    } catch (error) {
      const app = toAppError(error);
      batch(() => {
        if (kind === 'archive') putBack(new Set(unique));
        setState({ bulk: undefined, error: app });
      });
      void reload();
      if (state.archivedLoaded) void loadArchived(true);
      return { done: [], failed: unique.map((session) => ({ session, message: app.message })), error: app };
    }
  };

  void reload();
  openEvents();

  return {
    hostId,
    state,
    reload,
    patchCard(sessionId, patch) {
      if (!state.cards[sessionId]) return;
      setState('cards', sessionId, (prev) => {
        if (!prev) return prev;
        const next: Record<string, unknown> = { ...prev };
        for (const [k, v] of Object.entries(patch)) {
          if (v === undefined) delete next[k];
          else next[k] = v;
        }
        return next as SessionCard;
      });
    },
    async setMeta(sessionId, patch) {
      // Optimistic: the events stream will carry the host's card a moment later.
      setState('meta', sessionId, (m) => ({ ...m, ...(patch.pinned !== undefined ? { pinned: patch.pinned } : {}), ...(patch.unread !== undefined ? { unread: patch.unread } : {}), ...(patch.titleLock !== undefined ? { titleLock: patch.titleLock } : {}) }));
      try {
        await client.request('session_meta_set', {
          session: sessionId,
          ...(patch.pinned !== undefined ? { pinned: patch.pinned } : {}),
          ...(patch.unread !== undefined ? { unread: patch.unread } : {}),
          ...(patch.titleLock !== undefined ? { title_locked: patch.titleLock } : {}),
        });
      } catch (error) {
        setState('error', toAppError(error));
        void reload();
      }
    },
    loadArchived,
    archiveMany: (ids) => runBatch('archive', ids),
    restoreMany: (ids) => runBatch('restore', ids),
    async archive(sessionId) {
      const prevCard = state.cards[sessionId] ? { ...state.cards[sessionId] } : undefined;
      const prevMeta = state.meta[sessionId] ? { ...state.meta[sessionId] } : undefined;
      const hasFile = !!prevCard?.file;
      archiving.add(sessionId);
      const racing = inflight;
      batch(() => {
        setState('error', undefined);
        setState(
          produce((s) => {
            delete s.cards[sessionId];
            delete s.meta[sessionId];
            if (s.archivedLoaded && prevCard && hasFile && !s.archived.some((c) => c.session_id === sessionId)) {
              s.archived = [{ ...prevCard, run_state: 'idle' as const }, ...s.archived];
            }
          }),
        );
      });
      try {
        await client.request('session_archive', { session: sessionId });
        if (racing) {
          void racing.then(() => {
            if (!closed)
              setState(
                produce((s) => {
                  delete s.cards[sessionId];
                  delete s.meta[sessionId];
                }),
              );
          });
        }
        // An open Archived list gains the chat (re-read: its trash path and exact count).
        if (state.archivedLoaded) void loadArchived(true);
      } catch (error) {
        const app = toAppError(error);
        // E_NOT_FOUND means the session is already gone / had no file on disk: keep it removed silently.
        if (app.code === 'E_NOT_FOUND') {
          if (state.archivedLoaded) {
            setState('archived', (list) => list.filter((row) => row.session_id !== sessionId));
            void loadArchived(true);
          }
          void reload();
          return;
        }
        batch(() => {
          setState(
            produce((s) => {
              if (prevCard) {
                s.cards[sessionId] = prevCard;
                s.meta[sessionId] = prevMeta ?? metaOf(prevCard);
              }
              if (s.archivedLoaded && hasFile) s.archived = s.archived.filter((row) => row.session_id !== sessionId);
            }),
          );
          setState('error', app);
        });
        void reload();
      } finally {
        archiving.delete(sessionId);
      }
    },
    async restore(sessionId) {
      try {
        const card = await client.request<SessionCard>('session_restore', { session: sessionId });
        batch(() => {
          setState('archived', (list) => list.filter((row) => row.session_id !== sessionId));
          if (card && typeof card.session_id === 'string') {
            setState('cards', card.session_id, card);
            setState('meta', card.session_id, metaOf(card));
          }
        });
      } catch (error) {
        setState('error', toAppError(error));
      }
    },
    async deleteArchived(sessionId) {
      setState('error', undefined);
      try {
        // An archived fetch that raced this delete (fired before it finished — the
        // archive-then-delete flow does exactly that) answered from a snapshot that still
        // holds the row: its reconcile must not resurrect it. Re-remove after it lands.
        const racing = archivedInflight;
        await client.request('session_delete', { session: sessionId });
        setState('archived', (list) => list.filter((row) => row.session_id !== sessionId));
        if (racing) void racing.then(() => {
          if (!closed) setState('archived', (list) => list.filter((row) => row.session_id !== sessionId));
        });
      } catch (error) {
        const app = toAppError(error);
        if (app.code === 'E_NOT_FOUND') {
          setState('archived', (list) => list.filter((row) => row.session_id !== sessionId));
          return;
        }
        setState('error', app);
      }
    },
    async emptyTrash() {
      try {
        const data = await client.request<{ deleted?: unknown }>('trash_empty');
        setState('archived', []);
        return typeof data?.deleted === 'number' ? data.deleted : 0;
      } catch (error) {
        setState('error', toAppError(error));
        return 0;
      }
    },
    focus(session) {
      eventsFocus?.(session);
    },
    dismissNotice(id) {
      setState('notices', (list) => list.filter((n) => n.id !== id));
    },
    close() {
      closed = true;
      if (retry !== undefined) clearTimer(retry);
      if (archivedTimer !== undefined) clearTimer(archivedTimer);
      events?.close();
      events = undefined;
    },
  };
}

export function createHubRegistry(factory: (hostId: string) => HubHandle = createHub) {
  const open = new Map<string, { handle: HubHandle; dispose: () => void }>();
  const [rev, setRev] = createSignal(0);
  return {
    ensure(hostId: string): HubHandle {
      let entry = open.get(hostId);
      if (!entry) {
        entry = createRoot((dispose) => ({ handle: factory(hostId), dispose }));
        open.set(hostId, entry);
        setRev((n) => n + 1);
      }
      return entry.handle;
    },
    peek(hostId: string): HubHandle | undefined {
      rev();
      return open.get(hostId)?.handle;
    },
    close(hostId: string): void {
      const entry = open.get(hostId);
      if (!entry) return;
      entry.handle.close();
      entry.dispose();
      open.delete(hostId);
      setRev((n) => n + 1);
    },
    closeAll(): void {
      for (const entry of open.values()) {
        entry.handle.close();
        entry.dispose();
      }
      open.clear();
      setRev((n) => n + 1);
    },
    /** Host ids with an open hub, in insertion order. */
    keys(): string[] {
      rev();
      return [...open.keys()];
    },
  };
}

export type HubRegistry = ReturnType<typeof createHubRegistry>;

let shared: HubRegistry | undefined;
/** The app's per-host hubs. */
export function hubs(): HubRegistry {
  if (!shared) shared = createRoot(() => createHubRegistry());
  return shared;
}
export function resetHubs(): void {
  shared?.closeAll();
  shared = undefined;
}
