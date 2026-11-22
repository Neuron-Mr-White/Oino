/**
 * Session stream client (app spec §3.4, WP-A2): one live host session stream
 * with resume.
 *
 * - `open` → `HostClient.openSession({kind:'open', session, sinceSeq})`; the
 *   first frame (snapshot / resume patch) is applied at once.
 * - `events` frames are queued and applied **once per animation frame** (one
 *   handler call per frame with every queued item; the store wraps it in one
 *   Solid `batch()`).
 * - A seq gap (`gap`) reopens with `sinceSeq`; `lagged` (`resync`) reopens
 *   without it (full snapshot). A dropped stream reconnects with `sinceSeq`
 *   (backoff 0.5/1/2/5 s then every 5 s, `lib/reconnect.ts`); after 2 min it
 *   is `lost` (banner "Lost the connection · Retry"). `taken_over` stops
 *   reconnecting (a reopen would take the session back).
 * - `cmd()` goes to the current stream (the seam correlates replies by id);
 *   30 s budget, 2 s for `_oino/render`; rejects with `E_TRANSPORT` while
 *   not live.
 * - w3-hidden: while the document is hidden a live stream with no command in
 *   flight (and no approval or dialog waiting) is **paused** — hung up locally, nothing is delivered, decoded or
 *   queued (the host keeps numbering the run) — and on `visibilitychange` →
 *   visible it reopens with `sinceSeq`: a resume patch (or a snapshot when the
 *   host's ring has rolled over) lands in ONE frame, the same path a dropped
 *   link takes. A stream that cannot pause (a command in flight) folds its
 *   queued events instead (`coalesce.ts`), so memory stays bounded either way.
 */
import { batch } from 'solid-js';
import { appError, toAppError, type AppError } from '@unipi/protocol';
import type { HostClient, RpcCommand, SessionEventDto, SessionSnapshot, SessionStream } from '../host/client';
import { Reconnect } from '../reconnect';
import type { ApplyResult } from './reducer';
import { pushCoalescing, type QueuedEvent } from './coalesce';
import type { ConnState } from './types';

/** app spec §3.4 step 6. */
export const SESSION_BACKOFF_MS = [500, 1_000, 2_000, 5_000] as const;
export const SESSION_STEADY_MS = 5_000;
export const SESSION_GIVE_UP_MS = 120_000;
/**
 * The LOCAL host is on this machine: a refused connection costs nothing and the supervisor
 * restarts it within seconds, so poll flat every 500 ms (r5 D17: the Interrupted row was
 * 4.5 s late behind the 2/5 s steps). Remote/paired hosts keep the exponential backoff.
 */
export const LOCAL_BACKOFF_MS = [500] as const;
export const LOCAL_STEADY_MS = 500;
/** Local rejected-open retries (host up, runtime still starting): 1 s at most. */
export const LOCAL_OPEN_RETRY_MS = [500, 500, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000, 1_000] as const;
/**
 * An open the host *rejects* (not a dropped link): a session whose file is not
 * written yet, a runtime still starting, a host restarting — retried on its
 * own backoff before the chat shows "Couldn't open this chat · Retry".
 */
export const OPEN_RETRY_MS = [600, 1_500, 3_000, 6_000] as const;
/**
 * Rejections that no retry can fix (the session or the protocol is wrong). `E_NOT_FOUND`:
 * the host has no such session (deleted, moved away, parked) — retrying only costs the
 * host a lookup each time, so the chat forgets it instead (Workspace).
 */
const FINAL_OPEN_CODES = new Set(['E_SCHEMA', 'E_UNSUPPORTED', 'E_UNAUTHORIZED', 'E_FORBIDDEN', 'E_TAKEN_OVER', 'E_NOT_FOUND']);
/** app spec §3.2 command budgets. */
export const CMD_TIMEOUT_MS = 30_000;
export const RENDER_TIMEOUT_MS = 2_000;
/** [chat-tree] A tree jump with a branch summary waits for a model call (host `NAVIGATE_TIMEOUT`). */
export const NAVIGATE_TIMEOUT_MS = 300_000;

export type StreamHandlers = {
  /** The last applied seq (the resume point). */
  seq(): number;
  /** The first frame; applied synchronously. */
  snapshot(snapshot: SessionSnapshot): ApplyResult;
  /** One frame's worth of events, in order. */
  events(items: SessionEventDto[]): ApplyResult;
  conn(state: ConnState, error?: AppError): void;
  /**
   * w3-hidden: false keeps the stream open while the window is hidden. A chat that waits for an
   * answer (approval, dialog) stays connected so its response can be delivered immediately.
   */
  mayPause?(): boolean;
  /** `session_replaced`: the stream now serves another id. */
  replaced?(sessionId: string): void;
};

export type StreamOptions = {
  /** Frame scheduler (default `requestAnimationFrame`, `setTimeout(16)` without a DOM). */
  schedule?: (flush: () => void) => void;
  setTimer?: (ms: number, cb: () => void) => unknown;
  clearTimer?: (handle: unknown) => void;
  now?: () => number;
};

/**
 * perf-webview (spec §9 habit 4): ONE frame callback for every open stream. Ten chats
 * streaming apply their queued deltas in the same task, inside one Solid `batch()`, so the
 * visible chat re-renders once per frame however many chats got deltas.
 */
let frameQueue: Array<() => void> = [];
const runFrame = () => {
  const due = frameQueue;
  frameQueue = [];
  batch(() => {
    for (const flush of due) flush();
  });
};
const defaultSchedule = (flush: () => void) => {
  frameQueue.push(flush);
  if (frameQueue.length > 1) return;
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(runFrame);
  else setTimeout(runFrame, 16);
};

const transportError = (message: string): AppError => appError('E_TRANSPORT', message);

const isHidden = (): boolean => typeof document !== 'undefined' && document.hidden === true;

/** Every client that can pause; one `visibilitychange` listener serves them all. */
const pausable = new Set<SessionStreamClient>();
let visibilityInstalled = false;
function installVisibility(): void {
  if (visibilityInstalled || typeof document === 'undefined') return;
  visibilityInstalled = true;
  document.addEventListener('visibilitychange', () => {
    const hidden = isHidden();
    for (const client of [...pausable]) {
      if (hidden) client.pause();
      else client.resume();
    }
  });
}

/** A stream end worth reconnecting after (transport trouble, a clean host-side close). */
export function shouldReconnect(error: AppError | undefined): boolean {
  if (!error) return true;
  return error.retryable === true || error.code === 'E_TRANSPORT' || error.code === 'E_TIMEOUT';
}

export class SessionStreamClient {
  private stream?: SessionStream;
  private gen = 0;
  private queue: QueuedEvent[] = [];
  private scheduled = false;
  private closedByUs = false;
  private stopped = false;
  private isLive = false;
  /** Rejected opens in a row (reset once a snapshot arrives). */
  private openFailures = 0;
  /** A snapshot arrived on this client: the chat has been live, so a later failed open is a lost link. */
  private everLive = false;
  private openTimer: unknown;
  /** Hung up while the window is hidden (see the header); `resume()` reopens. */
  private paused = false;
  /** Commands sent and not answered yet: a stream with one in flight does not pause. */
  private inflight = 0;
  /** Paused and not live again yet: `cmd()` waits instead of failing (released by `releaseWaiters`). */
  private awaitingLive = false;
  private liveWaiters: Array<() => void> = [];
  private readonly reconnect: Reconnect;
  private readonly openRetryMs: readonly number[];
  private readonly schedule: (flush: () => void) => void;

  constructor(
    private readonly client: HostClient,
    private sessionId: string,
    private readonly handlers: StreamHandlers,
    private readonly opts: StreamOptions = {},
  ) {
    this.schedule = opts.schedule ?? defaultSchedule;
    pausable.add(this);
    installVisibility();
    const local = client.hostId === 'local';
    this.openRetryMs = local ? LOCAL_OPEN_RETRY_MS : OPEN_RETRY_MS;
    this.reconnect = new Reconnect({
      backoffMs: local ? LOCAL_BACKOFF_MS : SESSION_BACKOFF_MS,
      steadyMs: local ? LOCAL_STEADY_MS : SESSION_STEADY_MS,
      giveUpMs: SESSION_GIVE_UP_MS,
      setTimer: opts.setTimer,
      clearTimer: opts.clearTimer,
      now: opts.now,
      onRetry: () => this.connect(true),
      onState: (state) => {
        if (state.kind === 'reconnecting') {
          this.handlers.conn('reconnecting');
          this.reconnect.armTimer();
        } else if (state.kind === 'ended') {
          this.releaseWaiters();
          this.handlers.conn(state.reason === 'unreachable' ? 'lost' : 'closed', state.error);
        }
      },
    });
  }

  get id(): string {
    return this.sessionId;
  }

  get live(): boolean {
    return this.isLive;
  }

  /** Opens the stream (resuming from `handlers.seq()` when it is > 0). */
  open(): void {
    this.closedByUs = false;
    this.stopped = false;
    pausable.add(this);
    this.reconnect.beginConnecting();
    this.handlers.conn('connecting');
    this.connect(true);
  }

  /** Reopens now: `resume` keeps `sinceSeq` (gap), else a full snapshot (resync / replaced). */
  reopen(resume: boolean): void {
    if (this.closedByUs) return;
    this.connect(resume);
  }

  /** The banner's Retry: reconnect immediately, whatever the backoff says. */
  retryNow(): void {
    if (this.closedByUs) return;
    this.stopped = false;
    this.openFailures = 0;
    this.clearOpenTimer();
    this.reconnect.beginConnecting();
    this.handlers.conn('connecting');
    this.connect(true);
  }

  private connect(resume: boolean): void {
    this.detach();
    const gen = ++this.gen;
    this.paused = false;
    const since = resume ? this.handlers.seq() : 0;
    /** This attempt delivered its first frame. */
    let opened = false;
    this.stream = this.client.openSession(
      { kind: 'open', session: this.sessionId, ...(since > 0 ? { sinceSeq: since } : {}) },
      {
        snapshot: (snap) => {
          if (gen !== this.gen) return;
          opened = true;
          this.everLive = true;
          // Live before the snapshot reaches the store: the reducer sets `state.conn = 'live'`
          // inside `handlers.snapshot`, and a consumer reacting to that (the Session modal tabs
          // fetch on `conn === 'live'`) must not send a command the stream still rejects.
          this.isLive = true;
          this.openFailures = 0;
          this.reconnect.live();
          const r = this.handlers.snapshot(snap);
          this.handlers.conn('live');
          this.follow(r);
          if (gen === this.gen) {
            this.releaseWaiters();
            // Opened (or reconnected) while the window is hidden: do not sit on a stream nobody renders.
            if (isHidden()) this.pause();
          }
        },
        events: (items) => {
          if (gen !== this.gen || !items.length) return;
          // A hidden page gets no animation frame: fold instead of growing by one entry per delta.
          if (isHidden()) for (const item of items) pushCoalescing(this.queue, item);
          else for (const item of items) this.queue.push(item);
          if (!this.scheduled) {
            this.scheduled = true;
            this.schedule(() => this.flush(gen));
          }
        },
        resync: () => {
          // fix-frames: a skipped oversize/unreadable frame — reload from a full snapshot
          // instead of sitting on a dead banner while the turn keeps running.
          if (gen !== this.gen || this.closedByUs || this.stopped) return;
          this.connect(false);
        },
        closed: (error) => {
          if (gen !== this.gen) return;
          this.isLive = false;
          this.stream = undefined;
          if (this.closedByUs || this.stopped) return;
          const app = error ? toAppError(error) : undefined;
          if (!shouldReconnect(app)) {
            // r7 P2-3: a chat that was live and whose host is still coming back (a refused or
            // rejected open) stays on the reconnect machine — "Lost the connection" until a real
            // snapshot lands — not on the open-retry path, whose `connecting` hid the banner.
            if (!opened && this.everLive && app && !FINAL_OPEN_CODES.has(app.code)) {
              this.reconnect.dropped({ kind: 'closed', error: { ...app, retryable: true } });
              return;
            }
            // Rejected before it ever went live: retry the open on its own backoff.
            if (!opened && app && !FINAL_OPEN_CODES.has(app.code) && this.openFailures < this.openRetryMs.length) {
              const wait = this.openRetryMs[this.openFailures++]!;
              this.handlers.conn('connecting');
              this.clearOpenTimer();
              const setTimer = this.opts.setTimer ?? ((ms: number, cb: () => void) => setTimeout(cb, ms));
              this.openTimer = setTimer(wait, () => {
                this.openTimer = undefined;
                if (!this.closedByUs && !this.stopped) this.connect(resume);
              });
              return;
            }
            // Ended for this attempt, not for good: the state machine stays usable, so
            // Retry (or a re-click / revisit) reconnects and later drops still auto-reconnect.
            this.releaseWaiters();
            this.handlers.conn('closed', app);
            return;
          }
          this.reconnect.dropped({ kind: 'closed', error: app ? { ...app, retryable: true } : undefined });
        },
      },
    );
  }

  /** Applies everything queued since the last frame (also called by tests to flush synchronously). */
  flush(gen = this.gen): void {
    if (gen !== this.gen) return;
    this.scheduled = false;
    if (!this.queue.length) return;
    const items = this.queue;
    this.queue = [];
    this.follow(this.handlers.events(items));
  }

  private follow(r: ApplyResult): void {
    if (r.takenOver) {
      // The loser keeps its state read-only with a "Take back" banner; never auto-reopen.
      this.stopped = true;
      this.isLive = false;
      this.detach();
      this.releaseWaiters();
      this.handlers.conn('closed');
      return;
    }
    if (r.replacedBy && r.replacedBy !== this.sessionId) {
      this.sessionId = r.replacedBy;
      this.handlers.replaced?.(r.replacedBy);
      this.connect(false);
      return;
    }
    if (r.resync) this.connect(false);
    else if (r.gap) this.connect(true);
  }

  /** Hides the stream while the window is hidden (no-op unless live with nothing in flight). */
  pause(): void {
    if (this.paused || !this.isLive || !this.stream || this.inflight > 0 || this.closedByUs || this.stopped) return;
    if (this.handlers.mayPause && !this.handlers.mayPause()) return;
    this.paused = true;
    this.awaitingLive = true;
    // The unflushed queue goes with it: the resume replays from `handlers.seq()` (the last APPLIED seq).
    this.detach();
  }

  /** The window is visible again: reopen a paused stream from the last applied seq. */
  resume(): void {
    if (!this.paused) return;
    this.paused = false;
    if (this.closedByUs || this.stopped) return this.releaseWaiters();
    this.connect(true);
  }

  private releaseWaiters(): void {
    this.awaitingLive = false;
    for (const release of this.liveWaiters.splice(0)) release();
  }

  /** Sends one RPC command on the live stream. */
  cmd<T = unknown>(cmd: RpcCommand): Promise<T> {
    // Paused or reopening: wait for the stream (a pause is invisible to callers) instead of failing.
    if (this.awaitingLive) {
      return new Promise<void>((release) => this.liveWaiters.push(release)).then(() => this.cmd<T>(cmd));
    }
    const stream = this.stream;
    if (!stream || !this.isLive) return Promise.reject(transportError('The session is not connected'));
    const limit = cmd.type === '_oino/render' ? RENDER_TIMEOUT_MS : cmd.type === '_oino/navigate_tree' ? NAVIGATE_TIMEOUT_MS : CMD_TIMEOUT_MS;
    const setTimer = this.opts.setTimer ?? ((ms: number, cb: () => void) => setTimeout(cb, ms));
    const clearTimer = this.opts.clearTimer ?? ((h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>));
    this.inflight++;
    return new Promise<T>((resolve, reject) => {
      let done = false;
      const timer = setTimer(limit, () => {
        if (done) return;
        done = true;
        this.inflight--;
        reject({ code: 'E_TIMEOUT', message: 'The session did not answer in time', retryable: true });
      });
      stream.cmd<T>(cmd).then(
        (value) => {
          if (done) return;
          done = true;
          this.inflight--;
          clearTimer(timer);
          resolve(value);
        },
        (error: unknown) => {
          if (done) return;
          done = true;
          this.inflight--;
          clearTimer(timer);
          reject(toAppError(error));
        },
      );
    });
  }

  private detach(): void {
    const old = this.stream;
    this.stream = undefined;
    this.isLive = false;
    this.queue = [];
    this.scheduled = false;
    this.gen++;
    old?.close();
  }

  private clearOpenTimer(): void {
    if (this.openTimer === undefined) return;
    const clearTimer = this.opts.clearTimer ?? ((h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>));
    clearTimer(this.openTimer);
    this.openTimer = undefined;
  }

  /** The stream gave up for good (`closed` with an error): what a re-click / revisit retries. */
  get failed(): boolean {
    return !this.closedByUs && !this.stopped && !this.isLive && !this.stream && this.openTimer === undefined;
  }

  /** Local hang-up (the session keeps running on the host). */
  close(): void {
    this.closedByUs = true;
    pausable.delete(this);
    this.releaseWaiters();
    this.clearOpenTimer();
    this.reconnect.dispose();
    this.detach();
    this.handlers.conn('closed');
  }
}
