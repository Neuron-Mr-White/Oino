import { describe, expect, it } from 'vitest';
import { SessionStreamClient, SESSION_BACKOFF_MS } from './stream';
import type { HostClient } from '../host/client';

/** A host whose every open fails with a dropped link (host down). */
function downHost(hostId: string) {
  let opens = 0;
  const client = {
    hostId,
    openSession: (_req: unknown, sink: { closed: (e?: unknown) => void }) => {
      opens += 1;
      queueMicrotask(() => sink.closed({ code: 'E_TRANSPORT', message: 'refused', retryable: true }));
      return { cmd: async () => undefined, close: () => {} };
    },
  } as unknown as HostClient;
  return { client, opens: () => opens };
}

function run(hostId: string) {
  const host = downHost(hostId);
  const delays: number[] = [];
  const stream = new SessionStreamClient(
    host.client,
    's1',
    { seq: () => 0, snapshot: () => ({}), events: () => ({}), conn: () => {} },
    {
      setTimer: (ms, cb) => {
        delays.push(ms);
        // Run the retry once per tick, up to eight rounds.
        if (delays.length < 8) queueMicrotask(cb);
        return delays.length;
      },
      clearTimer: () => {},
    },
  );
  stream.open();
  return new Promise<number[]>((resolve) => setTimeout(() => resolve(delays), 20));
}

describe('session stream reconnect schedule (r5 D17)', () => {
  it('the local host is polled every 500 ms, never slower', async () => {
    const delays = await run('local');
    expect(delays.length).toBeGreaterThan(4);
    expect(delays.every((d) => d === 500)).toBe(true);
  });

  it('a paired host keeps the exponential backoff', async () => {
    const delays = await run('pc-2');
    expect(delays.slice(0, 4)).toEqual([...SESSION_BACKOFF_MS]);
    expect(delays.at(-1)).toBe(5_000);
  });
});
