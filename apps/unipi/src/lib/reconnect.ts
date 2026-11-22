/**
 * Reconnect state machine (DESIGN §3.3): idle → connecting → live →
 * reconnecting(attempt, nextDelayMs) → live | ended(reason).
 *
 * Pure: the machine never touches timers or the network itself. The
 * owner watches `state` and (a) when it enters `reconnecting`, arms a
 * timer for `nextDelayMs` and calls `fireRetry()`; (b) on
 * `visibilitychange` → visible while reconnecting, calls `fireRetry()`
 * (immediate retry). Backoff 500 ms → 1 s → 2 s → 4 s → 8 s, then
 * every 15 s; gives up 2 minutes after the drop with
 * `ended("unreachable")`.
 */
import type { AppError } from '@unipi/protocol';
import type { TermEvent } from './term';

export const BACKOFF_MS = [500, 1_000, 2_000, 4_000, 8_000] as const;
export const STEADY_MS = 15_000;
export const GIVE_UP_MS = 120_000;

export type ReconnectState =
  | { kind: 'idle' }
  | { kind: 'connecting' }
  | { kind: 'live' }
  | { kind: 'reconnecting'; attempt: number; nextDelayMs: number }
  | { kind: 'ended'; reason: 'exited' | 'unreachable' | 'error'; error?: AppError };

export type ReconnectOpts = {
  /** Invoked when the machine decides a retry should start now. */
  onRetry: () => void;
  /** Invoked on every state transition. */
  onState?: (state: ReconnectState) => void;
  /** Timer injection for tests; defaults to the globals. */
  setTimer?: (delayMs: number, callback: () => void) => unknown;
  clearTimer?: (handle: unknown) => void;
  now?: () => number;
  /** Backoff table override (session streams: 0.5/1/2/5 s, app spec §3.4). */
  backoffMs?: readonly number[];
  /** Delay after the table (default `STEADY_MS`). */
  steadyMs?: number;
  /** Give up this long after the drop (default `GIVE_UP_MS`). */
  giveUpMs?: number;
};

export class Reconnect {
  private currentState: ReconnectState = { kind: 'idle' };
  private droppedAt = 0;
  private attempt = 0;
  private timer: unknown;
  private readonly opts: ReconnectOpts;

  constructor(opts: ReconnectOpts) {
    this.opts = opts;
  }

  get state(): ReconnectState {
    return this.currentState;
  }

  private disposed = false;

  /** The owner went away (connection closed): cancel any pending
   * retry and ignore everything after this. */
  dispose(): void {
    this.disposed = true;
    this.clearPending();
  }

  private set(state: ReconnectState) {
    if (this.disposed) return;
    this.currentState = state;
    this.opts.onState?.(state);
  }

  beginConnecting() {
    this.clearPending();
    this.set({ kind: 'connecting' });
  }

  live() {
    this.clearPending();
    this.droppedAt = 0;
    this.attempt = 0;
    this.set({ kind: 'live' });
  }

  /** The stream ended. `exit` ends for good; `closed` reconnects
   * unless the error is non-retryable. */
  dropped(event: TermEvent | { kind: 'closed'; error?: AppError }): void {
    if ('type' in event && event.type === 'exit') {
      this.clearPending();
      this.set({ kind: 'ended', reason: 'exited' });
      return;
    }
    const error =
      'type' in event && event.type === 'closed' && event.error ? event.error : undefined;
    if (error && !error.retryable) {
      this.clearPending();
      this.set({ kind: 'ended', reason: 'error', error });
      return;
    }
    this.startReconnecting();
  }

  /** Immediate retry (e.g. visibility back to visible). Honours the
   * 2-minute give-up. */
  fireRetry() {
    if (this.disposed || this.currentState.kind !== 'reconnecting') return;
    const now = (this.opts.now ?? Date.now)();
    if (now - this.droppedAt >= (this.opts.giveUpMs ?? GIVE_UP_MS)) {
      this.clearPending();
      this.set({ kind: 'ended', reason: 'unreachable' });
      return;
    }
    this.clearPending();
    this.beginConnecting();
    this.opts.onRetry();
  }

  /** The owner's timer fired. */
  timerFired() {
    this.fireRetry();
  }

  private startReconnecting() {
    if (this.droppedAt === 0) {
      this.droppedAt = (this.opts.now ?? Date.now)();
    }
    this.attempt += 1;
    const elapsed = (this.opts.now ?? Date.now)() - this.droppedAt;
    if (elapsed >= (this.opts.giveUpMs ?? GIVE_UP_MS)) {
      this.clearPending();
      this.set({ kind: 'ended', reason: 'unreachable' });
      return;
    }
    const table = this.opts.backoffMs ?? BACKOFF_MS;
    const nextDelayMs =
      this.attempt <= table.length ? table[this.attempt - 1]! : (this.opts.steadyMs ?? STEADY_MS);
    this.set({ kind: 'reconnecting', attempt: this.attempt, nextDelayMs });
  }

  /** Called by the owner when its retry timer fires. */
  onRetryTimer() {
    if (this.disposed || this.currentState.kind !== 'reconnecting') return;
    const now = (this.opts.now ?? Date.now)();
    if (now - this.droppedAt >= (this.opts.giveUpMs ?? GIVE_UP_MS)) {
      this.clearPending();
      this.set({ kind: 'ended', reason: 'unreachable' });
      return;
    }
    this.beginConnecting();
    this.opts.onRetry();
  }

  /** Arm the owner's timer for the current reconnecting state. */
  armTimer(): void {
    if (this.disposed || this.currentState.kind !== 'reconnecting') return;
    this.clearPending();
    const delay = this.currentState.nextDelayMs;
    const setTimer =
      this.opts.setTimer ?? ((ms: number, cb: () => void) => setTimeout(cb, ms));
    this.timer = setTimer(delay, () => this.onRetryTimer());
  }

  private clearPending() {
    if (this.timer !== undefined) {
      (this.opts.clearTimer ?? ((h: unknown) => clearTimeout(h as ReturnType<typeof setTimeout>)))(this.timer);
      this.timer = undefined;
    }
  }
}
