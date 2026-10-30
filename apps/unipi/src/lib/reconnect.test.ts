import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Reconnect, BACKOFF_MS, GIVE_UP_MS } from './reconnect';
import type { AppError } from '@unipi/protocol';
import type { TermEvent } from './term';

const closedRetryable = { type: 'closed' as const };
const closedFatal = {
  type: 'closed' as const,
  error: { code: 'E_PAIRING_INVALID', message: 'bad', retryable: false } satisfies AppError,
};
const exit = { type: 'exit' as const, code: 0 };


let timers: { delay: number; fire: () => void }[] = [];
function fakeTimers() {
  timers = [];
  return {
    setTimer: (delayMs: number, callback: () => void) => {
      const handle = {
        delay: delayMs,
        fire: () => callback(),
      };
      timers.push(handle);
      return handle;
    },
    clearTimer: (handle: unknown) => {
      timers = timers.filter((t) => t !== handle);
    },
  };
}

function lastDelay() {
  return timers.at(-1)?.delay;
}

beforeEach(() => {
  vi.useFakeTimers();
  Date.now = () => 1_000_000;
});
afterEach(() => {
  vi.useRealTimers();
});

it('walks the backoff sequence while reconnecting', () => {
  const { setTimer, clearTimer } = fakeTimers();
  let retries = 0;
  const machine = new Reconnect({ onRetry: () => retries++, setTimer, clearTimer });
  machine.beginConnecting();
  machine.live();
  machine.dropped(closedRetryable);
  // Each retry fails (the stream drops again) without reaching live.
  for (const delay of [...BACKOFF_MS, 15_000, 15_000]) {
    machine.armTimer();
    expect(lastDelay()).toBe(delay);
    machine.timerFired();
    expect(machine.state.kind).toBe('connecting');
    machine.dropped(closedRetryable);
    retries += 0; // onRetry was already counted by timerFired
  }
  expect(retries).toBe(BACKOFF_MS.length + 2);
});

it('gives up after 2 minutes as unreachable', () => {
  const { setTimer, clearTimer } = fakeTimers();
  let now = 0;
  const machine = new Reconnect({
    onRetry: () => {},
    setTimer,
    clearTimer,
    now: () => now,
  });
  machine.beginConnecting();
  machine.live();
  machine.dropped(closedRetryable);
  now = GIVE_UP_MS + 1;
  machine.armTimer();
  machine.timerFired();
  expect(machine.state).toEqual({ kind: 'ended', reason: 'unreachable' });
});

it('exit ends without reconnecting', () => {
  const machine = new Reconnect({ onRetry: () => {} });
  machine.beginConnecting();
  machine.live();
  machine.dropped(exit);
  expect(machine.state).toEqual({ kind: 'ended', reason: 'exited' });
});

it('non-retryable closed ends with the error', () => {
  const machine = new Reconnect({ onRetry: () => {} });
  machine.beginConnecting();
  machine.live();
  machine.dropped(closedFatal);
  const state = machine.state;
  expect(state.kind).toBe('ended');
  if (state.kind === 'ended' && state.reason === 'error') {
    expect(state.error).toEqual(closedFatal.error);
  }
});

it('visible retry fires immediately while reconnecting', () => {
  const { setTimer, clearTimer } = fakeTimers();
  let retries = 0;
  const machine = new Reconnect({ onRetry: () => retries++, setTimer, clearTimer });
  machine.beginConnecting();
  machine.live();
  machine.dropped(closedRetryable);
  machine.armTimer();
  machine.fireRetry();
  expect(retries).toBe(1);
  expect(machine.state.kind).toBe('connecting');
  expect(timers).toHaveLength(0);
});

it('onState fires for each transition', () => {
  const { setTimer, clearTimer } = fakeTimers();
  const states: string[] = [];
  const machine = new Reconnect({
    onRetry: () => {},
    onState: (state) => states.push(state.kind),
    setTimer,
    clearTimer,
  });
  machine.beginConnecting();
  machine.live();
  machine.dropped(closedRetryable);
  machine.armTimer();
  machine.timerFired();
  machine.live();
  machine.dropped(exit);
  expect(states).toEqual([
    'connecting',
    'live',
    'reconnecting',
    'connecting',
    'live',
    'ended',
  ]);
});
