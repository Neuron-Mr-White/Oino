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
import { batch, createEffect, createMemo, createRoot, createSignal, untrack, type Accessor } from 'solid-js';
import { createStore, produce, unwrap, type SetStoreFunction } from 'solid-js/store';
import { toAppError, type AppError } from '@unipi/protocol';
import { appLog } from '../appLog';
import { hostClient } from '../host/factory';
import type { HostClient, RpcCommand } from '../host/client';
import { fetchFullEntries, fetchFullEntriesSettled } from '../host/fullEntries';
import { applyEvents, applySnapshot, initialState, prependEntries, reconcileEntries, reconcileSince, replaceBranch, trimHistory, type ApplyResult } from './reducer';
import { enqueue, loadOutbox, markAcked, markLost, markSending, modeCommandKeyOf, promptInFlight, removeItem, saveOutbox, sweep } from './outbox';
import { SessionStreamClient, type StreamOptions } from './stream';
import { createTurnsIndex, type Turn } from './turns';
import { assembleChunk, type HostPage } from './chunks';
import { hubs, onHostBack } from './hub';
import { autonomyKnown, autonomyLevel } from './autonomy';
import { rememberAutonomy } from './chatAutonomy';
import { resetFusionOverrides } from '../model/fusion';
import { routeNotify, reportTextOf } from '../reports';
import { isUnknownOpError } from '../../components/status/lhProgress';
import { createFullMessages, type FullMessages } from './fullMessages';
import type { CommandInfo, OutboxImage, OutboxItem, OutboxMode, SessionEntry, SessionState } from './types';

/** Live subscriptions kept open (app spec §3.3; perf-app: 3 — the shown chat and two recent ones). */
export const SESSION_LRU = 3;
/** perf-app: loaded entries a chat keeps once no view shows it (older pages load again on scroll). */
export const RELEASED_KEEP = 2 * 80;
/** perf-app: loaded entries a shown chat keeps once its reader is back at the bottom. */
export const LOADED_MAX = 1_200;
/** Entries per "load earlier" page (the host caps and turn-aligns it). */
export const EARLIER_PAGE = 80;
/** How often in-flight outbox items are time-settled. */
const SWEEP_MS = 2_000;

/** Where ui-protocol registry traffic goes (U9 installs its registry; default: nowhere). */
export type SessionUiSink = (hostId: string, sessionId: string, message: Record<string, unknown>) => void;
let uiSink: SessionUiSink = () => {};
export function setSessionUiSink(sink: SessionUiSink | undefined): void {
  uiSink = sink ?? (() => {});
}

export type SendMode = 'auto' | OutboxMode;
export type SendOptions = { images?: OutboxImage[]; mode?: SendMode; onResult?: (result: { accepted: boolean; reason?: string }) => void };

export type SessionHandle = {
  readonly key: string;
  readonly hostId: string;
  readonly sessionId: Accessor<string>;
  readonly state: SessionState;
  /** Turn units of the current branch (settled turns keep identity). */
  readonly turns: Accessor<Turn[]>;
  /** perf-webview: how often the turns index ran (`turns()` builds lazily, on read). */
  readonly turnsBuilds?: () => number;
  /** Sends a prompt (`auto`: steer while running, else prompt); shows as an outbox bubble until the host echoes it. */
  send(text: string, opts?: SendOptions): OutboxItem;
  /** Re-sends a `lost` / `waiting` item. */
  resend(ref: string): void;
  /** Removes an outbox item (✕). */
  dismiss(ref: string): void;
  abort(): Promise<void>;
  cmd<T = unknown>(cmd: RpcCommand): Promise<T>;
  /** Re-read the authoritative selection after a command, including a failed observer. */
  refreshModel?(): Promise<void>;
  /** Subscribes the stream to bus topics (`_oino/topics`); the returned sticky
   *  values are merged into `state.topics`, later `topic` events follow. */
  subscribeTopics(patterns: readonly string[]): void;
  /** Re-checks board scope after setup/archive, without restarting this chat. */
  refreshBoard?(): Promise<void>;
  /** Full entries by id (tool outputs the snapshot truncated, §3.4 step 7). */
  getEntries(ids: readonly string[]): Promise<SessionEntry[]>;
  /** Hotfix 1.0.9: fetches the full text of host-cut prose entries (assistant / user / custom) as their rows render. */
  readonly fullMessages?: FullMessages;
  /** perf-open: fetches the next older page of the branch (`state.history`); no-op when none is left or one is in flight. */
  loadEarlier?(): Promise<void>;
  /** The "Lost the connection · Retry" action. */
  retry(): void;
  /** Reopens after `taken_over` (takes the session back from the other runner). */
  takeBack(): void;
  /** Pins the session against LRU eviction while a view shows it; returns the release. */
  acquire(): () => void;
  /** perf-app: drops loaded history beyond the newest `keep` entries (paged chats; reloads on scroll). */
  trimLoaded?(keep: number): boolean;
  /**
   * [chat-tree] pi `navigateTree` (`_oino/navigate_tree`): moves the leaf to `targetId` (a
   * user message ⇒ its parent, its text comes back as `editorText`), then reloads the new
   * branch's tail page. Rejects while a run streams (core's own text).
   */
  navigate?(targetId: string, opts?: NavigateOptions): Promise<NavigateReply>;
  /** [chat-tree] Reloads the current branch's tail page (`get_entries{limit}`); joins one in flight. */
  reloadBranch?(): Promise<void>;
  /** chat-vanish: hung up locally (`close()`: LRU eviction, archive) — a view still showing it must re-ensure. */
  readonly closed?: boolean;
  close(): void;
};

const keyOf = (hostId: string, sessionId: string) => `${hostId}/${sessionId}`;

/** `_oino/navigate_tree` options (core `NavigateTreeOptions`). */
export type NavigateOptions = { summarize?: boolean; customInstructions?: string; label?: string };
/** `_oino/navigate_tree` reply (core `NavigateTreeResult`). */
export type NavigateReply = { cancelled?: boolean; editorText?: string; aborted?: boolean; summaryEntry?: SessionEntry };
type BranchPage = { entries?: SessionEntry[]; leafId?: string | null; total?: number; hasMore?: boolean; cursor?: string | null };
/** [chat-tree] The tail page a tree jump reloads (the snapshot's page size, host `PAGE_LIMIT`). */
export const BRANCH_PAGE = 80;

/**
 * The wire command for an outbox item.
 *
 * A steer / follow-up goes out as a `prompt` with a `streamingBehavior`
 * (`steer` / `followUp`), NOT as the old `steer` / `follow_up` commands:
 * those refuse an extension command (`/graph …` ⇒ CommandNotAllowed), so a
 * slash command typed while the run streamed stayed "Not sent · Retry" for
 * good. Core's `prompt()` runs an extension command right away (pi parity)
 * and otherwise queues it by the behaviour — and ignores the behaviour when
 * the run is idle (a run starts instead).
 */
async function sendCommand(hostId: string, mode: OutboxMode, item: OutboxItem): Promise<RpcCommand> {
  if (item.images && item.images.reduce((n, image) => n + image.data.length, 0) > 700 * 1024) {
    const { blobTransport } = await import('../blob');
    const images: Array<{ type: 'image'; mediaRef: string; mimeType: string }> = [];
    // Start uploads in image order, at most four at a time, and retain that order.
    for (let i = 0; i < item.images.length; i += 4) {
      images.push(...await Promise.all(item.images.slice(i, i + 4).map(async (image) => {
        const bytes = Uint8Array.from(atob(image.data), (c) => c.charCodeAt(0));
        const blob = await blobTransport().putBytes(hostId, bytes, image.mimeType);
        return { type: 'image' as const, mediaRef: blob.sha256, mimeType: image.mimeType };
      })));
    }
    return { type: 'prompt', message: item.text, images, ...(mode !== 'prompt' ? { streamingBehavior: mode } : {}) };
  }
  const images = item.images?.map((img) => ({ type: 'image', data: img.data, mimeType: img.mimeType }));
  const base = { message: item.text, ...(images?.length ? { images } : {}) };
  if (mode === 'steer') return { type: 'prompt', ...base, streamingBehavior: 'steer' };
  if (mode === 'followUp') return { type: 'prompt', ...base, streamingBehavior: 'followUp' };
  return { type: 'prompt', ...base };
}

export type CreateSessionOptions = StreamOptions & { client?: HostClient; onEvict?: () => void };

/** One open session (call inside a reactive root). */
export function createSession(hostId: string, sessionId: string, opts: CreateSessionOptions = {}): SessionHandle {
  const client = opts.client ?? hostClient(hostId);
  const [state, setState] = createStore<SessionState>({ ...initialState(hostId, sessionId), outbox: loadOutbox(hostId, sessionId) });
  const [id, setId] = createSignal(sessionId);
  const index = createTurnsIndex();
  // perf-webview (spec §4.7): the memo only tracks what the turns depend on and counts changes;
  // the index runs when someone READS `turns()`. A subscribed chat no view shows (status pane,
  // fleet, LRU) updates its store on every delta but never builds turns for it.
  const turnsRev = createMemo((n: number) => {
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
    return n + 1;
  }, 0);
  let builtRev = -1;
  let built: Turn[] = [];
  let builds = 0;
  const turns: Accessor<Turn[]> = () => {
    const rev = turnsRev();
    if (rev !== builtRev) {
      built = untrack(() => index.update(unwrap(state)));
      builtRev = rev;
      builds++;
    }
    return built;
  };
  let refs = 0;
  let closed = false;
  let sweepTimer: ReturnType<typeof setInterval> | undefined;
  let reconciling = false;
  let reconcileAgain = false;
  let sendTail: Promise<void> = Promise.resolve();
  const preparing = new Set<string>();

  const mutate = (fn: (s: SessionState) => void) => setState(produce(fn));
  const persist = () => saveOutbox(hostId, id(), unwrap(state).outbox);

  const ensureSweep = () => {
    if (sweepTimer || !state.outbox.some((o) => o.status === 'sending' || o.status === 'acked')) return;
    sweepTimer = setInterval(() => {
      let lost: string[] = [];
      mutate((s) => {
        for (const o of s.outbox) if (preparing.has(o.ref)) o.at = Date.now();
        lost = sweep(s);
      });
      if (lost.length) persist();
      if (!state.outbox.some((o) => o.status === 'sending' || o.status === 'acked')) {
        clearInterval(sweepTimer);
        sweepTimer = undefined;
      }
    }, SWEEP_MS);
  };

  const sendResults = new Map<string, NonNullable<SendOptions['onResult']>>();
  const dispatch = (ref: string) => {
    const item = state.outbox.find((o) => o.ref === ref);
    if (!item) return;
    // The mode was picked when the message was typed; the run may have ended
    // since (a retry of a lost follow-up while the session is idle). An idle
    // session starts a run for a plain prompt — a queued one would wait for a
    // run that never comes.
    if (!state.run.running && !promptInFlight(state, ref) && item.mode !== 'prompt')
      mutate((s) => {
        const o = s.outbox.find((x) => x.ref === ref);
        if (o) o.mode = 'prompt';
      });
    // Not eligible for settlement until the command actually leaves sendTail.
    mutate((s) => { const o = s.outbox.find((x) => x.ref === ref); if (o) o.status = 'preparing'; });
    ensureSweep();
    preparing.add(ref);
    const requireItem = () => {
      if (state.outbox.some((o) => o.ref === ref)) return;
      const error = 'Message disappeared before dispatch. Retry to send it.';
      console.error('[outbox]', error, ref, item.messageId);
      mutate((s) => { s.outbox.push({ ...unwrap(item), status: 'lost', error }); });
      persist();
      throw new Error(error);
    };
    const send = sendTail.then(async () => {
      requireItem();
      const command = { ...await sendCommand(hostId, item.mode, unwrap(item)), ...(item.messageId ? { retryMessageId: item.messageId } : {}) };
      requireItem();
      preparing.delete(ref);
      mutate((s) => markSending(s, ref));
      ensureSweep();
      try {
        return await stream.cmd<{ disposition?: string; messageId?: string; reason?: string } | undefined>(command);
      } catch (error) {
        if (item.mode !== 'prompt' || !toAppError(error).message.includes('Agent is already processing')) throw error;
        mutate((s) => {
          s.run.running = true;
          const o = s.outbox.find((x) => x.ref === ref);
          if (o) o.mode = 'steer';
        });
        try {
          return await stream.cmd<{ disposition?: string; messageId?: string; reason?: string } | undefined>({ ...command, streamingBehavior: 'steer' });
        } finally {
          stream.reopen(false);
        }
      }
    });
    sendTail = send.then(() => undefined, () => undefined);
    void send.then(
      (reply) => {
        const onResult = sendResults.get(ref);
        sendResults.delete(ref);
        if (reply?.disposition === 'refused') {
          mutate((s) => removeItem(s, ref));
          persist();
          onResult?.({ accepted: false, reason: reply.reason ?? 'Command refused' });
          return;
        }
        onResult?.({ accepted: true });
        // An extension slash command (`/plan on`, `/permission auto`, …) runs
        // immediately and answers `handled`: no user message will ever settle
        // the optimistic bubble, so drop it instead of leaving it "Sending…".
        // w5-ui (D3): not `/goal` `/swarm` `/graph` `/ralph <text>` — the extension sends the user's
        // text as its own message only after its hooks ran (seconds), so that bubble stays (acked)
        // until the message settles it; the timeline hides it once the command echo shows it.
        if (reply?.disposition === 'handled' && !modeCommandKeyOf(item.text)) mutate((s) => removeItem(s, ref));
        else mutate((s) => {
          markAcked(s, ref);
          const item = s.outbox.find((o) => o.ref === ref);
          if (item) item.messageId = reply?.messageId ?? item.messageId;
        });
        persist();
      },
      (error: unknown) => {
        const failure = toAppError(error);
        sendResults.get(ref)?.({ accepted: false, reason: failure.message });
        sendResults.delete(ref);
        mutate((s) => {
          markLost(s, ref, failure.message);
          const item = s.outbox.find((o) => o.ref === ref);
          const detail = failure.detail as { messageId?: string } | undefined;
          if (item && detail?.messageId) item.messageId = detail.messageId;
        });
        persist();
      },
    ).finally(() => preparing.delete(ref));
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
    const revision = state.entriesRev;
    const leaf = state.leafId;
    stream
      .cmd<BranchPage>({ type: 'get_entries', ...(since ? { since } : { limit: BRANCH_PAGE }) })
      .then((reply) => {
        // A history read can finish after newer live message_end events. Its
        // older tail must not erase those messages (R-1).
        if (state.entriesRev !== revision || state.leafId !== leaf) {
          reconcileAgain = true;
          return;
        }
        if (reply && Array.isArray(reply.entries)) {
          mutate((s) => since === undefined
            ? replaceBranch(s, reply)
            : reconcileEntries(s, since, reply.entries!, reply.leafId));
        }
      })
      .catch(() => {
        // The whole tail did not come (too large for one frame — a long turn's subagent
        // steps): reload the branch as a bounded page instead, or the chat keeps its stale
        // provisional state (owner 2026-11-06). A failed reload keeps what we have.
        void reloadBranch().catch(() => undefined);
      })
      .finally(() => {
        reconciling = false;
        if (reconcileAgain) {
          reconcileAgain = false;
          reconcile();
        }
      });
  };

  /** `woken` past our leaf: append what the wake wrote after `since`. */
  const catchUp = (since: string) => {
    const revision = state.entriesRev;
    const leaf = state.leafId;
    stream
      .cmd<{ entries?: SessionEntry[]; leafId?: string | null }>({ type: 'get_entries', since })
      .then((reply) => {
        if (state.entriesRev !== revision || state.leafId !== leaf) {
          // The response predates a live append; fetch again from the same
          // durable anchor rather than replacing the newer live tail.
          catchUp(since);
          return;
        }
        if (reply && Array.isArray(reply.entries) && reply.entries.length) mutate((s) => reconcileEntries(s, since, reply.entries!, reply.leafId));
      })
      .catch(() => {});
  };

  // [chat-load] One chunk of WHOLE turns (lib/session/chunks): as many host pages as it
  // takes to reach a real turn start, committed in one mutation. One chunk in flight at a time.
  const loadEarlier = async (): Promise<void> => {
    const h = state.history;
    if (!h?.hasMore || !h.cursor || h.loading || !stream.live) return;
    setState('history', { loading: true, error: undefined });
    try {
      const chunk = await assembleChunk(
        h.cursor,
        (before, limit) => stream.cmd<HostPage>({ type: 'get_entries', before, limit }),
        (id) => state.byId[id] !== undefined,
      );
      batch(() => mutate((s) => prependEntries(s, { entries: chunk.entries, total: chunk.total, hasMore: chunk.hasMore, cursor: chunk.cursor ?? null })));
    } catch (error) {
      setState('history', { loading: false, error: toAppError(error).message || 'Could not load earlier messages' });
    }
  };

  // [chat-tree] One branch reload at a time; a `session_tree` notice joins the one in flight.
  let branchLoad: Promise<void> | undefined;
  const reloadBranch = (): Promise<void> => {
    if (branchLoad) return branchLoad;
    const revision = state.entriesRev;
    const leaf = state.leafId;
    let stale = false;
    const load = stream
      .cmd<BranchPage>({ type: 'get_entries', limit: BRANCH_PAGE })
      .then((reply) => {
        stale = state.entriesRev !== revision || state.leafId !== leaf;
        if (!stale && reply && Array.isArray(reply.entries)) batch(() => mutate((s) => replaceBranch(s, reply)));
      })
      .finally(() => {
        if (branchLoad === load) branchLoad = undefined;
      })
      .then(() => stale ? reloadBranch() : undefined);
    branchLoad = load;
    return load;
  };
  /** `session_tree{leafId}`: reload unless this view already shows that leaf (our own jump did). */
  const syncBranch = (leaf: string | null) => {
    const check = () => {
      if (unwrap(state).leafId !== leaf) void reloadBranch().catch(() => {});
    };
    if (branchLoad) void branchLoad.then(check, check);
    else check();
  };
  const navigate = async (targetId: string, opts: NavigateOptions = {}): Promise<NavigateReply> => {
    const reply = await stream.cmd<NavigateReply | undefined>({ type: '_oino/navigate_tree', targetId, ...opts });
    if (!reply?.cancelled) await reloadBranch();
    return reply ?? {};
  };

  const fullMessages = createFullMessages({
    entry: (entryId) => {
      const at = state.byId[entryId];
      return at === undefined ? undefined : state.entries[at];
    },
    getEntries: (ids) => fetchFullEntriesSettled((command) => stream.cmd(command), ids),
    apply: (full) =>
      mutate((s) => {
        let changed = false;
        for (const e of full) {
          const at = s.byId[e.id];
          if (at === undefined) continue;
          s.entries[at] = e;
          changed = true;
        }
        if (changed) s.entriesRev += 1;
      }),
  });

  // Reference-only entries cannot mount a row. Observe the common insertion sink,
  // covering snapshot/replay, pages, branch replacement and live reconciliation.
  createEffect(() => {
    const ids = state.entries.filter((entry) => !entry.type && entry.truncated === true).map((entry) => entry.id);
    if (ids.length) queueMicrotask(() => fullMessages.ensure(ids));
  });

  const trimLoaded = (keep: number): boolean => {
    let dropped = false;
    // Not while provisional entries wait for their host echo (the reconcile reads `since` by index).
    if (state.history && !state.history.loading && state.entries.length > keep && !reconciling && !reconcileSince(state).needed) mutate((s) => (dropped = trimHistory(s, keep)));
    return dropped;
  };

  const refreshCommands = () => {
    stream
      .cmd<{ commands?: CommandInfo[] }>({ type: 'get_commands' })
      .then((reply) => {
        if (reply && Array.isArray(reply.commands)) { setState('commands', reply.commands); void refreshBoard(); }
      })
      .catch(() => {});
  };

  let refreshingBoard = false;
  const refreshBoard = async () => {
    if (refreshingBoard || !state.commands.some((c) => c.name === 'kanboard')) return;
    refreshingBoard = true;
    try {
      const scope = await stream.cmd<{ available: boolean }>({ type: '_oino/service_call', target: 'unipi.kanboard', name: 'kanboard.scope', payload: {} });
      if (typeof scope?.available !== 'boolean') return;
      // Keep typed commands executable; only hide their completion/menu rows.
      setState('commands', (commands) => commands.map((c) =>
        c.name === 'kanboard' || c.name === 'kanboard-add' || c.name === 'skill:kanboard'
          ? { ...c, surfaces: scope.available ? undefined : [] } : c));
      await stream.cmd({ type: '_oino/service_call', target: 'unipi.skill-registry', name: 'skills.refresh-board', payload: scope });
    } catch { /* Older hosts/extensions do not expose live scope refresh. */ }
    finally { refreshingBoard = false; }
  };

  // A record-only snapshot has no commands; waking fills them later. Observe
  // catalog identity, not surfaces (our own scope update must not loop).
  const boardCatalog = createMemo(() => state.commands.map((c) => c.name).join('\n'));
  createEffect(() => {
    boardCatalog();
    untrack(() => { queueMicrotask(() => void refreshBoard()); });
  });

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
      .catch((e: unknown) => {
        // [graph-mode] an older agent host lacks `_oino/topics`: say so instead of a silent "Plain".
        if (isUnknownOpError(toAppError(e).message)) mutate((s) => void (s.topicsUnsupported = true));
      });
  };

  const follow = (r: ApplyResult) => {
    if (r.nameChanged !== undefined) hubs().peek(hostId)?.patchCard(id(), { name: r.nameChanged || undefined });
    if (r.ui) for (const m of r.ui) uiSink(hostId, id(), m);
    if (r.reconcile) queueMicrotask(reconcile);
    if (r.commandsChanged) queueMicrotask(refreshCommands);
    if (r.catchUp) {
      const since = r.catchUp;
      queueMicrotask(() => catchUp(since));
    }
    if (r.reloadTail) {
      queueMicrotask(() => void reloadBranch().catch(() => {}));
    }
    if (r.branchMoved !== undefined) {
      const leaf = r.branchMoved;
      queueMicrotask(() => syncBranch(leaf));
    }
  };

  const stream: SessionStreamClient = new SessionStreamClient(
    client,
    sessionId,
    {
      seq: () => state.seq,
      mayPause: () => !state.approvals?.length && state.dialogs.length === 0,
      snapshot: (snap) => {
        let r: ApplyResult = {};
        batch(() => mutate((s) => (r = applySnapshot(s, snap))));
        follow(r);
        queueMicrotask(() => void refreshBoard());
        // Reference-only stubs have no row type to render. Hydrate these eagerly;
        // ordinary prose/tool truncation still uses visible-row lazy loading.
        queueMicrotask(() => fullMessages.ensure(state.entries.filter((entry) => !entry.type && entry.truncated === true).map((entry) => entry.id)));
        if (desiredTopics.size > 0) queueMicrotask(() => stream.live && sendTopicSub([...desiredTopics]));
        return r;
      },
      events: (items) => {
        let r: ApplyResult = {};
        const before = state.seq;
        batch(() => mutate((s) => (r = applyEvents(s, items))));
        if (items.some((item) => item.seq > before && (item.event as { kind?: string })?.kind === 'queue_restore_failed')) persist();
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
        // r8 P2-A: the shell's real reconnect path leaves a trace (the Chromium bridge has no supervisor to show it).
        if (conn !== state.conn) appLog('info', 'session-conn', `${id()}: ${state.conn} → ${conn}${error ? ` (${error.code})` : ''}`);
        batch(() => {
          setState('conn', conn);
          if (conn !== 'live') setState('approvals', []);
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
    turnsBuilds: () => builds,
    send(text, sendOpts = {}) {
      // D23: while an earlier prompt is still unacknowledged the run has not started (`run.running` is
      // false until agent_start): the message is a queued follow-up (its own turn), never a steer.
      const mode: OutboxMode = !sendOpts.mode || sendOpts.mode === 'auto' ? (state.run.running ? 'steer' : promptInFlight(state) ? 'followUp' : 'prompt') : sendOpts.mode;
      let item!: OutboxItem;
      mutate((s) => (item = { ...enqueue(s, { text, images: sendOpts.images, mode }, stream.live) }));
      if (sendOpts.onResult) sendResults.set(item.ref, sendOpts.onResult);
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
    // A Stop before the host took the prompt waits for it (D23): an abort sent first finds no run.
    abort: async () => {
      if (promptInFlight(state)) await sendTail;
      await stream.cmd({ type: 'abort' });
    },
    cmd: async <T = unknown>(cmd: RpcCommand): Promise<T> => {
      const reply = await stream.cmd<T>(cmd);
      if (cmd.type === 'set_autonomy') setState('info', 'autonomy', cmd.level);
      if (cmd.type === 'approval_response') setState('approvals', (list) => list?.filter((r) => r.requestId !== cmd.requestId));
      return reply;
    },
    refreshBoard,
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
    async refreshModel() {
      const info = await stream.cmd<Record<string, unknown>>({ type: 'get_state' });
      if (info && typeof info === 'object') mutate((s) => {
        s.model = info.model as SessionState['model'];
        if (typeof info.thinkingLevel === 'string') s.thinkingLevel = info.thinkingLevel;
      });
    },
    async getEntries(ids) {
      return fetchFullEntries((command) => stream.cmd(command), ids);
    },
    fullMessages,
    loadEarlier,
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
        // Nobody shows it any more: keep only the tail page's worth of parsed history.
        if (refs === 0) queueMicrotask(() => refs === 0 && trimLoaded(RELEASED_KEEP));
      };
    },
    trimLoaded,
    navigate,
    reloadBranch,
    get refs() {
      return refs;
    },
    get closed() {
      return closed;
    },
    close() {
      closed = true;
      fullMessages.dispose();
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
  // `closed` without an error too: the host ended the stream because the chat's runtime was
  // retired (the hub's `maxServices` eviction, an idle disposal, a host restart). The history
  // is safe in the file and only the runtime is gone, so a re-click, the window coming back or
  // the host reconnecting reopens it. A chat this app closed itself never reaches here — the
  // registry drops that handle.
  return state.conn === 'lost' || state.conn === 'closed';
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

let visibleInstalled = false;
/**
 * chat-vanish: the window comes back from the tray / a suspend — every chat that gave up
 * meanwhile (WebKit throttles hidden pages, so its reconnects ran out) tries again at once.
 */
function installVisibleRetry(retryAll: () => void): void {
  if (visibleInstalled || typeof document === 'undefined') return;
  visibleInstalled = true;
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') retryAll();
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

  /**
   * Least recently used first, never a pinned one — and never `keep`, the entry `ensure` is
   * about to hand out: with `limit` other chats pinned (status pane, model dialog, the chat
   * being left) the loop used to reach the new entry and close it before its view could
   * pin it, so every chat opened from then on was a dead, blank handle (chat-vanish).
   * The registry may then sit above `limit` by the pins; the next `ensure` trims it again.
   */
  const evict = (keep?: string) => {
    if (open.size <= limit) return;
    for (const [key, entry] of open) {
      if (open.size <= limit) break;
      if (key === keep || entry.handle.refs > 0) continue;
      entry.handle.close();
      entry.dispose();
      open.delete(key);
      setRev((n) => n + 1);
    }
  };

  return {
    /** The open session (created + subscribed when missing); marks it most recently used. */
    ensure(hostId: string, sessionId: string): SessionHandle {
      const key = keyOf(hostId, sessionId);
      const hit = open.get(key);
      if (hit && hit.handle.closed) {
        // Closed behind the registry's back: never hand a dead handle out again.
        hit.dispose();
        open.delete(key);
      } else if (hit) {
        open.delete(key);
        open.set(key, hit);
        evict(key);
        // Coming back to a chat that could not open tries again (never a blank chat forever).
        if (hit.handle.state && untrack(() => openFailed(hit.handle.state))) retryLater(hit.handle);
        return hit.handle;
      }
      const entry = createRoot((dispose) => {
        const handle = factory(hostId, sessionId) as Entry['handle'];
        // N2: Settings › MCP still needs this chat's level after the registry lets its handle go.
        createEffect(() => {
          if (handle.state?.info && autonomyKnown(handle)) rememberAutonomy(hostId, sessionId, autonomyLevel(handle));
        });
        return { handle, dispose };
      });
      open.set(key, entry);
      evict(key);
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
    /** chat-vanish: the host answers again — every chat of it that gave up (`lost`, a failed open) reconnects. */
    retryFailed(hostId?: string): number {
      let n = 0;
      for (const entry of open.values()) {
        if ((hostId !== undefined && entry.handle.hostId !== hostId) || entry.handle.closed || !entry.handle.state) continue;
        if (untrack(() => openFailed(entry.handle.state))) {
          n++;
          retryLater(entry.handle);
        }
      }
      return n;
    },
    /** Refresh only already-open chats; never allocate a new chat or project. */
    refreshBoards(hostId: string): void {
      for (const { handle } of open.values()) {
        if (handle.hostId === hostId && !handle.closed) void handle.refreshBoard?.();
      }
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
let offHostBack: (() => void) | undefined;
/** The app's session registry. */
export function sessions(): SessionRegistry {
  if (!shared) {
    shared = createRoot(() => createSessionRegistry());
    installReclickRetry({ peek: (h, id) => shared?.peek(h, id) });
    offHostBack = onHostBack((hostId) => shared?.retryFailed(hostId));
    installVisibleRetry(() => shared?.retryFailed());
  }
  return shared;
}
/** Tests: drop the shared registry (closing every session). */
export function resetSessions(): void {
  shared?.closeAll();
  shared = undefined;
  offHostBack?.();
  offHostBack = undefined;
  resetFusionOverrides();
}

export type { SetStoreFunction };
