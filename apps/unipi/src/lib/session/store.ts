/**
 * Session store (app spec §3.3 `session`, §3.4; WP-A2): one Solid store per
 * open session fed by its host stream, plus the turns index, the outbox and
 * the commands the UI sends. At most **8** sessions stay subscribed (LRU);
 * an evicted session drops its state and reopens with a snapshot.
 *
 *   const s = sessions().ensure(hostId, sessionId); const release = s.acquire();
 *   s.state.run.running · s.turns() · s.send('hi') · s.retry() · release()
 *
 * Components never call `invoke`: everything goes through `lib/host`.
 */
import { batch, createMemo, createRoot, createSignal, untrack, type Accessor } from 'solid-js';
import { createStore, produce, unwrap, type SetStoreFunction } from 'solid-js/store';
import { toAppError, type AppError } from '@unipi/protocol';
import { hostClient } from '../host/factory';
import type { HostClient, RpcCommand } from '../host/client';
import { applyEvents, applySnapshot, initialState, reconcileEntries, reconcileSince, type ApplyResult } from './reducer';
import { enqueue, loadOutbox, markAcked, markLost, markSending, removeItem, saveOutbox, sweep } from './outbox';
import { SessionStreamClient, type StreamOptions } from './stream';
import { createTurnsIndex, type Turn } from './turns';
import { hubs } from './hub';
import { routeNotify, reportTextOf } from '../reports';
import type { CommandInfo, OutboxImage, OutboxItem, OutboxMode, SessionEntry, SessionState } from './types';

/** Live subscriptions kept open (app spec §3.3). */
export const SESSION_LRU = 4;
/** How often in-flight outbox items are time-settled. */
const SWEEP_MS = 2_000;

/** Where ui-protocol registry traffic goes (U9 installs its registry; default: nowhere). */
export type SessionUiSink = (hostId: string, sessionId: string, message: Record<string, unknown>) => void;
let uiSink: SessionUiSink = () => {};
export function setSessionUiSink(sink: SessionUiSink | undefined): void {
  uiSink = sink ?? (() => {});
}

export type SendMode = 'auto' | OutboxMode;
export type SendOptions = { images?: OutboxImage[]; mode?: SendMode };

export type SessionHandle = {
  readonly key: string;
  readonly hostId: string;
  readonly sessionId: Accessor<string>;
  readonly state: SessionState;
  /** Turn units of the current branch (settled turns keep identity). */
  readonly turns: Accessor<Turn[]>;
  /** Sends a prompt (`auto`: steer while running, else prompt); shows as an outbox bubble until the host echoes it. */
  send(text: string, opts?: SendOptions): OutboxItem;
  /** Re-sends a `lost` / `waiting` item. */
  resend(ref: string): void;
  /** Removes an outbox item (✕). */
  dismiss(ref: string): void;
  abort(): Promise<void>;
  cmd<T = unknown>(cmd: RpcCommand): Promise<T>;
  /** Subscribes the stream to bus topics (`_oino/topics`); the returned sticky
   *  values are merged into `state.topics`, later `topic` events follow. */
  subscribeTopics(patterns: readonly string[]): void;
  /** Full entries by id (tool outputs the snapshot truncated, §3.4 step 7). */
  getEntries(ids: readonly string[]): Promise<SessionEntry[]>;
  /** The "Lost the connection · Retry" action. */
  retry(): void;
  /** Reopens after `taken_over` (takes the session back from the other runner). */
  takeBack(): void;
  /** Pins the session against LRU eviction while a view shows it; returns the release. */
  acquire(): () => void;
  close(): void;
};

const keyOf = (hostId: string, sessionId: string) => `${hostId}/${sessionId}`;

function sendCommand(mode: OutboxMode, item: OutboxItem): RpcCommand {
  const images = item.images?.map((img) => ({ type: 'image', data: img.data, mimeType: img.mimeType }));
  const base = { message: item.text, ...(images?.length ? { images } : {}) };
  if (mode === 'steer') return { type: 'steer', ...base };
  if (mode === 'followUp') return { type: 'follow_up', ...base };
  return { type: 'prompt', ...base };
}

export type CreateSessionOptions = StreamOptions & { client?: HostClient; onEvict?: () => void };

/** One open session (call inside a reactive root). */
export function createSession(hostId: string, sessionId: string, opts: CreateSessionOptions = {}): SessionHandle {
  const client = opts.client ?? hostClient(hostId);
  const [state, setState] = createStore<SessionState>({ ...initialState(hostId, sessionId), outbox: loadOutbox(hostId, sessionId) });
  const [id, setId] = createSignal(sessionId);
  const index = createTurnsIndex();
  const turns = createMemo(() => {
    // Track only these signals; the index itself reads the raw (unwrapped) state untracked,
    // so a 2 000-turn rebuild never subscribes to every entry field.
    void state.entries.length;
    void state.entriesRev;
    void state.leafId;
    void state.run.running;
    void state.run.startedAt;
    const s = state.streaming;
    if (s) {
      const content = s.message.content;
      if (Array.isArray(content)) {
        for (const p of content) {
          void p.text;
          void p.thinking;
          void p.arguments;
          void p.partialJson;
        }
      }
    }
    for (const t of Object.values(state.liveTools)) {
      void t.done;
      void t.partial;
      void t.isError;
    }
    return untrack(() => index.update(unwrap(state)));
  });
  let refs = 0;
  let sweepTimer: ReturnType<typeof setInterval> | undefined;
  let reconciling = false;
  let reconcileAgain = false;

  const mutate = (fn: (s: SessionState) => void) => setState(produce(fn));
  const persist = () => saveOutbox(hostId, id(), unwrap(state).outbox);

  const ensureSweep = () => {
    if (sweepTimer || !state.outbox.some((o) => o.status === 'sending' || o.status === 'acked')) return;
    sweepTimer = setInterval(() => {
      let lost: string[] = [];
      mutate((s) => (lost = sweep(s)));
      if (lost.length) persist();
      if (!state.outbox.some((o) => o.status === 'sending' || o.status === 'acked')) {
        clearInterval(sweepTimer);
        sweepTimer = undefined;
      }
    }, SWEEP_MS);
  };

  const dispatch = (ref: string) => {
    const item = state.outbox.find((o) => o.ref === ref);
    if (!item) return;
    mutate((s) => markSending(s, ref));
    ensureSweep();
    stream.cmd<{ disposition?: string } | undefined>(sendCommand(item.mode, unwrap(item))).then(
      (reply) => {
        // An extension slash command (`/plan on`, `/permission auto`, …) runs
        // immediately and answers `handled`: no user message will ever settle
        // the optimistic bubble, so drop it instead of leaving it "Sending…".
        if (reply?.disposition === 'handled') mutate((s) => removeItem(s, ref));
        else mutate((s) => markAcked(s, ref));
        persist();
      },
      (error: unknown) => {
        mutate((s) => markLost(s, ref, toAppError(error).message));
        persist();
      },
    );
  };

  const flushWaiting = () => {
    for (const o of state.outbox.filter((x) => x.status === 'waiting')) dispatch(o.ref);
    persist();
  };

  const reconcile = () => {
    if (reconciling) {
      reconcileAgain = true;
      return;
    }
    const { since, needed } = reconcileSince(state);
    if (!needed) return;
    reconciling = true;
    stream
      .cmd<{ entries?: SessionEntry[]; leafId?: string | null }>({ type: 'get_entries', ...(since ? { since } : {}) })
      .then((reply) => {
        if (reply && Array.isArray(reply.entries)) mutate((s) => reconcileEntries(s, since, reply.entries!, reply.leafId));
      })
      .catch(() => {
        // Keep the provisional entries; the next run end or a reset fixes them.
      })
      .finally(() => {
        reconciling = false;
        if (reconcileAgain) {
          reconcileAgain = false;
          reconcile();
        }
      });
  };

  const refreshCommands = () => {
    stream
      .cmd<{ commands?: CommandInfo[] }>({ type: 'get_commands' })
      .then((reply) => {
        if (reply && Array.isArray(reply.commands)) setState('commands', reply.commands);
      })
      .catch(() => {});
  };

  const desiredTopics = new Set<string>();
  const sendTopicSub = (patterns: readonly string[]) => {
    if (!patterns.length) return;
    stream
      .cmd<{ topics?: unknown[] }>({ type: '_oino/topics', subscribe: [...patterns], unsubscribe: [] })
      .then((reply) => {
        const list = Array.isArray(reply?.topics) ? reply!.topics! : [];
        if (!list.length) return;
        mutate((s) => {
          for (const raw of list) {
            const t = raw as { topic?: unknown; data?: unknown; sticky?: unknown; publisher?: unknown };
            if (typeof t.topic !== 'string' || !t.topic) continue;
            s.topics[t.topic] = { data: t.data, sticky: t.sticky !== false, publisher: typeof t.publisher === 'string' ? t.publisher : '' };
          }
        });
      })
      .catch(() => {});
  };

  const follow = (r: ApplyResult) => {
    if (r.nameChanged !== undefined) hubs().peek(hostId)?.patchCard(id(), { name: r.nameChanged || undefined });
    if (r.ui) for (const m of r.ui) uiSink(hostId, id(), m);
    if (r.reconcile) queueMicrotask(reconcile);
    if (r.commandsChanged) queueMicrotask(refreshCommands);
  };

  const stream: SessionStreamClient = new SessionStreamClient(
    client,
    sessionId,
    {
      seq: () => state.seq,
      snapshot: (snap) => {
        let r: ApplyResult = {};
        batch(() => mutate((s) => (r = applySnapshot(s, snap))));
        follow(r);
        if (desiredTopics.size > 0) queueMicrotask(() => stream.live && sendTopicSub([...desiredTopics]));
        return r;
      },
      events: (items) => {
        let r: ApplyResult = {};
        const before = state.seq;
        batch(() => mutate((s) => (r = applyEvents(s, items))));
        // E-05: an extension command's reply (`unipi-response`) is a report/toast, not a silent row.
        for (const it of items) {
          if (it.seq <= before) continue;
          const text = reportTextOf((it.event ?? {}) as Record<string, unknown>);
          if (text) routeNotify(text, 'info', keyOf(hostId, sessionId), Date.now(), { inTimeline: true });
        }
        follow(r);
        return r;
      },
      conn: (conn, error?: AppError) => {
        batch(() => {
          setState('conn', conn);
          if (error) setState('connError', error);
          else if (conn === 'live') setState('connError', undefined);
        });
        if (conn === 'live') {
          flushWaiting();
          if (reconcileSince(state).needed && !state.run.running) queueMicrotask(reconcile);
        }
      },
      replaced: (next) => {
        setId(next);
        setState('sessionId', next);
      },
    },
    opts,
  );
  stream.open();

  return {
    key: keyOf(hostId, sessionId),
    hostId,
    sessionId: id,
    state,
    turns,
    send(text, sendOpts = {}) {
      const mode: OutboxMode = !sendOpts.mode || sendOpts.mode === 'auto' ? (state.run.running ? 'steer' : 'prompt') : sendOpts.mode;
      let item!: OutboxItem;
      mutate((s) => (item = { ...enqueue(s, { text, images: sendOpts.images, mode }, stream.live) }));
      if (stream.live) dispatch(item.ref);
      else persist();
      return item;
    },
    resend(ref) {
      if (stream.live) dispatch(ref);
      else {
        mutate((s) => {
          const o = s.outbox.find((x) => x.ref === ref);
          if (o) o.status = 'waiting';
        });
        persist();
      }
    },
    dismiss(ref) {
      mutate((s) => removeItem(s, ref));
      persist();
    },
    abort: () => stream.cmd({ type: 'abort' }).then(() => undefined),
    cmd: (cmd) => stream.cmd(cmd),
    subscribeTopics(patterns) {
      const fresh: string[] = [];
      for (const p of patterns) {
        if (!desiredTopics.has(p)) {
          desiredTopics.add(p);
          fresh.push(p);
        }
      }
      if (!stream.live) return;
      sendTopicSub(fresh.length ? fresh : patterns);
    },
    async getEntries(ids) {
      const reply = await stream.cmd<{ entries?: SessionEntry[] }>({ type: 'get_entries', ids: [...ids] });
      return Array.isArray(reply?.entries) ? reply.entries : [];
    },
    retry: () => stream.retryNow(),
    takeBack: () => {
      setState('banner', undefined);
      stream.open();
    },
    acquire() {
      refs++;
      let released = false;
      return () => {
        if (released) return;
        released = true;
        refs = Math.max(0, refs - 1);
      };
    },
    get refs() {
      return refs;
    },
    close() {
      if (sweepTimer) clearInterval(sweepTimer);
      sweepTimer = undefined;
      stream.close();
    },
  } as SessionHandle & { readonly refs: number };
}

/** For tests: the store setter is internal; this exposes a session's raw state for assertions. */
export function snapshotOf(handle: SessionHandle): SessionState {
  return structuredClone(unwrap(handle.state));
}

type Entry = { handle: SessionHandle & { readonly refs: number }; dispose: () => void };

/**
 * A session whose open failed for good (`closed` with an error, or `lost`)
 * — not one taken over (that one waits for "Take back").
 */
export function openFailed(state: Pick<SessionState, 'conn' | 'connError' | 'banner'>): boolean {
  if (state.banner?.kind === 'taken_over') return false;
  // Gone for good (the host has no such session): never re-opened behind the user's back.
  if (sessionMissing(state)) return false;
  return state.conn === 'lost' || (state.conn === 'closed' && !!state.connError);
}

/** The host answered that this session does not exist (`E_NOT_FOUND`). */
export function sessionMissing(state: Pick<SessionState, 'conn' | 'connError'>): boolean {
  return state.conn === 'closed' && state.connError?.code === 'E_NOT_FOUND';
}

/** Retries a failed session outside the caller's computation (ensure runs inside memos). */
function retryLater(handle: SessionHandle): void {
  queueMicrotask(() => {
    if (openFailed(handle.state)) handle.retry();
  });
}

let reclickInstalled = false;
/**
 * Re-clicking the row of a chat that could not open retries it: the link's
 * href is the route already shown, so the router does nothing. One delegated
 * listener (sidebar rows, the phone Chat list, any `#/s/…` link).
 */
function installReclickRetry(registry: { peek(hostId: string, sessionId: string): SessionHandle | undefined }): void {
  if (reclickInstalled || typeof document === 'undefined') return;
  reclickInstalled = true;
  document.addEventListener(
    'click',
    (e) => {
      const link = (e.target as Element | null)?.closest?.('a[href^="#/s/"]');
      const href = link?.getAttribute('href');
      if (!href || href !== location.hash) return;
      const m = /^#\/s\/([^/]+)\/([^/?#]+)/.exec(href);
      if (!m) return;
      const handle = registry.peek(decodeURIComponent(m[1]!), decodeURIComponent(m[2]!));
      if (handle) retryLater(handle);
    },
    true,
  );
}

/** The per-app registry of open sessions (LRU 8, pinned while acquired). */
export function createSessionRegistry(limit = SESSION_LRU, factory: (hostId: string, sessionId: string) => SessionHandle = createSession) {
  const open = new Map<string, Entry>();
  const [rev, setRev] = createSignal(0);

  const evict = () => {
    if (open.size <= limit) return;
    for (const [key, entry] of open) {
      if (open.size <= limit) break;
      if (entry.handle.refs > 0) continue;
      entry.handle.close();
      entry.dispose();
      open.delete(key);
    }
  };

  return {
    /** The open session (created + subscribed when missing); marks it most recently used. */
    ensure(hostId: string, sessionId: string): SessionHandle {
      const key = keyOf(hostId, sessionId);
      const hit = open.get(key);
      if (hit) {
        open.delete(key);
        open.set(key, hit);
        // Coming back to a chat that could not open tries again (never a blank chat forever).
        if (hit.handle.state && untrack(() => openFailed(hit.handle.state))) retryLater(hit.handle);
        return hit.handle;
      }
      const entry = createRoot((dispose) => ({ handle: factory(hostId, sessionId) as Entry['handle'], dispose }));
      open.set(key, entry);
      evict();
      setRev((n) => n + 1);
      return entry.handle;
    },
    /** The open session without opening or touching it. */
    peek(hostId: string, sessionId: string): SessionHandle | undefined {
      rev();
      return open.get(keyOf(hostId, sessionId))?.handle;
    },
    close(hostId: string, sessionId: string): void {
      const key = keyOf(hostId, sessionId);
      const entry = open.get(key);
      if (!entry) return;
      entry.handle.close();
      entry.dispose();
      open.delete(key);
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
    /** Open keys, least recently used first. */
    keys(): string[] {
      rev();
      return [...open.keys()];
    },
  };
}

export type SessionRegistry = ReturnType<typeof createSessionRegistry>;

let shared: SessionRegistry | undefined;
/** The app's session registry. */
export function sessions(): SessionRegistry {
  if (!shared) {
    shared = createRoot(() => createSessionRegistry());
    installReclickRetry({ peek: (h, id) => shared?.peek(h, id) });
  }
  return shared;
}
/** Tests: drop the shared registry (closing every session). */
export function resetSessions(): void {
  shared?.closeAll();
  shared = undefined;
}

export type { SetStoreFunction };
